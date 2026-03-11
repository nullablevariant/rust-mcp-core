//! Shared test fixtures for inline #[cfg(test)] modules.
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "streamable_http")]
use axum::body::Body;
#[cfg(feature = "streamable_http")]
use axum::http::{HeaderValue, Request};
#[cfg(feature = "streamable_http")]
use axum::middleware::Next;
#[cfg(feature = "streamable_http")]
use axum::response::Response;
#[cfg(feature = "streamable_http")]
use axum::routing::get;
#[cfg(feature = "streamable_http")]
use axum::Router;
#[cfg(feature = "streamable_http")]
use rmcp::model::{Extensions, Meta, NumberOrString};
#[cfg(feature = "streamable_http")]
use rmcp::service::RequestContext;
use rmcp::ErrorData as McpError;
#[cfg(any(feature = "streamable_http", feature = "http_tools"))]
use serde_json::json;
#[cfg(feature = "streamable_http")]
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncReadExt;
#[cfg(feature = "streamable_http")]
use tokio_util::sync::CancellationToken;

#[cfg(feature = "streamable_http")]
use crate::build_auth_state_with_plugins;
#[cfg(feature = "auth")]
use crate::config::AuthProviderConfig;
use crate::config::{
    AuthConfig, HttpRouterTargetType, McpConfig, PluginConfig, PluginTargetConfig, ServerSection,
    StreamableHttpTransportConfig, ToolsConfig, TransportConfig, TransportMode,
};
#[cfg(feature = "http_tools")]
use crate::config::{ExecuteConfig, ExecuteHttpConfig, TaskSupport, ToolConfig, UpstreamConfig};
#[cfg(feature = "streamable_http")]
use crate::http::router::build_streamable_http_router;
#[cfg(feature = "streamable_http")]
use crate::plugins::http_router::{
    HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RouterTransform, RuntimeContext,
};
#[cfg(feature = "streamable_http")]
use crate::plugins::{PluginCallParams, ToolPlugin};
use crate::plugins::{PluginRegistry, PluginType};
use crate::Engine;
#[cfg(feature = "streamable_http")]
use crate::EngineConfig;

#[cfg(feature = "streamable_http")]
use async_trait::async_trait;

pub(crate) fn base_config() -> McpConfig {
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
            auth: Some(AuthConfig {
                enabled: Some(false),
                ..AuthConfig::default()
            }),
            errors: crate::config::ErrorExposureConfig::default(),
            logging: crate::config::ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: crate::config::ClientCompatConfig::default(),
            info: None,
        },
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: crate::config::ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::default(),
        tools: Some(ToolsConfig::default()),
        plugins: Vec::new(),
        outbound_http: None,
    }
}

pub(crate) fn stdio_base_config() -> McpConfig {
    let mut config = base_config();
    config.server.transport.mode = TransportMode::Stdio;
    config.server.auth = Some(AuthConfig {
        enabled: Some(false),
        ..AuthConfig::default()
    });
    config.upstreams.clear();
    config.tools = None;
    config
}

#[cfg(feature = "http_tools")]
pub(crate) fn stdio_http_tool_config() -> McpConfig {
    let mut config = stdio_base_config();
    config.upstreams.insert(
        "noop".to_owned(),
        UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: None,
        },
    );
    config.tools = Some(ToolsConfig {
        enabled: None,
        notify_list_changed: false,
        items: vec![ToolConfig {
            name: "noop".to_owned(),
            title: None,
            description: "No-op tool".to_owned(),
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
        }],
    });
    config
}

#[cfg(feature = "auth")]
pub(crate) fn base_provider() -> AuthProviderConfig {
    let mut provider = AuthProviderConfig::jwks("test");
    provider.as_jwks_mut().expect("jwks provider").issuer =
        Some("https://issuer.example".to_owned());
    provider
}

pub(crate) struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

static ENV_MUTEX: Mutex<()> = Mutex::new(());

pub(crate) fn set_env(key: &'static str, value: &str) -> EnvGuard {
    let lock = ENV_MUTEX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = std::env::var(key).ok();
    std::env::set_var(key, value);
    EnvGuard {
        key,
        previous,
        _lock: lock,
    }
}

pub(crate) fn clear_env(key: &'static str) -> EnvGuard {
    let lock = ENV_MUTEX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = std::env::var(key).ok();
    std::env::remove_var(key);
    EnvGuard {
        key,
        previous,
        _lock: lock,
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) fn request_context(
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

pub(crate) async fn read_frame(stream: &mut tokio::io::DuplexStream) -> Option<String> {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(Duration::from_millis(250), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
        _ => None,
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) async fn read_frame_with_timeout(
    stream: &mut tokio::io::DuplexStream,
    timeout: Duration,
) -> Option<String> {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(timeout, stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
        _ => None,
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct RegistryMutationTool {
    pub(crate) name: &'static str,
}

#[cfg(feature = "streamable_http")]
#[async_trait]
impl ToolPlugin for RegistryMutationTool {
    fn name(&self) -> &str {
        self.name
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        Ok(rmcp::model::CallToolResult::structured(
            json!({ "ok": true }),
        ))
    }
}

pub(crate) async fn ok_http_hook(
    _engine: Arc<Engine>,
    _auth: Arc<crate::AuthState>,
    _plugins: Arc<PluginRegistry>,
) -> Result<(), McpError> {
    Ok(())
}

pub(crate) async fn ok_stdio_hook(_engine: Arc<Engine>) -> Result<(), McpError> {
    Ok(())
}

#[cfg(feature = "streamable_http")]
pub(crate) fn wrap_target(path: &str) -> PluginTargetConfig {
    PluginTargetConfig {
        target_type: HttpRouterTargetType::Wrap,
        path: path.to_owned(),
    }
}

pub(crate) fn route_target(path: &str) -> PluginTargetConfig {
    PluginTargetConfig {
        target_type: HttpRouterTargetType::Route,
        path: path.to_owned(),
    }
}

pub(crate) fn router_router_plugin_config(
    name: &str,
    targets: Vec<PluginTargetConfig>,
) -> PluginConfig {
    PluginConfig {
        name: name.to_owned(),
        plugin_type: PluginType::HttpRouter,
        targets: Some(targets),
        config: None,
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) fn wrap_with_label(label: &'static str) -> RouterTransform {
    Arc::new(move |router: Router| {
        let label = label.to_owned();
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: Next| {
                let label = label.clone();
                async move {
                    let mut response: Response = next.run(request).await;
                    let existing = response
                        .headers()
                        .get("x-order")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let combined = format!("{existing}{label}");
                    response
                        .headers_mut()
                        .insert("x-order", HeaderValue::from_str(&combined).expect("header"));
                    response
                }
            },
        ))
    })
}

#[cfg(feature = "streamable_http")]
pub(crate) struct TestPlugin {
    pub(crate) name: &'static str,
    pub(crate) label: &'static str,
}

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for TestPlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for target in targets {
            match target.target_type {
                HttpRouterTargetType::Wrap => {
                    ops.push(HttpRouterOp::Wrap(wrap_with_label(self.label)));
                }
                HttpRouterTargetType::Route => {
                    let router = Router::new().route("/", get(|| async { "ok" }));
                    ops.push(HttpRouterOp::Route(router));
                }
            }
        }
        Ok(ops)
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct ConfigCheckPlugin;

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for ConfigCheckPlugin {
    fn name(&self) -> &'static str {
        "config_check"
    }

    // Test infrastructure — panicking on setup failure is intentional.
    #[allow(clippy::panic_in_result_fn)]
    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let object = config.as_object().expect("config must be object");
        assert_eq!(object.get("base"), Some(&json!(true)));
        assert_eq!(object.get("limit"), Some(&json!(120)));
        assert_eq!(object.get("mode"), Some(&json!("override")));

        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            let router = Router::new().route("/", get(|| async { "ok" }));
            ops.push(HttpRouterOp::Route(router));
        }
        Ok(ops)
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct AuthWrapPlugin;

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for AuthWrapPlugin {
    fn name(&self) -> &'static str {
        "auth_wrap"
    }

    fn apply(
        &self,
        ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for target in targets {
            match target.target_type {
                HttpRouterTargetType::Route => {
                    let router = Router::new().route("/", get(|| async { "ok" }));
                    ops.push(HttpRouterOp::Route(router));
                }
                HttpRouterTargetType::Wrap => {
                    let wrap = ctx.auth_wrap().ok_or_else(|| {
                        McpError::invalid_request(
                            "auth_wrap requested with auth disabled".to_owned(),
                            None,
                        )
                    })?;
                    ops.push(HttpRouterOp::Wrap(wrap));
                }
            }
        }
        Ok(ops)
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct MismatchedOpsPlugin;

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for MismatchedOpsPlugin {
    fn name(&self) -> &'static str {
        "mismatch"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        _targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        Ok(Vec::new())
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct WrongRouteOpPlugin;

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for WrongRouteOpPlugin {
    fn name(&self) -> &'static str {
        "wrong_route"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            ops.push(HttpRouterOp::Wrap(wrap_with_label("W")));
        }
        Ok(ops)
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) struct WrongWrapOpPlugin;

#[cfg(feature = "streamable_http")]
impl HttpRouterPlugin for WrongWrapOpPlugin {
    fn name(&self) -> &'static str {
        "wrong_wrap"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            let router = Router::new().route("/", get(|| async { "ok" }));
            ops.push(HttpRouterOp::Route(router));
        }
        Ok(ops)
    }
}

#[cfg(feature = "streamable_http")]
pub(crate) fn build_router(
    config: &McpConfig,
    registry: PluginRegistry,
) -> Result<Router, McpError> {
    let engine = Engine::from_config(EngineConfig {
        config: config.clone(),
        plugins: PluginRegistry::default(),
        list_refresh_handle: None,
    })?;
    let auth_state = build_auth_state_with_plugins(config, None)?;
    build_streamable_http_router(Arc::new(engine), &auth_state, &Arc::new(registry))
}

#[cfg(test)]
#[derive(Clone)]
struct TestLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

#[cfg(test)]
impl std::io::Write for TestLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn capture_logs<F>(level: tracing::Level, operation: F) -> String
where
    F: FnOnce(),
{
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer_buffer = std::sync::Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .without_time()
        .with_ansi(false)
        .with_writer(move || TestLogWriter(std::sync::Arc::clone(&writer_buffer)))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    operation();
    let bytes = buffer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
#[cfg_attr(not(feature = "auth"), allow(dead_code))]
pub(crate) async fn capture_logs_async<F, Fut>(level: tracing::Level, operation: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer_buffer = std::sync::Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .without_time()
        .with_ansi(false)
        .with_writer(move || TestLogWriter(std::sync::Arc::clone(&writer_buffer)))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    operation().await;
    let bytes = buffer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}
