#![cfg(feature = "http_tools")]

mod engine_common;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use engine_common::{
    fixture_path, load_config_fixture as load_engine_config_fixture, load_fixture,
    load_tool_fixture,
};
use httpmock::{Method::GET, MockServer};
use rmcp::{model::CallToolResult, ErrorData as McpError};
use rust_mcp_core::{
    config::{
        ExecuteConfig, ExecutePluginConfig, McpConfig, OutboundHttpConfig, OutboundRetryConfig,
        PluginConfig, ResponseLimitsConfig, SecretValueConfig, SecretValueSource, ToolConfig,
        ToolsConfig, UpstreamAuth, UpstreamOauth2AuthConfig, UpstreamOauth2ClientAuthMethod,
        UpstreamOauth2GrantType, UpstreamOauth2MtlsConfig, UpstreamOauth2RefreshConfig,
    },
    engine::{Engine, EngineConfig},
    mcp::Content,
    plugins::{
        ListFeature, ListRefreshHandle, PluginCallParams, PluginRegistry, PluginType, ToolPlugin,
    },
    OutboundHttpRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct PluginFixturePayload {
    args: Value,
}

struct PluginFixture {
    config: rust_mcp_core::config::McpConfig,
    args: Value,
}

fn load_plugin_fixture(name: &str) -> PluginFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    let parsed: PluginFixturePayload = serde_yaml::from_str(&raw).expect("fixture should parse");
    let config_fixture_name = format!("{name}_config");
    PluginFixture {
        config: load_engine_config_fixture(&config_fixture_name).config,
        args: parsed.args,
    }
}

fn first_content_text(result: &CallToolResult) -> String {
    serde_json::to_value(result)
        .expect("serialize call result")
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("content[0].text should exist")
}

fn client_credentials_oauth2_auth(token_url: String, retry_on_401_once: bool) -> UpstreamAuth {
    UpstreamAuth::Oauth2(Box::new(UpstreamOauth2AuthConfig {
        grant: UpstreamOauth2GrantType::ClientCredentials,
        token_url,
        client_id: "client-id".to_owned(),
        client_secret: SecretValueConfig {
            source: SecretValueSource::Inline,
            value: "client-secret".to_owned(),
        },
        auth_method: Some(UpstreamOauth2ClientAuthMethod::RequestBody),
        scopes: vec!["reports.read".to_owned()],
        audience: None,
        extra_token_params: std::collections::HashMap::new(),
        refresh: Some(UpstreamOauth2RefreshConfig {
            skew_sec: Some(0),
            retry_on_401_once: Some(retry_on_401_once),
        }),
        refresh_token: None,
        mtls: None,
    }))
}

fn refresh_token_oauth2_auth(token_url: String, retry_on_401_once: bool) -> UpstreamAuth {
    UpstreamAuth::Oauth2(Box::new(UpstreamOauth2AuthConfig {
        grant: UpstreamOauth2GrantType::RefreshToken,
        token_url,
        client_id: "client-id".to_owned(),
        client_secret: SecretValueConfig {
            source: SecretValueSource::Inline,
            value: "client-secret".to_owned(),
        },
        auth_method: Some(UpstreamOauth2ClientAuthMethod::RequestBody),
        scopes: vec!["reports.read".to_owned()],
        audience: None,
        extra_token_params: std::collections::HashMap::new(),
        refresh: Some(UpstreamOauth2RefreshConfig {
            skew_sec: Some(0),
            retry_on_401_once: Some(retry_on_401_once),
        }),
        refresh_token: Some(SecretValueConfig {
            source: SecretValueSource::Inline,
            value: "bootstrap-refresh".to_owned(),
        }),
        mtls: None,
    }))
}

fn oauth_probe_tool_config() -> ToolConfig {
    ToolConfig {
        name: "plugin.oauth_probe".to_owned(),
        title: None,
        description: "oauth helper probe".to_owned(),
        cancellable: true,
        input_schema: serde_json::json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.oauth_helper_probe".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }
}

fn oauth_probe_plugin_config() -> PluginConfig {
    PluginConfig {
        name: "plugin.oauth_helper_probe".to_owned(),
        plugin_type: PluginType::Tool,
        targets: None,
        config: None,
    }
}

fn oauth_send_probe_tool_config() -> ToolConfig {
    ToolConfig {
        name: "plugin.oauth_send_probe".to_owned(),
        title: None,
        description: "oauth send probe".to_owned(),
        cancellable: true,
        input_schema: serde_json::json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.oauth_send_probe".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }
}

fn oauth_send_probe_plugin_config() -> PluginConfig {
    PluginConfig {
        name: "plugin.oauth_send_probe".to_owned(),
        plugin_type: PluginType::Tool,
        targets: None,
        config: None,
    }
}

struct RefreshProbePlugin;

#[async_trait]
impl ToolPlugin for RefreshProbePlugin {
    fn name(&self) -> &'static str {
        "plugin.refresh_probe"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let _ = params.ctx.request_list_refresh(ListFeature::Tools).await?;
        Ok(CallToolResult::structured(serde_json::json!({"ok": true})))
    }
}

struct RefreshHandleProbe {
    called: Arc<AtomicBool>,
}

#[async_trait]
impl ListRefreshHandle for RefreshHandleProbe {
    async fn refresh_list(&self, _feature: ListFeature) -> Result<bool, McpError> {
        self.called.store(true, Ordering::SeqCst);
        Ok(true)
    }
}

struct EchoPlugin;

#[async_trait]
impl ToolPlugin for EchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(args))
    }
}

struct EmptyContentPlugin;

#[async_trait]
impl ToolPlugin for EmptyContentPlugin {
    fn name(&self) -> &'static str {
        "plugin.empty"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let mut result = CallToolResult::default();
        result.structured_content = Some(args);
        Ok(result)
    }
}

struct InternalErrorPlugin;

#[async_trait]
impl ToolPlugin for InternalErrorPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error(
            "sensitive path detail: /tmp/secret.txt".to_owned(),
            None,
        ))
    }
}

struct LargeTextPlugin;

#[async_trait]
impl ToolPlugin for LargeTextPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            "x".repeat(256),
        )]))
    }
}

struct LargeStructuredPlugin;

#[async_trait]
impl ToolPlugin for LargeStructuredPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(
            serde_json::json!({ "payload": "x".repeat(256) }),
        ))
    }
}

struct HttpProbePlugin;

#[async_trait]
impl ToolPlugin for HttpProbePlugin {
    fn name(&self) -> &'static str {
        "plugin.http_probe"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_request("url argument is required".to_owned(), None))?
            .to_owned();
        let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64);
        let max_response_bytes = args.get("max_response_bytes").and_then(Value::as_u64);
        let response = params
            .ctx
            .send_raw(OutboundHttpRequest {
                method: "GET".to_owned(),
                url,
                timeout_ms,
                max_response_bytes,
                ..OutboundHttpRequest::default()
            })
            .await?;

        Ok(CallToolResult::structured(
            serde_json::json!({ "status": response.status() }),
        ))
    }
}

struct OauthHelperProbePlugin {
    upstream_name: &'static str,
    force_refresh: bool,
}

#[async_trait]
impl ToolPlugin for OauthHelperProbePlugin {
    fn name(&self) -> &'static str {
        "plugin.oauth_helper_probe"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let token = params
            .ctx
            .upstream_access_token(self.upstream_name, self.force_refresh)
            .await?;
        let _ = format!("{token:?}");
        let token_value = token.clone().into_string();
        let (header_name, header_value) = params
            .ctx
            .upstream_bearer_header(self.upstream_name, self.force_refresh)
            .await?;
        Ok(CallToolResult::structured(serde_json::json!({
            "token": token_value,
            "header_name": header_name,
            "header_value": header_value,
        })))
    }
}

struct OauthSendProbePlugin {
    upstream_name: &'static str,
}

#[async_trait]
impl ToolPlugin for OauthSendProbePlugin {
    fn name(&self) -> &'static str {
        "plugin.oauth_send_probe"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let response = params
            .ctx
            .send(
                self.upstream_name,
                OutboundHttpRequest {
                    method: "GET".to_owned(),
                    url: "/protected".to_owned(),
                    ..OutboundHttpRequest::default()
                },
            )
            .await?;
        Ok(CallToolResult::structured(
            serde_json::json!({ "status": response.status() }),
        ))
    }
}

#[tokio::test]
async fn executes_engine_http_tool_fixture() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items").query_param("q", "alpha");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .base_url = server.base_url();

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.structured_content, Some(fixture.expected));
}

#[tokio::test]
async fn execute_http_tool_retries_retryable_status_when_enabled() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();

    let mock = server.mock(|when, then| {
        when.method(GET).path("/items").query_param("q", "alpha");
        then.status(503).body("unavailable");
    });

    fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .base_url = server.base_url();
    fixture.config.outbound_http = Some(OutboundHttpConfig {
        headers: std::collections::HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: None,
        retry: Some(OutboundRetryConfig {
            max_attempts: 2,
            delay_ms: 1,
            on_network_errors: false,
            on_statuses: vec![503],
        }),
    });

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should return error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "internal server error",
        "retried 503 error should be redacted by default"
    );
    mock.assert_calls(2);
}

#[tokio::test]
async fn execute_http_tool_does_not_retry_non_idempotent_method() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();

    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/items")
            .query_param("q", "alpha");
        then.status(503).body("unavailable");
    });

    fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .base_url = server.base_url();
    fixture.config.outbound_http = Some(OutboundHttpConfig {
        headers: std::collections::HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: None,
        retry: Some(OutboundRetryConfig {
            max_attempts: 3,
            delay_ms: 1,
            on_network_errors: false,
            on_statuses: vec![503],
        }),
    });
    fixture.config.tools_items_mut()[0]
        .execute
        .as_http_mut()
        .expect("fixture tool must be http")
        .method = "POST".to_owned();

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should return error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "internal server error",
        "non-retried POST 503 error should be redacted by default"
    );
    mock.assert_calls(1);
}

#[tokio::test]
async fn execute_tool_redacts_internal_errors_by_default() {
    let fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    let plugins = PluginRegistry::new()
        .register_tool(InternalErrorPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect("plugin should return tool error result");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(first_content_text(&result), "internal server error");
}

#[tokio::test]
async fn execute_tool_exposes_internal_errors_when_enabled() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    let plugins = PluginRegistry::new()
        .register_tool(InternalErrorPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect("plugin should return tool error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "sensitive path detail: /tmp/secret.txt",
        "exposed error should include full sensitive detail"
    );
}

#[tokio::test]
async fn execute_tool_rejects_payload_over_response_limit() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: Some(32),
        structured_bytes: None,
        binary_bytes: None,
        other_bytes: None,
        total_bytes: None,
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeTextPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized response should fail");
    assert_eq!(
        error.message, "tool response exceeds configured text_bytes limit",
        "exposed text channel error should use exact contract"
    );
}

#[tokio::test]
async fn execute_tool_redacts_response_limit_error_by_default() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: Some(32),
        structured_bytes: None,
        binary_bytes: None,
        other_bytes: None,
        total_bytes: None,
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeTextPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized response should fail");
    assert_eq!(
        error.message, "internal server error",
        "redacted error should not expose channel details"
    );
}

#[tokio::test]
async fn execute_tool_rejects_structured_payload_over_response_limit() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: None,
        structured_bytes: Some(32),
        binary_bytes: None,
        other_bytes: None,
        total_bytes: None,
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeStructuredPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized structured response should fail");
    assert_eq!(
        error.message, "tool response exceeds configured structured_bytes limit",
        "exposed structured channel error should use exact contract"
    );
}

#[tokio::test]
async fn execute_tool_redacts_structured_response_limit_error_by_default() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: None,
        structured_bytes: Some(32),
        binary_bytes: None,
        other_bytes: None,
        total_bytes: None,
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeStructuredPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized structured response should fail");
    assert_eq!(
        error.message, "internal server error",
        "redacted structured channel error should be hidden"
    );
}

#[tokio::test]
async fn execute_tool_rejects_total_payload_over_response_limit() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: None,
        structured_bytes: None,
        binary_bytes: None,
        other_bytes: None,
        total_bytes: Some(32),
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeTextPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized total response should fail");
    assert_eq!(
        error.message, "tool response exceeds configured total_bytes limit",
        "exposed total channel error should use exact contract"
    );
}

#[tokio::test]
async fn execute_tool_redacts_total_response_limit_error_by_default() {
    let mut fixture = load_engine_config_fixture("engine/engine_plugin_not_registered_fixture");
    fixture.config.server.response_limits = Some(ResponseLimitsConfig {
        text_bytes: None,
        structured_bytes: None,
        binary_bytes: None,
        other_bytes: None,
        total_bytes: Some(32),
    });
    let plugins = PluginRegistry::new()
        .register_tool(LargeTextPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let error = engine
        .execute_tool("plugin.echo", json!({}))
        .await
        .expect_err("oversized total response should fail");
    assert_eq!(
        error.message, "internal server error",
        "redacted total channel error should be hidden"
    );
}

#[tokio::test]
async fn execute_plugin_tool_http_raw_send_does_not_apply_outbound_defaults() {
    let server = MockServer::start();
    let body = "x".repeat(96);
    let response_mock = server.mock(|when, then| {
        when.method(GET).path("/plugin-http-default-limit");
        then.status(200).body(body.as_str());
    });

    let mut config =
        load_engine_config_fixture("engine/engine_plugin_not_registered_fixture").config;
    config.server.errors.expose_internal_details = true;
    config.outbound_http = Some(OutboundHttpConfig {
        headers: std::collections::HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: Some(64),
        retry: None,
    });
    config.set_tools_items(vec![ToolConfig {
        name: "plugin.http_probe".to_owned(),
        title: None,
        description: "http probe".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.http_probe".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.plugins = vec![PluginConfig {
        name: "plugin.http_probe".to_owned(),
        plugin_type: PluginType::Tool,
        targets: None,
        config: None,
    }];

    let registry = PluginRegistry::new()
        .register_tool(HttpProbePlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool(
            "plugin.http_probe",
            json!({"url": server.url("/plugin-http-default-limit")}),
        )
        .await
        .expect("plugin should execute");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"status": 200}))
    );
    response_mock.assert_calls(1);
}

fn configure_search_mocks(server: &MockServer) -> (httpmock::Mock<'_>, httpmock::Mock<'_>) {
    let default_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/search")
            .query_param("q", "mcp")
            .query_param("tags", "rust,sdk")
            .query_param("page", "1")
            .query_param("per_page", "25");
        then.status(200).json_body(json!({
            "query_params": {
                "q": "mcp",
                "tags": "rust,sdk",
                "page": "1",
                "per_page": "25"
            }
        }));
    });
    let override_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/search")
            .query_param("q", "mcp")
            .query_param("tags", "rust")
            .query_param("page", "3")
            .query_param("per_page", "5");
        then.status(200).json_body(json!({
            "query_params": {
                "q": "mcp",
                "tags": "rust",
                "page": "3",
                "per_page": "5"
            }
        }));
    });
    (default_mock, override_mock)
}

fn expected_query_params(tags: &str, page: &str, per_page: &str) -> Value {
    json!({
        "query_params": {
            "q": "mcp",
            "tags": tags,
            "page": page,
            "per_page": per_page
        }
    })
}

#[tokio::test]
async fn executes_http_tool_templating_from_loaded_yaml_config() {
    let mut config =
        load_engine_config_fixture("engine/engine_http_templating_fixture_config").config;
    let server = MockServer::start();
    let (default_mock, override_mock) = configure_search_mocks(&server);

    config
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .base_url = server.base_url();

    let engine = Engine::new(config).expect("engine should build");

    let default_result = engine
        .execute_tool(
            "search.advanced",
            json!({
                "query": "mcp",
                "tags": ["rust", "sdk"]
            }),
        )
        .await
        .expect("defaulted query tool call should succeed");
    assert_eq!(
        default_result.structured_content,
        Some(expected_query_params("rust,sdk", "1", "25"))
    );

    let explicit_result = engine
        .execute_tool(
            "search.advanced",
            json!({
                "query": "mcp",
                "tags": ["rust"],
                "page": 3,
                "per_page": 5
            }),
        )
        .await
        .expect("explicit query tool call should succeed");
    assert_eq!(
        explicit_result.structured_content,
        Some(expected_query_params("rust", "3", "5"))
    );

    default_mock.assert_calls(1);
    override_mock.assert_calls(1);
}

#[tokio::test]
async fn http_tool_with_missing_upstream_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_builtin_tool_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let error = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("http tool with missing upstream should fail");
    assert_eq!(
        error.message, "unknown upstream: api",
        "missing upstream error should include upstream name"
    );
}

#[tokio::test]
async fn unknown_upstream_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_unknown_upstream_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let error = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("unknown upstream should fail");
    assert_eq!(
        error.message, "unknown upstream: missing",
        "unknown upstream error should include upstream name"
    );
}

#[tokio::test]
async fn execute_http_tool_non_cancellable_path_fixture() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }
    if let Some(tool) = config
        .tools_items_mut()
        .iter_mut()
        .find(|tool| tool.name == fixture.tool)
    {
        tool.cancellable = false;
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool ok");
    assert_eq!(
        result.is_error,
        Some(false),
        "non-cancellable HTTP tool should succeed"
    );
    assert_eq!(result.content.len(), 2, "fixture defines 2 content blocks");
}

#[tokio::test]
async fn execute_http_tool_oauth2_injects_bearer_header() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials")
            .body_includes("client_id=client-id")
            .body_includes("client_secret=client-secret");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.structured_content, Some(fixture.expected));
    token_mock.assert_calls(1);
    api_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_reuses_cached_token_when_still_fresh() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));

    let engine = Engine::new(fixture.config).expect("engine should build");

    let first = engine
        .execute_tool("api.list", fixture.args.clone())
        .await
        .expect("first tool call should execute");
    let second = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("second tool call should execute");

    assert_eq!(first.structured_content, Some(fixture.expected.clone()));
    assert_eq!(second.structured_content, Some(fixture.expected));
    token_mock.assert_calls(1);
    api_mock.assert_calls(2);
}

#[tokio::test]
async fn execute_http_tool_oauth2_without_expires_in_stays_cached() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer"}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));

    let engine = Engine::new(fixture.config).expect("engine should build");

    let _ = engine
        .execute_tool("api.list", fixture.args.clone())
        .await
        .expect("first tool call should execute");
    let _ = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("second tool call should execute");

    token_mock.assert_calls(1);
    api_mock.assert_calls(2);
}

#[tokio::test]
async fn execute_http_tool_oauth2_coalesces_concurrent_token_acquisition() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));

    let engine = Arc::new(Engine::new(fixture.config).expect("engine should build"));
    let args_a = fixture.args.clone();
    let args_b = fixture.args;

    let engine_a = Arc::clone(&engine);
    let first = tokio::spawn(async move { engine_a.execute_tool("api.list", args_a).await });
    let engine_b = Arc::clone(&engine);
    let second = tokio::spawn(async move { engine_b.execute_tool("api.list", args_b).await });

    let result_a = first.await.expect("first join").expect("first tool result");
    let result_b = second
        .await
        .expect("second join")
        .expect("second tool result");

    assert_eq!(result_a.is_error, Some(false));
    assert_eq!(result_b.is_error, Some(false));
    token_mock.assert_calls(1);
    api_mock.assert_calls(2);
}

#[tokio::test]
async fn execute_http_tool_oauth2_retries_once_on_401() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_first = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200).header("content-type", "application/json").body(
            r#"{"access_token":"token-a","token_type":"Bearer","refresh_token":"rotated-1","expires_in":300}"#,
        );
    });
    let token_second = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=rotated-1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-b","token_type":"Bearer","expires_in":300}"#);
    });
    let api_unauthorized = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(401);
    });
    let api_retry_ok = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-b");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, true));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.structured_content, Some(fixture.expected));
    token_first.assert_calls(1);
    token_second.assert_calls(1);
    api_unauthorized.assert_calls(1);
    api_retry_ok.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_does_not_retry_401_when_disabled() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200).header("content-type", "application/json").body(
            r#"{"access_token":"token-a","token_type":"Bearer","refresh_token":"rotated-1","expires_in":300}"#,
        );
    });
    let api_unauthorized = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(401);
    });

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, false));

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "upstream returned 401",
        "401 without retry should report exact upstream status"
    );
    token_mock.assert_calls(1);
    api_unauthorized.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_initial_token_failure_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.auth = Some(client_credentials_oauth2_auth(
        "not-a-valid-url".to_owned(),
        false,
    ));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("invalid oauth2 token_url:"),
        "initial token failure should use token_url prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_mtls_inline_invalid_identity_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = client_credentials_oauth2_auth(token_url, false);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: None,
            client_cert: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "invalid-cert".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "invalid-key".to_owned(),
            },
        });
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("invalid oauth2 mTLS client identity PEM:"),
        "mTLS invalid identity should use PEM prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_mtls_path_source_missing_file_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = client_credentials_oauth2_auth(token_url, false);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: None,
            client_cert: SecretValueConfig {
                source: SecretValueSource::Path,
                value: "/tmp/missing-oauth2-mtls-client-cert.pem".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "unused-key".to_owned(),
            },
        });
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("failed to resolve oauth2 mTLS client_cert:"),
        "missing mTLS cert path should use client_cert prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_client_credentials_basic_with_extra_params_maps_parse_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .header_exists("authorization")
            .body_includes("grant_type=client_credentials")
            .body_includes("resource=reports-api");
        then.status(200)
            .header("content-type", "application/json")
            .body("not-json");
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = client_credentials_oauth2_auth(token_url, false);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config.auth_method = Some(UpstreamOauth2ClientAuthMethod::Basic);
        config
            .extra_token_params
            .insert("resource".to_owned(), "reports-api".to_owned());
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("oauth2 token endpoint returned invalid JSON:"),
        "invalid JSON parse error should include parse details: {error_text}"
    );
    token_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_client_credentials_request_failure_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.auth = Some(client_credentials_oauth2_auth(
        "http://127.0.0.1:9/oauth/token".to_owned(),
        false,
    ));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("oauth2 token request failed:"),
        "request failure should use token request prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_retry_refresh_failure_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_first = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200).header("content-type", "application/json").body(
            r#"{"access_token":"token-a","token_type":"Bearer","refresh_token":"rotated-1","expires_in":300}"#,
        );
    });
    let token_second = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=rotated-1");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"error":"server_error"}"#);
    });
    let api_unauthorized = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(401);
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, true));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("oauth2 token endpoint returned an error response:"),
        "retry refresh failure should include provider error details: {error_text}"
    );
    token_first.assert_calls(1);
    token_second.assert_calls(1);
    api_unauthorized.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_refresh_grant_invalid_token_url_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.auth = Some(refresh_token_oauth2_auth(
        "not-a-valid-url".to_owned(),
        true,
    ));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("invalid oauth2 token_url:"),
        "refresh grant invalid URL should use token_url prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_retry_second_401_returns_tool_error() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    fixture.config.server.errors.expose_internal_details = true;
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_first = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200).header("content-type", "application/json").body(
            r#"{"access_token":"token-a","token_type":"Bearer","refresh_token":"rotated-1","expires_in":300}"#,
        );
    });
    let token_second = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=rotated-1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-b","token_type":"Bearer","expires_in":300}"#);
    });
    let api_unauthorized_first = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(401);
    });
    let api_unauthorized_second = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-b");
        then.status(401);
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, true));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "upstream returned 401",
        "second 401 should report exact upstream status"
    );
    token_first.assert_calls(1);
    token_second.assert_calls(1);
    api_unauthorized_first.assert_calls(1);
    api_unauthorized_second.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_refresh_grant_sends_extra_token_params() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh")
            .body_includes("resource=reports-api");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = refresh_token_oauth2_auth(token_url, false);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config
            .extra_token_params
            .insert("resource".to_owned(), "reports-api".to_owned());
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.structured_content, Some(fixture.expected));
    token_mock.assert_calls(1);
    api_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_refresh_grant_requires_refresh_token() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = refresh_token_oauth2_auth(token_url, true);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config.refresh_token = None;
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool call should return a tool error result");

    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "oauth2 refresh_token grant requires refresh_token",
        "missing refresh token should use exact contract"
    );
}

#[tokio::test]
async fn execute_http_tool_oauth2_refresh_grant_supports_basic_client_auth() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .header_exists("authorization")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });
    let api_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    let mut auth = refresh_token_oauth2_auth(token_url, false);
    if let UpstreamAuth::Oauth2(config) = &mut auth {
        config.auth_method = Some(UpstreamOauth2ClientAuthMethod::Basic);
    }
    upstream.auth = Some(auth);

    let engine = Engine::new(fixture.config).expect("engine should build");
    let result = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.structured_content, Some(fixture.expected));
    token_mock.assert_calls(1);
    api_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_http_tool_oauth2_refresh_grant_reuses_cached_refresh_token_when_not_rotated() {
    let mut fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_first = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200).header("content-type", "application/json").body(
            r#"{"access_token":"token-a","token_type":"Bearer","refresh_token":"rotated-1","expires_in":0}"#,
        );
    });
    let token_second = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=rotated-1");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-b","token_type":"Bearer","expires_in":0}"#);
    });
    let api_first = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-a");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });
    let api_second = server.mock(|when, then| {
        when.method(GET)
            .path("/items")
            .query_param("q", "alpha")
            .header("authorization", "Bearer token-b");
        then.status(200)
            .json_body(serde_json::json!({"items": [{"id": 1}]}));
    });

    let upstream = fixture
        .config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, true));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let _ = engine
        .execute_tool("api.list", fixture.args.clone())
        .await
        .expect("first call should execute");
    let _ = engine
        .execute_tool("api.list", fixture.args.clone())
        .await
        .expect("second call should execute");
    let _ = engine
        .execute_tool("api.list", fixture.args)
        .await
        .expect("third call should execute");

    token_first.assert_calls(1);
    token_second.assert_calls(2);
    api_first.assert_calls(1);
    api_second.assert_calls(2);
}

#[tokio::test]
async fn execute_plugin_tool_with_refresh_handle_path_fixture() {
    let config = McpConfig {
        version: 1,
        server: load_engine_config_fixture("engine/engine_get_info_fixture")
            .config
            .server,
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: rust_mcp_core::config::ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: std::collections::HashMap::default(),
        tools: Some(ToolsConfig {
            enabled: None,
            notify_list_changed: false,
            items: vec![ToolConfig {
                name: "tool.refresh".to_owned(),
                title: None,
                description: "refresh probe".to_owned(),
                cancellable: true,
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                    plugin: "plugin.refresh_probe".to_owned(),
                    config: None,
                    task_support: rust_mcp_core::config::TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: vec![PluginConfig {
            name: "plugin.refresh_probe".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }],
        outbound_http: None,
    };
    let called = Arc::new(AtomicBool::new(false));
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::new()
            .register_tool(RefreshProbePlugin)
            .expect("register plugin"),
        list_refresh_handle: Some(Arc::new(RefreshHandleProbe {
            called: Arc::clone(&called),
        })),
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("tool.refresh", serde_json::json!({}))
        .await
        .expect("tool ok");
    assert_eq!(result.is_error, Some(false), "refresh probe should succeed");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
    assert!(
        called.load(Ordering::SeqCst),
        "refresh handle should have been called"
    );
    assert!(
        !result.content.is_empty(),
        "result should have content fallback"
    );
}

#[tokio::test]
async fn executes_plugin_tool_fixture() {
    let fixture = load_plugin_fixture("plugins/plugin_success_fixture");
    let registry = PluginRegistry::new().register_tool(EchoPlugin).unwrap();
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.echo", fixture.args)
        .await
        .expect("plugin should execute");

    assert_eq!(result.is_error, Some(false), "plugin tool should succeed");
    let structured = result
        .structured_content
        .expect("structured content should be present");
    assert_eq!(
        structured,
        serde_json::json!({"ok": true}),
        "EchoPlugin should echo the fixture args"
    );
    assert!(
        !result.content.is_empty(),
        "plugin result should have at least one content block (text fallback)"
    );
}

#[tokio::test]
async fn rejects_invalid_plugin_output_fixture() {
    let fixture = load_plugin_fixture("plugins/plugin_invalid_fixture");
    let registry = PluginRegistry::new().register_tool(EchoPlugin).unwrap();
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.echo", fixture.args)
        .await
        .expect("invalid plugin output should map to tool error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("output schema validation failed:"),
        "invalid output error should use schema-validation prefix contract: {error_text}"
    );
}

#[tokio::test]
async fn plugin_structured_fallback_fixture() {
    let fixture = load_plugin_fixture("plugins/plugin_structured_fallback_fixture");
    let registry = PluginRegistry::new()
        .register_tool(EmptyContentPlugin)
        .unwrap();
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.empty", fixture.args)
        .await
        .expect("plugin should execute");

    assert_ne!(
        result.is_error,
        Some(true),
        "plugin should not return error"
    );
    let structured = result
        .structured_content
        .expect("structured content should be present");
    assert_eq!(
        structured,
        serde_json::json!({"ok": true}),
        "EmptyContentPlugin echoes args as structured content"
    );
    assert_eq!(
        result.content.len(),
        1,
        "fallback should produce exactly one text content block"
    );
}

#[tokio::test]
async fn execute_plugin_tool_oauth_helper_fetches_token_and_builds_header() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });

    let mut config = fixture.config;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));
    config.set_tools_items(vec![oauth_probe_tool_config()]);
    config.plugins = vec![oauth_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthHelperProbePlugin {
            upstream_name: "api",
            force_refresh: false,
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_probe", serde_json::json!({}))
        .await
        .expect("plugin tool should execute");

    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({
            "token": "token-a",
            "header_name": "Authorization",
            "header_value": "Bearer token-a"
        }))
    );
    token_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_plugin_tool_oauth_helper_force_refresh_reacquires_token() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=client_credentials");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300}"#);
    });

    let mut config = fixture.config;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(client_credentials_oauth2_auth(token_url, false));
    config.set_tools_items(vec![oauth_probe_tool_config()]);
    config.plugins = vec![oauth_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthHelperProbePlugin {
            upstream_name: "api",
            force_refresh: true,
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_probe", serde_json::json!({}))
        .await
        .expect("plugin tool should execute");

    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({
            "token": "token-a",
            "header_name": "Authorization",
            "header_value": "Bearer token-a"
        }))
    );
    token_mock.assert_calls(2);
}

#[tokio::test]
async fn execute_plugin_tool_send_retries_oauth2_refresh_once_on_401() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let server = MockServer::start();
    let token_url = format!("{}/oauth/token", server.base_url());

    let bootstrap_token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=bootstrap-refresh");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                r#"{"access_token":"token-a","token_type":"Bearer","expires_in":300,"refresh_token":"rotated-refresh"}"#,
            );
    });
    let rotated_token_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/oauth/token")
            .body_includes("grant_type=refresh_token")
            .body_includes("refresh_token=rotated-refresh");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"access_token":"token-b","token_type":"Bearer","expires_in":300}"#);
    });
    let first_protected_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/protected")
            .header("authorization", "Bearer token-a");
        then.status(401).body("unauthorized");
    });
    let second_protected_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/protected")
            .header("authorization", "Bearer token-b");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let mut config = fixture.config;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.base_url = server.base_url();
    upstream.auth = Some(refresh_token_oauth2_auth(token_url, true));
    config.set_tools_items(vec![oauth_send_probe_tool_config()]);
    config.plugins = vec![oauth_send_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthSendProbePlugin {
            upstream_name: "api",
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_send_probe", serde_json::json!({}))
        .await
        .expect("plugin tool should execute");

    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({ "status": 200 }))
    );
    bootstrap_token_mock.assert_calls(1);
    rotated_token_mock.assert_calls(1);
    first_protected_mock.assert_calls(1);
    second_protected_mock.assert_calls(1);
}

#[tokio::test]
async fn execute_plugin_tool_oauth_helper_rejects_unknown_upstream() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let mut config = fixture.config;
    config.set_tools_items(vec![oauth_probe_tool_config()]);
    config.plugins = vec![oauth_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthHelperProbePlugin {
            upstream_name: "missing",
            force_refresh: false,
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_probe", serde_json::json!({}))
        .await
        .expect("plugin should return tool error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "upstream 'missing' not found",
        "unknown upstream should produce exact error message"
    );
}

#[tokio::test]
async fn execute_plugin_tool_oauth_helper_rejects_non_oauth2_upstream() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let mut config = fixture.config;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.auth = Some(UpstreamAuth::Bearer {
        token: "test-token".to_owned(),
    });
    config.set_tools_items(vec![oauth_probe_tool_config()]);
    config.plugins = vec![oauth_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthHelperProbePlugin {
            upstream_name: "api",
            force_refresh: false,
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_probe", serde_json::json!({}))
        .await
        .expect("plugin should return tool error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert_eq!(
        error_text, "upstream 'api' does not use oauth2 auth",
        "non-oauth2 upstream should produce exact error message"
    );
}

#[tokio::test]
async fn execute_plugin_tool_oauth_helper_propagates_oauth_exchange_error() {
    let fixture = load_fixture("engine/engine_http_fixture");
    let mut config = fixture.config;
    let upstream = config
        .upstreams
        .get_mut("api")
        .expect("api upstream should exist");
    upstream.auth = Some(client_credentials_oauth2_auth(
        "not-a-valid-url".to_owned(),
        false,
    ));
    config.set_tools_items(vec![oauth_probe_tool_config()]);
    config.plugins = vec![oauth_probe_plugin_config()];

    let registry = PluginRegistry::new()
        .register_tool(OauthHelperProbePlugin {
            upstream_name: "api",
            force_refresh: false,
        })
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.oauth_probe", serde_json::json!({}))
        .await
        .expect("plugin should return tool error result");
    assert_eq!(result.is_error, Some(true));
    let error_text = first_content_text(&result);
    assert!(
        error_text.starts_with("invalid oauth2 token_url:"),
        "propagated oauth exchange error should use token_url prefix contract: {error_text}"
    );
}
