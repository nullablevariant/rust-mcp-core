use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
#[cfg(feature = "client_logging")]
use rmcp::model::SetLevelRequestParams;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, Extensions, Meta, NumberOrString},
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
#[cfg(feature = "http_tools")]
use rust_mcp_core::config::UpstreamConfig;
#[cfg(feature = "client_logging")]
use rust_mcp_core::errors::CANCELLED_ERROR_CODE;
#[cfg(feature = "client_logging")]
use rust_mcp_core::errors::CANCELLED_ERROR_MESSAGE;
use rust_mcp_core::{
    config::{
        AuthConfig, ClientFeaturesConfig, ExecuteConfig, ExecutePluginConfig, McpConfig,
        PluginConfig, ServerSection, StreamableHttpTransportConfig, ToolConfig, ToolsConfig,
        TransportConfig, TransportMode,
    },
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, PluginType, ToolPlugin},
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "http_tools")]
use rust_mcp_core::config::ExecuteHttpConfig;

struct CancellationProbePlugin;

#[async_trait]
impl ToolPlugin for CancellationProbePlugin {
    fn name(&self) -> &'static str {
        "tool.cancel_probe"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        tokio::time::sleep(Duration::from_millis(25)).await;
        Ok(CallToolResult::structured(serde_json::json!({
            "token_cancelled": params.ctx.cancellation.is_cancelled()
        })))
    }
}

fn base_mcp() -> ServerSection {
    ServerSection {
        host: "127.0.0.1".to_owned(),
        port: 3000,
        endpoint_path: "/mcp".to_owned(),
        transport: TransportConfig {
            mode: TransportMode::StreamableHttp,
            streamable_http: StreamableHttpTransportConfig::default(),
        },
        auth: Some(AuthConfig::default()),
        errors: rust_mcp_core::config::ErrorExposureConfig::default(),
        logging: rust_mcp_core::config::ServerLoggingConfig::default(),
        response_limits: None,
        client_compat: rust_mcp_core::config::ClientCompatConfig::default(),
        info: None,
    }
}

fn build_call() -> CallToolRequestParams {
    CallToolRequestParams::new("tool.cancel_probe").with_arguments(serde_json::Map::new())
}

fn cancel_probe_config(cancellable: bool) -> McpConfig {
    McpConfig {
        version: 1,
        server: base_mcp(),
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::new(),
        tools: Some(ToolsConfig {
            enabled: None,
            notify_list_changed: false,
            items: vec![ToolConfig {
                name: "tool.cancel_probe".to_owned(),
                title: None,
                description: "probe cancellation".to_owned(),
                cancellable,
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                    plugin: "tool.cancel_probe".to_owned(),
                    config: None,
                    task_support: rust_mcp_core::config::TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: vec![PluginConfig {
            name: "tool.cancel_probe".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }],
        outbound_http: None,
    }
}

#[cfg(feature = "http_tools")]
fn http_cancel_probe_config() -> McpConfig {
    McpConfig {
        version: 1,
        server: base_mcp(),
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::from([(
            "api".to_owned(),
            UpstreamConfig {
                // TEST-NET-3 host keeps the HTTP future pending long enough for
                // cancellation to deterministically win the race in tokio::select!.
                base_url: "http://203.0.113.1:81".to_owned(),
                headers: HashMap::new(),
                user_agent: None,
                timeout_ms: Some(5_000),
                max_response_bytes: None,
                retry: None,
                auth: None,
            },
        )]),
        tools: Some(ToolsConfig {
            enabled: None,
            notify_list_changed: false,
            items: vec![ToolConfig {
                name: "tool.cancel_probe".to_owned(),
                title: None,
                description: "probe cancellation".to_owned(),
                cancellable: true,
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Http(ExecuteHttpConfig {
                    upstream: "api".to_owned(),
                    method: "GET".to_owned(),
                    path: "/slow".to_owned(),
                    query: None,
                    headers: None,
                    body: None,
                    retry: None,
                    task_support: rust_mcp_core::config::TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: Vec::new(),
        outbound_http: None,
    }
}

fn first_content_text(result: &CallToolResult) -> String {
    let block = result.content.first().expect("content[0] should exist");
    let value = serde_json::to_value(block).expect("content block should serialize");
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("content[0].text should exist")
}

fn request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    cancelled: bool,
) -> RequestContext<rmcp::service::RoleServer> {
    let token = CancellationToken::new();
    if cancelled {
        token.cancel();
    }
    RequestContext {
        peer: service.peer().clone(),
        ct: token,
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

#[tokio::test]
async fn cancellable_plugin_tool_returns_cancelled_tool_result() {
    let engine = Engine::from_config(EngineConfig {
        config: cancel_probe_config(true),
        plugins: PluginRegistry::new()
            .register_tool(CancellationProbePlugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::call_tool(
        service.service(),
        build_call(),
        request_context(&service, true),
    )
    .await
    .expect("cancelled result");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.content.len(),
        1,
        "cancelled result should have exactly one content block"
    );
    assert_eq!(
        first_content_text(&result),
        "request cancelled",
        "cancellation content should use exact cancellation message"
    );
    assert!(
        result.structured_content.is_none(),
        "cancelled result should have no structured content"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn non_cancellable_plugin_tool_ignores_request_cancellation() {
    let engine = Engine::from_config(EngineConfig {
        config: cancel_probe_config(false),
        plugins: PluginRegistry::new()
            .register_tool(CancellationProbePlugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::call_tool(
        service.service(),
        build_call(),
        request_context(&service, true),
    )
    .await
    .expect("tool result");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"token_cancelled": false}))
    );
    assert_eq!(result.is_error, Some(false));
    let _ = service.close().await;
}

#[tokio::test]
#[cfg(feature = "http_tools")]
async fn cancellable_http_tool_returns_cancelled_tool_result() {
    let engine = Engine::from_config(EngineConfig {
        config: http_cancel_probe_config(),
        plugins: PluginRegistry::new(),
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::call_tool(
        service.service(),
        build_call(),
        request_context(&service, true),
    )
    .await
    .expect("cancelled result");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.content.len(),
        1,
        "cancelled result: one content block"
    );
    assert_eq!(
        first_content_text(&result),
        "request cancelled",
        "HTTP cancellation should use exact cancellation message"
    );
    assert!(
        result.structured_content.is_none(),
        "no structured content on cancel"
    );
    let _ = service.close().await;
}

#[tokio::test]
#[cfg(feature = "client_logging")]
async fn non_tool_requests_use_custom_cancelled_error() {
    let config = McpConfig {
        version: 1,
        server: base_mcp(),
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::new(),
        tools: Some(ToolsConfig::default()),
        plugins: Vec::new(),
        outbound_http: None,
    };

    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::new(),
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let set_level_error = ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(rmcp::model::LoggingLevel::Info),
        request_context(&service, true),
    )
    .await
    .expect_err("set level should cancel");
    assert_eq!(set_level_error.code, CANCELLED_ERROR_CODE);
    assert_eq!(set_level_error.message, CANCELLED_ERROR_MESSAGE);

    let list_tools_error =
        ServerHandler::list_tools(service.service(), None, request_context(&service, true))
            .await
            .expect_err("list tools should cancel");
    assert_eq!(list_tools_error.code, CANCELLED_ERROR_CODE);
    assert_eq!(list_tools_error.message, CANCELLED_ERROR_MESSAGE);

    let _ = service.close().await;
}

#[test]
fn tool_cancellable_explicit_false_is_parsed() {
    let config: McpConfig = serde_yaml::from_str(
        r"
version: 1
server:
  host: 127.0.0.1
  port: 3000
  endpoint_path: /mcp
  logging:
    level: info
tools:
  items:
    - name: tool.cancel_probe
      description: probe cancellation
      cancellable: false
      input_schema:
        type: object
      execute:
        type: http
        upstream: api
        method: GET
        path: /
",
    )
    .expect("parse config");
    assert!(
        !config.tools_items()[0].cancellable,
        "explicit false must parse as false"
    );
}

#[test]
fn tool_cancellable_defaults_to_true_when_omitted() {
    let config: McpConfig = serde_yaml::from_str(
        r"
version: 1
server:
  host: 127.0.0.1
  port: 3000
  endpoint_path: /mcp
  logging:
    level: info
tools:
  items:
    - name: tool.cancel_probe
      description: probe cancellation
      input_schema:
        type: object
      execute:
        type: http
        upstream: api
        method: GET
        path: /
",
    )
    .expect("parse config");
    assert!(config.tools_items()[0].cancellable);
}
