use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct ProgressDemoPlugin;

#[async_trait]
impl ToolPlugin for ProgressDemoPlugin {
    fn name(&self) -> &'static str {
        "progress.demo"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        ctx.notify_progress(10.0, Some(100.0), Some("started".to_owned()))
            .await?;
        ctx.notify_progress(50.0, Some(100.0), Some("running".to_owned()))
            .await?;
        ctx.notify_progress(100.0, Some(100.0), Some("done".to_owned()))
            .await?;

        Ok(CallToolResult::structured(json!({"status": "complete"})))
    }
}
