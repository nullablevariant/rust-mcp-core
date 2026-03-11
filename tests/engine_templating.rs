use std::path::PathBuf;

use rust_mcp_core::engine::templating::{render_value, RenderContext};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct TemplateFixture {
    template: Value,
    args: Value,
    response: Option<Value>,
    expected: Value,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> TemplateFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[test]
fn renders_basic_values_fixture() {
    let fixture = load_fixture("templating/templating_basic_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn omits_optional_values_fixture() {
    let fixture = load_fixture("templating/templating_optional_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_csv_filter_fixture() {
    let fixture = load_fixture("templating/templating_csv_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_response_path_fixture() {
    let fixture = load_fixture("templating/templating_response_path_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_optional_root_fixture() {
    let fixture = load_fixture("templating/templating_optional_root_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_scalar_values_fixture() {
    let fixture = load_fixture("templating/templating_scalar_values_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_default_empty_fixture() {
    let fixture = load_fixture("templating/templating_default_empty_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_default_string_fixture() {
    let fixture = load_fixture("templating/templating_default_string_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_response_root_fixture() {
    let fixture = load_fixture("templating/templating_response_root_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_dot_path_fixture() {
    let fixture = load_fixture("templating/templating_dot_path_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_csv_null_fixture() {
    let fixture = load_fixture("templating/templating_csv_null_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_bool_object_in_string_fixture() {
    let fixture = load_fixture("templating/templating_bool_object_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_optional_omit_in_nested_structures_fixture() {
    let fixture = load_fixture("templating/templating_optional_omit_nested_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}

#[test]
fn renders_single_placeholder_fixture() {
    let fixture = load_fixture("templating/templating_single_placeholder_fixture");
    let ctx = RenderContext::new(&fixture.args, fixture.response.as_ref());
    let rendered = render_value(&fixture.template, &ctx).expect("render ok");
    assert_eq!(rendered, fixture.expected);
}
