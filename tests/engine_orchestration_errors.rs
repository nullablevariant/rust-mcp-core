#![cfg(feature = "http_tools")]

mod engine_common;

use engine_common::load_tool_fixture;
use rmcp::model::ErrorCode;
use rust_mcp_core::config::load_mcp_config_from_path;
use rust_mcp_core::engine::Engine;

#[tokio::test]
async fn unknown_tool_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_unknown_tool_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let error = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("unknown tool should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "tool not found: missing");
}

#[tokio::test]
async fn http_tool_missing_upstream_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_missing_upstream_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let error = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("missing upstream should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "unknown upstream: api");
}

#[test]
fn http_tool_missing_path_returns_error_fixture() {
    let error = load_mcp_config_from_path(engine_common::fixture_path(
        "engine/engine_missing_path_fixture_config",
    ))
    .expect_err("missing path should fail during config load");
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

#[tokio::test]
async fn plugin_tool_missing_plugin_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_missing_plugin_fixture");
    let error = Engine::new(fixture.config).expect_err("missing plugin should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "plugin tool requires plugin");
}

#[tokio::test]
async fn plugin_not_registered_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_plugin_not_registered_fixture");
    let error = Engine::new(fixture.config).expect_err("unregistered plugin should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "tool plugin not registered: plugin.echo");
}

#[tokio::test]
async fn plugin_not_allowlisted_returns_error_fixture() {
    let fixture = load_tool_fixture("engine/engine_plugin_not_allowlisted_fixture");
    let error = Engine::new(fixture.config).expect_err("non-allowlisted plugin should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "tool plugin not allowlisted: plugin.echo");
}
