#![cfg(feature = "client_logging")]

mod e2e_common;

use async_trait::async_trait;
use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    SmokeTestClient,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, LoggingLevel, SetLevelRequestParams};
use rust_mcp_core::plugins::{
    LogChannel, LogEventParams, PluginCallParams, PluginRegistry, ToolPlugin,
};
use serde_json::{json, Value};

struct EmitDebugLogPlugin;

#[async_trait]
impl ToolPlugin for EmitDebugLogPlugin {
    fn name(&self) -> &'static str {
        "emit_log"
    }

    async fn call(
        &self,
        _args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        params
            .ctx
            .log_event(LogEventParams {
                level: LoggingLevel::Debug,
                message: "debug event".to_owned(),
                data: Some(json!({"source": "test"})),
                channels: &[LogChannel::Client],
            })
            .await?;
        Ok(CallToolResult::structured(json!({"ok": true})))
    }
}

#[tokio::test]
async fn e2e_logging_set_level() {
    let mut config = make_minimal_config();
    config.client_logging = Some(rust_mcp_core::config::ClientLoggingConfig::default());
    config.set_tools_items(vec![make_plugin_tool("emit_log", "emit_log")]);
    config.plugins = vec![make_plugin_allowlist("emit_log")];

    let registry = PluginRegistry::new()
        .register_tool(EmitDebugLogPlugin)
        .expect("register emit_log plugin");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let state = std::sync::Arc::clone(&client.state);
    let (client_service, _server) = spawn_e2e(engine, client).await;

    // 1) Set level to Warning and emit a debug log: should be filtered out.
    client_service
        .peer()
        .set_level(SetLevelRequestParams::new(LoggingLevel::Warning))
        .await
        .expect("set_level should succeed");
    client_service
        .peer()
        .call_tool(CallToolRequestParams::new("emit_log"))
        .await
        .expect("emit_log call at warning level");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        state.lock().await.logging_messages.is_empty(),
        "debug log should be filtered at warning level"
    );

    // 2) Set level to Debug and emit again: should now be delivered.
    client_service
        .peer()
        .set_level(SetLevelRequestParams::new(LoggingLevel::Debug))
        .await
        .expect("set_level to debug should succeed after setting to warning");
    client_service
        .peer()
        .call_tool(CallToolRequestParams::new("emit_log"))
        .await
        .expect("emit_log call at debug level");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let messages = state.lock().await.logging_messages.clone();
    assert_eq!(
        messages.len(),
        1,
        "debug log should be delivered at debug level"
    );
    assert_eq!(messages[0].level, LoggingLevel::Debug);
    assert_eq!(
        messages[0].data,
        json!({"message": "debug event", "source": "test"})
    );
}
