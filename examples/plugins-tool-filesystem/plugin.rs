use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct FsReadPlugin;
pub(crate) struct FsWritePlugin;
pub(crate) struct FsDeletePlugin;

#[async_trait]
impl ToolPlugin for FsReadPlugin {
    fn name(&self) -> &'static str {
        "fs.read"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(
            json!({"op": "read", "args": args}),
        ))
    }
}

#[async_trait]
impl ToolPlugin for FsWritePlugin {
    fn name(&self) -> &'static str {
        "fs.write"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(
            json!({"op": "write", "args": args}),
        ))
    }
}

#[async_trait]
impl ToolPlugin for FsDeletePlugin {
    fn name(&self) -> &'static str {
        "fs.delete"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(
            json!({"op": "delete", "args": args}),
        ))
    }
}
