use std::sync::Arc;

use axum::{routing::get, Router};
use rust_mcp_core::McpError;
use rust_mcp_core::{
    config::HttpRouterTargetType,
    plugins::{HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RouterTransform, RuntimeContext},
};
use serde_json::Value;

pub(crate) struct HealthzRouterPlugin;
pub(crate) struct McpGuardRouterPlugin;

impl HttpRouterPlugin for HealthzRouterPlugin {
    fn name(&self) -> &'static str {
        "healthz"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::new();
        for target in targets {
            if target.target_type == HttpRouterTargetType::Route {
                let router = Router::new().route(&target.path, get(|| async { "ok" }));
                ops.push(HttpRouterOp::Route(router));
            }
        }
        Ok(ops)
    }
}

impl HttpRouterPlugin for McpGuardRouterPlugin {
    fn name(&self) -> &'static str {
        "mcp_guard"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        _config: &Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        let mut ops = Vec::new();
        for target in targets {
            if target.target_type == HttpRouterTargetType::Wrap {
                let passthrough: RouterTransform = Arc::new(|router| router);
                let _ = &target.path;
                ops.push(HttpRouterOp::Wrap(passthrough));
            }
        }
        Ok(ops)
    }
}
