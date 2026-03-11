#![cfg(all(
    feature = "streamable_http",
    feature = "prompts",
    feature = "resources"
))]

use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
mod config_common;

use crate::config_common::base_config_streamable_http_with_builtin_noop as base_config;
use rmcp::{
    model::{
        AnnotateAble, ErrorCode, GetPromptResult, Prompt, PromptMessageRole, ReadResourceResult,
        ResourceContents,
    },
    ErrorData as McpError,
};
use rust_mcp_core::{
    config::{
        PluginConfig, PromptItemConfig, PromptMessageRoleConfig, PromptProviderConfig,
        PromptTemplateConfig, PromptTemplateMessageConfig, PromptsConfig, ResourceContentConfig,
        ResourceItemConfig, ResourceProviderConfig, ResourcesConfig,
    },
    plugins::{
        ListFeature, PluginCallParams, PromptEntry, PromptPlugin, ResourceEntry, ResourcePlugin,
    },
    runtime, PluginRegistry, PluginType,
};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::time::{sleep, Duration, Instant};

const BLOCKING_PROMPT_PLUGIN_NAME: &str = "prompt.blocking";
const BLOCKING_RESOURCE_PLUGIN_NAME: &str = "resource.blocking";

struct BlockingPromptPlugin {
    started: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl PromptPlugin for BlockingPromptPlugin {
    fn name(&self) -> &str {
        BLOCKING_PROMPT_PLUGIN_NAME
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
        if call_index == 0 {
            return Ok(vec![PromptEntry {
                prompt: Prompt::new("prompt.initial", Some("initial prompt"), None),
                arguments_schema: json!({"type": "object"}),
                completions: None,
            }]);
        }
        self.started.notify_one();
        self.release.notified().await;

        Ok(vec![PromptEntry {
            prompt: Prompt::new("prompt.blocking", Some("blocking prompt"), None),
            arguments_schema: json!({"type": "object"}),
            completions: None,
        }])
    }

    async fn get(
        &self,
        _name: &str,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError> {
        Ok(
            GetPromptResult::new(vec![rmcp::model::PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "ok",
            )])
            .with_description("blocking prompt"),
        )
    }
}

struct BlockingResourcePlugin {
    started: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ResourcePlugin for BlockingResourcePlugin {
    fn name(&self) -> &str {
        BLOCKING_RESOURCE_PLUGIN_NAME
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
        if call_index == 0 {
            return Ok(vec![ResourceEntry {
                resource: rmcp::model::RawResource::new(
                    "resource://blocking/initial",
                    "blocking-initial",
                )
                .no_annotation(),
            }]);
        }
        self.started.notify_one();
        self.release.notified().await;
        Ok(vec![ResourceEntry {
            resource: rmcp::model::RawResource::new("resource://blocking/one", "blocking-one")
                .no_annotation(),
        }])
    }

    async fn read(
        &self,
        uri: &str,
        _params: PluginCallParams,
    ) -> Result<ReadResourceResult, McpError> {
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: uri.to_owned(),
                mime_type: Some("text/plain".to_owned()),
                text: "ok".to_owned(),
                meta: None,
            },
        ]))
    }

    async fn subscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }

    async fn unsubscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }
}

fn inline_prompts(name: &str, text: &str) -> PromptsConfig {
    PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Inline {
            items: vec![PromptItemConfig {
                name: name.to_owned(),
                title: None,
                description: Some(text.to_owned()),
                icons: None,
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    }
                }),
                template: PromptTemplateConfig {
                    messages: vec![PromptTemplateMessageConfig {
                        role: PromptMessageRoleConfig::Assistant,
                        content: Value::String(text.to_owned()),
                    }],
                },
                completions: None,
            }],
        }],
    }
}

fn inline_resources(uri: &str, name: &str) -> ResourcesConfig {
    ResourcesConfig {
        enabled: None,
        notify_list_changed: false,
        clients_can_subscribe: false,
        pagination: None,
        providers: vec![ResourceProviderConfig::Inline {
            items: Some(vec![ResourceItemConfig {
                uri: uri.to_owned(),
                name: name.to_owned(),
                title: None,
                description: Some("inline resource".to_owned()),
                mime_type: Some("text/plain".to_owned()),
                size: None,
                icons: None,
                annotations: None,
                content: Some(ResourceContentConfig {
                    text: Some("inline content".to_owned()),
                    blob: None,
                }),
            }]),
            templates: None,
        }],
    }
}

fn reserve_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback port");
    listener.local_addr().expect("local addr").port()
}

async fn wait_for_server(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("streamable HTTP server did not start on port {port}");
}

async fn initialize_session(client: &reqwest::Client, endpoint: &str) -> String {
    let initialize = json!({
        "jsonrpc":"2.0",
        "id": 1,
        "method":"initialize",
        "params":{
            "protocolVersion":"2025-06-18",
            "capabilities":{},
            "clientInfo":{"name":"test-client","version":"1.0.0"}
        }
    });
    let initialize_response = client
        .post(endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .expect("initialize request should complete");
    assert_eq!(initialize_response.status(), reqwest::StatusCode::OK);
    let session_id = initialize_response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("initialize response should include MCP session id");

    let initialized = json!({
        "jsonrpc":"2.0",
        "method":"notifications/initialized",
        "params":{}
    });
    let initialized_response = client
        .post(endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&initialized)
        .send()
        .await
        .expect("initialized request should complete");
    assert_eq!(initialized_response.status(), reqwest::StatusCode::ACCEPTED);

    session_id
}

fn parse_list_payload(list_body: &str) -> Value {
    serde_json::from_str(list_body).unwrap_or_else(|_| {
        list_body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|line| !line.trim().is_empty())
            .find_map(|line| serde_json::from_str::<Value>(line).ok())
            .expect("list response should contain JSON payload")
    })
}

async fn snapshot_list_names(
    runtime: &runtime::Runtime,
    port: u16,
    method: &str,
    field: &str,
) -> Vec<String> {
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let runtime_for_server = runtime.clone();
    let server = tokio::spawn(async move { runtime_for_server.run().await });
    wait_for_server(port).await;

    let client = reqwest::Client::new();
    let session_id = initialize_session(&client, &endpoint).await;

    let list_request = json!({
        "jsonrpc":"2.0",
        "id": 2,
        "method": method,
        "params":{}
    });
    let list_response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&list_request)
        .send()
        .await
        .expect("list request should complete");
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body = list_response
        .text()
        .await
        .expect("list response body should read");
    let body = parse_list_payload(&list_body);
    let mut names = body["result"][field]
        .as_array()
        .expect("list field should be an array")
        .iter()
        .map(|entry| {
            entry["name"]
                .as_str()
                .expect("list entry should have name")
                .to_owned()
        })
        .collect::<Vec<_>>();
    names.sort();

    server.abort();
    let _ = server.await;
    names
}

#[tokio::test]
async fn refresh_list_returns_false_when_payload_unchanged() {
    let port = reserve_loopback_port();
    let mut config = base_config();
    config.server.port = port;
    config.prompts = Some(inline_prompts("prompt.stable", "stable"));
    config.resources = Some(inline_resources(
        "resource://inline/stable",
        "inline-stable",
    ));

    let runtime = runtime::build_runtime(config, PluginRegistry::new())
        .await
        .expect("runtime should build");

    // First refresh: payload unchanged, returns false.
    let tools_changed = runtime
        .refresh_list(ListFeature::Tools)
        .await
        .expect("tools refresh");
    assert!(
        !tools_changed,
        "tools should be unchanged after initial build"
    );

    let prompts_changed = runtime
        .refresh_list(ListFeature::Prompts)
        .await
        .expect("prompts refresh");
    assert!(
        !prompts_changed,
        "prompts should be unchanged after initial build"
    );

    let resources_changed = runtime
        .refresh_list(ListFeature::Resources)
        .await
        .expect("resources refresh");
    assert!(
        !resources_changed,
        "resources should be unchanged after initial build"
    );

    // Second refresh: still unchanged — idempotency.
    let tools_changed_again = runtime
        .refresh_list(ListFeature::Tools)
        .await
        .expect("tools refresh second call");
    assert!(
        !tools_changed_again,
        "tools should remain unchanged on second refresh"
    );

    let prompts_changed_again = runtime
        .refresh_list(ListFeature::Prompts)
        .await
        .expect("prompts refresh second call");
    assert!(
        !prompts_changed_again,
        "prompts should remain unchanged on second refresh"
    );

    let resources_changed_again = runtime
        .refresh_list(ListFeature::Resources)
        .await
        .expect("resources refresh second call");
    assert!(
        !resources_changed_again,
        "resources should remain unchanged on second refresh"
    );

    let tool_names = snapshot_list_names(&runtime, port, "tools/list", "tools").await;
    assert_eq!(tool_names, vec!["tool.noop".to_owned()]);

    let prompt_names = snapshot_list_names(&runtime, port, "prompts/list", "prompts").await;
    assert_eq!(prompt_names, vec!["prompt.stable".to_owned()]);

    let resource_names = snapshot_list_names(&runtime, port, "resources/list", "resources").await;
    assert_eq!(resource_names, vec!["inline-stable".to_owned()]);
}

#[tokio::test]
async fn reload_config_validation_error_keeps_previous_runtime_state() {
    let port = reserve_loopback_port();
    let mut config = base_config();
    config.server.port = port;

    let runtime = runtime::build_runtime(config.clone(), PluginRegistry::new())
        .await
        .expect("runtime should build");

    let tools_before = snapshot_list_names(&runtime, port, "tools/list", "tools").await;
    assert_eq!(tools_before, vec!["tool.noop".to_owned()]);

    let mut invalid = config;
    invalid.server.host = String::new();

    let error = runtime
        .reload_config(invalid)
        .await
        .expect_err("reload with empty host should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "server.host is required for streamable_http");

    let tools_after = snapshot_list_names(&runtime, port, "tools/list", "tools").await;
    assert_eq!(tools_after, tools_before);
}

#[tokio::test]
async fn reload_config_swaps_runtime_state_on_success() {
    let port = reserve_loopback_port();
    let mut initial = base_config();
    initial.server.port = port;
    initial.prompts = Some(inline_prompts("prompt.initial", "initial"));

    let runtime = runtime::build_runtime(initial, PluginRegistry::new())
        .await
        .expect("runtime should build");

    let tools_before = snapshot_list_names(&runtime, port, "tools/list", "tools").await;
    let prompts_before = snapshot_list_names(&runtime, port, "prompts/list", "prompts").await;
    assert_eq!(prompts_before, vec!["prompt.initial".to_owned()]);

    let mut reloaded = base_config();
    reloaded.server.port = port;
    reloaded.prompts = Some(inline_prompts("prompt.dynamic", "dynamic"));

    runtime
        .reload_config(reloaded)
        .await
        .expect("reload with valid config should succeed");

    let prompts_after = snapshot_list_names(&runtime, port, "prompts/list", "prompts").await;
    assert_eq!(prompts_after, vec!["prompt.dynamic".to_owned()]);
    let tools_after = snapshot_list_names(&runtime, port, "tools/list", "tools").await;
    assert_eq!(tools_after, tools_before);
}

#[tokio::test]
async fn refresh_prompt_list_drops_stale_payload_after_reload() {
    let started = std::sync::Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));

    let registry = PluginRegistry::new()
        .register_prompt(BlockingPromptPlugin {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        })
        .expect("register prompt plugin");

    let mut initial = base_config();
    initial.prompts = Some(PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Plugin {
            plugin: BLOCKING_PROMPT_PLUGIN_NAME.to_owned(),
            config: None,
        }],
    });
    initial.plugins.push(PluginConfig {
        name: BLOCKING_PROMPT_PLUGIN_NAME.to_owned(),
        plugin_type: PluginType::Prompt,
        targets: None,
        config: None,
    });

    let runtime = runtime::build_runtime(initial, registry)
        .await
        .expect("runtime should build");

    let refresh_runtime = runtime.clone();
    let refresh_task =
        tokio::spawn(async move { refresh_runtime.refresh_list(ListFeature::Prompts).await });

    started.notified().await;

    let mut reloaded = base_config();
    reloaded.prompts = Some(inline_prompts("prompt.inline", "inline"));
    runtime
        .reload_config(reloaded)
        .await
        .expect("reload should succeed");

    release.notify_waiters();

    let changed = refresh_task
        .await
        .expect("refresh task should join")
        .expect("refresh should succeed");
    assert!(!changed, "stale prompt refresh write must be dropped");

    // Verify the cache now matches the reloaded inline prompt, not the stale
    // plugin prompt. A refresh returning false proves the cache is in sync.
    let post_reload_changed = runtime
        .refresh_list(ListFeature::Prompts)
        .await
        .expect("prompt cache should match reloaded state");
    assert!(
        !post_reload_changed,
        "prompt cache should be identical to reloaded inline config"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "blocking prompt plugin should not be queried after reload to inline prompt provider"
    );

    // Tools and resources should be unaffected by prompt-only reload.
    let tools_stable = runtime
        .refresh_list(ListFeature::Tools)
        .await
        .expect("tools refresh after prompt reload");
    assert!(
        !tools_stable,
        "tools should be unchanged after prompt-only reload"
    );
}

#[tokio::test]
async fn refresh_resource_lists_drops_stale_payload_after_reload() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));

    let registry = PluginRegistry::new()
        .register_resource(BlockingResourcePlugin {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
        })
        .expect("register resource plugin");

    let mut initial = base_config();
    initial.resources = Some(ResourcesConfig {
        enabled: None,
        notify_list_changed: false,
        clients_can_subscribe: false,
        pagination: None,
        providers: vec![ResourceProviderConfig::Plugin {
            plugin: BLOCKING_RESOURCE_PLUGIN_NAME.to_owned(),
            config: None,
            templates: None,
        }],
    });
    initial.plugins.push(PluginConfig {
        name: BLOCKING_RESOURCE_PLUGIN_NAME.to_owned(),
        plugin_type: PluginType::Resource,
        targets: None,
        config: None,
    });

    let runtime = runtime::build_runtime(initial, registry)
        .await
        .expect("runtime should build");

    let refresh_runtime = runtime.clone();
    let refresh_task =
        tokio::spawn(async move { refresh_runtime.refresh_list(ListFeature::Resources).await });

    started.notified().await;

    let mut reloaded = base_config();
    reloaded.resources = Some(inline_resources("resource://inline/two", "inline-two"));
    runtime
        .reload_config(reloaded)
        .await
        .expect("reload should succeed");

    release.notify_waiters();

    let changed = refresh_task
        .await
        .expect("refresh task should join")
        .expect("refresh should succeed");
    assert!(!changed, "stale resource refresh write must be dropped");

    // Verify the cache now matches the reloaded inline resource, not the stale
    // plugin resource. A refresh returning false proves the cache is in sync.
    let post_reload_changed = runtime
        .refresh_list(ListFeature::Resources)
        .await
        .expect("resource cache should match reloaded state");
    assert!(
        !post_reload_changed,
        "resource cache should be identical to reloaded inline config"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "blocking resource plugin should not be queried after reload to inline resource provider"
    );

    // Prompts and tools should be unaffected by resource-only reload.
    let prompts_stable = runtime
        .refresh_list(ListFeature::Prompts)
        .await
        .expect("prompts refresh after resource reload");
    assert!(
        !prompts_stable,
        "prompts should be unchanged after resource-only reload"
    );
}
