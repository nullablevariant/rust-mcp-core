#![cfg(feature = "http_tools")]

mod engine_common;

use async_trait::async_trait;
use engine_common::{fixture_path, load_tool_fixture, EngineToolFixture};
use httpmock::{Method::GET, MockServer};
use rmcp::{model::CallToolResult, ErrorData as McpError};
use rust_mcp_core::{
    config::{
        ExecuteConfig, ExecutePluginConfig, OutboundHttpConfig, PluginConfig, ResponseConfig,
        ResponseContentConfig, ToolConfig,
    },
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, ToolPlugin},
};
use serde_json::{json, Value};

fn load_tool_fixture_without_schema_validation(name: &str) -> EngineToolFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

async fn execute_with_content_blocks(
    content: Vec<Value>,
    upstream_payload: Value,
) -> Result<CallToolResult, McpError> {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let mut config = fixture.config;
    config.tools_items_mut()[0].response = Some(ResponseConfig::Content(ResponseContentConfig {
        items: content,
    }));

    let server = MockServer::start();
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(upstream_payload);
    });

    let engine = Engine::new(config).expect("engine should build");
    engine.execute_tool(&fixture.tool, fixture.args).await
}

fn assert_invalid_params(error: &McpError, expected_message: &str) {
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, expected_message);
    assert_eq!(error.data, None);
}

fn assert_invalid_params_message_prefix(error: &McpError, expected_prefix: &str) {
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.starts_with(expected_prefix),
        "expected invalid_params message '{}' to start with '{}'",
        error.message,
        expected_prefix
    );
    assert_eq!(error.data, None);
}

fn assert_tool_error_text(result: &CallToolResult, expected_text: &str) {
    assert_eq!(result.is_error, Some(true));
    assert!(result.structured_content.is_none());
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(content, json!([{ "type": "text", "text": expected_text }]));
}

#[tokio::test]
async fn content_mode_renders_rich_content_blocks_fixture() {
    let fixture = load_tool_fixture("engine/engine_rich_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/rich");
        then.status(200).json_body(json!({
            "summary": "hello",
            "image_base64": "aW1hZ2U=",
            "audio_base64": "YXVkaW8=",
            "resource_uri": "file:///tmp/data.txt",
            "resource_text": "resource text"
        }));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool ok");

    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert_eq!(result.content.len(), 5);

    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(content[0]["type"], json!("text"));
    assert_eq!(content[0]["text"], json!("hello"));
    assert_eq!(
        content[0]["annotations"]["lastModified"],
        json!("2025-01-01T00:00:00Z")
    );
    assert!(content[0].get("_meta").is_none());
    assert_eq!(content[1]["type"], json!("image"));
    assert_eq!(content[1]["mimeType"], json!("image/png"));
    assert!(content[1].get("_meta").is_none());
    assert_eq!(content[2]["type"], json!("audio"));
    assert_eq!(content[2]["mimeType"], json!("audio/wav"));
    assert!(content[2].get("_meta").is_none());
    assert_eq!(content[3]["type"], json!("resource_link"));
    assert_eq!(content[3]["uri"], json!("file:///tmp/data.txt"));
    assert_eq!(content[3]["icons"][0]["mimeType"], json!("image/png"));
    assert!(content[3].get("_meta").is_none());
    assert_eq!(content[4]["type"], json!("resource"));
    assert_eq!(content[4]["resource"]["uri"], json!("file:///tmp/data.txt"));
    assert_eq!(content[4]["resource"]["text"], json!("resource text"));
    assert_eq!(content[4]["annotations"]["audience"], json!(["assistant"]));
    let priority = content[4]["annotations"]["priority"]
        .as_f64()
        .expect("priority should be numeric");
    assert!((priority - 0.6).abs() < 0.0001);
    assert!(content[4].get("_meta").is_none());
    assert!(content[4]["resource"].get("_meta").is_none());
}

#[tokio::test]
async fn content_mode_invalid_block_returns_tool_error_fixture() {
    // Intentional bypass: verifies tool error shaping for malformed content
    // block fixtures that schema validation would reject before engine logic.
    let fixture =
        load_tool_fixture_without_schema_validation("engine/engine_content_invalid_block_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/bad-content");
        then.status(200).json_body(json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("invalid content block should fail fast");
    assert_invalid_params(&result, "tool response content item must be an object");
}

#[tokio::test]
async fn http_execution_failure_returns_tool_error_result() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(502);
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool result should be returned");

    assert_tool_error_text(&result, "upstream returned 502");
}

#[tokio::test]
async fn non_cancellable_http_failure_returns_tool_error_result() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    config.tools_items_mut()[0].cancellable = false;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(504);
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool result should be returned");

    assert_tool_error_text(&result, "upstream returned 504");
}

#[tokio::test]
async fn http_tool_respects_outbound_http_max_response_bytes() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.server.errors.expose_internal_details = true;
    config.outbound_http = Some(OutboundHttpConfig {
        headers: std::collections::HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: Some(8),
        retry: None,
    });
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200)
            .json_body(serde_json::json!({"message": "0123456789"}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool result should be returned");

    assert_tool_error_text(
        &result,
        "outbound response body exceeds configured max_response_bytes",
    );
}

#[tokio::test]
async fn upstream_max_response_bytes_overrides_global_limit() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.outbound_http = Some(OutboundHttpConfig {
        headers: std::collections::HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: Some(8),
        retry: None,
    });
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
        upstream.max_response_bytes = Some(1_024);
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool should execute");

    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert_eq!(result.content.len(), 2);
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(
        content,
        json!([
            {"type": "text", "text": "static"},
            {"type": "text", "text": "5"}
        ])
    );
}

#[tokio::test]
async fn content_mode_text_block_stringifies_non_string_values() {
    let result = execute_with_content_blocks(
        vec![json!({"type": "text", "text": "${$.count}"})],
        json!({"count": 7}),
    )
    .await
    .expect("tool ok");
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(content[0]["text"], json!("7"));
}

#[tokio::test]
async fn content_mode_rejects_unsupported_type() {
    let result = execute_with_content_blocks(
        vec![json!({"type": "video", "data": "abc"})],
        json!({"count": 7}),
    )
    .await
    .expect_err("unsupported type should fail");
    assert_invalid_params(&result, "unsupported tool response content type: video");
}

#[tokio::test]
async fn content_mode_rejects_image_without_mime_type() {
    let result = execute_with_content_blocks(
        vec![json!({"type": "image", "data": "aW1hZw=="})],
        json!({}),
    )
    .await
    .expect_err("image without mime type should fail");
    assert_invalid_params(&result, "image content requires mime_type");
}

#[tokio::test]
async fn content_mode_rejects_audio_without_mime_type() {
    let result = execute_with_content_blocks(
        vec![json!({"type": "audio", "data": "YXVkaW8="})],
        json!({}),
    )
    .await
    .expect_err("audio without mime type should fail");
    assert_invalid_params(&result, "audio content requires mime_type");
}

#[tokio::test]
async fn content_mode_rejects_invalid_resource_link_icons_shape() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt",
            "icons": "not-array"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid icons should fail");
    assert_invalid_params(&result, "icons must be an array");
}

#[tokio::test]
async fn content_mode_rejects_resource_without_payload() {
    let result = execute_with_content_blocks(vec![json!({"type": "resource"})], json!({}))
        .await
        .expect_err("resource without payload should fail");
    assert_invalid_params(&result, "resource content requires resource");
}

#[tokio::test]
async fn content_mode_rejects_resource_when_resource_is_not_object() {
    let result = execute_with_content_blocks(
        vec![json!({"type": "resource", "resource": "nope"})],
        json!({}),
    )
    .await
    .expect_err("resource value must be an object");
    assert_invalid_params(&result, "resource content requires resource object");
}

#[tokio::test]
async fn content_mode_rejects_resource_with_both_text_and_blob() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "resource": {
                "uri": "file:///tmp/data.txt",
                "text": "hello",
                "blob": "aGVsbG8="
            }
        })],
        json!({}),
    )
    .await
    .expect_err("resource with both text and blob should fail");
    assert_invalid_params(
        &result,
        "resource content cannot include both text and blob",
    );
}

#[tokio::test]
async fn content_mode_rejects_resource_without_text_or_blob() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "resource": {
                "uri": "file:///tmp/data.txt"
            }
        })],
        json!({}),
    )
    .await
    .expect_err("resource without text/blob should fail");
    assert_invalid_params(&result, "resource content requires text or blob");
}

#[tokio::test]
async fn content_mode_rejects_resource_annotations_in_both_locations() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "annotations": {"priority": 0.1},
            "resource": {
                "uri": "file:///tmp/data.txt",
                "text": "hello",
                "annotations": {"priority": 0.2}
            }
        })],
        json!({}),
    )
    .await
    .expect_err("duplicate annotation locations should fail");
    assert_invalid_params(
        &result,
        "resource content annotations must be set either at top level or in resource",
    );
}

#[tokio::test]
async fn content_mode_rejects_invalid_annotations_shape() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "text",
            "text": "hello",
            "annotations": "bad"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid annotations should fail");
    assert_invalid_params(&result, "annotations must be an object");
}

#[tokio::test]
async fn content_mode_rejects_invalid_text_meta() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "text",
            "text": "hello",
            "_meta": "bad"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid text meta should fail");
    assert_invalid_params_message_prefix(&result, "invalid text _meta:");
}

#[tokio::test]
async fn content_mode_resource_link_defaults_name_when_missing() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt"
        })],
        json!({}),
    )
    .await
    .expect("tool ok");
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(content[0]["name"], json!("resource"));
}

#[tokio::test]
async fn content_mode_rejects_resource_link_icon_entries_that_are_not_objects() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt",
            "icons": ["bad"]
        })],
        json!({}),
    )
    .await
    .expect_err("invalid icon item should fail");
    assert_invalid_params(&result, "icon entries must be objects");
}

#[tokio::test]
async fn content_mode_rejects_invalid_resource_link_meta() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt",
            "_meta": "bad"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid resource_link meta should fail");
    assert_invalid_params_message_prefix(&result, "invalid resource_link _meta:");
}

#[tokio::test]
async fn content_mode_rejects_invalid_resource_link_annotations() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt",
            "annotations": "bad"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid resource_link annotations should fail");
    assert_invalid_params(&result, "annotations must be an object");
}

#[tokio::test]
async fn content_mode_rejects_invalid_image_meta() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "image",
            "data": "aW1hZw==",
            "mime_type": "image/png",
            "_meta": "bad"
        })],
        json!({}),
    )
    .await
    .expect_err("invalid image meta should fail");
    assert_invalid_params_message_prefix(&result, "invalid image _meta:");
}

#[tokio::test]
async fn content_mode_rejects_resource_when_uri_missing() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "resource": {
                "text": "hello"
            }
        })],
        json!({}),
    )
    .await
    .expect_err("resource without uri should fail");
    assert_invalid_params(&result, "resource content requires uri");
}

#[tokio::test]
async fn content_mode_rejects_invalid_resource_content_meta() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "resource": {
                "uri": "file:///tmp/data.txt",
                "text": "hello",
                "_meta": "bad"
            }
        })],
        json!({}),
    )
    .await
    .expect_err("invalid resource content meta should fail");
    assert_invalid_params_message_prefix(&result, "invalid resource content _meta:");
}

#[tokio::test]
async fn content_mode_rejects_invalid_resource_embedded_meta() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "_meta": "bad",
            "resource": {
                "uri": "file:///tmp/data.txt",
                "text": "hello"
            }
        })],
        json!({}),
    )
    .await
    .expect_err("invalid embedded meta should fail");
    assert_invalid_params_message_prefix(&result, "invalid resource _meta:");
}

#[tokio::test]
async fn content_mode_accepts_audio_annotations() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "audio",
            "data": "YXVkaW8=",
            "mime_type": "audio/wav",
            "annotations": {
                "priority": 0.5
            }
        })],
        json!({}),
    )
    .await
    .expect("tool ok");
    let content = serde_json::to_value(&result.content).expect("serialize content");
    assert_eq!(content[0]["annotations"]["priority"], json!(0.5));
}

#[tokio::test]
async fn content_mode_resource_accepts_top_level_annotations() {
    let result = execute_with_content_blocks(
        vec![json!({
            "type": "resource",
            "annotations": { "priority": 0.7 },
            "resource": {
                "uri": "file:///tmp/data.txt",
                "mimeType": "text/plain",
                "text": "hello"
            }
        })],
        json!({}),
    )
    .await
    .expect("tool ok");

    let content = serde_json::to_value(&result.content).expect("serialize content");
    let priority = content[0]["annotations"]["priority"]
        .as_f64()
        .expect("priority should be numeric");
    assert!((priority - 0.7).abs() < 0.0001);
    assert_eq!(content[0]["resource"]["mimeType"], json!("text/plain"));
}

#[derive(Default)]
struct FailingPlugin;

#[async_trait]
impl ToolPlugin for FailingPlugin {
    fn name(&self) -> &'static str {
        "plugin.fail"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("plugin exploded".to_owned(), None))
    }
}

#[derive(Default)]
struct ToolErrorResultPlugin;

#[async_trait]
impl ToolPlugin for ToolErrorResultPlugin {
    fn name(&self) -> &'static str {
        "plugin.tool_error"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::error(vec![rmcp::model::Content::text(
            "invalid user input",
        )]))
    }
}

#[tokio::test]
async fn plugin_execution_failure_returns_tool_error_result() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let mut config = fixture.config;
    config.set_tools_items(vec![ToolConfig {
        name: "plugin.fail".to_owned(),
        title: None,
        description: "failing plugin".to_owned(),
        cancellable: true,
        input_schema: json!({"type": "object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.fail".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.plugins = vec![PluginConfig {
        name: "plugin.fail".to_owned(),
        plugin_type: rust_mcp_core::plugins::PluginType::Tool,
        targets: None,
        config: None,
    }];

    let registry = PluginRegistry::new()
        .register_tool(FailingPlugin)
        .expect("plugin should register");

    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");
    let result = engine
        .execute_tool("plugin.fail", json!({}))
        .await
        .expect("plugin errors should return tool error result");
    assert_tool_error_text(&result, "internal server error");
}

#[tokio::test]
async fn non_cancellable_plugin_failure_returns_tool_error_result() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let mut config = fixture.config;
    config.set_tools_items(vec![ToolConfig {
        name: "plugin.fail".to_owned(),
        title: None,
        description: "failing plugin".to_owned(),
        cancellable: false,
        input_schema: json!({"type": "object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.fail".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.plugins = vec![PluginConfig {
        name: "plugin.fail".to_owned(),
        plugin_type: rust_mcp_core::plugins::PluginType::Tool,
        targets: None,
        config: None,
    }];

    let registry = PluginRegistry::new()
        .register_tool(FailingPlugin)
        .expect("plugin should register");

    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.fail", json!({}))
        .await
        .expect("plugin errors should return tool error result");
    assert_tool_error_text(&result, "internal server error");
}

#[tokio::test]
async fn plugin_tool_error_result_is_preserved() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let mut config = fixture.config;
    config.set_tools_items(vec![ToolConfig {
        name: "plugin.tool_error".to_owned(),
        title: None,
        description: "plugin tool error result".to_owned(),
        cancellable: true,
        input_schema: json!({"type": "object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "plugin.tool_error".to_owned(),
            config: None,
            task_support: rust_mcp_core::config::TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.plugins = vec![PluginConfig {
        name: "plugin.tool_error".to_owned(),
        plugin_type: rust_mcp_core::plugins::PluginType::Tool,
        targets: None,
        config: None,
    }];

    let registry = PluginRegistry::new()
        .register_tool(ToolErrorResultPlugin)
        .expect("plugin should register");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let result = engine
        .execute_tool("plugin.tool_error", json!({}))
        .await
        .expect("tool result should be returned");
    assert_tool_error_text(&result, "invalid user input");
}
