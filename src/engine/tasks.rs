//! Task store for long-running operations with TTL, peer isolation, and status tracking.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rmcp::{
    model::{CallToolResult, Task, TaskStatus},
    service::{Peer, RoleServer},
    ErrorData as McpError,
};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::errors::cancelled_tool_result;

const DEFAULT_POLL_INTERVAL_MS: u64 = 500;

#[derive(Clone)]
pub(crate) struct TaskStore {
    inner: std::sync::Arc<Mutex<TaskStoreInner>>,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStore {
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(TaskStoreInner::default())),
        }
    }

    pub(crate) async fn create(
        &self,
        peer: Peer<RoleServer>,
        peer_key: String,
        ttl_ms: Option<u64>,
    ) -> CreatedTask {
        let mut inner = self.inner.lock().await;
        inner.cleanup_expired();
        let task_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let mut task = Task::new(
            task_id.clone(),
            TaskStatus::Working,
            now.to_rfc3339(),
            now.to_rfc3339(),
        )
        .with_status_message("Task accepted")
        .with_poll_interval(DEFAULT_POLL_INTERVAL_MS);
        if let Some(ttl_ms) = ttl_ms {
            task = task.with_ttl(ttl_ms);
        }
        let cancellation = CancellationToken::new();
        let notify = std::sync::Arc::new(Notify::new());
        inner.entries.insert(
            task_id,
            TaskEntry {
                task: task.clone(),
                created_at: now,
                terminal_result: None,
                cancellation: cancellation.clone(),
                notify,
                handle: None,
                peer,
                peer_key,
            },
        );

        CreatedTask { task, cancellation }
    }

    pub(crate) async fn attach_handle(&self, task_id: &str, handle: JoinHandle<()>) {
        let mut inner = self.inner.lock().await;
        if let Some(entry) = inner.entries.get_mut(task_id) {
            entry.handle = Some(handle);
        } else {
            handle.abort();
        }
    }

    pub(crate) async fn finish(
        &self,
        task_id: &str,
        result: Result<CallToolResult, McpError>,
    ) -> Option<(Task, Peer<RoleServer>)> {
        let mut inner = self.inner.lock().await;
        inner.cleanup_expired();
        let entry = inner.entries.get_mut(task_id)?;

        // Cancellation is terminal and must remain the status even if work completes later.
        if is_terminal(&entry.task.status) {
            return None;
        }

        if let Some(handle) = entry.handle.take() {
            handle.abort();
        }

        let (status, status_message) = match &result {
            Ok(call_result) if call_result.is_error == Some(true) => (
                TaskStatus::Failed,
                Some("Task completed with tool execution error".to_owned()),
            ),
            Ok(_) => (TaskStatus::Completed, None),
            Err(error) => (TaskStatus::Failed, Some(error.message.to_string())),
        };

        let now = Utc::now();
        entry.task.status = status;
        entry.task.status_message = status_message;
        entry.task.last_updated_at = now.to_rfc3339();
        entry.terminal_result = Some(result);
        entry.notify.notify_waiters();
        Some((entry.task.clone(), entry.peer.clone()))
    }

    pub(crate) async fn get_task(&self, task_id: &str, peer_key: &str) -> Option<Task> {
        let mut inner = self.inner.lock().await;
        inner.cleanup_expired();
        inner
            .entries
            .get(task_id)
            .filter(|entry| entry.peer_key == peer_key)
            .map(|entry| entry.task.clone())
    }

    pub(crate) async fn list_tasks(&self, peer_key: &str) -> Vec<Task> {
        let mut inner = self.inner.lock().await;
        inner.cleanup_expired();
        inner
            .entries
            .values()
            .filter(|entry| entry.peer_key == peer_key)
            .map(|entry| entry.task.clone())
            .collect()
    }

    pub(crate) async fn wait_for_result(
        &self,
        task_id: &str,
        peer_key: &str,
    ) -> Result<Result<CallToolResult, McpError>, TaskLookupError> {
        loop {
            let notify = {
                let mut inner = self.inner.lock().await;
                inner.cleanup_expired();
                let Some(entry) = inner.entries.get(task_id) else {
                    return Err(TaskLookupError::NotFound);
                };
                if entry.peer_key != peer_key {
                    return Err(TaskLookupError::NotFound);
                }
                if let Some(result) = entry.terminal_result.clone() {
                    return Ok(result);
                }
                std::sync::Arc::clone(&entry.notify)
            };

            notify.notified().await;
        }
    }

    pub(crate) async fn cancel(
        &self,
        task_id: &str,
        peer_key: &str,
    ) -> Result<(Task, Peer<RoleServer>), CancelTaskError> {
        let mut inner = self.inner.lock().await;
        inner.cleanup_expired();
        let Some(entry) = inner.entries.get_mut(task_id) else {
            return Err(CancelTaskError::NotFound);
        };

        if entry.peer_key != peer_key {
            return Err(CancelTaskError::NotFound);
        }

        if is_terminal(&entry.task.status) {
            return Err(CancelTaskError::Terminal(entry.task.status.clone()));
        }

        entry.cancellation.cancel();
        if let Some(handle) = entry.handle.take() {
            handle.abort();
        }

        let now = Utc::now();
        entry.task.status = TaskStatus::Cancelled;
        entry.task.status_message = Some("The task was cancelled by request.".to_owned());
        entry.task.last_updated_at = now.to_rfc3339();
        entry.terminal_result = Some(Ok(cancelled_tool_result()));
        entry.notify.notify_waiters();
        Ok((entry.task.clone(), entry.peer.clone()))
    }
}

#[derive(Default)]
struct TaskStoreInner {
    entries: HashMap<String, TaskEntry>,
}

impl TaskStoreInner {
    fn cleanup_expired(&mut self) {
        let now = Utc::now();
        self.entries.retain(|_, entry| !entry.is_expired(now));
    }
}

struct TaskEntry {
    task: Task,
    created_at: DateTime<Utc>,
    terminal_result: Option<Result<CallToolResult, McpError>>,
    cancellation: CancellationToken,
    notify: std::sync::Arc<Notify>,
    handle: Option<JoinHandle<()>>,
    peer: Peer<RoleServer>,
    peer_key: String,
}

impl TaskEntry {
    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        let Some(ttl_ms) = self.task.ttl else {
            return false;
        };
        let ttl_ms = i64::try_from(ttl_ms).unwrap_or(i64::MAX);
        (now - self.created_at).num_milliseconds() >= ttl_ms
    }
}

impl Drop for TaskEntry {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

#[derive(Clone)]
pub(crate) struct CreatedTask {
    pub(crate) task: Task,
    pub(crate) cancellation: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskLookupError {
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelTaskError {
    NotFound,
    Terminal(TaskStatus),
}

const fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        &TaskStatus::Completed | &TaskStatus::Failed | &TaskStatus::Cancelled
    )
}
