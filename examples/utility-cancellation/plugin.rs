use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, Content, McpError};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

pub(crate) struct CancellableSleepPlugin;
pub(crate) struct NonCancellableSleepPlugin;

#[async_trait]
impl ToolPlugin for CancellableSleepPlugin {
    fn name(&self) -> &'static str {
        "sleep.cancellable"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let delay_ms = args.get("delay_ms").and_then(Value::as_u64).unwrap_or(5000);
        tokio::select! {
            () = ctx.cancellation.cancelled() => {
                Ok(CallToolResult::error(vec![Content::text("request cancelled")]))
            }
            () = sleep(Duration::from_millis(delay_ms)) => {
                Ok(CallToolResult::structured(json!({"tool": self.name(), "delay_ms": delay_ms})))
            }
        }
    }
}

#[async_trait]
impl ToolPlugin for NonCancellableSleepPlugin {
    fn name(&self) -> &'static str {
        "sleep.non_cancellable"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let delay_ms = args.get("delay_ms").and_then(Value::as_u64).unwrap_or(5000);
        sleep(Duration::from_millis(delay_ms)).await;
        Ok(CallToolResult::structured(
            json!({"tool": self.name(), "delay_ms": delay_ms}),
        ))
    }
}
