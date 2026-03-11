#![cfg(feature = "http_tools")]

mod engine_common;

use httpmock::{Method::GET, MockServer};
use rmcp::model::ErrorCode;
use rust_mcp_core::{
    config::{ResponseConfig, ResponseContentConfig},
    engine::Engine,
};
use std::path::PathBuf;

use engine_common::load_tool_fixture;
use rmcp::model::Content;
use rust_mcp_core::engine::tool_response::{build_content_result, build_structured_result};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct OutputFixture {
    structured: Option<Value>,
    output_schema: Option<Value>,
    fallback: Option<String>,
    content: Option<Vec<String>>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> OutputFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[test]
fn builds_structured_result_fixture() {
    let fixture = load_fixture("output/output_structured_fixture");
    let result = build_structured_result(
        fixture.structured.expect("structured"),
        fixture.output_schema.as_ref(),
        fixture.fallback.as_deref(),
    )
    .expect("structured result should build");

    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content should be present");
    assert_eq!(structured, &serde_json::json!({"ok": true}));
    assert_eq!(result.content.len(), 1);
    let text = result.content[0]
        .as_text()
        .expect("content should be text")
        .text
        .as_str();
    assert_eq!(text, "{\"ok\":true}");
}

#[test]
fn rejects_invalid_structured_output_fixture() {
    let fixture = load_fixture("output/output_structured_invalid_fixture");
    let error = build_structured_result(
        fixture.structured.expect("structured"),
        fixture.output_schema.as_ref(),
        fixture.fallback.as_deref(),
    )
    .expect_err("invalid structured output should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(
        error
            .message
            .starts_with("output schema validation failed:"),
        "expected message starting with 'output schema validation failed:' but got: {}",
        error.message
    );
}

#[test]
fn builds_content_result_fixture() {
    let fixture = load_fixture("output/output_content_fixture");
    let content = fixture
        .content
        .unwrap_or_default()
        .into_iter()
        .map(Content::text)
        .collect::<Vec<_>>();

    let result = build_content_result(content);
    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert_eq!(result.content.len(), 2);
    let first_text = result.content[0]
        .as_text()
        .expect("first content should be text")
        .text
        .as_str();
    assert_eq!(first_text, "first");
    let second_text = result.content[1]
        .as_text()
        .expect("second content should be text")
        .text
        .as_str();
    assert_eq!(second_text, "second");
}

#[test]
fn builds_structured_json_text_fallback_fixture() {
    let fixture = load_fixture("output/output_structured_json_text_fixture");
    let result = build_structured_result(
        fixture.structured.expect("structured"),
        fixture.output_schema.as_ref(),
        fixture.fallback.as_deref(),
    )
    .expect("structured result should build");

    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content should be present");
    assert_eq!(structured, &serde_json::json!({"ok": true}));

    assert_eq!(result.content.len(), 1);
    let text = result.content[0]
        .as_text()
        .expect("fallback content should be text")
        .text
        .as_str();
    assert_eq!(text, "{\"ok\":true}");
}

#[test]
fn structured_text_fallback_uses_raw_string_fixture() {
    let fixture = load_fixture("output/output_structured_text_string_fixture");
    let result = build_structured_result(
        fixture.structured.expect("structured"),
        fixture.output_schema.as_ref(),
        fixture.fallback.as_deref(),
    )
    .expect("structured result should build");

    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content should be present");
    assert_eq!(structured, &serde_json::json!("hello world"));

    assert_eq!(result.content.len(), 1);
    let text = result.content[0]
        .as_text()
        .expect("fallback content should be text")
        .text
        .as_str();
    assert_eq!(text, "hello world");
}

#[test]
fn invalid_output_schema_returns_error_fixture() {
    let fixture = load_fixture("output/output_invalid_schema_fixture");
    let error = build_structured_result(
        fixture.structured.expect("structured"),
        fixture.output_schema.as_ref(),
        fixture.fallback.as_deref(),
    )
    .expect_err("invalid output schema should fail");

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(
        error.message.contains("anyOf"),
        "invalid schema compile error should mention anyOf keyword, got: {}",
        error.message
    );
    assert!(
        error.message.contains("123"),
        "invalid schema compile error should include bad value token, got: {}",
        error.message
    );
}

#[tokio::test]
async fn resource_link_size_overflow_returns_error() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    config.tools_items_mut()[0].response = Some(ResponseConfig::Content(ResponseContentConfig {
        items: vec![serde_json::json!({
            "type": "resource_link",
            "uri": "file:///tmp/data.txt",
            "size": 4_294_967_296_u64
        })],
    }));
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let error = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect_err("overflowing resource_link size should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "resource_link size must be <= 4294967295");
}
