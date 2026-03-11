//! Tool call dispatch: routes to HTTP execution or plugin execution.
use std::{any::Any, panic::AssertUnwindSafe, sync::Arc};

use futures_util::FutureExt;
use rmcp::{
    model::CallToolResult,
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde_json::Value;

use crate::{
    config::{ExecuteType, ToolConfig},
    errors::cancelled_tool_result,
    plugins::{PluginCallParams, PluginContext, PluginType},
};

#[cfg(feature = "http_tools")]
use crate::config::{
    ResponseConfig, ResponseContentConfig, ResponseStructuredConfig, UpstreamAuth,
};
use crate::errors::tool_execution_error_result;
#[cfg(feature = "http_tools")]
use secrecy::ExposeSecret;
#[cfg(feature = "http_tools")]
use tokio_util::sync::CancellationToken;

#[cfg(feature = "http_tools")]
use super::super::http_executor::{execute_http_detailed, HttpExecutionError, HttpRequestTemplate};
#[cfg(feature = "http_tools")]
use super::super::templating::{render_value, RenderContext};
use super::super::tool_response::validate_structured_output_with_cache;
#[cfg(feature = "http_tools")]
use super::super::tool_response::{build_content_result, build_structured_result_with_cache};
use super::{helpers::lookup_tool_plugin, Engine};

#[cfg(feature = "http_tools")]
struct Oauth2ExecutionContext<'a> {
    auth: &'a crate::config::UpstreamOauth2AuthConfig,
    bearer: secrecy::SecretString,
    retry_on_401_once: bool,
}

#[cfg(feature = "http_tools")]
#[derive(Clone, Copy)]
struct HttpRequestExecutionParams<'a> {
    tool_cfg: &'a ToolConfig,
    upstream_base_url: &'a str,
    args: &'a Value,
    request: Option<&'a RequestContext<RoleServer>>,
}

#[cfg(feature = "http_tools")]
#[derive(Clone, Copy)]
struct HttpRetryExecutionParams<'a> {
    retry: Option<&'a crate::config::OutboundRetryConfig>,
    cancellation: Option<&'a CancellationToken>,
    oauth401_refresh_enabled: bool,
}

#[cfg(feature = "http_tools")]
#[derive(Clone, Copy)]
struct HttpDispatchParams<'a> {
    upstream_name: &'a str,
    upstream: &'a crate::config::UpstreamConfig,
    request: HttpRequestExecutionParams<'a>,
    retry: HttpRetryExecutionParams<'a>,
}

impl Engine {
    pub async fn execute_tool(&self, name: &str, args: Value) -> Result<CallToolResult, McpError> {
        self.execute_tool_with_context(name, args, None).await
    }

    pub async fn execute_tool_with_context(
        &self,
        name: &str,
        args: Value,
        request: Option<RequestContext<RoleServer>>,
    ) -> Result<CallToolResult, McpError> {
        let tool_cfg = self
            .tool_map
            .get(name)
            .ok_or_else(|| McpError::invalid_params(format!("tool not found: {name}"), None))?;
        let result = AssertUnwindSafe(self.execute_tool_config(tool_cfg, args, request))
            .catch_unwind()
            .await
            .unwrap_or_else(|panic_payload| {
                let error = map_tool_panic_to_error(panic_payload);
                Ok(self.tool_error_result_from_error(&error))
            });
        match result {
            Ok(result) => {
                super::response_limits::enforce_response_limits(
                    &result,
                    self.config.response_limits(),
                )
                .map_err(|error| self.sanitize_client_error(error))?;
                Ok(result)
            }
            Err(error) => Err(self.sanitize_client_error(error)),
        }
    }

    async fn execute_tool_config(
        &self,
        tool_cfg: &ToolConfig,
        args: Value,
        request: Option<RequestContext<RoleServer>>,
    ) -> Result<CallToolResult, McpError> {
        match tool_cfg.execute.execute_type() {
            ExecuteType::Http => {
                #[cfg(feature = "http_tools")]
                {
                    self.execute_http_tool(tool_cfg, args, request).await
                }
                #[cfg(not(feature = "http_tools"))]
                {
                    let _ = (tool_cfg, args, request);
                    Err(McpError::invalid_request(
                        "http_tools feature disabled but execute.type=http is not available"
                            .to_owned(),
                        None,
                    ))
                }
            }
            ExecuteType::Plugin => self.execute_plugin_tool(tool_cfg, args, request).await,
        }
    }

    fn tool_error_result_from_error(&self, error: &McpError) -> CallToolResult {
        tool_execution_error_result(self.sanitize_tool_error_message(error))
    }

    // Assembles a PluginContext with all engine-level state (upstreams, HTTP client,
    // logging, progress, list refresh, client features) so plugins don't need
    // direct access to Engine internals.
    pub(crate) fn build_plugin_context(
        &self,
        request: Option<RequestContext<RoleServer>>,
    ) -> PluginContext {
        let mut ctx = PluginContext::new(
            request,
            Arc::clone(&self.upstreams),
            Arc::clone(&self.http_client),
        )
        .with_outbound_http(self.config.outbound_http.clone())
        .with_client_logging_state(self.client_logging.clone())
        .with_server_log_payload_max_bytes(self.config.log_payload_max_bytes())
        .with_progress_state(self.progress_state.clone());
        if let Some(handle) = self.list_refresh_handle.as_ref() {
            ctx = ctx.with_list_refresh_handle(Arc::clone(handle));
        }
        #[cfg(feature = "http_tools")]
        {
            ctx = ctx.with_outbound_token_manager(self.outbound_token_manager.clone());
        }
        #[cfg(feature = "client_features")]
        {
            ctx = ctx.with_client_features(self.config.client_features.clone());
        }
        ctx
    }

    // Looks up the named plugin, merges tool-level + global plugin config,
    // runs the plugin with optional cancellation support, then validates
    // structured output against the tool's output_schema if present.
    // Also backfills a text content block when the plugin returns structured
    // content but no content array (clients need at least one content item).
    async fn execute_plugin_tool(
        &self,
        tool_cfg: &ToolConfig,
        args: Value,
        request: Option<RequestContext<RoleServer>>,
    ) -> Result<CallToolResult, McpError> {
        let plugin_execute = tool_cfg.execute.as_plugin().ok_or_else(|| {
            McpError::invalid_request("plugin tool requires execute.type=plugin".to_owned(), None)
        })?;
        let plugin_name = &plugin_execute.plugin;
        if plugin_name.is_empty() {
            return Err(McpError::invalid_request(
                "plugin tool requires plugin".to_owned(),
                None,
            ));
        }

        let plugin = lookup_tool_plugin(&self.plugins, plugin_name)?;

        let mut ctx = self.build_plugin_context(request);
        if !tool_cfg.cancellable {
            ctx.cancellation = tokio_util::sync::CancellationToken::new();
        }

        let config = crate::engine::value_helpers::merge_plugin_config(
            &self.config,
            PluginType::Tool,
            plugin_name,
            plugin_execute.config.as_ref(),
        );
        let cancellation = ctx.cancellation.clone();

        let mut result = if tool_cfg.cancellable {
            tokio::select! {
                result = plugin.call(args, PluginCallParams { config, ctx }) => match result {
                    Ok(result) => result,
                    Err(error) => return Ok(self.tool_error_result_from_error(&error)),
                },
                () = cancellation.cancelled() => return Ok(cancelled_tool_result()),
            }
        } else {
            match plugin.call(args, PluginCallParams { config, ctx }).await {
                Ok(result) => result,
                Err(error) => return Ok(self.tool_error_result_from_error(&error)),
            }
        };

        let Some(structured) = result.structured_content.as_ref() else {
            return Ok(result);
        };
        if let Some(schema) = tool_cfg.output_schema.as_ref() {
            if let Err(error) = validate_structured_output_with_cache(
                structured,
                schema,
                &self.schema_validator_cache,
            ) {
                return Ok(self.tool_error_result_from_error(&error));
            }
        }
        if result.content.is_empty() {
            result
                .content
                .push(rmcp::model::Content::text(structured.to_string()));
        }

        Ok(result)
    }

    // Resolves the upstream, builds an HttpRequestTemplate from tool config
    // (method, path, query, headers, body templates + timeout/size limits),
    // executes the HTTP call with optional cancellation, then maps the
    // response to either content blocks or structured output depending on
    // response.type config.
    #[cfg(feature = "http_tools")]
    async fn execute_http_tool(
        &self,
        tool_cfg: &ToolConfig,
        args: Value,
        request: Option<RequestContext<RoleServer>>,
    ) -> Result<CallToolResult, McpError> {
        let execute_http = tool_cfg.execute.as_http().ok_or_else(|| {
            McpError::invalid_request("http tool requires execute.type=http".to_owned(), None)
        })?;
        let upstream_name = execute_http.upstream.as_str();
        let upstream = self.config.upstreams.get(upstream_name).ok_or_else(|| {
            McpError::invalid_request(format!("unknown upstream: {upstream_name}"), None)
        })?;
        let oauth2_context = match self.resolve_oauth2_context(upstream_name, upstream).await {
            Ok(context) => context,
            Err(error) => return Ok(self.tool_error_result_from_error(&error)),
        };
        let template = self.build_http_request_template(
            execute_http,
            upstream,
            oauth2_context
                .as_ref()
                .map(|context| context.bearer.expose_secret()),
        );
        let retry_config = crate::http::outbound_pipeline::resolve_retry_config(
            execute_http.retry.as_ref(),
            upstream,
            self.config.outbound_http.as_ref(),
        );
        let cancellation = tool_cfg
            .cancellable
            .then(|| request.as_ref().map(|ctx| ctx.ct.clone()))
            .flatten();
        let dispatch = HttpDispatchParams {
            upstream_name,
            upstream,
            request: HttpRequestExecutionParams {
                tool_cfg,
                upstream_base_url: &upstream.base_url,
                args: &args,
                request: request.as_ref(),
            },
            retry: HttpRetryExecutionParams {
                retry: retry_config,
                cancellation: cancellation.as_ref(),
                oauth401_refresh_enabled: oauth2_context
                    .as_ref()
                    .is_some_and(|context| context.retry_on_401_once),
            },
        };

        let response_result = self
            .execute_http_request_flow(dispatch, template, oauth2_context.as_ref())
            .await;

        let response = match response_result {
            Ok(response) => response,
            Err(error) => {
                if matches!(error, HttpExecutionError::Cancelled) {
                    return Ok(cancelled_tool_result());
                }
                let error = error.into_mcp_error();
                return Ok(self.tool_error_result_from_error(&error));
            }
        };

        build_http_tool_result(tool_cfg, &args, response, &self.schema_validator_cache)
    }

    #[cfg(feature = "http_tools")]
    async fn execute_http_request_flow(
        &self,
        dispatch: HttpDispatchParams<'_>,
        mut template: HttpRequestTemplate,
        oauth2_context: Option<&Oauth2ExecutionContext<'_>>,
    ) -> Result<Value, HttpExecutionError> {
        let mut response_result = self
            .execute_http_request_with_retry(dispatch.request, &template, dispatch.retry)
            .await;

        if dispatch.retry.oauth401_refresh_enabled
            && response_result
                .as_ref()
                .err()
                .is_some_and(|error| error.status_code() == Some(401))
        {
            let Some(context) = oauth2_context else {
                return response_result;
            };
            let refreshed = self
                .outbound_token_manager
                .access_token(dispatch.upstream_name, context.auth, true)
                .await
                .map_err(HttpExecutionError::Request)?;
            template.headers = crate::engine::tool_builders::build_headers(
                self.config.outbound_http.as_ref(),
                dispatch.upstream,
                Some(refreshed.expose_secret()),
                dispatch
                    .request
                    .tool_cfg
                    .execute
                    .as_http()
                    .and_then(|execute| execute.headers.as_ref()),
            );
            response_result = self
                .execute_http_request_with_retry(dispatch.request, &template, dispatch.retry)
                .await;
        }

        response_result
    }

    #[cfg(feature = "http_tools")]
    async fn resolve_oauth2_context<'a>(
        &self,
        upstream_name: &str,
        upstream: &'a crate::config::UpstreamConfig,
    ) -> Result<Option<Oauth2ExecutionContext<'a>>, McpError> {
        let Some(UpstreamAuth::Oauth2(auth)) = upstream.auth.as_ref() else {
            return Ok(None);
        };
        let retry_on_401_once =
            crate::auth::outbound::config::OutboundOauth2RefreshPolicy::from_auth(auth)
                .retry_on_401_once;
        let bearer = self
            .outbound_token_manager
            .access_token(upstream_name, auth, false)
            .await?;

        Ok(Some(Oauth2ExecutionContext {
            auth,
            bearer,
            retry_on_401_once,
        }))
    }

    #[cfg(feature = "http_tools")]
    fn build_http_request_template(
        &self,
        execute_http: &crate::config::ExecuteHttpConfig,
        upstream: &crate::config::UpstreamConfig,
        oauth2_bearer: Option<&str>,
    ) -> HttpRequestTemplate {
        let method = execute_http.method.clone();
        let path = execute_http.path.clone();
        let headers = crate::engine::tool_builders::build_headers(
            self.config.outbound_http.as_ref(),
            upstream,
            oauth2_bearer,
            execute_http.headers.as_ref(),
        );
        let timeout_ms = crate::http::outbound_pipeline::resolve_timeout_ms(
            None,
            upstream,
            self.config.outbound_http.as_ref(),
        );
        let max_response_bytes = crate::http::outbound_pipeline::resolve_max_response_bytes(
            None,
            upstream,
            self.config.outbound_http.as_ref(),
        );

        HttpRequestTemplate {
            method,
            path,
            query: execute_http.query.clone(),
            headers,
            body: execute_http.body.clone(),
            timeout_ms,
            max_response_bytes,
        }
    }

    #[cfg(feature = "http_tools")]
    async fn execute_http_request_with_cancellation(
        &self,
        params: HttpRequestExecutionParams<'_>,
        template: &HttpRequestTemplate,
    ) -> Result<Value, HttpExecutionError> {
        if params.tool_cfg.cancellable {
            if let Some(request) = params.request {
                let cancellation = request.ct.clone();
                tokio::select! {
                    response = execute_http_detailed(self.http_client.as_ref(), params.upstream_base_url, template, params.args) => response,
                    () = cancellation.cancelled() => Err(HttpExecutionError::Cancelled),
                }
            } else {
                execute_http_detailed(
                    self.http_client.as_ref(),
                    params.upstream_base_url,
                    template,
                    params.args,
                )
                .await
            }
        } else {
            execute_http_detailed(
                self.http_client.as_ref(),
                params.upstream_base_url,
                template,
                params.args,
            )
            .await
        }
    }

    #[cfg(feature = "http_tools")]
    async fn execute_http_request_with_retry(
        &self,
        params: HttpRequestExecutionParams<'_>,
        template: &HttpRequestTemplate,
        retry: HttpRetryExecutionParams<'_>,
    ) -> Result<Value, HttpExecutionError> {
        crate::http::outbound_pipeline::execute_with_retry(
            crate::http::outbound_pipeline::RetryExecutionParams {
                method: template.method.as_str(),
                retry: retry.retry,
                cancellation: retry.cancellation,
                cancelled_error: || HttpExecutionError::Cancelled,
            },
            || self.execute_http_request_with_cancellation(params, template),
            |result, retry_cfg| {
                should_retry_http_execution(result, retry_cfg, retry.oauth401_refresh_enabled)
            },
        )
        .await
    }
}

#[cfg(feature = "http_tools")]
fn should_retry_http_execution(
    result: &Result<Value, HttpExecutionError>,
    retry: &crate::config::OutboundRetryConfig,
    oauth401_refresh_enabled: bool,
) -> bool {
    match result {
        Ok(_)
        | Err(
            HttpExecutionError::Template(_)
            | HttpExecutionError::Response(_)
            | HttpExecutionError::Cancelled,
        ) => false,
        Err(HttpExecutionError::UpstreamStatus { status }) => {
            if oauth401_refresh_enabled && *status == 401 {
                return false;
            }
            retry.on_statuses.contains(status)
        }
        Err(HttpExecutionError::Request(_)) => retry.on_network_errors,
    }
}

fn map_tool_panic_to_error(panic_payload: Box<dyn Any + Send>) -> McpError {
    let message = match panic_payload.downcast::<String>() {
        Ok(message) => format!("tool execution panicked: {message}"),
        Err(panic_payload) => match panic_payload.downcast::<&'static str>() {
            Ok(message) => format!("tool execution panicked: {message}"),
            Err(_) => "tool execution panicked".to_owned(),
        },
    };
    McpError::internal_error(message, None)
}

// Maps an HTTP response value to a CallToolResult using the tool's response config.
// Handles content mode, structured content templates, fallback text, and output schema
// validation. Extracted from execute_http_tool to keep that function within line limits.
#[cfg(feature = "http_tools")]
fn build_http_tool_result(
    tool_cfg: &ToolConfig,
    args: &Value,
    response: Value,
    schema_validator_cache: &crate::engine::SchemaValidatorCache,
) -> Result<CallToolResult, McpError> {
    let ctx = RenderContext::new(args, Some(&response));
    let response_cfg = tool_cfg.response.as_ref();

    if let Some(response_cfg) = response_cfg {
        return match response_cfg {
            ResponseConfig::Content(ResponseContentConfig { items }) => {
                let contents =
                    crate::engine::tool_response::build_content_blocks(Some(items), &ctx)?;
                Ok(build_content_result(contents))
            }
            ResponseConfig::Structured(ResponseStructuredConfig { template, fallback }) => {
                let structured = if let Some(template) = template.as_ref() {
                    render_value(template, &ctx)?
                } else {
                    response
                };
                build_structured_result_with_cache(
                    structured,
                    tool_cfg.output_schema.as_ref(),
                    fallback
                        .as_ref()
                        .map(crate::engine::pagination::content_fallback_str),
                    schema_validator_cache,
                )
            }
        };
    }

    build_structured_result_with_cache(
        response,
        tool_cfg.output_schema.as_ref(),
        None,
        schema_validator_cache,
    )
}

#[cfg(test)]
// Inline tests here cover private helpers and internal branches that aren't
// reachable from integration tests without changing visibility or adding
// test-only seams.
mod tests {
    use super::{map_tool_panic_to_error, Engine};
    use crate::config::{
        ExecuteConfig, ExecutePluginConfig, PluginConfig, TaskSupport, ToolConfig,
    };
    use crate::engine::client_notifications::ClientNotificationHub;
    use crate::engine::orchestration::EngineConfig;
    #[cfg(feature = "tasks_utility")]
    use crate::engine::tasks::TaskStore;
    use crate::http::client::default_http_client;
    use crate::inline_test_fixtures::base_config;
    use crate::plugins::{
        ClientLoggingState, PluginCallParams, PluginRegistry, PluginType, ProgressState, ToolPlugin,
    };
    use rmcp::{
        model::{CallToolResult, ErrorCode},
        ErrorData as McpError,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    #[cfg(any(feature = "prompts", feature = "resources"))]
    use tokio::sync::RwLock;

    fn build_engine() -> Engine {
        let config = base_config();
        let http_client = default_http_client();
        Engine {
            config: Arc::new(config),
            tools: Vec::new(),
            tool_map: HashMap::new(),
            #[cfg(feature = "http_tools")]
            outbound_token_manager: crate::auth::outbound::token_manager::OutboundTokenManager::new(
                Arc::clone(&http_client),
            ),
            http_client,
            plugins: PluginRegistry::default(),
            upstreams: Arc::new(HashMap::new()),
            #[cfg(feature = "completion")]
            completion_sources: HashMap::new(),
            client_logging: ClientLoggingState::default(),
            progress_state: ProgressState::default(),
            #[cfg(feature = "tasks_utility")]
            task_store: TaskStore::new(),
            schema_validator_cache: crate::engine::SchemaValidatorCache::default(),
            #[cfg(feature = "prompts")]
            prompt_catalog: Arc::new(RwLock::new(None)),
            #[cfg(feature = "resources")]
            resource_catalog: Arc::new(RwLock::new(None)),
            list_refresh_handle: None,
            notification_hub: ClientNotificationHub::default(),
        }
    }

    fn plugin_tool_config(plugin: Option<String>) -> ToolConfig {
        ToolConfig {
            name: "plugin.test".to_owned(),
            title: None,
            description: "test".to_owned(),
            cancellable: true,
            input_schema: json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
            execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                plugin: plugin.unwrap_or_default(),
                config: None,
                task_support: TaskSupport::Forbidden,
            }),
            response: None,
        }
    }

    fn tool_plugin_config(name: &str, output_schema: Option<Value>) -> ToolConfig {
        ToolConfig {
            name: name.to_owned(),
            title: None,
            description: "test".to_owned(),
            cancellable: true,
            input_schema: json!({"type":"object"}),
            output_schema,
            annotations: None,
            icons: None,
            meta: None,
            execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                plugin: name.to_owned(),
                config: None,
                task_support: TaskSupport::Forbidden,
            }),
            response: None,
        }
    }

    fn build_plugin_engine<T>(name: &str, output_schema: Option<Value>, plugin: T) -> Engine
    where
        T: ToolPlugin + Send + Sync + 'static,
    {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: name.to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }];
        *config.tools_items_mut() = vec![tool_plugin_config(name, output_schema)];

        let registry = PluginRegistry::new()
            .register_tool(plugin)
            .expect("registry");
        Engine::from_config(EngineConfig {
            config,
            plugins: registry,
            list_refresh_handle: None,
        })
        .expect("engine")
    }

    struct StructuredPlugin;

    #[async_trait::async_trait]
    impl ToolPlugin for StructuredPlugin {
        fn name(&self) -> &'static str {
            "plugin.structured"
        }

        async fn call(
            &self,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<CallToolResult, McpError> {
            let mut result = CallToolResult::default();
            result.structured_content = Some(json!({"ok": true}));
            Ok(result)
        }
    }

    struct NoStructuredPlugin;

    #[async_trait::async_trait]
    impl ToolPlugin for NoStructuredPlugin {
        fn name(&self) -> &'static str {
            "plugin.nostructured"
        }

        async fn call(
            &self,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<CallToolResult, McpError> {
            let mut result = CallToolResult::default();
            result.content = vec![rmcp::model::Content::text("ok")];
            Ok(result)
        }
    }

    struct PanicPlugin;

    #[async_trait::async_trait]
    impl ToolPlugin for PanicPlugin {
        fn name(&self) -> &'static str {
            "plugin.panic"
        }

        async fn call(
            &self,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<CallToolResult, McpError> {
            panic!("panic plugin");
        }
    }

    struct InvalidStructuredPlugin;

    #[async_trait::async_trait]
    impl ToolPlugin for InvalidStructuredPlugin {
        fn name(&self) -> &'static str {
            "plugin.structured.invalid"
        }

        async fn call(
            &self,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<CallToolResult, McpError> {
            let mut result = CallToolResult::default();
            result.structured_content = Some(json!({"ok": "not-bool"}));
            Ok(result)
        }
    }

    #[tokio::test]
    async fn execute_plugin_tool_requires_plugin_name() {
        let engine = build_engine();
        let tool_cfg = plugin_tool_config(None);
        let error = engine
            .execute_plugin_tool(&tool_cfg, Value::Null, None)
            .await
            .expect_err("missing plugin config should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "plugin tool requires plugin");
        assert!(error.data.is_none());
    }

    #[tokio::test]
    async fn execute_plugin_tool_requires_registered_plugin() {
        let engine = build_engine();
        let tool_cfg = plugin_tool_config(Some("plugin.missing".to_owned()));
        let error = engine
            .execute_plugin_tool(&tool_cfg, Value::Null, None)
            .await
            .expect_err("missing registry plugin should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "tool plugin not registered: plugin.missing");
        assert!(error.data.is_none());
    }

    #[tokio::test]
    async fn execute_plugin_tool_validates_structured_schema_and_adds_fallback_content() {
        let engine = build_plugin_engine(
            "plugin.structured",
            Some(json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}})),
            StructuredPlugin,
        );
        let result = engine
            .execute_tool("plugin.structured", json!({}))
            .await
            .expect("tool");

        assert_eq!(result.is_error, None);
        assert_eq!(result.structured_content, Some(json!({"ok": true})));
        let result_json = serde_json::to_value(result).expect("serialize tool result");
        assert_eq!(result_json["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(result_json["content"][0]["type"], "text");
        assert_eq!(result_json["content"][0]["text"], "{\"ok\":true}");
        assert!(result_json["content"][0].get("_meta").is_none());
    }

    #[tokio::test]
    async fn execute_plugin_tool_returns_early_when_no_structured_content() {
        let engine = build_plugin_engine("plugin.nostructured", None, NoStructuredPlugin);
        let result = engine
            .execute_tool("plugin.nostructured", json!({}))
            .await
            .expect("tool");
        assert_eq!(result.is_error, None);
        assert!(result.structured_content.is_none());
        let result_json = serde_json::to_value(result).expect("serialize tool result");
        assert_eq!(result_json["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(result_json["content"][0]["type"], "text");
        assert_eq!(result_json["content"][0]["text"], "ok");
        assert!(result_json.get("structuredContent").is_none());
    }

    #[tokio::test]
    async fn execute_plugin_tool_skips_schema_validation_when_output_schema_absent() {
        let engine = build_plugin_engine("plugin.structured", None, StructuredPlugin);
        let result = engine
            .execute_tool("plugin.structured", json!({}))
            .await
            .expect("tool");
        assert_eq!(result.structured_content, Some(json!({"ok": true})));
    }

    #[tokio::test]
    async fn execute_plugin_tool_returns_error_when_output_schema_validation_fails() {
        let engine = build_plugin_engine(
            "plugin.structured.invalid",
            Some(json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}})),
            InvalidStructuredPlugin,
        );
        let result = engine
            .execute_tool("plugin.structured.invalid", json!({}))
            .await
            .expect("invalid structured output should map to tool error result");
        assert_eq!(result.is_error, Some(true));
        let result_json = serde_json::to_value(result).expect("serialize tool result");
        assert_eq!(result_json["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(result_json["content"][0]["type"], "text");
        assert!(
            result_json["content"][0]["text"]
                .as_str()
                .is_some_and(|message| {
                    message.starts_with(
                        "output schema validation failed: \"not-bool\" is not of type \"boolean\"",
                    )
                }),
            "unexpected schema error message: {}",
            result_json["content"][0]["text"]
        );
        assert!(result_json.get("structuredContent").is_none());
    }

    #[tokio::test]
    async fn execute_tool_with_context_catches_plugin_panics() {
        let engine = build_plugin_engine("plugin.panic", None, PanicPlugin);
        let result = engine
            .execute_tool("plugin.panic", json!({}))
            .await
            .expect("panic should map to tool result error");
        assert_eq!(result.is_error, Some(true));
        let result_json = serde_json::to_value(result).expect("serialize tool result");
        assert_eq!(result_json["isError"], true);
        assert_eq!(result_json["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(result_json["content"][0]["type"], "text");
        assert_eq!(result_json["content"][0]["text"], "internal server error");
        assert!(result_json.get("structuredContent").is_none());
        assert!(result_json["content"][0].get("_meta").is_none());
    }

    #[test]
    fn map_tool_panic_to_error_handles_str_payload() {
        let error = map_tool_panic_to_error(Box::new("panic str"));
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "tool execution panicked: panic str");
        assert!(error.data.is_none());
    }

    #[test]
    fn map_tool_panic_to_error_handles_string_payload() {
        let error = map_tool_panic_to_error(Box::new(String::from("panic string")));
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "tool execution panicked: panic string");
        assert!(error.data.is_none());
    }

    #[test]
    fn map_tool_panic_to_error_handles_non_string_payload() {
        let error = map_tool_panic_to_error(Box::new(42_u8));
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "tool execution panicked");
        assert!(error.data.is_none());
    }
}
