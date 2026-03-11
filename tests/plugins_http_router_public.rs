#![cfg(feature = "streamable_http")]

use std::path::PathBuf;

use rmcp::ErrorData as McpError;
use rust_mcp_core::plugins::http_router::{
    AuthSummary, HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RuntimeContext,
};
use rust_mcp_core::{PluginLookup, PluginRef, PluginRegistry, PluginType};
use serde::Deserialize;

#[derive(Deserialize)]
struct RegistryFixture {
    name: String,
}

#[derive(Deserialize)]
struct AuthSummaryFixture {
    auth_enabled: bool,
    oauth_enabled: bool,
    resource_url: Option<String>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

struct NoopPlugin {
    name: String,
}

impl HttpRouterPlugin for NoopPlugin {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        _targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        Ok(Vec::new())
    }
}

#[test]
fn registry_registers_and_lists_names_fixture() {
    let fixture: RegistryFixture = load_fixture("http_router_plugins/http_router_registry_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(NoopPlugin {
            name: fixture.name.clone(),
        })
        .unwrap();
    let plugin_ref = registry
        .get_plugin(PluginType::HttpRouter, &fixture.name)
        .expect("plugin should be registered");
    assert!(
        matches!(plugin_ref, PluginRef::HttpRouter(_)),
        "plugin should be HttpRouter variant"
    );
    let names = registry.names();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].0, "noop");
    assert_eq!(names[0].1, PluginType::HttpRouter);
}

#[test]
fn auth_wrap_returns_none_when_auth_disabled_fixture() {
    let fixture: AuthSummaryFixture =
        load_fixture("http_router_plugins/http_router_auth_wrap_none_fixture");
    let summary = AuthSummary {
        auth_enabled: fixture.auth_enabled,
        oauth_enabled: fixture.oauth_enabled,
        resource_url: fixture.resource_url,
    };
    let ctx = RuntimeContext::new(summary, None);
    assert!(
        ctx.auth_wrap().is_none(),
        "auth_wrap should be None when auth is disabled"
    );
    assert!(!ctx.auth().auth_enabled);
    assert!(!ctx.auth().oauth_enabled);
    assert_eq!(ctx.auth().resource_url, None);
}

#[test]
fn auth_accessor_returns_summary_fixture() {
    let fixture: AuthSummaryFixture =
        load_fixture("http_router_plugins/http_router_auth_accessor_fixture");
    let summary = AuthSummary {
        auth_enabled: fixture.auth_enabled,
        oauth_enabled: fixture.oauth_enabled,
        resource_url: fixture.resource_url,
    };
    let ctx = RuntimeContext::new(summary, None);
    assert!(ctx.auth().auth_enabled);
    assert!(!ctx.auth().oauth_enabled);
    assert_eq!(
        ctx.auth().resource_url,
        Some("http://example.com/mcp".to_owned())
    );
}
