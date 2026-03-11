//! Runtime lifecycle: config validation, engine construction, transport startup, and reload.
use super::list_cache::{
    build_list_cache, list_changed_enabled, list_feature_label, prompt_list_payload,
    resource_list_payload, resource_templates_list_payload, serialize_payload, tools_list_payload,
    ListCache, RefreshLocks,
};
use super::runtime_checks::{compiled_feature_validation_error, init_logging, validate_transport};

#[cfg(test)]
use std::future::Future;
use std::{collections::HashSet, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::{Mutex as TokioMutex, RwLock};
use tracing::{debug, info, warn};

use crate::config::{McpConfig, TransportMode};
use crate::engine::ClientNotificationHub;
#[cfg(feature = "streamable_http")]
use crate::http::server::run_streamable_http;
use crate::plugins::{ListFeature, ListRefreshHandle, PluginRegistry};
#[cfg(any(feature = "streamable_http", test))]
use crate::{build_auth_state_with_plugins, AuthState};
use crate::{Engine, EngineConfig, McpError};

const REGISTRY_LIST_CHANGED_DEBOUNCE: Duration = Duration::from_secs(1);

// Boxed async-read/write pair plus an optional duplex guard used in test stdio stubs.
type StdioHandles = (
    Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    Option<tokio::io::DuplexStream>,
);

/// Long-lived runtime handle that owns server state and lifecycle.
///
/// Construct with [`Runtime::new`] or [`build_runtime`], then start serving with
/// [`Runtime::run`] or [`run_from_config`].
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

struct RuntimeInner {
    plugins: Arc<PluginRegistry>,
    reload_state: RwLock<Option<ReloadState>>,
    list_cache: RwLock<ListCache>,
    refresh_locks: RefreshLocks,
    notification_hub: ClientNotificationHub,
    pending_registry_notifications: TokioMutex<HashSet<ListFeature>>,
}

#[derive(Clone)]
struct ReloadState {
    transport_mode: TransportMode,
    engine: Arc<Engine>,
    #[cfg(any(feature = "streamable_http", test))]
    auth_state: Option<Arc<AuthState>>,
}

struct BuiltRuntimeState {
    reload_state: ReloadState,
    list_cache: ListCache,
}

impl Runtime {
    /// Builds a runtime from config and plugin registry, including startup validation.
    pub async fn new(config: McpConfig, plugins: PluginRegistry) -> Result<Self, McpError> {
        let runtime = Self {
            inner: Arc::new(RuntimeInner {
                plugins: Arc::new(plugins),
                reload_state: RwLock::new(None),
                list_cache: RwLock::new(ListCache::default()),
                refresh_locks: RefreshLocks::default(),
                notification_hub: ClientNotificationHub::default(),
                pending_registry_notifications: TokioMutex::new(HashSet::new()),
            }),
        };

        let initial_state = runtime.build_runtime_state(config).await?;
        {
            let mut reload_guard = runtime.inner.reload_state.write().await;
            *reload_guard = Some(initial_state.reload_state);
        }
        {
            let mut cache_guard = runtime.inner.list_cache.write().await;
            *cache_guard = initial_state.list_cache;
        }

        runtime.spawn_registry_event_worker();
        Ok(runtime)
    }

    /// Starts the configured transport and serves requests until shutdown/error.
    ///
    /// Uses streamable HTTP when `server.transport.mode=streamable_http`, otherwise stdio.
    pub async fn run(&self) -> Result<(), McpError> {
        let transport_mode = self.current_transport_mode().await?;
        if transport_mode == TransportMode::StreamableHttp {
            #[cfg(feature = "streamable_http")]
            {
                let auth_state = self.current_auth_state().await?.ok_or_else(|| {
                    McpError::internal_error("auth state unavailable".to_owned(), None)
                })?;
                let engine = self.current_engine().await?;
                run_streamable_http(engine, auth_state, Arc::clone(&self.inner.plugins)).await
            }
            #[cfg(not(feature = "streamable_http"))]
            {
                Err(McpError::invalid_request(
                    "streamable_http feature disabled but server.transport.mode=streamable_http"
                        .to_owned(),
                    None,
                ))
            }
        } else {
            let engine = self.current_engine().await?;
            run_stdio(engine).await
        }
    }

    /// Applies a new config snapshot at runtime and refreshes list-changed state.
    ///
    /// Reload is consumer-triggered: core does not watch config files.
    /// Build your own trigger (file watcher, admin endpoint, signal handler, etc.)
    /// and pass a newly loaded [`McpConfig`] here.
    ///
    /// If this returns `Err(...)`, the previous runtime state remains active.
    pub async fn reload_config(&self, new_config: McpConfig) -> Result<(), McpError> {
        let new_state = self.build_runtime_state(new_config).await?;
        let config = Arc::clone(&new_state.reload_state.engine.config);
        let (tools_changed, prompts_changed, resources_changed) = {
            let cache_guard = self.inner.list_cache.read().await;
            (
                cache_guard.tools != new_state.list_cache.tools,
                cache_guard.prompts != new_state.list_cache.prompts,
                cache_guard.resources != new_state.list_cache.resources
                    || cache_guard.resource_templates != new_state.list_cache.resource_templates,
            )
        };
        {
            let mut reload_guard = self.inner.reload_state.write().await;
            *reload_guard = Some(new_state.reload_state);
        }
        {
            let mut cache_guard = self.inner.list_cache.write().await;
            *cache_guard = new_state.list_cache;
        }
        check_and_notify_feature(tools_changed, config.as_ref(), self, ListFeature::Tools).await;
        check_and_notify_feature(prompts_changed, config.as_ref(), self, ListFeature::Prompts)
            .await;
        check_and_notify_feature(
            resources_changed,
            config.as_ref(),
            self,
            ListFeature::Resources,
        )
        .await;
        Ok(())
    }

    async fn current_transport_mode(&self) -> Result<TransportMode, McpError> {
        let guard = self.inner.reload_state.read().await;
        guard
            .as_ref()
            .map(|state| state.transport_mode)
            .ok_or_else(|| McpError::internal_error("runtime state unavailable".to_owned(), None))
    }

    async fn current_engine(&self) -> Result<Arc<Engine>, McpError> {
        let guard = self.inner.reload_state.read().await;
        guard
            .as_ref()
            .map(|state| Arc::clone(&state.engine))
            .ok_or_else(|| McpError::internal_error("runtime state unavailable".to_owned(), None))
    }

    #[cfg(any(feature = "streamable_http", test))]
    async fn current_auth_state(&self) -> Result<Option<Arc<AuthState>>, McpError> {
        let guard = self.inner.reload_state.read().await;
        guard
            .as_ref()
            .map(|state| state.auth_state.clone())
            .ok_or_else(|| McpError::internal_error("runtime state unavailable".to_owned(), None))
    }

    // Validates feature flags, initializes logging, selects transport, builds
    // the engine with plugins, constructs auth state, and snapshots list caches.
    async fn build_runtime_state(&self, config: McpConfig) -> Result<BuiltRuntimeState, McpError> {
        if let Some(error) = compiled_feature_validation_error(&config) {
            return Err(error);
        }

        init_logging(&config.server.logging.level)?;
        debug!("config loaded");

        let transport_mode = validate_transport(&config)?;
        debug!("transport mode selected: {:?}", transport_mode);

        let list_refresh_handle: Arc<dyn ListRefreshHandle> = Arc::new(self.clone());
        let engine = Engine::from_config(EngineConfig {
            config: config.clone(),
            plugins: (*self.inner.plugins).clone(),
            list_refresh_handle: Some(list_refresh_handle),
        })?
        .with_notification_hub(self.inner.notification_hub.clone());

        #[cfg(any(feature = "streamable_http", test))]
        let auth_state =
            build_auth_state_with_plugins(&config, Some(Arc::clone(&self.inner.plugins)))?;
        let list_cache = build_list_cache(&engine).await?;

        Ok(BuiltRuntimeState {
            reload_state: ReloadState {
                transport_mode,
                engine: Arc::new(engine),
                #[cfg(any(feature = "streamable_http", test))]
                auth_state: Some(auth_state),
            },
            list_cache,
        })
    }

    /// Triggers an MCP list refresh notification for the specified feature if enabled.
    pub async fn refresh_list(&self, feature: ListFeature) -> Result<bool, McpError> {
        match feature {
            ListFeature::Tools => {
                let _guard = self.inner.refresh_locks.tools.lock().await;
                self.refresh_tools_list().await
            }
            ListFeature::Prompts => {
                let _guard = self.inner.refresh_locks.prompts.lock().await;
                self.refresh_prompt_list().await
            }
            ListFeature::Resources => {
                let _guard = self.inner.refresh_locks.resources.lock().await;
                self.refresh_resource_lists().await
            }
        }
    }

    // Listens for plugin registry mutations (tool/prompt/resource changes) on a
    // broadcast channel and debounces list_changed notifications to connected
    // clients. Uses a weak Arc so the worker shuts down when Runtime is dropped.
    fn spawn_registry_event_worker(&self) {
        let mut rx = self.inner.plugins.subscribe_events();
        let weak_inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("registry event worker lagged by {} messages", skipped);
                        continue;
                    }
                };

                let Some(inner) = weak_inner.upgrade() else {
                    break;
                };
                let runtime = Self { inner };
                let feature = match event {
                    crate::plugins::RegistryEvent::ToolChanged => ListFeature::Tools,
                    crate::plugins::RegistryEvent::PromptChanged => ListFeature::Prompts,
                    crate::plugins::RegistryEvent::ResourceChanged => ListFeature::Resources,
                };
                runtime.schedule_registry_list_changed(feature).await;
            }
        });
    }

    // Debounces registry-triggered list_changed notifications: inserts the
    // feature into a pending set and spawns a delayed flush. If the feature
    // is already pending, skips the duplicate spawn.
    async fn schedule_registry_list_changed(&self, feature: ListFeature) {
        let enabled = {
            let guard = self.inner.reload_state.read().await;
            guard
                .as_ref()
                .is_some_and(|state| list_changed_enabled(state.engine.config.as_ref(), feature))
        };
        if !enabled {
            return;
        }

        let should_spawn = {
            let mut pending = self.inner.pending_registry_notifications.lock().await;
            pending.insert(feature)
        };
        if !should_spawn {
            return;
        }

        let weak_inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            tokio::time::sleep(REGISTRY_LIST_CHANGED_DEBOUNCE).await;
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            let runtime = Self { inner };
            runtime.flush_registry_list_changed(feature).await;
        });
    }

    async fn flush_registry_list_changed(&self, feature: ListFeature) {
        {
            let mut pending = self.inner.pending_registry_notifications.lock().await;
            if !pending.remove(&feature) {
                return;
            }
        }

        let engine = match self.current_engine().await {
            Ok(state) => state,
            Err(error) => {
                warn!("failed to emit list_changed notification: {}", error);
                return;
            }
        };

        if !list_changed_enabled(engine.config.as_ref(), feature) {
            return;
        }

        let recipients = self.notify_list_changed_via_hub(feature).await;
        debug!(
            "{} list_changed notifications sent to {}",
            list_feature_label(feature),
            recipients
        );
    }

    async fn current_engine_if_fresh(
        &self,
        snapshot: &Arc<Engine>,
        stale_log: &'static str,
    ) -> Result<Option<Arc<Engine>>, McpError> {
        let current_engine = self.current_engine().await?;
        if Arc::ptr_eq(snapshot, &current_engine) {
            Ok(Some(current_engine))
        } else {
            debug!("{stale_log}");
            Ok(None)
        }
    }

    async fn refresh_tools_list(&self) -> Result<bool, McpError> {
        let engine = self.current_engine().await?;
        let old_payload = { self.inner.list_cache.read().await.tools.clone() };
        let new_tools = serialize_payload(&tools_list_payload(engine.as_ref())?)?;
        if old_payload == new_tools {
            return Ok(false);
        }

        let Some(current_engine) = self
            .current_engine_if_fresh(&engine, "discarded stale tools refresh result after reload")
            .await?
        else {
            return Ok(false);
        };

        let changed = {
            let mut guard = self.inner.list_cache.write().await;
            let changed = guard.tools != new_tools;
            if changed {
                guard.tools = new_tools;
            }
            changed
        };
        check_and_notify_feature(
            changed,
            current_engine.config.as_ref(),
            self,
            ListFeature::Tools,
        )
        .await;
        Ok(changed)
    }

    // Refreshes the prompt list cache with stale-write protection. If a reload
    // swapped the engine between snapshot and write-back, the stale result is discarded.
    #[allow(clippy::cognitive_complexity)]
    async fn refresh_prompt_list(&self) -> Result<bool, McpError> {
        let engine = self.current_engine().await?;
        let old_payload = { self.inner.list_cache.read().await.prompts.clone() };
        let new_prompts = serialize_payload(&prompt_list_payload(engine.as_ref()).await?)?;
        if old_payload == new_prompts {
            return Ok(false);
        }

        let Some(current_engine) = self
            .current_engine_if_fresh(
                &engine,
                "discarded stale prompts refresh result after reload",
            )
            .await?
        else {
            return Ok(false);
        };

        let changed = {
            let mut guard = self.inner.list_cache.write().await;
            let changed = guard.prompts != new_prompts;
            if changed {
                guard.prompts = new_prompts;
            }
            changed
        };
        check_and_notify_feature(
            changed,
            current_engine.config.as_ref(),
            self,
            ListFeature::Prompts,
        )
        .await;
        Ok(changed)
    }

    // Refreshes resources and resource templates with stale-write protection.
    #[allow(clippy::cognitive_complexity)]
    async fn refresh_resource_lists(&self) -> Result<bool, McpError> {
        let engine = self.current_engine().await?;
        let (old_resources, old_templates) = {
            let guard = self.inner.list_cache.read().await;
            (guard.resources.clone(), guard.resource_templates.clone())
        };
        let new_resources = serialize_payload(&resource_list_payload(engine.as_ref()).await?)?;
        let new_templates =
            serialize_payload(&resource_templates_list_payload(engine.as_ref()).await?)?;
        if old_resources == new_resources && old_templates == new_templates {
            return Ok(false);
        }

        let Some(current_engine) = self
            .current_engine_if_fresh(
                &engine,
                "discarded stale resources refresh result after reload",
            )
            .await?
        else {
            return Ok(false);
        };

        let changed = {
            let mut guard = self.inner.list_cache.write().await;
            let changed =
                guard.resources != new_resources || guard.resource_templates != new_templates;
            if changed {
                guard.resources = new_resources;
                guard.resource_templates = new_templates;
            }
            changed
        };
        check_and_notify_feature(
            changed,
            current_engine.config.as_ref(),
            self,
            ListFeature::Resources,
        )
        .await;
        Ok(changed)
    }

    async fn notify_list_changed_via_hub(&self, feature: ListFeature) -> usize {
        match feature {
            ListFeature::Tools => {
                self.inner
                    .notification_hub
                    .notify_tools_list_changed()
                    .await
            }
            ListFeature::Prompts => {
                self.inner
                    .notification_hub
                    .notify_prompts_list_changed()
                    .await
            }
            ListFeature::Resources => {
                self.inner
                    .notification_hub
                    .notify_resources_list_changed()
                    .await
            }
        }
    }
}

#[async_trait]
impl ListRefreshHandle for Runtime {
    async fn refresh_list(&self, feature: ListFeature) -> Result<bool, McpError> {
        self.refresh_list(feature).await
    }
}

/// Builds a [`Runtime`] without starting transport.
///
/// Use this entrypoint when your host needs runtime control such as
/// [`Runtime::reload_config`]. For one-shot startup with no runtime handle,
/// use [`run_from_config`].
pub async fn build_runtime(
    config: McpConfig,
    plugins: PluginRegistry,
) -> Result<Runtime, McpError> {
    Runtime::new(config, plugins).await
}

/// Convenience entry point: build runtime and immediately run it.
pub async fn run_from_config(config: McpConfig, plugins: PluginRegistry) -> Result<(), McpError> {
    let runtime = Runtime::new(config, plugins).await?;
    runtime.run().await
}

#[cfg(test)]
async fn run_from_config_with_hooks<FHttp, FStdio, FutHttp, FutStdio>(
    config: McpConfig,
    plugins: PluginRegistry,
    run_http: FHttp,
    run_stdio_hook: FStdio,
) -> Result<(), McpError>
where
    FHttp: FnOnce(Arc<Engine>, Arc<AuthState>, Arc<PluginRegistry>) -> FutHttp,
    FStdio: FnOnce(Arc<Engine>) -> FutStdio,
    FutHttp: Future<Output = Result<(), McpError>>,
    FutStdio: Future<Output = Result<(), McpError>>,
{
    let runtime = Runtime::new(config, plugins).await?;
    if runtime.current_transport_mode().await? == TransportMode::StreamableHttp {
        let auth_state = runtime
            .current_auth_state()
            .await?
            .ok_or_else(|| McpError::internal_error("auth state unavailable".to_owned(), None))?;
        let engine = runtime.current_engine().await?;
        return run_http(engine, auth_state, Arc::clone(&runtime.inner.plugins)).await;
    }
    let engine = runtime.current_engine().await?;
    run_stdio_hook(engine).await
}

async fn run_stdio(engine: Arc<Engine>) -> Result<(), McpError> {
    info!("starting stdio transport");
    let (stdin, stdout, _guard) = stdio_handles_for_run();
    // rmcp serves by owned handler type; clone at this boundary only.
    let service = rmcp::service::serve_directly((*engine).clone(), (stdin, stdout), None);
    service
        .waiting()
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    Ok(())
}

fn stdio_handles_for_run() -> StdioHandles {
    if cfg!(test) && std::env::var("MCP_TEST_STDIO").is_ok() {
        let (stdin_reader, stdin_writer) = tokio::io::duplex(64);
        let (stdout_reader, stdout_writer) = tokio::io::duplex(64);
        drop(stdin_writer);
        return (
            Box::new(stdin_reader),
            Box::new(stdout_writer),
            Some(stdout_reader),
        );
    }
    (
        Box::new(tokio::io::stdin()),
        Box::new(tokio::io::stdout()),
        None,
    )
}

// Sends a list_changed notification for `feature` if `changed` is true and the
// feature has list_changed notifications enabled in config. The label is derived
// from the feature variant via `list_feature_label`.
async fn check_and_notify_feature(
    changed: bool,
    config: &McpConfig,
    runtime: &Runtime,
    feature: ListFeature,
) {
    if changed && list_changed_enabled(config, feature) {
        let label = list_feature_label(feature);
        debug!("{label} changed");
        let recipients = runtime.notify_list_changed_via_hub(feature).await;
        debug!("{label} notifications sent to {recipients}");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "streamable_http")]
    use super::build_runtime;
    #[cfg(feature = "streamable_http")]
    use super::Runtime;
    use super::{run_from_config, run_from_config_with_hooks, run_stdio, stdio_handles_for_run};
    use crate::config::TransportMode;
    use crate::inline_test_fixtures::{
        base_config, clear_env, ok_http_hook, ok_stdio_hook, set_env,
    };
    #[cfg(feature = "streamable_http")]
    use crate::inline_test_fixtures::{
        read_frame, read_frame_with_timeout, request_context, RegistryMutationTool,
    };
    #[cfg(feature = "streamable_http")]
    use crate::mcp::{ClientCapabilities, Implementation, InitializeRequestParams};
    #[cfg(feature = "streamable_http")]
    use crate::plugins::ListFeature;
    use crate::plugins::PluginRegistry;
    use crate::{Engine, EngineConfig, McpError};
    #[cfg(feature = "streamable_http")]
    use rmcp::ServerHandler;
    #[cfg(feature = "streamable_http")]
    use serde::Deserialize;
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    use serde_json::json;
    #[cfg(feature = "streamable_http")]
    use serde_json::Value;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    #[cfg(feature = "streamable_http")]
    use std::time::Duration;

    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    fn builtin_noop_tool(name: &str) -> crate::config::ToolConfig {
        crate::config::ToolConfig {
            name: name.to_owned(),
            title: None,
            description: "noop".to_owned(),
            cancellable: true,
            input_schema: json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
            execute: crate::config::ExecuteConfig::Http(crate::config::ExecuteHttpConfig {
                upstream: "noop".to_owned(),
                method: "GET".to_owned(),
                path: "/".to_owned(),
                query: None,
                headers: None,
                body: None,
                retry: None,
                task_support: crate::config::TaskSupport::Forbidden,
            }),
            response: None,
        }
    }

    #[test]
    fn env_guard_restores_existing_value() {
        std::env::set_var("MCP_RUNTIME_TEST_ENV", "original");
        {
            let _guard = set_env("MCP_RUNTIME_TEST_ENV", "temporary");
            assert_eq!(
                std::env::var("MCP_RUNTIME_TEST_ENV").as_deref(),
                Ok("temporary")
            );
        }
        assert_eq!(
            std::env::var("MCP_RUNTIME_TEST_ENV").as_deref(),
            Ok("original")
        );
        std::env::remove_var("MCP_RUNTIME_TEST_ENV");
    }

    #[test]
    fn env_guard_restores_absent_variable() {
        std::env::remove_var("MCP_RUNTIME_TEST_ENV_ABSENT");
        {
            let _guard = set_env("MCP_RUNTIME_TEST_ENV_ABSENT", "temporary");
            assert_eq!(
                std::env::var("MCP_RUNTIME_TEST_ENV_ABSENT").as_deref(),
                Ok("temporary")
            );
        }
        assert!(
            std::env::var("MCP_RUNTIME_TEST_ENV_ABSENT").is_err(),
            "guard must restore missing variable to absent state"
        );
    }

    #[cfg(feature = "streamable_http")]
    fn parse_jsonrpc_notification(frame: &str) -> Value {
        let json_start = frame.find('{').expect("frame must contain JSON object");
        let json_str = &frame[json_start..];
        let mut deserializer = serde_json::Deserializer::from_str(json_str);
        Value::deserialize(&mut deserializer).expect("frame must contain valid JSON")
    }

    #[cfg(feature = "streamable_http")]
    fn assert_jsonrpc_list_changed_notification(frame: &str, expected_method: &str) {
        let parsed = parse_jsonrpc_notification(frame);
        assert_eq!(
            parsed.get("jsonrpc").and_then(|v| v.as_str()),
            Some("2.0"),
            "notification must be JSON-RPC 2.0"
        );
        assert_eq!(
            parsed.get("method").and_then(|v| v.as_str()),
            Some(expected_method),
            "unexpected notification method"
        );
        assert!(
            parsed.get("id").is_none(),
            "list_changed notification must not include id"
        );
        assert!(
            parsed.get("params").is_none(),
            "list_changed notification must not include params"
        );
    }

    #[tokio::test]
    #[cfg(feature = "streamable_http")]
    async fn run_uses_streamable_http_with_shutdown_flag() {
        let _guard = set_env("MCP_TEST_HTTP_SHUTDOWN", "1");
        let runtime = Runtime::new(base_config(), PluginRegistry::default())
            .await
            .expect("runtime");
        assert_eq!(
            runtime
                .current_transport_mode()
                .await
                .expect("runtime mode"),
            TransportMode::StreamableHttp
        );
        assert!(
            runtime
                .current_auth_state()
                .await
                .expect("auth state query")
                .is_some(),
            "streamable runtime must build auth state"
        );
        let result = runtime.run().await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn run_from_config_stdio_path_runs_real_entrypoint() {
        let _guard = set_env("MCP_TEST_STDIO", "1");
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        let result = run_from_config(config, PluginRegistry::default()).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    #[cfg(feature = "streamable_http")]
    async fn build_runtime_helper_constructs_runtime() {
        let runtime = build_runtime(base_config(), PluginRegistry::default())
            .await
            .expect("runtime");
        let transport_mode = runtime.current_transport_mode().await.expect("state");
        assert_eq!(transport_mode, TransportMode::StreamableHttp);
        runtime
            .current_engine()
            .await
            .expect("runtime engine must exist");
        assert!(
            runtime
                .current_auth_state()
                .await
                .expect("auth state query")
                .is_some(),
            "streamable runtime must include auth state"
        );
    }

    #[tokio::test]
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    async fn run_from_config_streamable_uses_hook() {
        let mut config = base_config();
        config
            .tools_items_mut()
            .push(builtin_noop_tool("tool.noop"));
        let http_hit = Arc::new(AtomicBool::new(false));
        let stdio_hit = Arc::new(AtomicBool::new(false));
        let http_hit_for_hook = Arc::clone(&http_hit);
        let stdio_hit_for_hook = Arc::clone(&stdio_hit);

        let result = run_from_config_with_hooks(
            config,
            PluginRegistry::default(),
            move |_engine, auth_state, _plugins| {
                http_hit_for_hook.store(true, Ordering::SeqCst);
                assert!(
                    !auth_state.oauth_enabled(),
                    "base config auth mode=none should not enable oauth state"
                );
                async { Ok::<(), McpError>(()) }
            },
            move |_engine| {
                stdio_hit_for_hook.store(true, Ordering::SeqCst);
                async { Ok::<(), McpError>(()) }
            },
        )
        .await;
        assert_eq!(result, Ok(()));
        assert!(http_hit.load(Ordering::SeqCst));
        assert!(
            !stdio_hit.load(Ordering::SeqCst),
            "stdio hook must not run in streamable branch"
        );
    }

    #[tokio::test]
    async fn run_from_config_stdio_uses_hook() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        let hit = Arc::new(AtomicBool::new(false));
        let hit_for_hook = Arc::clone(&hit);
        let result = run_from_config_with_hooks(
            config,
            PluginRegistry::default(),
            ok_http_hook,
            move |_engine| {
                hit_for_hook.store(true, Ordering::SeqCst);
                async { Ok::<(), McpError>(()) }
            },
        )
        .await;
        assert_eq!(result, Ok(()));
        assert!(hit.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn run_from_config_stdio_uses_ok_stdio_hook_function() {
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        let result = run_from_config_with_hooks(
            config,
            PluginRegistry::default(),
            ok_http_hook,
            ok_stdio_hook,
        )
        .await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    async fn refresh_tools_list_detects_cache_drift() {
        let mut config = base_config();
        config.set_tools_notify_list_changed(true);
        config
            .tools_items_mut()
            .push(builtin_noop_tool("tool.noop"));

        let runtime = Runtime::new(config, PluginRegistry::default())
            .await
            .expect("runtime");
        let expected_tools = {
            let engine = runtime.current_engine().await.expect("state");
            super::serialize_payload(&super::tools_list_payload(engine.as_ref()).expect("tools"))
                .expect("serialize tools payload")
        };
        {
            let mut guard = runtime.inner.list_cache.write().await;
            guard.tools = Vec::new();
        }

        assert!(runtime
            .refresh_list(ListFeature::Tools)
            .await
            .expect("refresh"));
        {
            let guard = runtime.inner.list_cache.read().await;
            assert_eq!(guard.tools, expected_tools);
        }
        assert!(!runtime
            .refresh_list(ListFeature::Tools)
            .await
            .expect("refresh"));
    }

    #[tokio::test]
    #[cfg(feature = "streamable_http")]
    async fn refresh_tools_list_emits_tools_list_changed_notification() {
        let mut config = base_config();
        config.set_tools_notify_list_changed(true);
        let runtime = Runtime::new(config, PluginRegistry::default())
            .await
            .expect("runtime");
        let engine = runtime.current_engine().await.expect("state");

        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly((*engine).clone(), server_io, None);

        service
            .service()
            .initialize(
                InitializeRequestParams::new(
                    ClientCapabilities::default(),
                    Implementation::new("test-client", "1.0.0"),
                ),
                request_context(&service),
            )
            .await
            .expect("initialize");

        ServerHandler::list_tools(service.service(), None, request_context(&service))
            .await
            .expect("list tools");

        {
            let mut guard = runtime.inner.list_cache.write().await;
            guard.tools = b"stale-cache".to_vec();
        }

        assert!(runtime
            .refresh_list(ListFeature::Tools)
            .await
            .expect("refresh tools"));

        let frame = read_frame(&mut client_io)
            .await
            .expect("tools list_changed notification");
        assert_jsonrpc_list_changed_notification(&frame, "notifications/tools/list_changed");
        assert!(
            read_frame_with_timeout(&mut client_io, Duration::from_millis(200))
                .await
                .is_none(),
            "refresh should emit exactly one list_changed frame"
        );

        let _ = service.close().await;
    }

    #[tokio::test]
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    async fn reload_config_emits_tools_list_changed_notification_on_delta() {
        let mut config = base_config();
        config.set_tools_notify_list_changed(true);
        let runtime = Runtime::new(config, PluginRegistry::default())
            .await
            .expect("runtime");
        let engine = runtime.current_engine().await.expect("state");

        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly((*engine).clone(), server_io, None);

        service
            .service()
            .initialize(
                InitializeRequestParams::new(
                    ClientCapabilities::default(),
                    Implementation::new("test-client", "1.0.0"),
                ),
                request_context(&service),
            )
            .await
            .expect("initialize");

        ServerHandler::list_tools(service.service(), None, request_context(&service))
            .await
            .expect("list tools");

        let mut reloaded = base_config();
        reloaded.set_tools_notify_list_changed(true);
        reloaded
            .tools_items_mut()
            .push(builtin_noop_tool("tool.after_reload"));
        runtime
            .reload_config(reloaded)
            .await
            .expect("reload config should succeed");

        let frame = read_frame_with_timeout(&mut client_io, Duration::from_millis(1000))
            .await
            .expect("tools list_changed notification");
        assert_jsonrpc_list_changed_notification(&frame, "notifications/tools/list_changed");
        assert!(
            read_frame_with_timeout(&mut client_io, Duration::from_millis(200))
                .await
                .is_none(),
            "reload should emit exactly one list_changed frame"
        );

        let _ = service.close().await;
    }

    #[tokio::test]
    #[cfg(feature = "streamable_http")]
    async fn registry_tool_mutation_emits_single_debounced_notification() {
        let mut config = base_config();
        config.set_tools_notify_list_changed(true);

        let registry = PluginRegistry::default();
        let event_registry = registry.clone();
        let runtime = Runtime::new(config, registry).await.expect("runtime");
        let engine = runtime.current_engine().await.expect("state");

        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly((*engine).clone(), server_io, None);

        service
            .service()
            .initialize(
                InitializeRequestParams::new(
                    ClientCapabilities::default(),
                    Implementation::new("test-client", "1.0.0"),
                ),
                request_context(&service),
            )
            .await
            .expect("initialize");

        let updated_registry = event_registry
            .register_tool(RegistryMutationTool {
                name: "registry.mutation",
            })
            .expect("register");
        let _updated_registry = updated_registry
            .replace_tool(RegistryMutationTool {
                name: "registry.mutation",
            })
            .expect("replace");

        let frame = read_frame_with_timeout(&mut client_io, Duration::from_millis(1500))
            .await
            .expect("debounced tools list_changed notification");
        assert_jsonrpc_list_changed_notification(&frame, "notifications/tools/list_changed");
        assert!(read_frame(&mut client_io).await.is_none());

        let _ = service.close().await;
    }

    #[tokio::test]
    #[cfg(feature = "streamable_http")]
    async fn refresh_prompt_and_resource_lists_detect_config_changes() {
        let runtime = Runtime::new(base_config(), PluginRegistry::default())
            .await
            .expect("runtime");
        let (expected_prompts, expected_resources, expected_resource_templates) = {
            let engine = runtime.current_engine().await.expect("state");
            let prompts = super::serialize_payload(
                &super::prompt_list_payload(engine.as_ref())
                    .await
                    .expect("prompt payload"),
            )
            .expect("serialize prompts payload");
            let resources = super::serialize_payload(
                &super::resource_list_payload(engine.as_ref())
                    .await
                    .expect("resource payload"),
            )
            .expect("serialize resources payload");
            let templates = super::serialize_payload(
                &super::resource_templates_list_payload(engine.as_ref())
                    .await
                    .expect("resource templates payload"),
            )
            .expect("serialize templates payload");
            (prompts, resources, templates)
        };

        {
            let mut guard = runtime.inner.list_cache.write().await;
            guard.prompts = Vec::new();
        }
        assert!(runtime
            .refresh_list(ListFeature::Prompts)
            .await
            .expect("refresh"));
        assert!(!runtime
            .refresh_list(ListFeature::Prompts)
            .await
            .expect("refresh"));
        {
            let guard = runtime.inner.list_cache.read().await;
            assert_eq!(guard.prompts, expected_prompts);
        }

        {
            let mut guard = runtime.inner.list_cache.write().await;
            guard.resources = Vec::new();
            guard.resource_templates = Vec::new();
        }
        assert!(runtime
            .refresh_list(ListFeature::Resources)
            .await
            .expect("refresh"));
        {
            let guard = runtime.inner.list_cache.read().await;
            assert_eq!(guard.resources, expected_resources);
            assert_eq!(guard.resource_templates, expected_resource_templates);
        }
    }

    #[tokio::test]
    async fn run_stdio_returns_ok_with_stub() {
        let _guard = set_env("MCP_TEST_STDIO", "1");
        let mut config = base_config();
        config.server.transport.mode = TransportMode::Stdio;
        let engine = Engine::from_config(EngineConfig {
            config,
            plugins: PluginRegistry::default(),
            list_refresh_handle: None,
        })
        .expect("engine should build");
        let result = run_stdio(Arc::new(engine)).await;
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn stdio_handles_for_run_defaults_to_stdio() {
        let _guard = clear_env("MCP_TEST_STDIO");
        let (_stdin, _stdout, guard) = stdio_handles_for_run();
        assert!(guard.is_none());
    }

    #[test]
    fn stdio_handles_for_run_uses_stub_when_test_flag_is_set() {
        let _guard = set_env("MCP_TEST_STDIO", "1");
        let (_stdin, _stdout, guard) = stdio_handles_for_run();
        assert!(guard.is_some());
    }
}
