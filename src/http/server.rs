//! Axum server startup and graceful shutdown for streamable HTTP transport.
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::ErrorData as McpError;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::{StreamableHttpSessionMode, TransportConfig};
use crate::plugins::PluginRegistry;
use crate::{AuthState, Engine};

use super::router::build_streamable_http_router;

pub(crate) async fn run_streamable_http(
    engine: Arc<Engine>,
    auth_state: Arc<AuthState>,
    plugins: Arc<PluginRegistry>,
) -> Result<(), McpError> {
    let app = build_streamable_http_router(Arc::clone(&engine), &auth_state, &plugins)?;

    let bind_address = format!(
        "{}:{}",
        engine.config.server.host, engine.config.server.port
    );
    info!("starting streamable HTTP on {}", bind_address);
    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

    let shutdown = streamable_http_shutdown_signal();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| McpError::internal_error(format!("server error: {e}"), None))?;
    Ok(())
}

pub(crate) fn streamable_http_shutdown_signal() -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
{
    if cfg!(debug_assertions) && std::env::var("MCP_TEST_HTTP_SHUTDOWN").is_ok() {
        return Box::pin(std::future::ready(()));
    }
    Box::pin(std::future::pending())
}

pub(crate) fn build_streamable_http_config(
    transport: &TransportConfig,
) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig {
        cancellation_token: CancellationToken::new(),
        ..Default::default()
    };
    if let Some(ms) = transport.streamable_http.sse_keep_alive_ms {
        config.sse_keep_alive = Some(Duration::from_millis(ms));
    }
    if let Some(ms) = transport.streamable_http.sse_retry_ms {
        config.sse_retry = Some(Duration::from_millis(ms));
    }
    config.stateful_mode = !matches!(
        transport.streamable_http.session_mode,
        StreamableHttpSessionMode::None
    );
    config
}

#[cfg(test)]
mod tests {
    use super::{build_streamable_http_config, streamable_http_shutdown_signal};
    use crate::config::{
        ProtocolVersionNegotiationConfig, StreamableHttpSessionMode, StreamableHttpTransportConfig,
        TransportConfig,
    };
    use crate::inline_test_fixtures::{clear_env, set_env};
    use std::time::Duration;

    #[test]
    fn build_streamable_http_config_applies_overrides() {
        let transport = TransportConfig {
            mode: crate::config::TransportMode::StreamableHttp,
            streamable_http: StreamableHttpTransportConfig {
                enable_get_stream: true,
                enable_sse_resumption: false,
                session_mode: StreamableHttpSessionMode::None,
                allow_delete_session: false,
                sse_keep_alive_ms: Some(2500),
                sse_retry_ms: Some(750),
                protocol_version_negotiation: ProtocolVersionNegotiationConfig::default(),
                hardening: None,
            },
        };
        let config = build_streamable_http_config(&transport);
        assert_eq!(config.sse_keep_alive, Some(Duration::from_millis(2500)));
        assert_eq!(config.sse_retry, Some(Duration::from_millis(750)));
        assert!(!config.stateful_mode);
    }

    #[test]
    fn build_streamable_http_config_defaults_unset_fields() {
        let transport = TransportConfig::default();
        let config = build_streamable_http_config(&transport);
        assert_eq!(config.sse_keep_alive, Some(Duration::from_secs(15)));
        assert_eq!(config.sse_retry, Some(Duration::from_secs(3)));
        assert!(config.stateful_mode);
    }

    #[tokio::test]
    async fn streamable_http_shutdown_signal_defaults_to_pending() {
        let _guard = clear_env("MCP_TEST_HTTP_SHUTDOWN");
        let shutdown = streamable_http_shutdown_signal();
        let waited = tokio::time::timeout(Duration::from_millis(5), shutdown).await;
        assert!(
            waited.is_err(),
            "shutdown signal should remain pending when env is unset"
        );
    }

    #[tokio::test]
    async fn streamable_http_shutdown_signal_completes_when_test_env_set() {
        let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");
        let shutdown = streamable_http_shutdown_signal();
        let waited = tokio::time::timeout(Duration::from_millis(5), shutdown).await;
        assert!(
            waited.is_ok(),
            "shutdown signal should resolve immediately when test env is set"
        );
    }
}
