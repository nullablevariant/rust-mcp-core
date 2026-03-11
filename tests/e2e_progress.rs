#![cfg(feature = "progress_utility")]

mod e2e_common;

use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, make_plugin_tool, spawn_e2e,
    ProgressPlugin, SmokeTestClient,
};
use rmcp::model::CallToolRequestParams;
use rust_mcp_core::{config::ProgressConfig, plugins::PluginRegistry};
use serde_json::json;

#[tokio::test]
async fn e2e_progress_notifications() {
    let mut config = make_minimal_config();
    config.progress = Some(ProgressConfig {
        notification_interval_ms: 0,
    });
    config.set_tools_items(vec![make_plugin_tool("progress", "progress")]);
    config.plugins = vec![make_plugin_allowlist("progress")];

    let registry = PluginRegistry::new()
        .register_tool(ProgressPlugin)
        .expect("register progress");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let state = std::sync::Arc::clone(&client.state);
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let mut request = CallToolRequestParams::new("progress")
        .with_arguments(json!({}).as_object().unwrap().clone());
    request.meta = Some(rmcp::model::Meta::with_progress_token(
        rmcp::model::ProgressToken(rmcp::model::NumberOrString::Number(1)),
    ));
    let _result = client_service
        .peer()
        .call_tool(request)
        .await
        .expect("call_tool progress");

    // Brief sleep to allow notifications to be processed
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let notifications = state.lock().await.progress_notifications.clone();
    assert_eq!(
        notifications.len(),
        3,
        "should have received exactly 3 progress notifications (step 1, step 2, done)"
    );

    // Assert first notification: progress=25, total=100
    assert!(
        (notifications[0].progress - 25.0).abs() < f64::EPSILON,
        "first notification progress should be 25.0, got: {}",
        notifications[0].progress
    );
    assert!(
        notifications[0]
            .total
            .is_some_and(|t| (t - 100.0).abs() < f64::EPSILON),
        "first notification total should be 100.0, got: {:?}",
        notifications[0].total
    );
    assert_eq!(
        notifications[0].message.as_deref(),
        Some("step 1"),
        "first notification message should be 'step 1'"
    );

    // Assert last notification: progress=100, total=100
    assert!(
        (notifications[2].progress - 100.0).abs() < f64::EPSILON,
        "last notification progress should be 100.0, got: {}",
        notifications[2].progress
    );
    assert!(
        notifications[2]
            .total
            .is_some_and(|t| (t - 100.0).abs() < f64::EPSILON),
        "last notification total should be 100.0, got: {:?}",
        notifications[2].total
    );
    assert_eq!(
        notifications[2].message.as_deref(),
        Some("done"),
        "last notification message should be 'done'"
    );

    // All notifications should have the same progress token
    let token = &notifications[0].progress_token;
    for (i, n) in notifications.iter().enumerate() {
        assert_eq!(
            &n.progress_token, token,
            "notification {i} should have the same progress token"
        );
    }
}
