//! Handler implementations for tasks/get, tasks/list, and tasks/cancel.
use rmcp::{
    model::{
        CallToolRequestParams, CancelTaskParams, CancelTaskResult, CreateTaskResult,
        GetTaskInfoParams, GetTaskPayloadResult, GetTaskResult, GetTaskResultParams,
        PaginatedRequestParams,
    },
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde_json::{json, Value};

use crate::config::TaskSupport;
use crate::errors::cancelled_error;

use super::super::orchestration::Engine;
use super::super::tasks::CancelTaskError;
use super::common::global_page_size;

const RELATED_TASK_KEY: &str = "io.modelcontextprotocol/related-task";

impl Engine {
    pub(super) async fn handle_enqueue_task_request(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CreateTaskResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.tasks_active() {
            return Err(McpError::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >());
        }

        let tool_name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let tool_cfg = self.tool_map.get(&tool_name).ok_or_else(|| {
            McpError::invalid_params(format!("tool not found: {tool_name}"), None)
        })?;

        match tool_cfg.execute.task_support() {
            TaskSupport::Forbidden => {
                return Err(McpError::method_not_found::<
                    rmcp::model::CallToolRequestMethod,
                >());
            }
            TaskSupport::Optional | TaskSupport::Required => {}
        }

        let ttl_ms = parse_task_ttl(request.task.as_ref())?;
        let peer_key = crate::engine::client_notifications::peer_key(&context);
        let created = self
            .task_store
            .create(context.peer.clone(), peer_key, ttl_ms)
            .await;
        let task_id = created.task.task_id.clone();
        let task_id_for_worker = task_id.clone();
        let cancellation = created.cancellation.clone();
        let engine = self.clone();
        let task_context = RequestContext {
            peer: context.peer.clone(),
            ct: cancellation,
            id: context.id.clone(),
            meta: context.meta.clone(),
            extensions: context.extensions.clone(),
        };
        let handle = tokio::spawn(async move {
            let result = engine
                .execute_tool_with_context(&tool_name, args, Some(task_context))
                .await;
            if let Some((task, peer)) = engine.task_store.finish(&task_id_for_worker, result).await
            {
                let _ = engine.notify_task_status(task, &peer).await;
            }
        });
        self.task_store.attach_handle(&task_id, handle).await;

        Ok(CreateTaskResult::new(created.task))
    }

    pub(super) async fn handle_list_tasks_request(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListTasksResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.tasks_list_active() {
            return Err(McpError::method_not_found::<rmcp::model::ListTasksMethod>());
        }

        let peer_key = crate::engine::client_notifications::peer_key(&context);
        let request_cursor = request.and_then(|params| params.cursor);
        let tasks = self.task_store.list_tasks(&peer_key).await;
        if let Some(page_size) = global_page_size(&self.config) {
            let (tasks, next_cursor) =
                crate::engine::pagination::paginate_items(&tasks, request_cursor, page_size)?;
            let mut result = rmcp::model::ListTasksResult::new(tasks);
            result.next_cursor = next_cursor;
            Ok(result)
        } else {
            Ok(rmcp::model::ListTasksResult::new(tasks))
        }
    }

    pub(super) async fn handle_get_task_info_request(
        &self,
        request: GetTaskInfoParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.tasks_active() {
            return Err(McpError::method_not_found::<rmcp::model::GetTaskInfoMethod>());
        }

        let peer_key = crate::engine::client_notifications::peer_key(&context);
        let task = self
            .task_store
            .get_task(&request.task_id, &peer_key)
            .await
            .ok_or_else(task_not_found_error)?;
        Ok(GetTaskResult { meta: None, task })
    }

    pub(super) async fn handle_get_task_result_request(
        &self,
        request: GetTaskResultParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskPayloadResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.tasks_active() {
            return Err(McpError::method_not_found::<rmcp::model::GetTaskResultMethod>());
        }

        let peer_key = crate::engine::client_notifications::peer_key(&context);
        let task_id = request.task_id;
        let result = self
            .task_store
            .wait_for_result(&task_id, &peer_key)
            .await
            .map_err(|_| task_not_found_error())?;

        match result {
            Ok(mut call_result) => {
                add_related_task_meta(&mut call_result.meta, &task_id);
                let value = serde_json::to_value(call_result)
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(GetTaskPayloadResult::new(value))
            }
            Err(error) => Err(with_related_task_error(error, &task_id)),
        }
    }

    pub(super) async fn handle_cancel_task_request(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CancelTaskResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.tasks_cancel_active() {
            return Err(McpError::method_not_found::<rmcp::model::CancelTaskMethod>());
        }
        let peer_key = crate::engine::client_notifications::peer_key(&context);
        let task_id = request.task_id;
        let (task, peer) = self
            .task_store
            .cancel(&task_id, &peer_key)
            .await
            .map_err(|error| match error {
                CancelTaskError::NotFound => task_not_found_error(),
                CancelTaskError::Terminal(status) => McpError::invalid_params(
                    format!("cannot cancel task: already in terminal status '{status:?}'")
                        .to_lowercase(),
                    None,
                ),
            })?;
        self.notify_task_status(task.clone(), &peer).await?;
        Ok(CancelTaskResult { meta: None, task })
    }
}

fn parse_task_ttl(task: Option<&rmcp::model::JsonObject>) -> Result<Option<u64>, McpError> {
    let Some(task) = task else {
        return Ok(None);
    };
    let Some(ttl) = task.get("ttl") else {
        return Ok(None);
    };
    ttl.as_u64().map(Some).ok_or_else(|| {
        McpError::invalid_params("task.ttl must be a non-negative integer".to_owned(), None)
    })
}

fn task_not_found_error() -> McpError {
    McpError::invalid_params("failed to retrieve task: task not found".to_owned(), None)
}

fn add_related_task_meta(meta: &mut Option<rmcp::model::Meta>, task_id: &str) {
    let related = json!({ "taskId": task_id });
    if let Some(meta) = meta.as_mut() {
        meta.insert(RELATED_TASK_KEY.to_owned(), related);
        return;
    }

    let mut meta_value = rmcp::model::Meta::new();
    meta_value.insert(RELATED_TASK_KEY.to_owned(), related);
    *meta = Some(meta_value);
}

fn with_related_task_error(mut error: McpError, task_id: &str) -> McpError {
    match error.data.take() {
        Some(Value::Object(mut object)) => {
            object.insert(RELATED_TASK_KEY.to_owned(), json!({ "taskId": task_id }));
            error.data = Some(Value::Object(object));
        }
        _ => {
            error.data = Some(json!({
                RELATED_TASK_KEY: {
                    "taskId": task_id
                }
            }));
        }
    }
    error
}
