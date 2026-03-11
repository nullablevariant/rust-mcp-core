//! Engine construction, tool dispatch orchestration, and ServerHandler implementation.

use std::{collections::HashMap, fmt, sync::Arc};

use rmcp::{
    model::Tool,
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
#[cfg(any(feature = "prompts", feature = "resources"))]
use tokio::sync::RwLock;

#[cfg(feature = "completion")]
use crate::config::CompletionProviderConfig;
#[cfg(feature = "tasks_utility")]
use rmcp::model::{CustomNotification, ServerNotification};

use crate::{
    config::{McpConfig, ToolConfig},
    engine::SchemaValidatorCache,
    http::client::SharedHttpClient,
    plugins::{ClientLoggingState, ListRefreshHandle, PluginRegistry, ProgressState},
};

use super::client_notifications::ClientNotificationHub;
#[cfg(feature = "tasks_utility")]
use super::tasks::TaskStore;

#[cfg(feature = "completion")]
mod completion_dispatch;
mod construction;
mod helpers;
#[cfg(feature = "prompts")]
mod prompt_dispatch;
#[cfg(feature = "resources")]
mod resource_dispatch;
mod response_limits;
#[cfg(feature = "resources")]
pub(crate) use resource_dispatch::ResourcePluginCallParams;
mod tool_dispatch;

// Configuration bundle for constructing an [`Engine`].
//
// Combines the parsed [`McpConfig`], a [`PluginRegistry`], and an optional
// list refresh handle for triggering `notifications/list_changed`.
#[derive(Clone)]
#[doc(hidden)]
pub struct EngineConfig {
    // The parsed and validated MCP configuration.
    pub config: McpConfig,
    // Registry of all plugins (tool, auth, prompt, resource, completion, HTTP router).
    pub plugins: PluginRegistry,
    // Optional handle for triggering list-changed notifications from plugins.
    pub list_refresh_handle: Option<Arc<dyn ListRefreshHandle>>,
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineConfig")
            .field("config", &self.config)
            .field("plugins", &self.plugins)
            .finish_non_exhaustive()
    }
}

// The MCP server engine that implements `rmcp::ServerHandler`.
//
// Owns tool definitions, plugin registries, and config state. Dispatches
// `tools/call` to either HTTP execution or plugin execution, and handles
// all other MCP methods (prompts, resources, completion, logging, progress, tasks).
//
// Typically constructed via [`Engine::from_config`] and passed to the runtime.
//
// # Examples
//
// ```rust
// use rust_mcp_core::{Engine, EngineConfig, McpConfig, PluginRegistry};
// use serde_json::json;
//
// let config: McpConfig = serde_json::from_value(json!({
//     "version": 1,
//     "mcp": {
//         "transport": { "mode": "stdio" },
//         "auth": { "mode": "none" }
//     },
//     "tools": [{
//         "name": "ping",
//         "description": "Ping tool",
//         "input_schema": { "type": "object" },
//         "execute": {
//             "type": "builtin"
//         }
//     }]
// }))
// .expect("valid config");
//
// let engine = Engine::from_config(EngineConfig {
//     config,
//     plugins: PluginRegistry::new(),
//     list_refresh_handle: None,
// })
// .expect("build engine");
//
// # assert_eq!(engine.list_tools().len(), 1);
// ```
#[derive(Clone)]
#[doc(hidden)]
pub struct Engine {
    pub(crate) config: Arc<McpConfig>,
    pub(crate) tools: Vec<Tool>,
    pub(crate) tool_map: HashMap<String, ToolConfig>,
    http_client: SharedHttpClient,
    #[cfg(feature = "http_tools")]
    outbound_token_manager: crate::auth::outbound::token_manager::OutboundTokenManager,
    pub(crate) plugins: PluginRegistry,
    upstreams: Arc<HashMap<String, crate::config::UpstreamConfig>>,
    #[cfg(feature = "completion")]
    #[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
    completion_sources: HashMap<String, CompletionProviderConfig>,
    pub(crate) client_logging: ClientLoggingState,
    progress_state: ProgressState,
    #[cfg(feature = "tasks_utility")]
    pub(crate) task_store: TaskStore,
    pub(crate) schema_validator_cache: SchemaValidatorCache,
    #[cfg(feature = "prompts")]
    prompt_catalog: Arc<RwLock<Option<Arc<crate::engine::prompts::PromptCatalog>>>>,
    #[cfg(feature = "resources")]
    resource_catalog: Arc<RwLock<Option<Arc<crate::engine::resources::ResourceCatalog>>>>,
    list_refresh_handle: Option<Arc<dyn ListRefreshHandle>>,
    notification_hub: ClientNotificationHub,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("tools", &self.tools)
            .field("plugins", &self.plugins)
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn new(config: McpConfig) -> Result<Self, McpError> {
        Self::from_config(EngineConfig {
            config,
            plugins: PluginRegistry::default(),
            list_refresh_handle: None,
        })
    }

    #[must_use]
    pub fn with_notification_hub(mut self, notification_hub: ClientNotificationHub) -> Self {
        self.notification_hub = notification_hub;
        self
    }

    pub fn list_tools(&self) -> Vec<Tool> {
        self.tools.clone()
    }

    pub async fn observe_request_peer(&self, context: &RequestContext<RoleServer>) {
        self.notification_hub.observe_peer(context).await;
    }

    pub async fn notify_tools_list_changed(&self) -> usize {
        self.notification_hub.notify_tools_list_changed().await
    }

    pub async fn notify_prompts_list_changed(&self) -> usize {
        self.notification_hub.notify_prompts_list_changed().await
    }

    pub async fn notify_resources_list_changed(&self) -> usize {
        self.notification_hub.notify_resources_list_changed().await
    }

    fn should_redact_internal_error_for_client(&self, error: &McpError) -> bool {
        error.code == crate::mcp::ErrorCode::INTERNAL_ERROR
            && !self.config.expose_internal_error_details()
    }

    fn log_internal_error_for_server(&self, error: &McpError, client_error_redacted: bool) {
        if error.code != crate::mcp::ErrorCode::INTERNAL_ERROR {
            return;
        }

        let safe_message = crate::log_safety::truncate_string_for_log(
            &error.message,
            self.config.log_payload_max_bytes(),
        );
        tracing::error!(
            error_code = ?error.code,
            error_message = %safe_message.value,
            error_message_bytes = safe_message.original_bytes,
            error_message_truncated = safe_message.truncated,
            client_error_redacted,
            "internal error encountered while handling request"
        );
    }

    pub(crate) fn sanitize_client_error(&self, error: McpError) -> McpError {
        let redact_for_client = self.should_redact_internal_error_for_client(&error);
        self.log_internal_error_for_server(&error, redact_for_client);

        if redact_for_client {
            McpError::internal_error("internal server error", None)
        } else {
            error
        }
    }

    pub(crate) fn sanitize_tool_error_message(&self, error: &McpError) -> String {
        let redact_for_client = self.should_redact_internal_error_for_client(error);
        self.log_internal_error_for_server(error, redact_for_client);

        if redact_for_client {
            "internal server error".to_owned()
        } else {
            error.message.to_string()
        }
    }

    #[cfg(feature = "tasks_utility")]
    pub(crate) async fn notify_task_status(
        &self,
        task: rmcp::model::Task,
        peer: &rmcp::service::Peer<RoleServer>,
    ) -> Result<(), McpError> {
        if !self.config.tasks_status_notifications_active() {
            return Ok(());
        }

        let params = serde_json::to_value(task)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        peer.send_notification(ServerNotification::CustomNotification(
            CustomNotification::new("notifications/tasks/status", Some(params)),
        ))
        .await
        .map_err(|error| {
            McpError::internal_error(
                format!("failed to send task status notification: {error}"),
                None,
            )
        })
    }
}

#[cfg(test)]
// Inline placement is required because ClientNotificationHub is crate-private.
mod tests {
    use super::{ClientNotificationHub, Engine};
    use crate::inline_test_fixtures::{base_config, capture_logs, read_frame};
    use crate::mcp::{ErrorCode, McpError};
    use tracing::Level;

    #[tokio::test]
    async fn with_notification_hub_keeps_notification_helpers_callable() {
        let config = base_config();
        let engine = Engine::new(config).expect("engine should build");
        let engine = engine.with_notification_hub(ClientNotificationHub::default());

        assert_eq!(engine.notify_tools_list_changed().await, 0);
        assert_eq!(engine.notify_prompts_list_changed().await, 0);
        assert_eq!(engine.notify_resources_list_changed().await, 0);
    }

    #[tokio::test]
    async fn with_notification_hub_delivers_to_observed_peer() {
        use rmcp::model::{Extensions, Meta, NumberOrString};
        use rmcp::service::RequestContext;
        use tokio_util::sync::CancellationToken;

        let hub = ClientNotificationHub::default();
        let config = base_config();
        let engine = Engine::new(config).expect("engine should build");
        let engine = engine.with_notification_hub(hub.clone());

        // Wire a real peer into the hub via observe_request_peer.
        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly(engine.clone(), server_io, None);

        let context = RequestContext {
            peer: service.peer().clone(),
            ct: CancellationToken::new(),
            id: NumberOrString::Number(1),
            meta: Meta::default(),
            extensions: Extensions::default(),
        };
        if context.peer.peer_info().is_none() {
            let client_info = rmcp::model::Implementation::new("test-client", "1.0.0");
            context
                .peer
                .set_peer_info(rmcp::model::InitializeRequestParams::new(
                    rmcp::model::ClientCapabilities::default(),
                    client_info,
                ));
        }

        // Observe the peer through the engine's injected hub.
        engine.observe_request_peer(&context).await;

        // Notify should now reach one peer.
        let sent = engine.notify_tools_list_changed().await;
        assert_eq!(sent, 1, "injected hub must deliver to observed peer");

        // Verify the notification was actually sent on the wire.
        let frame = read_frame(&mut client_io).await;
        assert!(
            frame.is_some(),
            "notification frame must be received by peer"
        );

        let _ = service.close().await;
    }

    #[test]
    fn sanitize_client_error_redacts_client_message_and_logs_truncated_error_details() {
        let mut config = base_config();
        config.server.errors.expose_internal_details = false;
        config.server.logging.log_payload_max_bytes = 8;
        let engine = Engine::new(config).expect("engine should build");
        let error = McpError::internal_error("abcdefghijk", None);

        let logs = capture_logs(Level::ERROR, || {
            let sanitized = engine.sanitize_client_error(error.clone());
            assert_eq!(sanitized.code, ErrorCode::INTERNAL_ERROR);
            assert_eq!(sanitized.message, "internal server error");
        });

        assert!(
            logs.contains("internal error encountered while handling request"),
            "missing internal error log line: {logs}"
        );
        assert!(
            logs.contains("error_message=abcdefgh"),
            "missing truncated error message field: {logs}"
        );
        assert!(
            logs.contains("error_message_bytes=11"),
            "missing original byte count field: {logs}"
        );
        assert!(
            logs.contains("error_message_truncated=true"),
            "missing truncation marker: {logs}"
        );
        assert!(
            logs.contains("client_error_redacted=true"),
            "missing redaction marker: {logs}"
        );
    }

    #[test]
    fn sanitize_client_error_keeps_client_message_and_still_logs_truncated_error_details() {
        let mut config = base_config();
        config.server.errors.expose_internal_details = true;
        config.server.logging.log_payload_max_bytes = 8;
        let engine = Engine::new(config).expect("engine should build");
        let error = McpError::internal_error("abcdefghijk", None);

        let logs = capture_logs(Level::ERROR, || {
            let sanitized = engine.sanitize_client_error(error.clone());
            assert_eq!(sanitized.code, ErrorCode::INTERNAL_ERROR);
            assert_eq!(sanitized.message, "abcdefghijk");
        });

        assert!(
            logs.contains("internal error encountered while handling request"),
            "missing internal error log line: {logs}"
        );
        assert!(
            logs.contains("error_message=abcdefgh"),
            "missing truncated error message field: {logs}"
        );
        assert!(
            logs.contains("error_message_bytes=11"),
            "missing original byte count field: {logs}"
        );
        assert!(
            logs.contains("error_message_truncated=true"),
            "missing truncation marker: {logs}"
        );
        assert!(
            logs.contains("client_error_redacted=false"),
            "missing redaction marker: {logs}"
        );
    }

    #[test]
    fn sanitize_client_error_does_not_log_non_internal_errors() {
        let mut config = base_config();
        config.server.logging.log_payload_max_bytes = 4;
        let engine = Engine::new(config).expect("engine should build");
        let error = McpError::invalid_params("bad request", None);

        let logs = capture_logs(Level::ERROR, || {
            let sanitized = engine.sanitize_client_error(error.clone());
            assert_eq!(sanitized.code, ErrorCode::INVALID_PARAMS);
            assert_eq!(sanitized.message, "bad request");
        });

        assert!(
            logs.is_empty(),
            "non-internal errors should not log: {logs}"
        );
    }

    #[test]
    fn sanitize_client_error_logs_utf8_safe_truncation_boundaries() {
        let mut config = base_config();
        config.server.errors.expose_internal_details = false;
        config.server.logging.log_payload_max_bytes = 5;
        let engine = Engine::new(config).expect("engine should build");
        let error = McpError::internal_error("ab🦀cd", None);

        let logs = capture_logs(Level::ERROR, || {
            let sanitized = engine.sanitize_client_error(error.clone());
            assert_eq!(sanitized.message, "internal server error");
        });

        assert!(
            logs.contains("error_message=ab"),
            "utf-8 truncation should keep valid boundary: {logs}"
        );
        assert!(
            logs.contains("error_message_bytes=8"),
            "missing original utf-8 byte count field: {logs}"
        );
        assert!(
            logs.contains("error_message_truncated=true"),
            "missing truncation marker: {logs}"
        );
    }

    #[test]
    fn sanitize_tool_error_message_logs_internal_errors_even_when_details_are_exposed() {
        let mut config = base_config();
        config.server.errors.expose_internal_details = true;
        config.server.logging.log_payload_max_bytes = 4;
        let engine = Engine::new(config).expect("engine should build");
        let error = McpError::internal_error("abcdef", None);

        let logs = capture_logs(Level::ERROR, || {
            let message = engine.sanitize_tool_error_message(&error);
            assert_eq!(message, "abcdef");
        });

        assert!(
            logs.contains("error_message=abcd"),
            "tool-path internal errors should still emit capped server logs: {logs}"
        );
        assert!(
            logs.contains("client_error_redacted=false"),
            "missing redaction marker: {logs}"
        );
    }
}
