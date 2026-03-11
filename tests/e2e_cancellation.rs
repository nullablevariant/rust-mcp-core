mod e2e_common;

use std::time::Duration;

use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    SleepPlugin, SmokeTestClient,
};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientRequest, ServerResult,
};
use rmcp::service::PeerRequestOptions;
use rust_mcp_core::plugins::PluginRegistry;
use serde_json::json;

#[tokio::test]
async fn e2e_cancellation() {
    let mut config = make_minimal_config();
    config.set_tools_items(vec![make_plugin_tool("sleep", "sleep")]);
    config.plugins = vec![make_plugin_allowlist("sleep")];

    let registry = PluginRegistry::new()
        .register_tool(SleepPlugin)
        .expect("register sleep");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let handle = client_service
        .peer()
        .send_cancellable_request(
            ClientRequest::CallToolRequest(CallToolRequest::new(
                CallToolRequestParams::new("sleep")
                    .with_arguments(json!({"ms": 10000}).as_object().unwrap().clone()),
            )),
            PeerRequestOptions::default(),
        )
        .await
        .expect("send_cancellable_request");

    // Brief delay to let the server start processing
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send cancellation without consuming the handle, so we can assert the terminal response.
    handle
        .peer
        .notify_cancelled(CancelledNotificationParam {
            request_id: handle.id.clone(),
            reason: Some("integration-test-cancel".to_owned()),
        })
        .await
        .expect("cancel notification should send");

    // Assert a bounded-time terminal response and validate cancellation contract.
    let response = tokio::time::timeout(Duration::from_secs(2), handle.await_response())
        .await
        .expect("cancelled request should complete quickly");

    match response {
        Err(rmcp::service::ServiceError::Cancelled { reason }) => {
            assert_eq!(reason.as_deref(), Some("integration-test-cancel"));
        }
        Ok(ServerResult::CallToolResult(result)) => {
            // Some transports can race and still deliver a cancelled tool result.
            assert_eq!(
                result.is_error,
                Some(true),
                "cancelled tool should be error"
            );
        }
        other => panic!("expected cancelled error or cancelled tool result, got: {other:?}"),
    }
}
