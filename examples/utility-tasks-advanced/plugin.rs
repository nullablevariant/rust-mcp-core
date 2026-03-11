use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, Content, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

pub(crate) struct ReportsGeneratePlugin;
pub(crate) struct ReportsPreviewPlugin;

#[async_trait]
impl ToolPlugin for ReportsGeneratePlugin {
    fn name(&self) -> &'static str {
        "reports.generate"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let report_id = args
            .get("report_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        tokio::select! {
            () = ctx.cancellation.cancelled() => {
                Ok(CallToolResult::error(vec![Content::text("request cancelled")]))
            }
            () = sleep(Duration::from_secs(2)) => {
                Ok(CallToolResult::structured(json!({
                    "report_id": report_id,
                    "status": "complete"
                })))
            }
        }
    }
}

#[async_trait]
impl ToolPlugin for ReportsPreviewPlugin {
    fn name(&self) -> &'static str {
        "reports.preview"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "report_id": args.get("report_id").cloned().unwrap_or(json!(null)),
            "preview": true
        })))
    }
}
