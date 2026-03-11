#![cfg(feature = "tasks_utility")]

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, CancelTaskParams, Content, ErrorCode, Extensions,
        GetTaskInfoParams, GetTaskPayloadResult, GetTaskResultParams, JsonObject, ListTasksResult,
        Meta, NumberOrString, TaskStatus,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    config::{
        AuthConfig, ClientFeaturesConfig, ExecuteConfig, ExecutePluginConfig, McpConfig,
        PluginConfig, ServerSection, TaskCapabilities, TaskSupport, TasksConfig, ToolConfig,
        ToolsConfig, TransportConfig, TransportMode,
    },
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, PluginType, ToolPlugin},
};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

struct TaskPlugin;

#[async_trait]
impl ToolPlugin for TaskPlugin {
    fn name(&self) -> &'static str {
        "tool.tasks"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("ok");
        match mode {
            "error" => Err(McpError::internal_error(
                "plugin execution failed".to_owned(),
                None,
            )),
            "panic" => panic!("task plugin panic"),
            "error_with_data" => Err(McpError::invalid_params(
                "plugin execution failed with data".to_owned(),
                Some(json!({"reason": "bad input"})),
            )),
            "tool_error" => Ok(CallToolResult::error(vec![Content::text(
                "tool execution error",
            )])),
            "meta" => {
                let mut result = CallToolResult::default();
                result.content = vec![Content::text("ok with meta")];
                result.is_error = Some(false);
                result.meta = Some(rmcp::model::Meta::new());
                Ok(result)
            }
            "sleep" => {
                let sleep_ms = args.get("sleep_ms").and_then(Value::as_u64).unwrap_or(250);
                tokio::select! {
                    () = ctx.cancellation.cancelled() => Ok(CallToolResult::error(vec![Content::text("cancelled")])),
                    () = tokio::time::sleep(Duration::from_millis(sleep_ms)) => {
                        Ok(CallToolResult::structured(json!({"ok": true})))
                    }
                }
            }
            _ => Ok(CallToolResult::structured(json!({"ok": true}))),
        }
    }
}

fn base_config(task_support: TaskSupport, tasks: TasksConfig) -> McpConfig {
    McpConfig {
        version: 1,
        server: ServerSection {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            endpoint_path: "/mcp".to_owned(),
            transport: TransportConfig {
                mode: TransportMode::Stdio,
                ..TransportConfig::default()
            },
            auth: Some(AuthConfig::default()),
            errors: rust_mcp_core::config::ErrorExposureConfig::default(),
            logging: rust_mcp_core::config::ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: rust_mcp_core::config::ClientCompatConfig::default(),
            info: None,
        },
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: Some(tasks),
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::new(),
        tools: Some(ToolsConfig {
            enabled: None,
            notify_list_changed: false,
            items: vec![ToolConfig {
                name: "tool.tasks".to_owned(),
                title: None,
                description: "task test tool".to_owned(),
                cancellable: true,
                input_schema: json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Plugin(ExecutePluginConfig {
                    plugin: "tool.tasks".to_owned(),
                    config: None,
                    task_support,
                }),
                response: None,
            }],
        }),
        plugins: vec![PluginConfig {
            name: "tool.tasks".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }],
        outbound_http: None,
    }
}

fn build_engine(task_support: TaskSupport, tasks: TasksConfig) -> Engine {
    let config = base_config(task_support, tasks);
    let registry = PluginRegistry::new()
        .register_tool(TaskPlugin)
        .expect("register task plugin");
    Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine")
}

fn request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    id: i64,
) -> RequestContext<rmcp::service::RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(id),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

fn task_call_params(mode: &str, task: Option<JsonObject>) -> CallToolRequestParams {
    let mut args = serde_json::Map::new();
    args.insert("mode".to_owned(), Value::String(mode.to_owned()));
    let params = CallToolRequestParams::new("tool.tasks").with_arguments(args);
    if let Some(task) = task {
        params.with_task(task)
    } else {
        params
    }
}

fn extract_call_tool_result(result: GetTaskPayloadResult) -> CallToolResult {
    serde_json::from_value(result.0).expect("task result value should be CallToolResult")
}

async fn read_frame(stream: &mut tokio::io::DuplexStream) -> Option<String> {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
        _ => None,
    }
}

fn parse_notification_from_frame(frame: &str) -> Value {
    for line in frame.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                return value;
            }
        }
    }
    serde_json::from_str(frame).expect("frame should contain valid JSON")
}

fn assert_task_error_result(result: &CallToolResult, expected_text: &str, expected_task_id: &str) {
    assert_eq!(result.is_error, Some(true));
    let payload = serde_json::to_value(result).expect("serialize result");
    assert_eq!(payload["content"][0]["text"], expected_text);
    assert_eq!(
        payload["_meta"]["io.modelcontextprotocol/related-task"]["taskId"],
        expected_task_id
    );
}

async fn assert_tasks_pagination_page_one_and_two(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    context: RequestContext<rmcp::service::RoleServer>,
    created_ids: &[String],
) {
    let first = ServerHandler::list_tasks(service.service(), None, context.clone())
        .await
        .expect("list page one");
    assert_eq!(first.tasks.len(), 1);
    assert!(first.next_cursor.is_some());
    assert!(first.tasks[0].ttl.is_none());

    let second = ServerHandler::list_tasks(
        service.service(),
        Some(rmcp::model::PaginatedRequestParams::default().with_cursor(first.next_cursor)),
        context,
    )
    .await
    .expect("list page two");
    assert_eq!(second.tasks.len(), 1);
    assert!(second.next_cursor.is_none());
    assert_ne!(first.tasks[0].task_id, second.tasks[0].task_id);
    let listed_ids = [
        first.tasks[0].task_id.clone(),
        second.tasks[0].task_id.clone(),
    ];
    for task_id in created_ids {
        assert!(
            listed_ids.contains(task_id),
            "listed tasks should include task id {task_id}"
        );
    }
    assert_eq!(listed_ids.len(), created_ids.len());
}

async fn assert_tasks_list_and_cancel_disabled() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            capabilities: TaskCapabilities {
                list: false,
                cancel: false,
            },
            status_notifications: false,
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut disabled_service = rmcp::service::serve_directly(engine, server_io, None);
    let disabled_context = request_context(&disabled_service, 7);

    let list_error =
        ServerHandler::list_tasks(disabled_service.service(), None, disabled_context.clone())
            .await
            .expect_err("list disabled");
    assert_eq!(list_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(list_error.message, "tasks/list");
    assert_eq!(list_error.data, None);

    let cancel_error = ServerHandler::cancel_task(
        disabled_service.service(),
        CancelTaskParams {
            meta: None,
            task_id: "task-1".to_owned(),
        },
        disabled_context,
    )
    .await
    .expect_err("cancel disabled");
    assert_eq!(cancel_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(cancel_error.message, "tasks/cancel");
    assert_eq!(cancel_error.data, None);
    let _ = disabled_service.close().await;
}

#[tokio::test]
async fn tasks_capability_and_tool_metadata_reflect_task_support() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );

    let info = engine.get_info();
    assert!(info.capabilities.tasks.is_some());
    let tasks_capability = info.capabilities.tasks.expect("tasks capability");
    assert_eq!(tasks_capability.list, Some(JsonObject::new()));
    assert_eq!(tasks_capability.cancel, Some(JsonObject::new()));
    let requests = tasks_capability.requests.expect("requests capability");
    assert!(requests.sampling.is_none());
    assert!(requests.elicitation.is_none());
    assert_eq!(
        requests.tools.expect("tools capability").call,
        Some(JsonObject::new())
    );

    let tool = engine.list_tools().pop().expect("tool");
    let execution = tool.execution.expect("execution field");
    assert_eq!(execution.task_support, Some(TaskSupport::Optional));
}

#[tokio::test]
async fn get_tool_returns_execution_for_required_task_support() {
    let engine = build_engine(
        TaskSupport::Required,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    // Verify get_tool returns the tool with Required execution so SDK dispatch
    // can reject non-task calls (SDK validates via get_tool before routing).
    let tool = ServerHandler::get_tool(&engine, "tool.tasks").expect("tool must exist");
    assert_eq!(tool.task_support(), TaskSupport::Required);
}

#[tokio::test]
async fn call_tool_allows_optional_task_support_without_task_metadata() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 11);

    let result = ServerHandler::call_tool(service.service(), task_call_params("ok", None), context)
        .await
        .expect("optional support should allow non-task calls");
    assert_eq!(result.is_error, Some(false));
    let _ = service.close().await;
}

#[tokio::test]
async fn enqueue_task_runs_and_result_blocks_until_terminal() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 2);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params(
            "sleep",
            Some(json!({"ttl": 2000}).as_object().expect("object").clone()),
        ),
        context.clone(),
    )
    .await
    .expect("enqueue task");
    assert_eq!(created.task.status, TaskStatus::Working);
    assert_eq!(created.task.ttl, Some(2000));

    let result = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context.clone(),
    )
    .await
    .expect("task result");
    let call_result = extract_call_tool_result(result);
    assert_eq!(call_result.is_error, Some(false));
    assert_eq!(
        call_result
            .meta
            .as_ref()
            .and_then(|meta| meta.get("io.modelcontextprotocol/related-task"))
            .and_then(|value| value.get("taskId"))
            .and_then(Value::as_str),
        Some(created.task.task_id.as_str())
    );

    let info = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: created.task.task_id,
        },
        context,
    )
    .await
    .expect("task info");
    let status = info.task.status;
    assert!(matches!(status, TaskStatus::Completed));
    let _ = service.close().await;
}

#[tokio::test]
async fn enqueue_task_rejects_invalid_ttl() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 3);

    let error = ServerHandler::enqueue_task(
        service.service(),
        task_call_params(
            "ok",
            Some(json!({"ttl": "bad"}).as_object().expect("object").clone()),
        ),
        context,
    )
    .await
    .expect_err("invalid ttl should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "task.ttl must be a non-negative integer");
    assert_eq!(error.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn enqueue_task_accepts_large_ttl_without_immediate_expiration() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 17);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params(
            "ok",
            Some(
                json!({"ttl": u64::MAX})
                    .as_object()
                    .expect("object")
                    .clone(),
            ),
        ),
        context.clone(),
    )
    .await
    .expect("enqueue");
    assert_eq!(created.task.ttl, Some(u64::MAX));

    let listed: ListTasksResult = ServerHandler::list_tasks(service.service(), None, context)
        .await
        .expect("list");
    assert!(listed
        .tasks
        .iter()
        .any(|task| task.task_id == created.task.task_id));
    let _ = service.close().await;
}

#[tokio::test]
async fn enqueue_task_rejects_when_tasks_disabled_or_forbidden() {
    let tasks_disabled = build_engine(TaskSupport::Forbidden, TasksConfig::default());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut disabled_service = rmcp::service::serve_directly(tasks_disabled, server_io, None);
    let disabled_context = request_context(&disabled_service, 12);
    let disabled_error = ServerHandler::enqueue_task(
        disabled_service.service(),
        task_call_params("ok", Some(JsonObject::new())),
        disabled_context,
    )
    .await
    .expect_err("tasks disabled should reject enqueue");
    assert_eq!(disabled_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(disabled_error.message, "tools/call");
    assert_eq!(disabled_error.data, None);
    let _ = disabled_service.close().await;

    let forbidden = build_engine(
        TaskSupport::Forbidden,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut forbidden_service = rmcp::service::serve_directly(forbidden, server_io, None);
    let forbidden_context = request_context(&forbidden_service, 13);
    let forbidden_error = ServerHandler::enqueue_task(
        forbidden_service.service(),
        task_call_params("ok", Some(JsonObject::new())),
        forbidden_context,
    )
    .await
    .expect_err("forbidden task support should reject enqueue");
    assert_eq!(forbidden_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(forbidden_error.message, "tools/call");
    assert_eq!(forbidden_error.data, None);
    let _ = forbidden_service.close().await;
}

#[tokio::test]
async fn tasks_cancel_returns_cancelled_result_and_terminal_rejects_repeated_cancel() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 4);
    let mut args = serde_json::Map::new();
    args.insert("mode".to_owned(), Value::String("sleep".to_owned()));
    args.insert("sleep_ms".to_owned(), Value::Number(1000.into()));

    let created = ServerHandler::enqueue_task(
        service.service(),
        CallToolRequestParams::new("tool.tasks")
            .with_arguments(args)
            .with_task(json!({"ttl": 2000}).as_object().expect("object").clone()),
        context.clone(),
    )
    .await
    .expect("enqueue");

    let cancel_result = ServerHandler::cancel_task(
        service.service(),
        CancelTaskParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context.clone(),
    )
    .await
    .expect("cancel");
    assert_eq!(cancel_result.task.status, TaskStatus::Cancelled);

    let info = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context.clone(),
    )
    .await
    .expect("task info");
    assert_eq!(info.task.status, TaskStatus::Cancelled);

    let result = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context.clone(),
    )
    .await
    .expect("task result");
    let call_result = extract_call_tool_result(result);
    assert_eq!(call_result.is_error, Some(true));

    let second_cancel = ServerHandler::cancel_task(
        service.service(),
        CancelTaskParams {
            meta: None,
            task_id: created.task.task_id,
        },
        context,
    )
    .await
    .expect_err("terminal task cannot be cancelled");
    assert_eq!(second_cancel.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        second_cancel.message,
        "cannot cancel task: already in terminal status 'cancelled'"
    );
    assert_eq!(second_cancel.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn tasks_result_wraps_plugin_error_as_tool_error_with_related_task_metadata() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 5);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("error", Some(JsonObject::new())),
        context.clone(),
    )
    .await
    .expect("enqueue");

    let result = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context,
    )
    .await
    .expect("tool error result should be returned");
    let call_result = extract_call_tool_result(result);
    assert_task_error_result(&call_result, "internal server error", &created.task.task_id);
    let _ = service.close().await;
}

#[tokio::test]
async fn tasks_result_wraps_plugin_error_message_and_related_task_metadata() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 14);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("error_with_data", Some(JsonObject::new())),
        context.clone(),
    )
    .await
    .expect("enqueue");

    let result = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context,
    )
    .await
    .expect("tool error result should be returned");
    let call_result = extract_call_tool_result(result);
    assert_task_error_result(
        &call_result,
        "plugin execution failed with data",
        &created.task.task_id,
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn tool_execution_error_marks_task_failed_and_result_error() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 15);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("tool_error", Some(JsonObject::new())),
        context.clone(),
    )
    .await
    .expect("enqueue");

    let result = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: created.task.task_id.clone(),
        },
        context.clone(),
    )
    .await
    .expect("task result");
    let call_result = extract_call_tool_result(result);
    assert_task_error_result(&call_result, "tool execution error", &created.task.task_id);

    let info = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: created.task.task_id,
        },
        context,
    )
    .await
    .expect("task info");
    assert_eq!(info.task.status, TaskStatus::Failed);
    let _ = service.close().await;
}

#[tokio::test]
async fn panicking_tool_completes_task_with_internal_error_instead_of_hanging() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 115);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("panic", Some(JsonObject::new())),
        context.clone(),
    )
    .await
    .expect("enqueue");

    let result = tokio::time::timeout(
        Duration::from_millis(500),
        ServerHandler::get_task_result(
            service.service(),
            GetTaskResultParams {
                meta: None,
                task_id: created.task.task_id.clone(),
            },
            context.clone(),
        ),
    )
    .await
    .expect("task result should not hang")
    .expect("panic should map to tool error result");
    let call_result = extract_call_tool_result(result);
    assert_task_error_result(&call_result, "internal server error", &created.task.task_id);

    let info = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: created.task.task_id,
        },
        context,
    )
    .await
    .expect("task info");
    assert_eq!(info.task.status, TaskStatus::Failed);
    let _ = service.close().await;
}

#[tokio::test]
async fn list_tasks_respects_pagination_and_toggles() {
    let mut config = base_config(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    config.pagination = Some(rust_mcp_core::config::PaginationConfig { page_size: 1 });
    let registry = PluginRegistry::new()
        .register_tool(TaskPlugin)
        .expect("register task plugin");
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: registry,
        list_refresh_handle: None,
    })
    .expect("engine");
    let (_client_io, server_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 6);

    let mut created_ids = Vec::new();
    for _ in 0..2 {
        let created = ServerHandler::enqueue_task(
            service.service(),
            task_call_params("ok", Some(JsonObject::new())),
            context.clone(),
        )
        .await
        .expect("enqueue");
        created_ids.push(created.task.task_id);
    }

    assert_tasks_pagination_page_one_and_two(&service, context, &created_ids).await;

    let _ = service.close().await;
    assert_tasks_list_and_cancel_disabled().await;
}

#[tokio::test]
async fn tasks_status_notifications_emit_when_enabled() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            status_notifications: true,
            ..TasksConfig::default()
        },
    );
    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 8);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("ok", Some(JsonObject::new())),
        context,
    )
    .await
    .expect("enqueue");
    assert_eq!(created.task.status, TaskStatus::Working);

    let frame = read_frame(&mut client_io)
        .await
        .expect("status notification frame");
    let notification = parse_notification_from_frame(&frame);
    assert_eq!(notification["method"], "notifications/tasks/status");
    assert_eq!(notification["params"]["taskId"], created.task.task_id);
    assert_eq!(notification["params"]["status"], json!("completed"));
    let _ = service.close().await;
}

#[tokio::test]
async fn tasks_status_notifications_do_not_emit_when_disabled() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            status_notifications: false,
            ..TasksConfig::default()
        },
    );
    let (server_io, mut client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 16);

    let created = ServerHandler::enqueue_task(
        service.service(),
        task_call_params("ok", Some(JsonObject::new())),
        context,
    )
    .await
    .expect("enqueue");
    assert_eq!(created.task.status, TaskStatus::Working);
    assert!(
        read_frame(&mut client_io).await.is_none(),
        "status notifications are disabled, no frame should be emitted"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn tasks_cancel_returns_error_when_status_notification_send_fails() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            status_notifications: true,
            ..TasksConfig::default()
        },
    );
    let (server_io, client_io) = tokio::io::duplex(4096);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 17);

    let mut args = serde_json::Map::new();
    args.insert("mode".to_owned(), Value::String("sleep".to_owned()));
    args.insert("sleep_ms".to_owned(), Value::Number(1000.into()));
    let created = ServerHandler::enqueue_task(
        service.service(),
        CallToolRequestParams::new("tool.tasks")
            .with_arguments(args)
            .with_task(json!({"ttl": 2000}).as_object().expect("object").clone()),
        context.clone(),
    )
    .await
    .expect("enqueue");

    drop(client_io);

    let cancel_error = ServerHandler::cancel_task(
        service.service(),
        CancelTaskParams {
            meta: None,
            task_id: created.task.task_id,
        },
        context,
    )
    .await
    .expect_err("cancel should fail when notification cannot be sent");
    assert_eq!(cancel_error.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        cancel_error
            .message
            .starts_with("failed to send task status notification:"),
        "unexpected error message: {}",
        cancel_error.message
    );
    assert_eq!(cancel_error.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn tasks_disabled_with_non_forbidden_task_support_fails_fast() {
    let result = Engine::from_config(EngineConfig {
        config: base_config(
            TaskSupport::Optional,
            TasksConfig {
                enabled: Some(false),
                ..TasksConfig::default()
            },
        ),
        plugins: PluginRegistry::new()
            .register_tool(TaskPlugin)
            .expect("register task plugin"),
        list_refresh_handle: None,
    });
    let Err(error) = result else {
        panic!("inactive tasks config with task_support should fail");
    };
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "tasks config is inactive but tools.items[].execute.task_support=optional|required is configured"
    );
    assert_eq!(error.data, None);
}

#[tokio::test]
async fn tasks_capability_requests_omitted_when_no_tool_supports_tasks() {
    let engine = build_engine(
        TaskSupport::Forbidden,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let info = engine.get_info();
    let tasks = info.capabilities.tasks.expect("tasks capability");
    assert!(tasks.requests.is_none());
    assert_eq!(tasks.list, Some(JsonObject::new()));
    assert_eq!(tasks.cancel, Some(JsonObject::new()));
}

#[tokio::test]
async fn tasks_not_found_errors_use_invalid_params() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 9);

    let get_error = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: "missing".to_owned(),
        },
        context.clone(),
    )
    .await
    .expect_err("missing get");
    assert_eq!(get_error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(get_error.message, "failed to retrieve task: task not found");
    assert_eq!(get_error.data, None);

    let result_error = ServerHandler::get_task_result(
        service.service(),
        GetTaskResultParams {
            meta: None,
            task_id: "missing".to_owned(),
        },
        context.clone(),
    )
    .await
    .expect_err("missing result");
    assert_eq!(result_error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        result_error.message,
        "failed to retrieve task: task not found"
    );
    assert_eq!(result_error.data, None);

    let cancel_error = ServerHandler::cancel_task(
        service.service(),
        CancelTaskParams {
            meta: None,
            task_id: "missing".to_owned(),
        },
        context,
    )
    .await
    .expect_err("missing cancel");
    assert_eq!(cancel_error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        cancel_error.message,
        "failed to retrieve task: task not found"
    );
    assert_eq!(cancel_error.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn list_tasks_without_pagination_returns_all_tasks() {
    let engine = build_engine(
        TaskSupport::Optional,
        TasksConfig {
            enabled: Some(true),
            ..TasksConfig::default()
        },
    );
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let context = request_context(&service, 10);

    let mut created_ids = Vec::new();
    for _ in 0..2 {
        let created = ServerHandler::enqueue_task(
            service.service(),
            task_call_params("ok", Some(JsonObject::new())),
            context.clone(),
        )
        .await
        .expect("enqueue");
        created_ids.push(created.task.task_id);
    }

    let listed: ListTasksResult = ServerHandler::list_tasks(service.service(), None, context)
        .await
        .expect("list");
    assert_eq!(listed.tasks.len(), 2);
    assert!(listed.next_cursor.is_none());
    let listed_ids = listed
        .tasks
        .iter()
        .map(|task| task.task_id.clone())
        .collect::<Vec<_>>();
    for task_id in &created_ids {
        assert!(
            listed_ids.contains(task_id),
            "listed tasks should include task id {task_id}"
        );
    }
    assert_eq!(listed_ids.len(), created_ids.len());
    let _ = service.close().await;
}
