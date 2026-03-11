#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::{
    model::{
        CallToolResult, ClientCapabilities, ClientInfo, Content, LoggingMessageNotificationParam,
        ProgressNotificationParam,
    },
    service::{RoleClient, RunningService},
    ClientHandler, ErrorData as McpError, ServiceExt,
};
#[cfg(feature = "http_tools")]
use rust_mcp_core::config::{ToolsConfig, UpstreamConfig};
use rust_mcp_core::{
    config::{
        AuthConfig, ClientFeaturesConfig, ExecuteConfig, ExecuteHttpConfig, ExecutePluginConfig,
        McpConfig, PluginConfig, ServerSection, TaskSupport, ToolConfig, TransportConfig,
        TransportMode,
    },
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, PluginType, ToolPlugin},
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// SmokeTestClient
// ---------------------------------------------------------------------------

pub(crate) struct SmokeTestClient {
    info: ClientInfo,
    pub state: Arc<Mutex<SmokeTestClientState>>,
    roots: Vec<rmcp::model::Root>,
    sampling_response: Option<rmcp::model::CreateMessageResult>,
    #[cfg(feature = "client_features")]
    elicitation_response: Option<rmcp::model::CreateElicitationResult>,
}

#[derive(Default)]
pub(crate) struct SmokeTestClientState {
    pub progress_notifications: Vec<ProgressNotificationParam>,
    pub logging_messages: Vec<LoggingMessageNotificationParam>,
    #[allow(dead_code)]
    pub tool_list_changed_count: usize,
}

impl SmokeTestClient {
    pub(crate) fn new() -> Self {
        Self {
            info: ClientInfo::default(),
            state: Arc::new(Mutex::new(SmokeTestClientState::default())),
            roots: Vec::new(),
            sampling_response: None,
            #[cfg(feature = "client_features")]
            elicitation_response: None,
        }
    }

    pub(crate) fn with_capabilities(mut self, capabilities: ClientCapabilities) -> Self {
        self.info.capabilities = capabilities;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_roots(mut self, roots: Vec<rmcp::model::Root>) -> Self {
        self.roots = roots;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_sampling_response(
        mut self,
        response: rmcp::model::CreateMessageResult,
    ) -> Self {
        self.sampling_response = Some(response);
        self
    }

    #[cfg(feature = "client_features")]
    pub(crate) fn with_elicitation_response(
        mut self,
        response: rmcp::model::CreateElicitationResult,
    ) -> Self {
        self.elicitation_response = Some(response);
        self
    }
}

impl ClientHandler for SmokeTestClient {
    fn get_info(&self) -> ClientInfo {
        self.info.clone()
    }

    fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ListRootsResult, McpError>> + Send + '_
    {
        let roots = self.roots.clone();
        async move {
            let mut result = rmcp::model::ListRootsResult::default();
            result.roots = roots;
            Ok(result)
        }
    }

    fn create_message(
        &self,
        _params: rmcp::model::CreateMessageRequestParams,
        _context: rmcp::service::RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::CreateMessageResult, McpError>> + Send + '_
    {
        let response = self.sampling_response.clone();
        async move {
            response.ok_or_else(|| {
                McpError::internal_error("no sampling response configured".to_owned(), None)
            })
        }
    }

    #[cfg(feature = "client_features")]
    fn create_elicitation(
        &self,
        _request: rmcp::model::CreateElicitationRequestParams,
        _context: rmcp::service::RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::CreateElicitationResult, McpError>>
           + Send
           + '_ {
        let response = self.elicitation_response.clone();
        async move {
            response.ok_or_else(|| {
                McpError::internal_error("no elicitation response configured".to_owned(), None)
            })
        }
    }

    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let state = Arc::clone(&self.state);
        async move {
            state.lock().await.progress_notifications.push(params);
        }
    }

    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let state = Arc::clone(&self.state);
        async move {
            state.lock().await.logging_messages.push(params);
        }
    }

    fn on_tool_list_changed(
        &self,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let state = Arc::clone(&self.state);
        async move {
            state.lock().await.tool_list_changed_count += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Test Plugins
// ---------------------------------------------------------------------------

pub(crate) struct EchoPlugin;

#[async_trait]
impl ToolPlugin for EchoPlugin {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(args))
    }
}

pub(crate) struct SleepPlugin;

#[async_trait]
impl ToolPlugin for SleepPlugin {
    fn name(&self) -> &'static str {
        "sleep"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ms = args.get("ms").and_then(Value::as_u64).unwrap_or(10000);
        tokio::select! {
            () = params.ctx.cancellation.cancelled() => {
                Ok(CallToolResult::error(vec![Content::text("cancelled")]))
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {
                Ok(CallToolResult::structured(json!({"slept_ms": ms})))
            }
        }
    }
}

pub(crate) struct ProgressPlugin;

#[async_trait]
impl ToolPlugin for ProgressPlugin {
    fn name(&self) -> &'static str {
        "progress"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        params
            .ctx
            .notify_progress(25.0, Some(100.0), Some("step 1".to_owned()))
            .await?;
        params
            .ctx
            .notify_progress(50.0, Some(100.0), Some("step 2".to_owned()))
            .await?;
        params
            .ctx
            .notify_progress(100.0, Some(100.0), Some("done".to_owned()))
            .await?;
        Ok(CallToolResult::structured(json!({"done": true})))
    }
}

#[cfg(feature = "client_features")]
pub(crate) struct ClientFeaturesPlugin;

#[cfg(feature = "client_features")]
#[async_trait]
impl ToolPlugin for ClientFeaturesPlugin {
    fn name(&self) -> &'static str {
        "client_features"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("roots");
        match mode {
            "roots" => {
                let result = ctx.request_roots().await?;
                Ok(CallToolResult::structured(
                    json!({"roots": result.roots.len()}),
                ))
            }
            "sampling" => {
                let params = rmcp::model::CreateMessageRequestParams::new(
                    vec![rmcp::model::SamplingMessage::user_text("Hello")],
                    100,
                );
                let result = ctx.request_sampling(params).await?;
                Ok(CallToolResult::structured(
                    json!({"model": result.model, "role": format!("{:?}", result.message.role)}),
                ))
            }
            "elicitation" => {
                let params = rmcp::model::CreateElicitationRequestParams::FormElicitationParams {
                    meta: None,
                    message: "provide input".to_owned(),
                    requested_schema: rmcp::model::ElicitationSchema::builder()
                        .required_string("name")
                        .build()
                        .expect("schema"),
                };
                let result = ctx.request_elicitation(params).await?;
                Ok(CallToolResult::structured(
                    json!({"action": format!("{:?}", result.action)}),
                ))
            }
            _ => Err(McpError::invalid_params(
                format!("unknown mode: {mode}"),
                None,
            )),
        }
    }
}

pub(crate) struct FailPlugin;

#[async_trait]
impl ToolPlugin for FailPlugin {
    fn name(&self) -> &'static str {
        "fail"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError {
            code: rmcp::model::ErrorCode::INTERNAL_ERROR,
            message: "plugin failure".into(),
            data: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn make_minimal_config() -> McpConfig {
    #[cfg(feature = "http_tools")]
    let upstreams = std::collections::HashMap::from([(
        "noop".to_owned(),
        UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: std::collections::HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: None,
        },
    )]);
    #[cfg(not(feature = "http_tools"))]
    let upstreams = std::collections::HashMap::new();

    #[cfg(feature = "http_tools")]
    let tools = Some(ToolsConfig {
        enabled: None,
        notify_list_changed: false,
        items: vec![make_noop_tool()],
    });
    #[cfg(not(feature = "http_tools"))]
    let tools = None;

    McpConfig {
        version: 1,
        server: ServerSection {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            endpoint_path: "/mcp".to_owned(),
            transport: TransportConfig {
                mode: TransportMode::Stdio,
                ..TransportConfig::default()
            },
            auth: Some(AuthConfig {
                enabled: Some(false),
                ..AuthConfig::default()
            }),
            errors: rust_mcp_core::config::ErrorExposureConfig::default(),
            logging: rust_mcp_core::config::ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: rust_mcp_core::config::ClientCompatConfig::default(),
            info: None,
        },
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams,
        tools,
        plugins: Vec::new(),
        outbound_http: None,
    }
}

pub(crate) fn make_noop_tool() -> ToolConfig {
    ToolConfig {
        name: "noop".to_owned(),
        title: None,
        description: "no-op tool".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Http(ExecuteHttpConfig {
            upstream: "noop".to_owned(),
            method: "GET".to_owned(),
            path: "/".to_owned(),
            query: None,
            headers: None,
            body: None,
            retry: None,
            task_support: TaskSupport::Forbidden,
        }),
        response: None,
    }
}

pub(crate) fn make_plugin_tool(name: &str, plugin_name: &str) -> ToolConfig {
    ToolConfig {
        name: name.to_owned(),
        title: None,
        description: format!("{name} tool"),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: plugin_name.to_owned(),
            config: None,
            task_support: TaskSupport::Forbidden,
        }),
        response: None,
    }
}

pub(crate) fn make_plugin_allowlist(name: &str) -> PluginConfig {
    PluginConfig {
        name: name.to_owned(),
        plugin_type: PluginType::Tool,
        targets: None,
        config: None,
    }
}

pub(crate) async fn spawn_e2e(
    engine: Engine,
    client: SmokeTestClient,
) -> (
    RunningService<RoleClient, SmokeTestClient>,
    tokio::task::JoinHandle<()>,
) {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let server_handle = tokio::spawn(async move {
        let server = engine
            .serve(server_transport)
            .await
            .expect("server handshake should succeed");
        // Keep the server running until it closes naturally
        let _ = server.waiting().await;
    });

    let client_service = client
        .serve(client_transport)
        .await
        .expect("client handshake should succeed");

    (client_service, server_handle)
}

pub(crate) fn build_engine(config: McpConfig, registry: PluginRegistry) -> Engine {
    Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build")
}
