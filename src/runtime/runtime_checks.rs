//! Runtime startup checks for transport/feature compatibility and logging setup.

use std::sync::OnceLock;

use crate::config::feature_validation::shared_feature_validation_error;
use crate::config::{McpConfig, TransportMode};
use crate::plugins::PluginType;
use crate::McpError;
use tracing::warn;
use tracing_subscriber::{filter::LevelFilter, layer::SubscriberExt, reload, Registry};

type LogLevelReloadHandle = reload::Handle<LevelFilter, Registry>;

static LOG_LEVEL_RELOAD_HANDLE: OnceLock<LogLevelReloadHandle> = OnceLock::new();

pub(super) fn init_logging(level: &str) -> Result<(), McpError> {
    let level = parse_log_level(level)?;
    let level_filter = LevelFilter::from_level(level);

    if let Some(handle) = LOG_LEVEL_RELOAD_HANDLE.get() {
        return handle
            .modify(|filter| *filter = level_filter)
            .map_err(|error| {
                McpError::internal_error(format!("failed to update logging level: {error}"), None)
            });
    }

    let (reload_layer, reload_handle) = reload::Layer::new(level_filter);
    let subscriber = tracing_subscriber::registry()
        .with(reload_layer)
        .with(tracing_subscriber::fmt::layer());

    match tracing::subscriber::set_global_default(subscriber) {
        Ok(()) => {
            let _ = LOG_LEVEL_RELOAD_HANDLE.set(reload_handle);
        }
        Err(error) => {
            if let Some(handle) = LOG_LEVEL_RELOAD_HANDLE.get() {
                handle
                    .modify(|filter| *filter = level_filter)
                    .map_err(|reload_error| {
                        McpError::internal_error(
                            format!("failed to update logging level after init conflict: {reload_error}"),
                            None,
                        )
                    })?;
                warn!("global tracing subscriber already initialized; updated log level via reload handle");
            } else {
                // TODO: Use tracing_subscriber::reload handle from host integration when available.
                warn!(
                    "global tracing subscriber already initialized by external code; runtime log level '{}' was not applied ({})",
                    level,
                    error
                );
            }
        }
    }
    Ok(())
}

pub(super) fn parse_log_level(level: &str) -> Result<tracing::Level, McpError> {
    match level.to_lowercase().as_str() {
        "trace" => Ok(tracing::Level::TRACE),
        "debug" => Ok(tracing::Level::DEBUG),
        "info" => Ok(tracing::Level::INFO),
        "warn" => Ok(tracing::Level::WARN),
        "error" => Ok(tracing::Level::ERROR),
        _ => Err(McpError::invalid_request(
            format!("invalid log level: {level}"),
            None,
        )),
    }
}

pub(super) fn compiled_feature_validation_error(config: &McpConfig) -> Option<McpError> {
    if let Some(error) = shared_feature_validation_error(config) {
        return Some(error);
    }

    #[cfg(not(feature = "auth"))]
    {
        if config.server.auth_active() {
            return Some(McpError::invalid_request(
                "auth feature disabled but server.auth is active".to_owned(),
                None,
            ));
        }
    }

    #[cfg(not(feature = "streamable_http"))]
    {
        if config.server.transport.mode == TransportMode::StreamableHttp {
            return Some(McpError::invalid_request(
                "streamable_http feature disabled but server.transport.mode=streamable_http"
                    .to_owned(),
                None,
            ));
        }
        if config
            .plugins
            .iter()
            .any(|plugin| plugin.plugin_type == PluginType::HttpRouter)
        {
            return Some(McpError::invalid_request(
                "streamable_http feature disabled but plugins include type=http_router".to_owned(),
                None,
            ));
        }
    }

    None
}

pub(super) fn validate_transport(config: &McpConfig) -> Result<TransportMode, McpError> {
    let mode = config.server.transport.mode;
    match mode {
        TransportMode::Stdio => validate_stdio_transport(config)?,
        TransportMode::StreamableHttp => {
            #[cfg(feature = "streamable_http")]
            {
                validate_streamable_http_transport(config)?;
            }
            #[cfg(not(feature = "streamable_http"))]
            {
                return Err(McpError::invalid_request(
                    "streamable_http feature disabled but server.transport.mode=streamable_http"
                        .to_owned(),
                    None,
                ));
            }
        }
    }
    Ok(mode)
}

fn validate_stdio_transport(config: &McpConfig) -> Result<(), McpError> {
    if config.server.auth_active() {
        return Err(McpError::invalid_request(
            "stdio transport requires auth to be disabled".to_owned(),
            None,
        ));
    }
    let has_http_router = config
        .plugins
        .iter()
        .any(|plugin| plugin.plugin_type == PluginType::HttpRouter);
    if has_http_router {
        return Err(McpError::invalid_request(
            "http_router plugins require streamable_http transport".to_owned(),
            None,
        ));
    }
    Ok(())
}

#[cfg(feature = "streamable_http")]
fn validate_streamable_http_transport(config: &McpConfig) -> Result<(), McpError> {
    use crate::config::StreamableHttpSessionMode;

    if config.server.host.trim().is_empty() {
        return Err(McpError::invalid_request(
            "server.host is required for streamable_http".to_owned(),
            None,
        ));
    }
    if config.server.endpoint_path.trim().is_empty() {
        return Err(McpError::invalid_request(
            "server.endpoint_path is required for streamable_http".to_owned(),
            None,
        ));
    }

    let streamable = &config.server.transport.streamable_http;
    if !streamable.enable_get_stream && streamable.enable_sse_resumption {
        return Err(McpError::invalid_request(
            "transport.streamable_http.enable_sse_resumption requires enable_get_stream=true"
                .to_owned(),
            None,
        ));
    }
    if streamable.session_mode == StreamableHttpSessionMode::None {
        validate_streamable_stateless_constraints(streamable)?;
    }
    validate_streamable_hardening_rate_limits(streamable)?;

    Ok(())
}

#[cfg(feature = "streamable_http")]
fn validate_streamable_hardening_rate_limits(
    streamable: &crate::config::StreamableHttpTransportConfig,
) -> Result<(), McpError> {
    let hardening = streamable.hardening.as_ref();
    if let Some(rate_limit) = hardening.and_then(|candidate| candidate.rate_limit.as_ref()) {
        if rate_limit.is_active() && !rate_limit.has_any_bucket() {
            return Err(McpError::invalid_request(
                "transport.streamable_http.hardening.rate_limit requires global or per_ip when enabled"
                    .to_owned(),
                None,
            ));
        }
    }

    if let Some(creation_rate) = hardening
        .and_then(|candidate| candidate.session.as_ref())
        .and_then(|session| session.creation_rate.as_ref())
    {
        if creation_rate.is_active() && !creation_rate.has_any_bucket() {
            return Err(McpError::invalid_request(
                "transport.streamable_http.hardening.session.creation_rate requires global or per_ip when enabled"
                    .to_owned(),
                None,
            ));
        }
    }

    Ok(())
}

#[cfg(feature = "streamable_http")]
fn validate_streamable_stateless_constraints(
    streamable: &crate::config::StreamableHttpTransportConfig,
) -> Result<(), McpError> {
    if streamable.enable_sse_resumption {
        return Err(McpError::invalid_request(
            "transport.streamable_http.enable_sse_resumption requires session_mode=optional|required"
                .to_owned(),
            None,
        ));
    }
    if streamable.allow_delete_session {
        return Err(McpError::invalid_request(
            "transport.streamable_http.allow_delete_session requires session_mode=optional|required"
                .to_owned(),
            None,
        ));
    }
    if streamable
        .hardening
        .as_ref()
        .and_then(|hardening| hardening.session.as_ref())
        .is_some()
    {
        return Err(McpError::invalid_request(
            "transport.streamable_http.hardening.session requires session_mode=optional|required"
                .to_owned(),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_log_level, validate_transport};
    #[cfg(feature = "streamable_http")]
    use crate::config::StreamableHttpSessionMode;
    use crate::config::{AuthProviderConfig, TransportMode};
    use crate::inline_test_fixtures::{base_config, route_target, router_router_plugin_config};

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_resumption_without_get_stream() {
        let mut config = base_config();
        config.server.transport.streamable_http.enable_get_stream = false;
        config
            .server
            .transport
            .streamable_http
            .enable_sse_resumption = true;
        let error =
            validate_transport(&config).expect_err("resumption without GET stream must fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.enable_sse_resumption requires enable_get_stream=true"
        );
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_resumption_without_sessions() {
        let mut config = base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::None;
        config
            .server
            .transport
            .streamable_http
            .enable_sse_resumption = true;
        let error = validate_transport(&config)
            .expect_err("resumption with stateless session mode must fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.enable_sse_resumption requires session_mode=optional|required"
        );
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_delete_without_sessions() {
        let mut config = base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::None;
        config.server.transport.streamable_http.allow_delete_session = true;
        let error =
            validate_transport(&config).expect_err("delete session with stateless mode must fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.allow_delete_session requires session_mode=optional|required"
        );
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_hardening_session_without_sessions() {
        let mut config = base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::None;
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                session: Some(crate::config::StreamableHttpSessionHardeningConfig::default()),
                ..crate::config::StreamableHttpHardeningConfig::default()
            });
        let error = validate_transport(&config)
            .expect_err("session hardening with stateless mode must fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.session requires session_mode=optional|required"
        );
    }

    #[test]
    #[cfg(feature = "http_hardening")]
    fn validate_transport_rejects_hardening_rate_limit_without_buckets_when_enabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                rate_limit: Some(crate::config::StreamableHttpRateLimitConfig {
                    enabled: Some(true),
                    global: None,
                    per_ip: None,
                }),
                ..Default::default()
            });
        let result = validate_transport(&config);
        let error = result.expect_err("active hardening.rate_limit without buckets should fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.rate_limit requires global or per_ip when enabled"
        );
    }

    #[test]
    #[cfg(feature = "http_hardening")]
    fn validate_transport_rejects_hardening_creation_rate_without_buckets_when_enabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                    creation_rate: Some(crate::config::StreamableHttpRateLimitConfig {
                        enabled: Some(true),
                        global: None,
                        per_ip: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
        let result = validate_transport(&config);
        let error =
            result.expect_err("active hardening.session.creation_rate without buckets should fail");
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.session.creation_rate requires global or per_ip when enabled"
        );
    }

    #[test]
    fn validate_transport_rejects_stdio_with_auth() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![AuthProviderConfig::bearer("static", "token")];
        auth.enabled = Some(true);
        let error = validate_transport(&config).expect_err("stdio with auth must fail");
        assert_eq!(
            error.message,
            "stdio transport requires auth to be disabled"
        );
    }

    #[test]
    fn validate_transport_rejects_stdio_with_http_router_plugins() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        config.plugins = vec![router_router_plugin_config(
            "route",
            vec![route_target("/health")],
        )];
        let error =
            validate_transport(&config).expect_err("stdio with http_router plugins must fail");
        assert_eq!(
            error.message,
            "http_router plugins require streamable_http transport"
        );
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_streamable_with_missing_host() {
        let mut config = base_config();
        config.server.host = " ".to_owned();
        let error = validate_transport(&config).expect_err("blank host must fail");
        assert_eq!(error.message, "server.host is required for streamable_http");
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_rejects_streamable_with_missing_endpoint() {
        let mut config = base_config();
        config.server.endpoint_path = " ".to_owned();
        let error = validate_transport(&config).expect_err("blank endpoint must fail");
        assert_eq!(
            error.message,
            "server.endpoint_path is required for streamable_http"
        );
    }

    #[test]
    fn parse_log_level_rejects_unknown_value() {
        let error = parse_log_level("verbose")
            .expect_err("unknown level should map to deterministic invalid_request");
        assert_eq!(error.message, "invalid log level: verbose");
    }

    #[test]
    fn validate_transport_accepts_valid_stdio_config() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        config.server.auth_mut_or_insert().enabled = Some(false);
        assert_eq!(
            validate_transport(&config).expect("valid stdio config should pass"),
            TransportMode::Stdio
        );
    }

    #[test]
    #[cfg(feature = "streamable_http")]
    fn validate_transport_accepts_valid_streamable_config() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::StreamableHttp;
        config.server.host = "127.0.0.1".to_owned();
        config.server.endpoint_path = "/mcp".to_owned();
        assert_eq!(
            validate_transport(&config).expect("valid streamable config should pass"),
            TransportMode::StreamableHttp
        );
    }
}
