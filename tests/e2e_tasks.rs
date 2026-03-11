#![cfg(feature = "tasks_utility")]

mod e2e_common;

use e2e_common::{
    build_engine, make_minimal_config, make_plugin_allowlist, spawn_e2e, EchoPlugin, SleepPlugin,
    SmokeTestClient,
};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, JsonObject, ServerResult,
};
use rust_mcp_core::{
    config::{
        ExecuteConfig, ExecutePluginConfig, TaskCapabilities, TaskSupport, TasksConfig, ToolConfig,
    },
    plugins::PluginRegistry,
};
use serde_json::json;

#[tokio::test]
async fn e2e_tasks_enqueue_returns_create_task_result_for_long_running_tool() {
    let mut config = make_minimal_config();
    config.tasks = Some(TasksConfig {
        enabled: Some(true),
        capabilities: TaskCapabilities {
            list: true,
            cancel: true,
        },
        status_notifications: false,
    });
    config.set_tools_items(vec![ToolConfig {
        name: "sleep".to_owned(),
        title: None,
        description: "sleep tool".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "sleep".to_owned(),
            config: None,
            task_support: TaskSupport::Optional,
        }),
        response: None,
    }]);
    config.plugins = vec![make_plugin_allowlist("sleep")];

    let registry = PluginRegistry::new()
        .register_tool(SleepPlugin)
        .expect("register sleep");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .send_request(ClientRequest::CallToolRequest(CallToolRequest::new(
            CallToolRequestParams::new("sleep")
                .with_arguments(json!({"ms": 250}).as_object().unwrap().clone())
                .with_task(JsonObject::new()),
        )))
        .await
        .expect("call_tool with task");

    match result {
        ServerResult::CreateTaskResult(create) => {
            assert!(
                !create.task.task_id.is_empty(),
                "taskId should not be empty"
            );
            let task_value = serde_json::to_value(&create.task).expect("serialize task");
            assert!(
                task_value.get("taskId").is_some(),
                "task should have taskId field"
            );
            assert!(
                task_value.get("status").is_some(),
                "task should have status field"
            );
            let status = task_value
                .get("status")
                .and_then(|v| v.as_str())
                .expect("task status should be string");
            assert!(
                !matches!(status, "completed" | "failed" | "cancelled"),
                "newly created task should be in a non-terminal status, got: {status}"
            );
        }
        other => panic!("expected CreateTaskResult for queued task, got: {other:?}"),
    }
}

#[tokio::test]
async fn e2e_tasks_sync_call_without_task_returns_call_tool_result() {
    let mut config = make_minimal_config();
    config.tasks = Some(TasksConfig {
        enabled: Some(true),
        capabilities: TaskCapabilities {
            list: true,
            cancel: true,
        },
        status_notifications: false,
    });
    config.set_tools_items(vec![ToolConfig {
        name: "echo".to_owned(),
        title: None,
        description: "echo tool".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "echo".to_owned(),
            config: None,
            task_support: TaskSupport::Optional,
        }),
        response: None,
    }]);
    config.plugins = vec![make_plugin_allowlist("echo")];

    let registry = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .expect("register echo");
    let engine = build_engine(config, registry);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .call_tool(
            CallToolRequestParams::new("echo")
                .with_arguments(json!({"hello": "world"}).as_object().unwrap().clone()),
        )
        .await
        .expect("sync call should succeed");
    assert_eq!(
        result.is_error,
        Some(false),
        "successful sync call should set is_error=false"
    );
    assert_eq!(result.structured_content, Some(json!({"hello": "world"})));
    assert!(
        result.meta.is_none(),
        "sync call without task envelope should not attach related-task metadata"
    );
}
