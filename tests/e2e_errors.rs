mod e2e_common;

#[cfg(feature = "http_tools")]
use e2e_common::make_noop_tool;
use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    FailPlugin, SmokeTestClient,
};
use rmcp::model::{CallToolRequestParams, ErrorCode};
use rmcp::ServiceError;
#[cfg(feature = "http_tools")]
use rust_mcp_core::config::PaginationConfig;
use rust_mcp_core::plugins::PluginRegistry;
use serde_json::json;

fn assert_mcp_error(
    err: ServiceError,
    expected_code: ErrorCode,
    message_contains: &str,
    expected_field_token: Option<&str>,
) {
    match err {
        ServiceError::McpError(error_data) => {
            assert_eq!(
                error_data.code, expected_code,
                "expected error code {:?}, got {:?}: {}",
                expected_code, error_data.code, error_data.message
            );
            assert!(
                error_data.message.contains(message_contains),
                "error message should contain '{}', got: {}",
                message_contains,
                error_data.message
            );
            // If a field/path token is specified, assert it appears in the message
            // to differentiate this error from other errors of the same category
            if let Some(field_token) = expected_field_token {
                assert!(
                    error_data.message.contains(field_token),
                    "error message should contain field token '{}', got: {}",
                    field_token,
                    error_data.message
                );
            }
        }
        other => panic!("expected McpError, got: {other:?}"),
    }
}

fn assert_tool_error_text(result: &rmcp::model::CallToolResult, expected: &str) {
    assert_eq!(
        result.is_error,
        Some(true),
        "tool result should be flagged as error"
    );
    let value = serde_json::to_value(result).expect("serialize tool result");
    let content_array = value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .expect("tool error result should include content array");
    assert_eq!(
        content_array.len(),
        1,
        "tool error should have exactly 1 content block, got: {content_array:?}"
    );
    let block = &content_array[0];
    assert_eq!(
        block.get("type").and_then(serde_json::Value::as_str),
        Some("text"),
        "content block type should be 'text'"
    );
    let message = block
        .get("text")
        .and_then(serde_json::Value::as_str)
        .expect("content block should have text field");
    assert!(
        message.contains(expected),
        "error message should contain '{expected}', got: {message}"
    );
}

#[tokio::test]
async fn e2e_call_nonexistent_tool() {
    let config = make_minimal_config();
    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let err = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("does_not_exist")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .expect_err("should fail for nonexistent tool");

    assert_mcp_error(
        err,
        ErrorCode::INVALID_PARAMS,
        "not found",
        Some("does_not_exist"),
    );
}

#[tokio::test]
async fn e2e_plugin_tool_returns_error() {
    let mut config = make_minimal_config();
    config.set_tools_items(vec![make_plugin_tool("fail", "fail")]);
    config.plugins = vec![make_plugin_allowlist("fail")];

    let registry = PluginRegistry::new()
        .register_tool(FailPlugin)
        .expect("register fail");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("fail")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .expect("plugin failures should return tool errors");
    // When expose_internal_details is false (default), error message is redacted
    assert_tool_error_text(&result, "internal server error");
    // structured_content should be None for error results
    assert!(
        result.structured_content.is_none(),
        "error result should not have structured_content"
    );
}

#[tokio::test]
async fn e2e_plugin_tool_exposes_error_when_enabled() {
    let mut config = make_minimal_config();
    config.server.errors.expose_internal_details = true;
    config.set_tools_items(vec![make_plugin_tool("fail", "fail")]);
    config.plugins = vec![make_plugin_allowlist("fail")];

    let registry = PluginRegistry::new()
        .register_tool(FailPlugin)
        .expect("register fail");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("fail")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .expect("plugin failures should return tool errors");
    // When expose_internal_details is true, the original plugin error message is exposed
    assert_tool_error_text(&result, "plugin failure");
    // structured_content should be None for error results
    assert!(
        result.structured_content.is_none(),
        "error result should not have structured_content"
    );
}

#[cfg(feature = "prompts")]
#[tokio::test]
async fn e2e_get_nonexistent_prompt() {
    let mut config = make_minimal_config();
    config.prompts = Some(rust_mcp_core::config::PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let err = client_service
        .peer()
        .get_prompt(rmcp::model::GetPromptRequestParams::new("missing"))
        .await
        .expect_err("should fail for nonexistent prompt");

    assert_mcp_error(err, ErrorCode::INVALID_PARAMS, "not found", Some("missing"));
}

#[tokio::test]
#[cfg(feature = "http_tools")]
async fn e2e_list_tools_invalid_cursor_maps_to_invalid_params() {
    let mut config = make_minimal_config();
    config.pagination = Some(PaginationConfig { page_size: 1 });
    let mut second_tool = make_noop_tool();
    second_tool.name = "noop_second".to_owned();
    config.tools_items_mut().push(second_tool);

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let err = client_service
        .peer()
        .list_tools(Some(
            rmcp::model::PaginatedRequestParams::default()
                .with_cursor(Some("not-a-valid-cursor".to_owned())),
        ))
        .await
        .expect_err("invalid cursor should fail");

    assert_mcp_error(err, ErrorCode::INVALID_PARAMS, "invalid cursor", None);
}
