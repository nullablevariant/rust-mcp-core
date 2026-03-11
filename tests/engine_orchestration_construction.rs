#![cfg(feature = "http_tools")]

mod engine_common;

#[cfg(feature = "http_tools")]
use async_trait::async_trait;
use engine_common::{fixture_path, load_config_fixture, EngineConfigFixture};
use rmcp::model::ErrorCode;
#[cfg(feature = "http_tools")]
use rmcp::{model::CallToolResult, ErrorData as McpError};
#[cfg(feature = "http_tools")]
use rust_mcp_core::plugins::{PluginCallParams, ToolPlugin};
use rust_mcp_core::{
    engine::{Engine, EngineConfig},
    plugins::PluginRegistry,
};
#[cfg(feature = "http_tools")]
use serde_json::Value;

#[cfg(feature = "http_tools")]
struct NoopPlugin;

#[cfg(feature = "http_tools")]
#[async_trait]
impl ToolPlugin for NoopPlugin {
    fn name(&self) -> &'static str {
        "plugin.noop"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(Value::Null))
    }
}

fn load_config_fixture_without_schema_validation(name: &str) -> EngineConfigFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[test]
fn invalid_tool_name_returns_error_fixture() {
    let fixture = load_config_fixture("engine/engine_invalid_tool_name_fixture");
    let error = Engine::new(fixture.config).expect_err("invalid tool name should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "invalid tool name: Bad Name");
}

#[test]
fn invalid_input_schema_returns_error_fixture() {
    // Intentional bypass: this test validates Engine's rejection path for a
    // malformed input_schema shape that the config schema would reject first.
    let fixture =
        load_config_fixture_without_schema_validation("engine/engine_invalid_input_schema_fixture");
    let error = Engine::new(fixture.config).expect_err("invalid input schema should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "inputSchema must be an object");
}

#[test]
fn list_tools_includes_meta_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let tools = engine.list_tools();
    let tool = tools.first().expect("tool");

    assert_eq!(tool.name, "tool.meta");
    assert_eq!(
        tool.title.as_deref(),
        Some("Meta Tool"),
        "title should match fixture value"
    );
    let output_schema = tool
        .output_schema
        .as_ref()
        .expect("output_schema should be present");
    assert!(
        output_schema.contains_key("type"),
        "output_schema should have a type field"
    );
    let annotations = tool
        .annotations
        .as_ref()
        .expect("annotations should be present");
    assert_eq!(
        annotations.read_only_hint,
        Some(true),
        "annotations.readOnlyHint should be true from fixture"
    );
    let icons = tool.icons.as_ref().expect("icons should be present");
    assert_eq!(icons.len(), 1, "fixture defines exactly one icon");
    assert_eq!(icons[0].src, "https://example.com/icon.png");
    let meta = tool.meta.as_ref().expect("meta should be present");
    assert_eq!(
        meta.0.get("source").and_then(|v| v.as_str()),
        Some("config"),
        "meta.source should be 'config' from fixture"
    );
}

#[test]
fn engine_config_builds_engine_with_registry() {
    let fixture = load_config_fixture("engine/engine_list_tools_fixture");
    let config = EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new(),
        list_refresh_handle: None,
    };

    let engine = Engine::from_config(config).expect("engine should build");
    let tools = engine.list_tools();
    assert_eq!(tools.len(), 1, "fixture should build one tool");
    assert_eq!(tools[0].name, "tool.meta");
}

#[test]
fn engine_list_tools_is_stable_across_calls() {
    let fixture = load_config_fixture("engine/engine_list_tools_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");

    let first = engine.list_tools();
    let second = engine.list_tools();
    assert_eq!(first, second, "list_tools should be stable across calls");
}

#[test]
#[cfg(feature = "http_tools")]
fn allowlist_warns_on_extra_registered_plugins_fixture() {
    let fixture = load_config_fixture("engine/engine_allowlist_warn_fixture");
    let registry = PluginRegistry::new().register_tool(NoopPlugin).unwrap();

    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build even with extra registered plugins");

    // The engine should still build correctly — the extra plugin is just warned about,
    // not rejected. Verify the engine has the expected tool from the fixture.
    let tools = engine.list_tools();
    assert_eq!(tools.len(), 1, "fixture has exactly one tool");
    assert_eq!(tools[0].name, "api.ping", "fixture tool should be api.ping");
}

#[tokio::test]
async fn engine_notification_helpers_return_zero_without_observed_peers() {
    let fixture = load_config_fixture("engine/engine_list_tools_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");

    assert_eq!(engine.notify_tools_list_changed().await, 0);
    assert_eq!(engine.notify_prompts_list_changed().await, 0);
    assert_eq!(engine.notify_resources_list_changed().await, 0);
}
