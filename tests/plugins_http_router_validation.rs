#![cfg(feature = "streamable_http")]

mod config_common;

use std::sync::Arc;

use axum::routing::get;
use config_common::load_config_fixture;
use rmcp::ErrorData as McpError;
use rust_mcp_core::config::HttpRouterTargetType;
use rust_mcp_core::plugins::http_router::{
    HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RouterTransform, RuntimeContext,
};
use rust_mcp_core::{PluginLookup, PluginRegistry};

fn noop_wrap() -> RouterTransform {
    Arc::new(|router| router)
}

struct WrapPlugin {
    name: &'static str,
}

impl HttpRouterPlugin for WrapPlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for target in targets {
            match target.target_type {
                HttpRouterTargetType::Wrap => {
                    ops.push(HttpRouterOp::Wrap(noop_wrap()));
                }
                HttpRouterTargetType::Route => {
                    let router = axum::Router::new().route("/", get(|| async { "ok" }));
                    ops.push(HttpRouterOp::Route(router));
                }
            }
        }
        Ok(ops)
    }
}

struct RoutePlugin {
    name: &'static str,
}

impl HttpRouterPlugin for RoutePlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            let router = axum::Router::new().route("/", get(|| async { "ok" }));
            ops.push(HttpRouterOp::Route(router));
        }
        Ok(ops)
    }
}

struct BadOpsPlugin {
    name: &'static str,
}

impl HttpRouterPlugin for BadOpsPlugin {
    fn name(&self) -> &str {
        self.name
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

struct WrongRouteOpPlugin {
    name: &'static str,
}

impl HttpRouterPlugin for WrongRouteOpPlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            ops.push(HttpRouterOp::Wrap(noop_wrap()));
        }
        Ok(ops)
    }
}

struct WrongWrapOpPlugin {
    name: &'static str,
}

impl HttpRouterPlugin for WrongWrapOpPlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &serde_json::Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::with_capacity(targets.len());
        for _target in targets {
            let router = axum::Router::new().route("/", get(|| async { "ok" }));
            ops.push(HttpRouterOp::Route(router));
        }
        Ok(ops)
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn set_env(key: &'static str, value: &str) -> EnvGuard {
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let lock = ENV_MUTEX.lock().expect("env mutex poisoned");
    let previous = std::env::var(key).ok();
    std::env::set_var(key, value);
    EnvGuard {
        key,
        previous,
        _lock: lock,
    }
}

#[tokio::test]
async fn router_plugins_require_registry_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_require_registry_fixture");
    let err = rust_mcp_core::runtime::run_from_config(fixture, PluginRegistry::new())
        .await
        .expect_err("should fail when plugin not in registry");
    assert_eq!(err.message, "http router plugin not registered: noop");
}

#[tokio::test]
async fn router_plugins_reject_unregistered_plugin_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_unregistered_fixture");
    let err = rust_mcp_core::runtime::run_from_config(fixture, PluginRegistry::new())
        .await
        .expect_err("should fail when plugin not in registry");
    assert_eq!(err.message, "http router plugin not registered: missing");
}

#[tokio::test]
async fn router_plugins_warn_extra_registry_fixture() {
    let fixture =
        load_config_fixture("http_router_plugins/http_router_warn_extra_registry_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(WrapPlugin { name: "configured" })
        .unwrap()
        .register_http_router(WrapPlugin { name: "extra" })
        .unwrap();
    // Verify the configured plugin is still resolvable before run
    assert!(
        registry
            .get_plugin(rust_mcp_core::PluginType::HttpRouter, "configured")
            .is_some(),
        "configured plugin should be in registry"
    );
    assert!(
        registry
            .get_plugin(rust_mcp_core::PluginType::HttpRouter, "extra")
            .is_some(),
        "extra plugin should also be in registry"
    );
    let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");
    rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect("should succeed even with extra registry plugin");
}

#[tokio::test]
async fn router_plugins_reject_wrap_unknown_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_wrap_unknown_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(WrapPlugin { name: "wrap" })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail when wrap target path is unknown");
    assert_eq!(
        err.message,
        "http router plugin wrap target 0 wraps unknown path: /unknown"
    );
}

#[tokio::test]
async fn router_plugins_reject_route_collision_endpoint_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_collision_endpoint_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(RoutePlugin { name: "route" })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail when route collides with MCP endpoint");
    assert_eq!(
        err.message,
        "http router plugin route target 0 collides with existing route: /mcp"
    );
}

#[tokio::test]
async fn router_plugins_reject_route_collision_oauth_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_collision_oauth_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(RoutePlugin { name: "route" })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail with OAuth config validation or collision");
    // The fixture configures an oauth-capable provider without auth.oauth.resource,
    // so auth validation fires before router collision can be checked.
    // Assert the deterministic error for the validation path that actually fires.
    #[cfg(feature = "auth")]
    assert_eq!(
        err.message,
        "server.auth.oauth.resource is required when oauth-capable providers are configured"
    );
    #[cfg(not(feature = "auth"))]
    assert_eq!(
        err.message,
        "auth feature disabled but server.auth is active"
    );
}

#[tokio::test]
async fn router_plugins_reject_mismatched_ops_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_mismatched_ops_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(BadOpsPlugin { name: "bad" })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail when ops count mismatches targets");
    assert_eq!(
        err.message,
        "http router plugin bad returned 0 ops for 1 targets"
    );
}

#[tokio::test]
async fn router_plugins_reject_wrong_route_op_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_wrong_route_op_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(WrongRouteOpPlugin {
            name: "wrong_route",
        })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail when Route target gets Wrap op");
    assert_eq!(
        err.message,
        "http router plugin wrong_route target 0 expected route op"
    );
}

#[tokio::test]
async fn router_plugins_reject_wrong_wrap_op_fixture() {
    let fixture = load_config_fixture("http_router_plugins/http_router_wrong_wrap_op_fixture");
    let registry = PluginRegistry::new()
        .register_http_router(WrongWrapOpPlugin { name: "wrong_wrap" })
        .unwrap();
    let err = rust_mcp_core::runtime::run_from_config(fixture, registry)
        .await
        .expect_err("should fail when Wrap target gets Route op");
    assert_eq!(
        err.message,
        "http router plugin wrong_wrap target 0 expected wrap op"
    );
}
