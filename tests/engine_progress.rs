#![cfg(feature = "progress_utility")]

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, ErrorCode, Extensions, NumberOrString,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    config::{
        AuthConfig, ClientFeaturesConfig, ExecuteConfig, ExecutePluginConfig, McpConfig,
        PluginConfig, ProgressConfig, ServerSection, StreamableHttpTransportConfig, ToolConfig,
        ToolsConfig, TransportConfig, TransportMode,
    },
    default_http_client,
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginContext, PluginRegistry, PluginType, ToolPlugin},
};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

struct ProgressToolPlugin;

fn progress_result(result: Result<bool, McpError>) -> CallToolResult {
    match result {
        Ok(_) => CallToolResult::structured(Value::Null),
        Err(error) => CallToolResult::error(vec![Content::text(error.message)]),
    }
}

#[async_trait]
impl ToolPlugin for ProgressToolPlugin {
    fn name(&self) -> &'static str {
        "tool.progress"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("single");
        match mode {
            "single" => {
                let sent = ctx
                    .notify_progress(1.0, Some(2.0), Some("halfway".to_owned()))
                    .await?;
                Ok(CallToolResult::structured(
                    serde_json::json!({ "sent": sent }),
                ))
            }
            "burst" => {
                let first = ctx
                    .notify_progress(1.0, Some(2.0), Some("step-1".to_owned()))
                    .await?;
                let second = ctx
                    .notify_progress(2.0, Some(2.0), Some("step-2".to_owned()))
                    .await?;
                let sent_count = [first, second].into_iter().filter(|sent| *sent).count();
                Ok(CallToolResult::structured(
                    serde_json::json!({ "sent_count": sent_count }),
                ))
            }
            "non_increasing" => {
                let _ = ctx
                    .notify_progress(1.0, Some(2.0), Some("step-1".to_owned()))
                    .await?;
                Ok(progress_result(
                    ctx.notify_progress(1.0, Some(2.0), Some("step-1-again".to_owned()))
                        .await,
                ))
            }
            "nan_progress" => Ok(progress_result(
                ctx.notify_progress(f64::NAN, Some(2.0), Some("invalid".to_owned()))
                    .await,
            )),
            "nan_total" => Ok(progress_result(
                ctx.notify_progress(1.0, Some(f64::NAN), Some("invalid".to_owned()))
                    .await,
            )),
            _ => Err(McpError::invalid_params("unknown mode".to_owned(), None)),
        }
    }
}

fn base_config(progress: Option<ProgressConfig>) -> McpConfig {
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
        client_logging: None,
        progress,
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
                name: "tool.progress".to_owned(),
                title: None,
                description: "emit progress".to_owned(),
                cancellable: true,
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                    plugin: "tool.progress".to_owned(),
                    config: None,
                    task_support: rust_mcp_core::config::TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: vec![PluginConfig {
            name: "tool.progress".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }],
        outbound_http: None,
    }
}

fn request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    progress_token: Option<&str>,
) -> RequestContext<rmcp::service::RoleServer> {
    let mut meta = rmcp::model::Meta::default();
    if let Some(token) = progress_token {
        meta.set_progress_token(rmcp::model::ProgressToken(NumberOrString::String(
            token.to_owned().into(),
        )));
    }
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta,
        extensions: Extensions::default(),
    }
}

fn build_call(mode: &str) -> CallToolRequestParams {
    let mut arguments = serde_json::Map::new();
    arguments.insert("mode".to_owned(), Value::String(mode.to_owned()));
    CallToolRequestParams::new("tool.progress").with_arguments(arguments)
}

async fn read_frame(stream: &mut tokio::io::DuplexStream) -> Option<String> {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(Duration::from_millis(150), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
        _ => None,
    }
}

fn parse_notification_from_frame(frame: &str) -> Value {
    for line in frame.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return v;
            }
        }
    }
    serde_json::from_str(frame).expect("frame should contain valid JSON")
}

#[tokio::test]
async fn notify_progress_sends_when_enabled_and_token_present() {
    let config = base_config(Some(ProgressConfig {
        notification_interval_ms: 0,
    }));
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, Some("token-1"));
    let result = ServerHandler::call_tool(service.service(), build_call("single"), context)
        .await
        .expect("call");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({ "sent": true }))
    );

    let frame = read_frame(&mut client_io).await.expect("progress frame");
    let notification = parse_notification_from_frame(&frame);
    assert_eq!(
        notification["method"], "notifications/progress",
        "notification method must be notifications/progress"
    );
    let params = &notification["params"];
    assert_eq!(params["progressToken"], "token-1");
    assert_eq!(params["progress"], 1.0);
    assert_eq!(params["total"], 2.0);
    assert_eq!(params["message"], "halfway");
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_returns_false_when_progress_disabled() {
    let disabled_config = base_config(None);
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config: disabled_config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    // Progress capability should not be advertised when config is None.
    assert!(
        engine.get_info().capabilities.experimental.is_none()
            || !engine
                .get_info()
                .capabilities
                .experimental
                .as_ref()
                .is_some_and(|e| e.contains_key("progress")),
        "progress should not be advertised when disabled"
    );

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, Some("token-1"));
    let result = ServerHandler::call_tool(service.service(), build_call("single"), context)
        .await
        .expect("call");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({ "sent": false })),
        "disabled progress config must return sent=false even with a token"
    );
    assert!(
        read_frame(&mut client_io).await.is_none(),
        "no notification frame should be sent when progress is disabled"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_returns_false_when_token_missing() {
    let enabled_config = base_config(Some(ProgressConfig {
        notification_interval_ms: 0,
    }));
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config: enabled_config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    // No progress token in the request context
    let context = request_context(&service, None);
    let result = ServerHandler::call_tool(service.service(), build_call("single"), context)
        .await
        .expect("call");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({ "sent": false })),
        "missing progress token must return sent=false even when enabled"
    );
    assert!(
        read_frame(&mut client_io).await.is_none(),
        "no notification frame should be sent when token is missing"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_rate_limits_and_rejects_non_increasing_values() {
    let rate_limited_config = base_config(Some(ProgressConfig {
        notification_interval_ms: 60_000,
    }));
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config: rate_limited_config,
        plugins: registry.clone(),
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, Some("token-1"));
    let result = ServerHandler::call_tool(service.service(), build_call("burst"), context)
        .await
        .expect("call");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({ "sent_count": 1 }))
    );

    let first_frame = read_frame(&mut client_io).await.expect("first frame");
    let first_notification = parse_notification_from_frame(&first_frame);
    assert_eq!(first_notification["method"], "notifications/progress");
    assert_eq!(first_notification["params"]["progressToken"], "token-1");
    assert!(read_frame(&mut client_io).await.is_none());
    let _ = service.close().await;

    let non_increasing_config = base_config(Some(ProgressConfig {
        notification_interval_ms: 0,
    }));
    let engine = Engine::from_config(EngineConfig {
        config: non_increasing_config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");
    let (server_io, _client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, Some("token-1"));
    let result = ServerHandler::call_tool(service.service(), build_call("non_increasing"), context)
        .await
        .expect("tool result should be returned");
    assert_eq!(result.is_error, Some(true));
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(
        content[0]["text"].as_str().expect("error text"),
        "progress must increase with each notification",
        "non-increasing progress must produce exact error message"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_is_tracked_per_request_not_globally() {
    let config = base_config(Some(ProgressConfig {
        notification_interval_ms: 60_000,
    }));
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let first = ServerHandler::call_tool(
        service.service(),
        build_call("single"),
        request_context(&service, Some("token-1")),
    )
    .await
    .expect("first call");
    assert_eq!(
        first.structured_content,
        Some(serde_json::json!({ "sent": true }))
    );
    let first_frame = read_frame(&mut client_io).await.expect("first frame");
    let first_notification = parse_notification_from_frame(&first_frame);
    assert_eq!(first_notification["method"], "notifications/progress");
    assert_eq!(first_notification["params"]["progressToken"], "token-1");
    assert_eq!(first_notification["params"]["progress"], 1.0);
    assert_eq!(first_notification["params"]["message"], "halfway");

    let second = ServerHandler::call_tool(
        service.service(),
        build_call("single"),
        request_context(&service, Some("token-2")),
    )
    .await
    .expect("second call");
    assert_eq!(
        second.structured_content,
        Some(serde_json::json!({ "sent": true }))
    );
    let second_frame = read_frame(&mut client_io).await.expect("second frame");
    let second_notification = parse_notification_from_frame(&second_frame);
    assert_eq!(second_notification["method"], "notifications/progress");
    assert_eq!(second_notification["params"]["progressToken"], "token-2");
    assert_eq!(second_notification["params"]["progress"], 1.0);
    assert_eq!(second_notification["params"]["message"], "halfway");
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_rejects_non_finite_numbers() {
    let config = base_config(Some(ProgressConfig {
        notification_interval_ms: 0,
    }));
    let registry = PluginRegistry::new()
        .register_tool(ProgressToolPlugin)
        .expect("register plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");

    let (server_io, _client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let nan_progress = ServerHandler::call_tool(
        service.service(),
        build_call("nan_progress"),
        request_context(&service, Some("token-1")),
    )
    .await
    .expect("tool result should be returned");
    assert_eq!(nan_progress.is_error, Some(true));
    let nan_progress_content =
        serde_json::to_value(&nan_progress.content).expect("serialize content");
    assert_eq!(
        nan_progress_content[0]["text"]
            .as_str()
            .expect("error text"),
        "progress must be a finite number",
        "NaN progress must produce exact error message"
    );

    let nan_total = ServerHandler::call_tool(
        service.service(),
        build_call("nan_total"),
        request_context(&service, Some("token-2")),
    )
    .await
    .expect("tool result should be returned");
    assert_eq!(nan_total.is_error, Some(true));
    let nan_total_content = serde_json::to_value(&nan_total.content).expect("serialize content");
    assert_eq!(
        nan_total_content[0]["text"].as_str().expect("error text"),
        "total must be a finite number",
        "NaN total must produce exact error message"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn notify_progress_without_request_context_returns_false() {
    let context = PluginContext::new(None, Arc::new(HashMap::new()), default_http_client())
        .with_progress(true, 0);
    let sent = context
        .notify_progress(1.0, Some(1.0), Some("step".to_owned()))
        .await
        .expect("notify should not fail");
    assert!(!sent);
}

#[tokio::test]
async fn notify_progress_returns_internal_error_when_transport_send_fails() {
    let mut config = base_config(Some(ProgressConfig {
        notification_interval_ms: 0,
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
    let request = request_context(&service, Some("token-fail"));
    let context = PluginContext::new(
        Some(request),
        Arc::new(HashMap::new()),
        default_http_client(),
    )
    .with_progress(true, 0);

    let _ = service.close().await;
    let error = context
        .notify_progress(1.0, None, Some("after-close".to_owned()))
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
            .starts_with("failed to send progress notification:"),
        "error message must start with 'failed to send progress notification:', got: {}",
        error.message
    );
}
