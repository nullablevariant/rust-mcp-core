use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, Content, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct ReportsToolPlugin;

#[async_trait]
impl ToolPlugin for ReportsToolPlugin {
    fn name(&self) -> &'static str {
        "reports.tool"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        if ctx.cancellation.is_cancelled() {
            return Ok(CallToolResult::error(vec![Content::text(
                "request cancelled",
            )]));
        }

        Ok(CallToolResult::structured(json!({
            "report_id": args["report_id"],
            "status": "complete"
        })))
    }
}
