use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct WorkspaceToolPlugin;

#[async_trait]
impl ToolPlugin for WorkspaceToolPlugin {
    fn name(&self) -> &'static str {
        "workspace.tool"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let roots = ctx.request_roots().await?;
        let names: Vec<&str> = roots
            .roots
            .iter()
            .map(|root| root.name.as_deref().unwrap_or("unnamed"))
            .collect();

        Ok(CallToolResult::structured(json!({
            "workspace_roots": names
        })))
    }
}
