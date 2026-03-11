use std::path::PathBuf;

use rmcp::model::ErrorCode;
use rust_mcp_core::engine::templating::{render_value, RenderContext};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct TemplateErrorFixture {
    template: Value,
    args: Value,
    response: Option<Value>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> TemplateErrorFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[test]
fn missing_required_value_returns_error_fixture() {
    let fixture = load_fixture("templating/templating_missing_required_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let error =
        render_value(&fixture.template, &ctx).expect_err("missing required value should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "missing required value for 'missing'");
}

#[test]
fn invalid_filter_returns_error_fixture() {
    let fixture = load_fixture("templating/templating_invalid_filter_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let error = render_value(&fixture.template, &ctx).expect_err("invalid filter should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "unsupported filter 'unknown'");
}

#[test]
fn unterminated_expression_returns_error_fixture() {
    let fixture = load_fixture("templating/templating_unterminated_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let error =
        render_value(&fixture.template, &ctx).expect_err("unterminated expression should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "unterminated template expression");
}

#[test]
fn array_index_path_renders_first_element() {
    let args = serde_json::json!({});
    let response = serde_json::json!({"items": [{"name": "first"}, {"name": "second"}]});
    let template = serde_json::json!({"value": "${$.items[0].name}"});
    let ctx = RenderContext::new(&args, Some(&response));
    let result = render_value(&template, &ctx).expect("render first element ok");
    assert_eq!(result, serde_json::json!({"value": "first"}));
}

#[test]
fn array_index_path_renders_fixture() {
    let fixture = load_fixture("templating/templating_array_index_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let result = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(result, serde_json::json!({"value": "second"}));
}

#[test]
fn array_index_out_of_range_returns_error() {
    let args = serde_json::json!({});
    let response = serde_json::json!({"items": [{"name": "first"}, {"name": "second"}]});
    let template = serde_json::json!({"value": "${$.items[99].name}"});
    let ctx = RenderContext::new(&args, Some(&response));
    let error = render_value(&template, &ctx).expect_err("out-of-range array index should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message,
        "missing required value for '$.items[99].name'"
    );
}

#[test]
fn empty_expression_returns_error_fixture() {
    let fixture = load_fixture("templating/templating_empty_expression_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let error = render_value(&fixture.template, &ctx).expect_err("empty expression should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "empty template expression");
}

#[test]
fn nested_placeholder_returns_error_fixture() {
    let fixture = load_fixture("templating/templating_nested_placeholder_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let error = render_value(&fixture.template, &ctx).expect_err("nested placeholder should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "missing required value for 'value${nested'");
}
