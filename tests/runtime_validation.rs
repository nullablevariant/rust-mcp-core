mod engine_common;

#[cfg(feature = "streamable_http")]
use std::sync::Mutex;

#[cfg(feature = "streamable_http")]
use engine_common::fixture_path;
use engine_common::load_config_fixture;
use rmcp::model::ErrorCode;
use serde::Deserialize;

#[derive(Deserialize)]
struct RuntimeConfigFixture {
    config: rust_mcp_core::config::McpConfig,
}

#[derive(Deserialize)]
#[cfg(feature = "streamable_http")]
struct LogLevelsFixture {
    base_config: rust_mcp_core::config::McpConfig,
    levels: Vec<String>,
}

#[derive(Deserialize)]
#[cfg(feature = "streamable_http")]
struct LogLevelsFixturePayload {
    levels: Vec<String>,
}

fn load_runtime_fixture(name: &str) -> RuntimeConfigFixture {
    RuntimeConfigFixture {
        config: load_config_fixture(name).config,
    }
}

#[cfg(feature = "streamable_http")]
fn load_runtime_fixture_without_schema_validation(name: &str) -> RuntimeConfigFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[cfg(feature = "streamable_http")]
fn load_log_levels_fixture(name: &str) -> LogLevelsFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    let parsed: LogLevelsFixturePayload = serde_yaml::from_str(&raw).expect("fixture should parse");
    LogLevelsFixture {
        base_config: load_config_fixture("runtime/runtime_log_levels_fixture_config").config,
        levels: parsed.levels,
    }
}

#[cfg(feature = "streamable_http")]
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(feature = "streamable_http")]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[cfg(feature = "streamable_http")]
fn set_env(key: &'static str, value: &str) -> EnvGuard {
    static ENV_MUTEX: Mutex<()> = Mutex::new(());
    let lock = ENV_MUTEX.lock().expect("env mutex poisoned");
    let previous = std::env::var(key).ok();
    std::env::set_var(key, value);
    EnvGuard {
        key,
        previous,
        _lock: lock,
    }
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn log_levels_accept_fixture() {
    let fixture = load_log_levels_fixture("runtime/runtime_log_levels_fixture");
    let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");

    let expected_levels = ["trace", "debug", "info", "warn", "error"];
    assert_eq!(
        fixture.levels.len(),
        expected_levels.len(),
        "fixture should contain exactly the five standard log levels"
    );

    for (level, expected) in fixture.levels.iter().zip(expected_levels.iter()) {
        assert_eq!(
            level, expected,
            "fixture level ordering must match expected canonical levels"
        );
        let mut config = fixture.base_config.clone();
        config.server.logging.level = level.clone();
        config.server.port = 0;
        rust_mcp_core::runtime::run_from_config(config, rust_mcp_core::PluginRegistry::new())
            .await
            .unwrap_or_else(|err| panic!("log level '{level}' should be accepted, got: {err}"));
    }
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn log_level_rejects_invalid_fixture() {
    // Intentional bypass: test verifies runtime rejection semantics for an
    // invalid log level string that schema validation would reject first.
    let fixture =
        load_runtime_fixture_without_schema_validation("runtime/runtime_log_level_invalid_fixture");
    let error = rust_mcp_core::runtime::run_from_config(
        fixture.config,
        rust_mcp_core::PluginRegistry::new(),
    )
    .await
    .expect_err("invalid log level should be rejected");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "invalid log level: warning");
}

#[tokio::test]
async fn stdio_rejects_auth_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_stdio_auth_reject_fixture");
    let error = rust_mcp_core::runtime::run_from_config(
        fixture.config,
        rust_mcp_core::PluginRegistry::new(),
    )
    .await
    .expect_err("stdio with auth should be rejected");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    #[cfg(feature = "auth")]
    assert_eq!(
        error.message,
        "stdio transport requires auth to be disabled"
    );
    #[cfg(not(feature = "auth"))]
    assert_eq!(
        error.message,
        "auth feature disabled but server.auth is active"
    );
}

#[tokio::test]
async fn stdio_rejects_router_plugins_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_stdio_router_plugins_reject_fixture");
    let error = rust_mcp_core::runtime::run_from_config(
        fixture.config,
        rust_mcp_core::PluginRegistry::new(),
    )
    .await
    .expect_err("stdio with http_router plugins should be rejected");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    #[cfg(feature = "streamable_http")]
    assert_eq!(
        error.message,
        "http_router plugins require streamable_http transport"
    );
    #[cfg(not(feature = "streamable_http"))]
    assert_eq!(
        error.message,
        "streamable_http feature disabled but plugins include type=http_router"
    );
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn streamable_missing_host_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_streamable_missing_host_fixture");
    let error = rust_mcp_core::runtime::run_from_config(
        fixture.config,
        rust_mcp_core::PluginRegistry::new(),
    )
    .await
    .expect_err("missing host should be rejected");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "server.host is required for streamable_http");
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn streamable_missing_endpoint_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_streamable_missing_endpoint_fixture");
    let error = rust_mcp_core::runtime::run_from_config(
        fixture.config,
        rust_mcp_core::PluginRegistry::new(),
    )
    .await
    .expect_err("missing endpoint should be rejected");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "server.endpoint_path is required for streamable_http"
    );
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn streamable_ok_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_streamable_ok_fixture");
    let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");
    let runtime =
        rust_mcp_core::runtime::build_runtime(fixture.config, rust_mcp_core::PluginRegistry::new())
            .await
            .expect("streamable runtime should build successfully");
    // Verify runtime can refresh all list features without error after build.
    let tools_changed = runtime
        .refresh_list(rust_mcp_core::plugins::ListFeature::Tools)
        .await
        .expect("tools refresh should succeed after build");
    assert!(!tools_changed, "initial tools cache should be current");
    runtime
        .run()
        .await
        .expect("streamable runtime should run and shut down cleanly");
}

#[tokio::test]
#[cfg(all(feature = "streamable_http", feature = "auth"))]
async fn streamable_oauth_ok_fixture() {
    let fixture = load_runtime_fixture("runtime/runtime_streamable_oauth_ok_fixture");
    let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");
    let runtime =
        rust_mcp_core::runtime::build_runtime(fixture.config, rust_mcp_core::PluginRegistry::new())
            .await
            .expect("streamable oauth runtime should build successfully");
    // Verify runtime can refresh all list features without error after build.
    let tools_changed = runtime
        .refresh_list(rust_mcp_core::plugins::ListFeature::Tools)
        .await
        .expect("tools refresh should succeed after build");
    assert!(!tools_changed, "initial tools cache should be current");
    runtime
        .run()
        .await
        .expect("streamable oauth runtime should run and shut down cleanly");
}
