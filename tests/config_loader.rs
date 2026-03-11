use std::{env, path::PathBuf, sync::Mutex};

use rust_mcp_core::config::{
    load_mcp_config, load_mcp_config_from_path, ProtocolVersionNegotiationMode, SecretValueSource,
    StreamableHttpRateLimitKeySource, TopLevelCombinatorsPolicy, TransportMode, UpstreamAuth,
    UpstreamOauth2GrantType,
};
use rust_mcp_core::mcp::LoggingLevel;
use rust_mcp_core::plugins::PluginType;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn assert_default_activation_state(config: &rust_mcp_core::config::McpConfig) {
    assert!(!config.client_logging_active());
    assert_eq!(config.client_logging_level(), LoggingLevel::Info);
    assert!(!config.progress_active());
    assert_eq!(config.progress_interval_ms(), 250);
    assert!(!config.completion_active());
    assert!(config.completion_providers().is_empty());
    assert!(!config.tasks_active());
    assert!(!config.tasks_list_active());
    assert!(!config.tasks_cancel_active());
    assert!(!config.tasks_status_notifications_active());
    assert!(!config.client_roots_active());
    assert!(!config.client_sampling_active());
    assert!(!config.client_sampling_allow_tools());
    assert!(!config.client_elicitation_active());
    assert!(config.client_elicitation_mode().is_none());
}

#[test]
fn loads_yaml_and_expands_env() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    env::set_var("MCP_HOST", "127.0.0.1");
    env::set_var("MCP_PORT", "4001");
    env::set_var("MCP_BEARER_TOKEN", "secret-token");
    env::set_var("API_BASE_URL", "https://example.com");

    let config =
        load_mcp_config_from_path(fixture_path("config_loader/config_loader_basic_fixture"))
            .expect("config should load");

    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 4001);
    assert_eq!(
        config
            .server
            .auth
            .as_ref()
            .expect("auth should be present")
            .providers
            .iter()
            .find_map(rust_mcp_core::config::AuthProviderConfig::bearer_token),
        Some("secret-token")
    );
    assert_eq!(
        config.upstreams.get("api").expect("api upstream").base_url,
        "https://example.com"
    );

    env::remove_var("MCP_HOST");
    env::remove_var("MCP_PORT");
    env::remove_var("MCP_BEARER_TOKEN");
    env::remove_var("API_BASE_URL");
}

#[test]
fn rejects_missing_required_fields() {
    let error =
        load_mcp_config_from_path(fixture_path("config_loader/config_loader_missing_fixture"))
            .expect_err("missing required fields should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("required"),
        "error must mention 'required' for missing field: {}",
        error.message
    );
}

#[test]
fn rejects_active_completion_without_sources() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_completion_empty_fixture",
    ))
    .expect_err("active completion without sources should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf schema mismatch for empty completion: {}",
        error.message
    );
}

#[test]
fn allows_completion_disabled_without_sources() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_completion_disabled_fixture",
    ))
    .expect("completion.enabled=false without sources should load");
    let completion = config
        .completion
        .as_ref()
        .expect("completion section should be present");
    assert_eq!(completion.enabled, Some(false));
    assert!(completion.providers.is_empty());
}

#[test]
fn rejects_unclosed_env_placeholder() {
    let result = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_unclosed_placeholder_fixture",
    ));
    let error = result.expect_err("unclosed placeholder should fail");
    assert!(
        error
            .message
            .contains("malformed env placeholder: missing closing `}`"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn rejects_empty_namespaced_env_placeholder() {
    let result = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_empty_namespaced_placeholder_fixture",
    ));
    let error = result.expect_err("empty env placeholder should fail");
    assert!(
        error
            .message
            .contains("malformed env placeholder: empty env var name in `${env:}`"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn loads_defaults_for_mcp_fields() {
    let config =
        load_mcp_config_from_path(fixture_path("config_loader/config_loader_defaults_fixture"))
            .expect("config should load");

    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 3000);
    assert_eq!(config.server.endpoint_path, "/mcp");
    assert_eq!(config.server.logging.level, "info");
    assert_eq!(config.server.transport.mode, TransportMode::StreamableHttp);
    assert!(config.server.transport.streamable_http.enable_get_stream);
    assert!(
        !config
            .server
            .transport
            .streamable_http
            .enable_sse_resumption
    );
    assert_eq!(
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators,
        TopLevelCombinatorsPolicy::Warn
    );
    assert_eq!(
        config
            .server
            .transport
            .streamable_http
            .protocol_version_negotiation
            .mode,
        ProtocolVersionNegotiationMode::Strict
    );
    assert!(config.server.transport.streamable_http.hardening.is_none());
    assert!(!config.server.errors.expose_internal_details);
    assert_eq!(config.server.logging.log_payload_max_bytes, 4096);
    assert!(config.server.response_limits.is_none());
    assert!(!config.server.auth_active());
    assert!(config.prompts.is_none());
    assert!(config.pagination.is_none());
    assert!(!config.tools_notify_list_changed());
    assert_default_activation_state(&config);
}

#[test]
fn loads_outbound_http_defaults_when_configured() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_outbound_http_normalized_fixture",
    ))
    .expect("config should load");

    let outbound = config
        .outbound_http
        .as_ref()
        .expect("outbound_http should be present");
    assert_eq!(outbound.user_agent.as_deref(), Some("mcp-core-test/1.0"));
    assert_eq!(
        outbound.headers.get("X-Default").map(String::as_str),
        Some("yes")
    );
    assert_eq!(outbound.timeout_ms, Some(5000));
    assert_eq!(outbound.max_response_bytes, Some(2048));
}

#[test]
fn loads_outbound_retry_layers_when_configured() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_outbound_retry_valid_fixture",
    ))
    .expect("config should load");

    let global_retry = config
        .outbound_http
        .as_ref()
        .and_then(|outbound| outbound.retry.as_ref())
        .expect("global outbound retry should be present");
    assert!(global_retry.enabled());
    assert_eq!(global_retry.max_attempts, 4);
    assert_eq!(global_retry.delay_ms, 150);
    assert!(!global_retry.on_network_errors);
    assert_eq!(global_retry.on_statuses, vec![429, 503]);

    let upstream_retry = config
        .upstreams
        .get("api")
        .and_then(|upstream| upstream.retry.as_ref())
        .expect("upstream retry should be present");
    assert!(upstream_retry.enabled());
    assert_eq!(upstream_retry.max_attempts, 2);
    assert_eq!(upstream_retry.delay_ms, 300);
    assert!(upstream_retry.on_network_errors);
    assert_eq!(upstream_retry.on_statuses, vec![500, 502, 503]);

    let tool_retry = config.tools_items()[0]
        .execute
        .as_http()
        .and_then(|execute| execute.retry.as_ref())
        .expect("tool retry should be present");
    assert!(tool_retry.enabled());
    assert_eq!(tool_retry.max_attempts, 5);
    assert_eq!(tool_retry.delay_ms, 75);
    assert!(tool_retry.on_network_errors);
    assert_eq!(tool_retry.on_statuses, vec![408, 429, 502]);
}

#[test]
fn loads_outbound_retry_defaults_when_sections_are_empty() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_outbound_retry_defaults_fixture",
    ))
    .expect("config should load");

    let global_retry = config
        .outbound_http
        .as_ref()
        .and_then(|outbound| outbound.retry.as_ref())
        .expect("global retry should be present");
    assert!(global_retry.enabled());
    assert_eq!(global_retry.max_attempts, 3);
    assert_eq!(global_retry.delay_ms, 200);
    assert!(global_retry.on_network_errors);
    assert_eq!(global_retry.on_statuses, vec![429, 502, 503, 504]);

    let upstream_retry = config
        .upstreams
        .get("api")
        .and_then(|upstream| upstream.retry.as_ref())
        .expect("upstream retry should be present");
    assert!(upstream_retry.enabled());
    assert_eq!(upstream_retry.max_attempts, 3);
    assert_eq!(upstream_retry.delay_ms, 200);
    assert!(upstream_retry.on_network_errors);
    assert_eq!(upstream_retry.on_statuses, vec![429, 502, 503, 504]);

    let tool_retry = config.tools_items()[0]
        .execute
        .as_http()
        .and_then(|execute| execute.retry.as_ref())
        .expect("tool retry should be present");
    assert!(tool_retry.enabled());
    assert_eq!(tool_retry.max_attempts, 3);
    assert_eq!(tool_retry.delay_ms, 200);
    assert!(tool_retry.on_network_errors);
    assert_eq!(tool_retry.on_statuses, vec![429, 502, 503, 504]);
}

#[test]
fn rejects_outbound_retry_zero_max_attempts() {
    let fixture =
        fixture_path("config_loader/config_loader_outbound_retry_invalid_max_attempts_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("max_attempts: 0"),
        "fixture must target max_attempts field"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("invalid retry max_attempts should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // Schema enforces minimum: 1 for max_attempts; 0 triggers this specific error
    assert!(
        error.message.contains("0 is less than the minimum of 1"),
        "error must report minimum violation for max_attempts=0: {}",
        error.message
    );
}

#[test]
fn rejects_outbound_retry_zero_delay_ms() {
    let fixture = fixture_path("config_loader/config_loader_outbound_retry_invalid_delay_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("delay_ms: 0"),
        "fixture must target delay_ms field"
    );

    let error = load_mcp_config_from_path(fixture).expect_err("invalid retry delay_ms should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // Schema enforces minimum: 1 for delay_ms; 0 triggers this specific error
    assert!(
        error.message.contains("0 is less than the minimum of 1"),
        "error must report minimum violation for delay_ms=0: {}",
        error.message
    );
}

#[test]
fn rejects_tool_retry_for_non_http_execute_type() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_outbound_retry_invalid_non_http_tool_fixture",
    ))
    .expect_err("non-http execute retry should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error
            .message
            .contains("is not valid under any of the schemas listed in the 'oneOf' keyword"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn rejects_retry_without_network_errors_or_statuses() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_outbound_retry_invalid_no_targets_fixture",
    ))
    .expect_err("retry without network/status targets should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("has less than 1 item"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn loads_core_hardening_controls_when_configured() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_core_hardening_controls_fixture",
    ))
    .expect("config should load");

    assert!(config.server.errors.expose_internal_details);
    assert_eq!(config.server.logging.log_payload_max_bytes, 1024);
    let response_limits = config
        .server
        .response_limits
        .as_ref()
        .expect("response limits should be present");
    assert_eq!(response_limits.text_bytes, Some(512));
    assert_eq!(response_limits.structured_bytes, Some(4096));
    assert_eq!(response_limits.binary_bytes, Some(8192));
    assert_eq!(response_limits.other_bytes, Some(2048));
    assert_eq!(response_limits.total_bytes, Some(12288));
}

#[test]
fn rejects_invalid_server_logging_max_bytes_shape() {
    let fixture =
        fixture_path("config_loader/config_loader_core_hardening_invalid_server_logging_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("log_payload_max_bytes: \"large\""),
        "fixture must target log_payload_max_bytes field"
    );

    let error = load_mcp_config_from_path(fixture)
        .expect_err("invalid server_logging.log_payload_max_bytes should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // log_payload_max_bytes is set to "large" (string), schema expects integer
    assert!(
        error.message.contains("is not of type \"integer\""),
        "error must report type mismatch for log_payload_max_bytes: {}",
        error.message
    );
}

#[test]
fn rejects_invalid_error_exposure_type() {
    let fixture = fixture_path("config_loader/config_loader_core_hardening_invalid_errors_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("expose_internal_details: \"yes\""),
        "fixture must target expose_internal_details field"
    );

    let error = load_mcp_config_from_path(fixture)
        .expect_err("invalid errors.expose_internal_details type should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // expose_internal_details is set to "yes" (string), schema expects boolean
    assert!(
        error.message.contains("is not of type \"boolean\""),
        "error must report type mismatch for expose_internal_details: {}",
        error.message
    );
}

#[test]
fn rejects_invalid_response_limits_bounds() {
    let fixture =
        fixture_path("config_loader/config_loader_core_hardening_invalid_response_limits_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("total_bytes: -1"),
        "fixture must target total_bytes field"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("negative response limit should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // total_bytes is set to -1, schema enforces minimum: 0
    assert!(
        error.message.contains("-1 is less than the minimum of 0"),
        "error must report minimum violation for total_bytes=-1: {}",
        error.message
    );
}

#[test]
fn activation_helpers_respect_presence_and_enabled_overrides() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_activation_helpers_fixture",
    ))
    .expect("config should load");

    assert!(config.client_logging_active());
    assert_eq!(config.client_logging_level(), LoggingLevel::Warning);
    assert!(config.progress_active());
    assert_eq!(config.progress_interval_ms(), 777);
    assert!(config.completion_active());
    assert_eq!(config.completion_providers().len(), 1);

    // Hybrid sections stay inactive when explicitly disabled.
    assert!(!config.tasks_active());
    assert!(!config.tasks_list_active());
    assert!(!config.tasks_cancel_active());
    assert!(!config.tasks_status_notifications_active());
    assert!(config.client_roots_active());
    assert!(!config.client_sampling_active());
    assert!(!config.client_sampling_allow_tools());
    assert!(!config.client_elicitation_active());
    assert!(config.client_elicitation_mode().is_none());
}

#[test]
fn loads_streamable_http_protocol_version_negotiation_explicit_strict() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_protocol_version_negotiation_strict_fixture",
    ))
    .expect("config should load");

    assert_eq!(
        config
            .server
            .transport
            .streamable_http
            .protocol_version_negotiation
            .mode,
        ProtocolVersionNegotiationMode::Strict
    );
}

#[test]
fn loads_streamable_http_protocol_version_negotiation_explicit_negotiate() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_protocol_version_negotiation_negotiate_fixture",
    ))
    .expect("config should load");

    assert_eq!(
        config
            .server
            .transport
            .streamable_http
            .protocol_version_negotiation
            .mode,
        ProtocolVersionNegotiationMode::Negotiate
    );
}

#[test]
fn loads_streamable_http_hardening_defaults_when_present() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_hardening_defaults_fixture",
    ))
    .expect("config should load");

    let hardening = config
        .server
        .transport
        .streamable_http
        .hardening
        .as_ref()
        .expect("hardening section should be present");
    assert_eq!(hardening.max_request_bytes, 1_048_576);
    assert!(hardening.catch_panics);
    assert!(hardening.sanitize_sensitive_headers);
    assert!(hardening.session.is_none());
    assert!(hardening.rate_limit.is_none());
}

#[test]
fn loads_streamable_http_hardening_when_configured() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_hardening_configured_fixture",
    ))
    .expect("config should load");

    let hardening = config
        .server
        .transport
        .streamable_http
        .hardening
        .as_ref()
        .expect("hardening section should be present");
    assert_eq!(hardening.max_request_bytes, 2_097_152);
    assert!(!hardening.catch_panics);
    assert!(!hardening.sanitize_sensitive_headers);

    let session = hardening
        .session
        .as_ref()
        .expect("session hardening should be present");
    assert_eq!(session.max_sessions, Some(2048));
    assert_eq!(session.idle_ttl_secs, Some(900));
    assert_eq!(session.max_lifetime_secs, Some(86_400));
    let creation_rate = session
        .creation_rate
        .as_ref()
        .expect("session creation limiter should be present");
    assert_eq!(creation_rate.enabled, Some(true));
    let creation_global = creation_rate
        .global
        .as_ref()
        .expect("global creation limiter should be present");
    assert_eq!(creation_global.capacity, 60);
    assert_eq!(creation_global.refill_per_sec, 1);
    let creation_per_ip = creation_rate
        .per_ip
        .as_ref()
        .expect("per-ip creation limiter should be present");
    assert_eq!(creation_per_ip.capacity, 10);
    assert_eq!(creation_per_ip.refill_per_sec, 1);
    assert_eq!(
        creation_per_ip.key_source,
        StreamableHttpRateLimitKeySource::XForwardedFor
    );

    let rate_limit = hardening
        .rate_limit
        .as_ref()
        .expect("general rate limiter should be present");
    assert_eq!(rate_limit.enabled, Some(true));
    let global = rate_limit
        .global
        .as_ref()
        .expect("global limiter should be present");
    assert_eq!(global.capacity, 200);
    assert_eq!(global.refill_per_sec, 20);
    let per_ip = rate_limit
        .per_ip
        .as_ref()
        .expect("per-ip limiter should be present");
    assert_eq!(per_ip.capacity, 20);
    assert_eq!(per_ip.refill_per_sec, 2);
    assert_eq!(
        per_ip.key_source,
        StreamableHttpRateLimitKeySource::PeerAddr
    );
}

#[test]
fn rejects_streamable_http_hardening_session_in_stateless_mode() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_hardening_invalid_session_mode_none_fixture",
    ))
    .expect_err("session hardening with session_mode=none should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("session"),
        "error must reference session in the validation failure: {}",
        error.message
    );
}

#[test]
fn rejects_streamable_http_hardening_rate_limit_without_buckets_when_enabled() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_transport_hardening_invalid_rate_limit_no_buckets_fixture",
    ))
    .expect_err("hardening.rate_limit without buckets should fail");
    assert!(
        error
            .message
            .contains("hardening.rate_limit requires global or per_ip"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn defaults_input_schema_top_level_combinator_policy_to_warn() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_input_schema_client_compat_default_warn_fixture",
    ))
    .expect("config should load");

    assert_eq!(
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators,
        TopLevelCombinatorsPolicy::Warn
    );
}

#[test]
fn allows_top_level_input_schema_combinators_when_policy_off() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_input_schema_client_compat_off_fixture",
    ))
    .expect("config should load");

    assert_eq!(
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators,
        TopLevelCombinatorsPolicy::Off
    );
}

#[test]
fn allows_top_level_input_schema_combinators_when_policy_warn() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_input_schema_client_compat_warn_fixture",
    ))
    .expect("config should load");

    assert_eq!(
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators,
        TopLevelCombinatorsPolicy::Warn
    );
}

#[test]
fn rejects_top_level_input_schema_combinators_when_policy_error() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_input_schema_client_compat_error_fixture",
    ))
    .expect_err("top-level combinator should fail in error mode");

    assert!(
        error
            .message
            .contains("input schema client compatibility check failed"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("tools.0.input_schema"),
        "unexpected error: {}",
        error.message
    );
    assert!(
        error.message.contains("tool.compat.top_level_anyof"),
        "unexpected error: {}",
        error.message
    );
}

#[test]
fn accepts_compatible_schema_when_policy_error() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_input_schema_client_compat_error_compatible_fixture",
    ))
    .expect("compatible schema should load");

    assert_eq!(
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators,
        TopLevelCombinatorsPolicy::Error
    );
}

#[test]
fn rejects_invalid_input_schema_top_level_combinator_policy_enum() {
    let fixture = fixture_path(
        "config_loader/config_loader_input_schema_client_compat_invalid_policy_fixture",
    );
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("top_level_combinators: warning"),
        "fixture must target top_level_combinators field"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("invalid enum should fail schema validation");
    assert!(
        error.message.contains("config schema validation failed"),
        "unexpected error: {}",
        error.message
    );
    // top_level_combinators is set to "warning" (invalid), schema expects "off"|"warn"|"error"
    assert!(
        error
            .message
            .contains("is not one of \"off\", \"warn\" or \"error\""),
        "error must report invalid enum value for top_level_combinators: {}",
        error.message
    );
}

#[test]
fn load_mcp_config_requires_path_env() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    env::remove_var("MCP_CONFIG_PATH");
    let error = load_mcp_config(None).expect_err("missing MCP_CONFIG_PATH should fail");
    assert!(
        error.message.contains("MCP_CONFIG_PATH"),
        "error must mention MCP_CONFIG_PATH: {}",
        error.message
    );
}

#[test]
fn load_mcp_config_uses_env_path_fixture() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    let path = fixture_path("config_loader/config_loader_env_fixture");
    env::set_var("MCP_CONFIG_PATH", &path);
    let config = load_mcp_config(None).expect("config should load via env path");
    assert_eq!(config.version, 1);
    env::remove_var("MCP_CONFIG_PATH");
}

#[test]
fn load_mcp_config_parses_json_fixture() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_json_fixture.json",
    ))
    .expect("config should load from json");
    assert!(!config.server.auth_active());
}

#[test]
fn rejects_duplicate_plugin_names() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_duplicate_plugin_name_fixture",
    ))
    .expect_err("duplicate plugin names should fail");
    assert!(
        error.message.contains("duplicate plugin name"),
        "error must mention duplicate plugin name: {}",
        error.message
    );
    assert!(
        error.message.contains("my_plugin"),
        "error must name the duplicate plugin: {}",
        error.message
    );
}

#[test]
fn allows_completion_plugin_with_config() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_plugin_completion_config_allowed_fixture",
    ))
    .expect("completion plugin with config should load");
    assert_eq!(config.plugins.len(), 1, "expected exactly one plugin");
    assert_eq!(config.plugins[0].name, "my_completion");
    assert_eq!(config.plugins[0].plugin_type, PluginType::Completion);
    assert!(
        config.plugins[0].config.is_some(),
        "completion plugin config should be present"
    );
}

#[test]
fn rejects_http_router_plugin_without_targets() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_plugin_http_router_no_targets_fixture",
    ))
    .expect_err("http_router plugin without targets should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("targets"),
        "error must reference missing targets field: {}",
        error.message
    );
}

#[test]
fn rejects_tool_plugin_with_targets() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_plugin_tool_with_targets_fixture",
    ))
    .expect_err("tool plugin with targets should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("targets"),
        "error must reference disallowed targets field: {}",
        error.message
    );
}

#[test]
fn allows_prompt_plugin_with_config() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_plugin_prompt_config_allowed_fixture",
    ))
    .expect("prompt plugin with config should load");
    assert_eq!(config.plugins.len(), 1, "expected exactly one plugin");
    assert_eq!(config.plugins[0].name, "prompt.plugin");
    assert_eq!(config.plugins[0].plugin_type, PluginType::Prompt);
    assert!(
        config.plugins[0].config.is_some(),
        "prompt plugin config should be present"
    );
}

#[test]
fn allows_resource_plugin_with_config() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_plugin_resource_config_allowed_fixture",
    ))
    .expect("resource plugin with config should load");
    assert_eq!(config.plugins.len(), 1, "expected exactly one plugin");
    assert_eq!(config.plugins[0].name, "resource.plugin");
    assert_eq!(config.plugins[0].plugin_type, PluginType::Resource);
    assert!(
        config.plugins[0].config.is_some(),
        "resource plugin config should be present"
    );
}

#[test]
fn allows_audio_tool_response_content() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_response_content_audio_fixture",
    ))
    .expect("audio response content should load");
    let tool = config
        .tools_items()
        .iter()
        .find(|t| t.name == "tool.audio")
        .expect("tool.audio should exist");
    let response = tool.response.as_ref().expect("response should be present");
    let content = match response {
        rust_mcp_core::config::ResponseConfig::Content(content) => &content.items,
        rust_mcp_core::config::ResponseConfig::Structured(_) => {
            panic!("expected content response variant")
        }
    };
    assert_eq!(content.len(), 1, "expected exactly one content item");
    let item = &content[0];
    assert_eq!(
        item.get("type").and_then(|v| v.as_str()),
        Some("audio"),
        "content item type should be audio"
    );
}

#[test]
fn allows_tool_structured_content_template_string() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_structured_content_template_string_fixture",
    ))
    .expect("config should load");

    let tool = config
        .tools_items()
        .iter()
        .find(|tool| tool.name == "tool.structured.template")
        .expect("tool.structured.template should exist");

    let structured_template = tool
        .response
        .as_ref()
        .and_then(|response| match response {
            rust_mcp_core::config::ResponseConfig::Structured(structured) => {
                structured.template.as_ref()
            }
            rust_mcp_core::config::ResponseConfig::Content(_) => None,
        })
        .and_then(serde_json::Value::as_str)
        .expect("structured template should be a string");

    assert_eq!(structured_template, "${$}");
}

#[test]
fn rejects_tool_response_content_without_type() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_response_content_missing_type_fixture",
    ))
    .expect_err("response content without type should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("type"),
        "error must reference missing type field: {}",
        error.message
    );
}

#[test]
fn rejects_http_execute_with_plugin_field() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_execute_http_with_plugin_field_fixture",
    ))
    .expect_err("http execute variant should reject plugin field");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf variant mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_plugin_execute_with_upstream_field() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_execute_plugin_with_upstream_field_fixture",
    ))
    .expect_err("plugin execute variant should reject upstream field");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf variant mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_content_response_with_structured_template_field() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_response_content_with_template_fixture",
    ))
    .expect_err("content response variant should reject template field");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf variant mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_structured_response_with_content_items_field() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_response_structured_with_items_fixture",
    ))
    .expect_err("structured response variant should reject items field");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf variant mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_client_roots_without_enabled_field() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_client_features_roots_missing_enabled_fixture",
    ))
    .expect_err("client_features.roots requires enabled field");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("required"),
        "error must reference missing required field: {}",
        error.message
    );
}

#[test]
fn rejects_auth_enabled_with_empty_providers() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_auth_enabled_empty_providers_fixture",
    ))
    .expect_err("auth.enabled=true with empty providers should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error
            .message
            .contains("is not valid under any of the schemas listed in the 'anyOf' keyword"),
        "error must report auth anyOf mismatch when enabled=true with empty providers: {}",
        error.message
    );
}

#[test]
fn loads_server_info_and_instructions_from_yaml() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_server_info_instructions_fixture",
    ))
    .expect("config should load");

    let info = config
        .server
        .info
        .as_ref()
        .expect("server.info should be set");
    assert_eq!(info.name.as_deref(), Some("My MCP Server"));
    assert_eq!(info.version.as_deref(), Some("2.0.0"));
    assert_eq!(info.title.as_deref(), Some("Server Title"));
    assert_eq!(info.description.as_deref(), Some("Server description"));
    assert_eq!(info.website_url.as_deref(), Some("https://example.com"));
    assert_eq!(
        info.instructions.as_deref(),
        Some("Use this server for testing")
    );
}

#[test]
fn loads_server_info_and_instructions_from_json() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_server_info_instructions_fixture.json",
    ))
    .expect("config should load");

    let info = config
        .server
        .info
        .as_ref()
        .expect("server.info should be set");
    assert_eq!(info.name.as_deref(), Some("My MCP Server"));
    assert_eq!(info.version.as_deref(), Some("2.0.0"));
    assert_eq!(info.title.as_deref(), Some("Server Title"));
    assert_eq!(info.description.as_deref(), Some("Server description"));
    assert_eq!(info.website_url.as_deref(), Some("https://example.com"));
    assert_eq!(
        info.instructions.as_deref(),
        Some("Use this server for testing")
    );
}

#[test]
fn rejects_invalid_instructions_type() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_invalid_instructions_type_fixture",
    ))
    .expect_err("invalid instructions type should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    // instructions is set to {text: "not-a-string"} (object), schema expects string|null
    assert!(
        error
            .message
            .contains("is not of types \"null\", \"string\""),
        "error must report type mismatch for instructions: {}",
        error.message
    );
}

#[test]
fn rejects_upstream_basic_auth_without_password() {
    let fixture =
        fixture_path("config_loader/config_loader_upstream_auth_basic_missing_password_fixture");
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("type: basic"),
        "fixture must target basic auth variant"
    );
    assert!(
        !fixture_text.contains("password:"),
        "fixture must omit password to target missing-password branch"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("basic auth without password should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("basic"),
        "error must reference the basic auth type in the rejected input: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf schema mismatch: {}",
        error.message
    );
}

#[test]
fn allows_upstream_oauth2_client_credentials_auth() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_upstream_auth_oauth2_client_credentials_fixture",
    ))
    .expect("oauth2 client_credentials auth should load");
    let upstream = config
        .upstreams
        .get("api")
        .expect("api upstream should exist");
    let auth = upstream.auth.as_ref().expect("auth should be present");
    match auth {
        UpstreamAuth::Oauth2(oauth) => {
            assert_eq!(oauth.grant, UpstreamOauth2GrantType::ClientCredentials);
            assert_eq!(oauth.client_id, "example-client");
            assert!(oauth.scopes.contains(&"reports.read".to_owned()));
        }
        other => panic!("expected Oauth2 auth, got {other:?}"),
    }
}

#[test]
fn rejects_upstream_oauth2_without_client_secret() {
    let fixture = fixture_path(
        "config_loader/config_loader_upstream_auth_oauth2_missing_client_secret_fixture",
    );
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("type: oauth2"),
        "fixture must target oauth2 auth variant"
    );
    assert!(
        !fixture_text.contains("client_secret:"),
        "fixture must omit client_secret to target missing-client_secret branch"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("oauth2 without client_secret should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oauth2"),
        "error must reference the oauth2 auth type in the rejected input: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf schema mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_upstream_oauth2_refresh_grant_without_refresh_token() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_upstream_auth_oauth2_refresh_missing_refresh_token_fixture",
    ))
    .expect_err("oauth2 refresh grant without refresh_token should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oauth2"),
        "error must reference the oauth2 auth type in the rejected input: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf schema mismatch: {}",
        error.message
    );
    assert!(
        error.message.contains("refresh_token"),
        "error must reference missing refresh_token field/path: {}",
        error.message
    );
}

#[test]
fn allows_upstream_oauth2_with_mtls() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_upstream_auth_oauth2_mtls_fixture",
    ))
    .expect("oauth2 with mTLS auth should load");
    let upstream = config
        .upstreams
        .get("api")
        .expect("api upstream should exist");
    let auth = upstream.auth.as_ref().expect("auth should be present");
    match auth {
        UpstreamAuth::Oauth2(oauth) => {
            assert_eq!(oauth.grant, UpstreamOauth2GrantType::ClientCredentials);
            let mtls = oauth.mtls.as_ref().expect("mtls config should be present");
            assert!(mtls.ca_cert.is_some(), "ca_cert should be present");
            assert_eq!(mtls.client_cert.source, SecretValueSource::Inline);
            assert_eq!(mtls.client_key.source, SecretValueSource::Inline);
        }
        other => panic!("expected Oauth2 auth, got {other:?}"),
    }
}

#[test]
fn loads_mixed_env_and_runtime_placeholders_without_collision() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    env::set_var("API_BASE_URL", "https://api.example.com");
    env::set_var("query", "injected-from-env");

    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_mixed_placeholder_fixture",
    ))
    .expect("mixed placeholders should load");

    let upstream = config.upstreams.get("api").expect("api upstream");
    assert_eq!(upstream.base_url, "https://api.example.com");

    let tool = config
        .tools_items()
        .iter()
        .find(|tool| tool.name == "api.search")
        .expect("api.search tool");
    let query_template = tool
        .execute
        .as_http()
        .and_then(|execute| execute.query.as_ref())
        .and_then(|query| query.get("q"))
        .and_then(serde_json::Value::as_str)
        .expect("q query template");
    assert_eq!(query_template, "${query}");

    env::remove_var("API_BASE_URL");
    env::remove_var("query");
}

#[test]
fn rejects_upstream_oauth2_mtls_without_client_key() {
    let fixture = fixture_path(
        "config_loader/config_loader_upstream_auth_oauth2_mtls_missing_client_key_fixture",
    );
    let fixture_text = std::fs::read_to_string(&fixture).expect("fixture must be readable");
    assert!(
        fixture_text.contains("mtls:"),
        "fixture must target mTLS sub-configuration"
    );
    assert!(
        fixture_text.contains("client_cert:"),
        "fixture must include client_cert in mTLS config"
    );
    assert!(
        !fixture_text.contains("client_key:"),
        "fixture must omit client_key to target missing-client_key branch"
    );

    let error =
        load_mcp_config_from_path(fixture).expect_err("oauth2 mTLS without client_key should fail");
    assert!(
        error.message.contains("config schema validation failed"),
        "error must be a schema validation failure: {}",
        error.message
    );
    assert!(
        error.message.contains("oauth2"),
        "error must reference the oauth2 auth type in the rejected input: {}",
        error.message
    );
    assert!(
        error.message.contains("oneOf"),
        "error must reference oneOf schema mismatch: {}",
        error.message
    );
}
