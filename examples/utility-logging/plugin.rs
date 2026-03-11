use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, LoggingLevel, McpError};
use rust_mcp_core::{LogChannel, LogEventParams, PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct LoggingDemoPlugin;

#[async_trait]
impl ToolPlugin for LoggingDemoPlugin {
    fn name(&self) -> &'static str {
        "logging.demo"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("log demo event");
        let _result = ctx
            .log_event(LogEventParams {
                level: LoggingLevel::Info,
                message: message.to_owned(),
                data: Some(json!({"source": self.name()})),
                channels: &[LogChannel::Server, LogChannel::Client],
            })
            .await?;

        Ok(CallToolResult::structured(json!({
            "ok": true,
            "message": message
        })))
    }
}
