use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct ReportsAggregatePlugin;

#[async_trait]
impl ToolPlugin for ReportsAggregatePlugin {
    fn name(&self) -> &'static str {
        "reports.aggregate"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "plugin": self.name(),
            "args": args
        })))
    }
}
