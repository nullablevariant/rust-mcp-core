#![cfg(feature = "client_features")]

mod e2e_common;

use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    ClientFeaturesPlugin, SmokeTestClient,
};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, CreateElicitationResult, CreateMessageResult,
    ElicitationAction, Role, SamplingMessage, SamplingMessageContent,
};
use rust_mcp_core::plugins::PluginRegistry;
use serde_json::json;

fn client_features_config() -> rust_mcp_core::config::McpConfig {
    let mut config = make_minimal_config();
    config.set_tools_items(vec![make_plugin_tool("client_features", "client_features")]);
    config.plugins = vec![make_plugin_allowlist("client_features")];
    config
}

fn client_features_registry() -> PluginRegistry {
    PluginRegistry::new()
        .register_tool(ClientFeaturesPlugin)
        .expect("register client_features")
}

#[tokio::test]
async fn e2e_client_roots() {
    let mut config = client_features_config();
    config.client_features.roots = Some(rust_mcp_core::config::ClientRootsConfig { enabled: true });

    let engine = build_engine(config, client_features_registry());
    let root = serde_json::from_value(json!({
        "uri": "file:///tmp/test",
        "name": "test root"
    }))
    .expect("root");
    let client = SmokeTestClient::new()
        .with_capabilities(ClientCapabilities::builder().enable_roots().build())
        .with_roots(vec![root]);
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("client_features")
                .with_arguments(json!({"mode": "roots"}).as_object().unwrap().clone()),
        )
        .await
        .expect("call_tool roots");

    // Assert structured payload shape: count + expected key
    let structured = result
        .structured_content
        .expect("should have structured content");
    assert_eq!(
        structured.get("roots").and_then(serde_json::Value::as_u64),
        Some(1),
        "should report exactly 1 root"
    );
    // Verify no unexpected extra keys in the response
    let obj = structured
        .as_object()
        .expect("structured content should be an object");
    assert_eq!(
        obj.len(),
        1,
        "structured content should have exactly 1 key ('roots')"
    );
}

#[tokio::test]
async fn e2e_client_sampling() {
    let mut config = client_features_config();
    config.client_features.sampling = Some(rust_mcp_core::config::ClientSamplingConfig {
        enabled: Some(true),
        allow_tools: false,
    });

    let engine = build_engine(config, client_features_registry());
    let client = SmokeTestClient::new()
        .with_capabilities(ClientCapabilities::builder().enable_sampling().build())
        .with_sampling_response(
            CreateMessageResult::new(
                SamplingMessage::new(
                    Role::Assistant,
                    SamplingMessageContent::text("sampled response"),
                ),
                "test-model".to_owned(),
            )
            .with_stop_reason("end_turn"),
        );
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("client_features")
                .with_arguments(json!({"mode": "sampling"}).as_object().unwrap().clone()),
        )
        .await
        .expect("call_tool sampling");

    let structured = result
        .structured_content
        .expect("should have structured content");
    assert_eq!(
        structured.get("model").and_then(|v| v.as_str()),
        Some("test-model"),
        "model should be 'test-model'"
    );
    assert_eq!(
        structured.get("role").and_then(|v| v.as_str()),
        Some("Assistant"),
        "role should be 'Assistant' (Debug format of Role::Assistant)"
    );
    // The plugin returns {model, role} — verify no unexpected keys
    let obj = structured
        .as_object()
        .expect("structured content should be an object");
    assert_eq!(
        obj.len(),
        2,
        "structured content should have exactly 2 keys (model, role)"
    );
}

#[tokio::test]
async fn e2e_client_elicitation() {
    let mut config = client_features_config();
    config.client_features.elicitation = Some(rust_mcp_core::config::ClientElicitationConfig {
        enabled: Some(true),
        mode: rust_mcp_core::config::ElicitationMode::Form,
    });

    let engine = build_engine(config, client_features_registry());
    let client = SmokeTestClient::new()
        .with_capabilities(ClientCapabilities::builder().enable_elicitation().build())
        .with_elicitation_response(CreateElicitationResult {
            action: ElicitationAction::Accept,
            content: Some(json!({"name": "test"})),
        });
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("client_features")
                .with_arguments(json!({"mode": "elicitation"}).as_object().unwrap().clone()),
        )
        .await
        .expect("call_tool elicitation");

    let structured = result
        .structured_content
        .expect("should have structured content");
    assert_eq!(
        structured.get("action").and_then(|v| v.as_str()),
        Some("Accept"),
        "action should be 'Accept' (Debug format of ElicitationAction::Accept)"
    );
    // The plugin returns {action} — verify shape
    let obj = structured
        .as_object()
        .expect("structured content should be an object");
    assert_eq!(
        obj.len(),
        1,
        "structured content should have exactly 1 key (action)"
    );
}
