mod e2e_common;

use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    EchoPlugin, SmokeTestClient,
};
use rmcp::model::CallToolRequestParams;
use rust_mcp_core::plugins::PluginRegistry;
use serde_json::json;

#[tokio::test]
async fn e2e_list_tools_and_call_plugin() {
    let mut config = make_minimal_config();
    config.set_tools_items(vec![make_plugin_tool("echo", "echo")]);
    config.plugins = vec![make_plugin_allowlist("echo")];

    let registry = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .expect("register echo");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let tools = client_service
        .peer()
        .list_tools(None)
        .await
        .expect("list_tools");
    // Assert deterministic tool list shape: exactly 1 tool named "echo"
    assert_eq!(tools.tools.len(), 1, "should have exactly 1 tool listed");
    let echo_tool = &tools.tools[0];
    assert_eq!(
        echo_tool.name.as_ref(),
        "echo",
        "tool name should be 'echo'"
    );
    assert_eq!(
        echo_tool.description.as_deref(),
        Some("echo tool"),
        "tool description should be 'echo tool'"
    );

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("echo")
                .with_arguments(json!({"key": "value"}).as_object().unwrap().clone()),
        )
        .await
        .expect("call_tool echo");

    // Assert full call result contract: is_error is Some(false) for successful tool results
    assert_eq!(
        result.is_error,
        Some(false),
        "echo result is_error should be Some(false) for success"
    );
    assert_eq!(
        result.structured_content,
        Some(json!({"key": "value"})),
        "echo should return the args as structured content"
    );
}
