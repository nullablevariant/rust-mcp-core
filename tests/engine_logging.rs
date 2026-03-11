#![cfg(feature = "client_logging")]

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ErrorCode, Extensions, LoggingLevel, Meta,
        NumberOrString, SetLevelRequestParams,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    config::{
        AuthConfig, ClientFeaturesConfig, ClientLoggingConfig, ExecuteConfig, ExecutePluginConfig,
        McpConfig, PluginConfig, ServerSection, StreamableHttpTransportConfig, ToolConfig,
        ToolsConfig, TransportConfig, TransportMode,
    },
    default_http_client,
    engine::{Engine, EngineConfig},
    plugins::{
        ClientLoggingState, LogChannel, LogEventParams, PluginCallParams, PluginContext,
        PluginRegistry, PluginType, ToolPlugin,
    },
};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

struct LoggingToolPlugin;

#[async_trait]
impl ToolPlugin for LoggingToolPlugin {
    fn name(&self) -> &'static str {
        "tool.logger"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let level = parse_level(args.get("level").and_then(Value::as_str).unwrap_or("info"));
        let data = args.get("data").cloned();
        params
            .ctx
            .log_event(LogEventParams {
                level,
                message: "tool log".to_owned(),
                data,
                channels: &[LogChannel::Client],
            })
            .await?;
        Ok(CallToolResult::structured(serde_json::json!({"ok": true})))
    }
}

fn parse_level(level: &str) -> LoggingLevel {
    match level {
        "debug" => LoggingLevel::Debug,
        "notice" => LoggingLevel::Notice,
        "warning" => LoggingLevel::Warning,
        "error" => LoggingLevel::Error,
        "critical" => LoggingLevel::Critical,
        "alert" => LoggingLevel::Alert,
        "emergency" => LoggingLevel::Emergency,
        _ => LoggingLevel::Info,
    }
}

fn base_config(logging: Option<ClientLoggingConfig>) -> McpConfig {
    McpConfig {
        version: 1,
        server: ServerSection {
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
        },
        client_logging: logging,
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
                name: "tool.log".to_owned(),
                title: None,
                description: "emit log notification".to_owned(),
                cancellable: true,
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                    plugin: "tool.logger".to_owned(),
                    config: None,
                    task_support: rust_mcp_core::config::TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: vec![PluginConfig {
            name: "tool.logger".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }],
        outbound_http: None,
    }
}

fn build_call(level: &str, data: Option<Value>) -> CallToolRequestParams {
    let mut arguments = serde_json::Map::new();
    arguments.insert("level".to_owned(), Value::String(level.to_owned()));
    if let Some(data) = data {
        arguments.insert("data".to_owned(), data);
    }
    CallToolRequestParams::new("tool.log").with_arguments(arguments)
}

fn request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
) -> RequestContext<rmcp::service::RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

async fn read_frame(stream: &mut tokio::io::DuplexStream) -> Option<String> {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(Duration::from_millis(150), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
        _ => None,
    }
}

fn parse_notification_from_frame(frame: &str) -> Value {
    // rmcp frames are newline-delimited JSON; find the first valid JSON object.
    for line in frame.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return v;
            }
        }
    }
    // Fallback: parse the whole frame
    serde_json::from_str(frame).expect("frame should contain valid JSON")
}

async fn call_tool_ok(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    call: CallToolRequestParams,
    context: RequestContext<rmcp::service::RoleServer>,
) -> CallToolResult {
    let result = ServerHandler::call_tool(service.service(), call, context)
        .await
        .expect("call_tool should succeed");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
    result
}

fn assert_notification_method(notification: &Value, expected_method: &str) {
    assert_eq!(notification["method"], expected_method);
}

#[tokio::test]
async fn logging_info_filtered_when_level_set_to_error() {
    let config = base_config(Some(ClientLoggingConfig {
        level: LoggingLevel::Info,
    }));
    let registry = PluginRegistry::new()
        .register_tool(LoggingToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");
    assert!(engine.get_info().capabilities.logging.is_some());

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service);
    ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Error),
        context.clone(),
    )
    .await
    .expect("set level");

    call_tool_ok(
        &service,
        build_call("info", Some(serde_json::json!({"kind": "filtered"}))),
        context.clone(),
    )
    .await;
    assert!(
        read_frame(&mut client_io).await.is_none(),
        "info below threshold must not produce frame"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn logging_notification_object_data_payload_contract() {
    let config = base_config(Some(ClientLoggingConfig {
        level: LoggingLevel::Info,
    }));
    let registry = PluginRegistry::new()
        .register_tool(LoggingToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service);
    ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Error),
        context.clone(),
    )
    .await
    .expect("set level");

    call_tool_ok(
        &service,
        build_call("error", Some(serde_json::json!({"kind": "object"}))),
        context,
    )
    .await;
    let frame = read_frame(&mut client_io)
        .await
        .expect("object notification frame");
    let n = parse_notification_from_frame(&frame);
    assert_notification_method(&n, "notifications/message");
    assert_eq!(n["params"]["level"], "error");
    assert_eq!(n["params"]["data"]["message"], "tool log");
    assert_eq!(n["params"]["data"]["kind"], "object");
    let _ = service.close().await;
}

#[tokio::test]
async fn logging_notification_string_data_payload_contract() {
    let config = base_config(Some(ClientLoggingConfig {
        level: LoggingLevel::Info,
    }));
    let registry = PluginRegistry::new()
        .register_tool(LoggingToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service);
    ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Error),
        context.clone(),
    )
    .await
    .expect("set level");

    call_tool_ok(
        &service,
        build_call("error", Some(Value::String("string payload".to_owned()))),
        context,
    )
    .await;
    let frame = read_frame(&mut client_io)
        .await
        .expect("string notification frame");
    let n = parse_notification_from_frame(&frame);
    assert_notification_method(&n, "notifications/message");
    assert_eq!(n["params"]["data"]["message"], "tool log");
    assert_eq!(n["params"]["data"]["details"], "string payload");
    let _ = service.close().await;
}

#[tokio::test]
async fn logging_notification_no_data_payload_contract() {
    let config = base_config(Some(ClientLoggingConfig {
        level: LoggingLevel::Info,
    }));
    let registry = PluginRegistry::new()
        .register_tool(LoggingToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service);
    ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Error),
        context.clone(),
    )
    .await
    .expect("set level");

    call_tool_ok(&service, build_call("error", None), context).await;
    let frame = read_frame(&mut client_io)
        .await
        .expect("empty notification frame");
    let n = parse_notification_from_frame(&frame);
    assert_notification_method(&n, "notifications/message");
    assert_eq!(
        n["params"]["data"], "tool log",
        "no-data payload should be the message string"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn set_level_is_noop_when_logging_disabled() {
    let mut config = base_config(None);
    config.tools_items_mut().clear();
    config.plugins.clear();
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::default(),
        list_refresh_handle: None,
    })
    .expect("engine");
    assert!(engine.get_info().capabilities.logging.is_none());

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service);
    // set_level returns Ok(()) when logging is disabled — a silent no-op.
    ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Debug),
        context,
    )
    .await
    .expect("set_level should succeed silently when logging is disabled");
    // Verify the capability remains absent — the no-op did not enable logging.
    assert!(
        service.service().get_info().capabilities.logging.is_none(),
        "logging capability must remain None after no-op set_level"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn plugin_context_log_event_handles_server_only_and_missing_context() {
    let context = PluginContext::new(None, Arc::new(HashMap::new()), default_http_client())
        .with_client_logging_state(ClientLoggingState::new(true, LoggingLevel::Debug));

    let result = context
        .log_event(LogEventParams {
            level: LoggingLevel::Info,
            message: "server and client without request".to_owned(),
            data: None,
            channels: &[LogChannel::Server, LogChannel::Client],
        })
        .await
        .expect("log event");
    assert!(result.server_logged);
    assert!(!result.client_notified);

    let empty = context
        .log_event(LogEventParams {
            level: LoggingLevel::Info,
            message: "no channels".to_owned(),
            data: None,
            channels: &[],
        })
        .await
        .expect("empty channels");
    assert!(!empty.server_logged);
    assert!(!empty.client_notified);
}

#[test]
fn client_logging_state_respects_threshold_updates() {
    let state = ClientLoggingState::new(true, LoggingLevel::Warning);
    assert_eq!(state.min_level(), LoggingLevel::Warning);
    assert!(!state.should_notify(LoggingLevel::Info));
    assert!(state.should_notify(LoggingLevel::Error));

    state.set_min_level(LoggingLevel::Debug);
    assert_eq!(state.min_level(), LoggingLevel::Debug);
    assert!(state.should_notify(LoggingLevel::Info));
}

#[tokio::test]
async fn plugin_context_log_event_server_channel_supports_all_levels() {
    use rust_mcp_core::plugins::LogResult;

    let context = PluginContext::new(None, Arc::new(HashMap::new()), default_http_client());
    let all_levels = [
        LoggingLevel::Debug,
        LoggingLevel::Info,
        LoggingLevel::Notice,
        LoggingLevel::Warning,
        LoggingLevel::Error,
        LoggingLevel::Critical,
        LoggingLevel::Alert,
        LoggingLevel::Emergency,
    ];
    for level in all_levels {
        let result = context
            .log_event(LogEventParams {
                level,
                message: "server only".to_owned(),
                data: Some(serde_json::json!({"kind": "server"})),
                channels: &[LogChannel::Server],
            })
            .await
            .expect("server log");
        assert_eq!(
            result,
            LogResult {
                server_logged: true,
                client_notified: false,
            },
            "server-only channel must set server_logged=true, client_notified=false for {level:?}"
        );
    }
    // Verify all 8 levels were tested
    assert_eq!(all_levels.len(), 8);
}

#[tokio::test]
async fn plugin_context_log_event_returns_internal_error_when_transport_send_fails() {
    let mut config = base_config(Some(ClientLoggingConfig {
        level: LoggingLevel::Debug,
    }));
    config.tools_items_mut().clear();
    config.plugins.clear();
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::new(),
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let request = request_context(&service);
    let context = PluginContext::new(
        Some(request),
        Arc::new(HashMap::new()),
        default_http_client(),
    )
    .with_client_logging_state(ClientLoggingState::new(true, LoggingLevel::Debug));

    let _ = service.close().await;
    let error = context
        .log_event(LogEventParams {
            level: LoggingLevel::Info,
            message: "client send after close".to_owned(),
            data: None,
            channels: &[LogChannel::Client],
        })
        .await
        .expect_err("send should fail after close");
    assert_eq!(
        error.code,
        ErrorCode::INTERNAL_ERROR,
        "transport send failure must produce INTERNAL_ERROR"
    );
    assert!(
        error
            .message
            .starts_with("failed to send logging notification:"),
        "error message must start with 'failed to send logging notification:', got: {}",
        error.message
    );
}
