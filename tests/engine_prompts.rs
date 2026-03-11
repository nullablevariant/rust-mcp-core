#![cfg(feature = "prompts")]

mod engine_common;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use engine_common::{fixture_path, load_config_fixture as load_engine_config_fixture};
use rmcp::{
    model::{
        Extensions, GetPromptRequestParams, Meta, NumberOrString, PaginatedRequestParams, Prompt,
        PromptMessage, PromptMessageContent, PromptMessageRole,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::config::McpConfig;
use rust_mcp_core::{
    config::{PromptMessageRoleConfig, PromptProviderConfig, PromptTemplateMessageConfig},
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, PromptEntry, PromptPlugin},
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct PromptConfigFixture {
    config: rust_mcp_core::config::McpConfig,
}

#[derive(Deserialize)]
struct PromptGetFixture {
    config: rust_mcp_core::config::McpConfig,
    name: String,
    args: Map<String, Value>,
}

#[derive(Deserialize)]
struct PromptGetFixturePayload {
    name: String,
    args: Map<String, Value>,
}

fn load_config_fixture(name: &str) -> PromptConfigFixture {
    let companion_name = format!("{name}_config");
    let config_fixture_name = if fixture_path(&companion_name).exists() {
        companion_name
    } else {
        name.to_owned()
    };
    PromptConfigFixture {
        config: load_engine_config_fixture(&config_fixture_name).config,
    }
}

fn load_get_fixture(name: &str) -> PromptGetFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    let parsed: PromptGetFixturePayload = serde_yaml::from_str(&raw).expect("fixture should parse");
    let config_fixture_name = format!("{name}_config");
    PromptGetFixture {
        config: load_engine_config_fixture(&config_fixture_name).config,
        name: parsed.name,
        args: parsed.args,
    }
}

fn request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
) -> RequestContext<rmcp::service::RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

fn cancelled_request_context(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
) -> RequestContext<rmcp::service::RoleServer> {
    let context = request_context(service);
    context.ct.cancel();
    context
}

fn first_inline_provider_item_mut(
    config: &mut McpConfig,
) -> &mut rust_mcp_core::config::PromptItemConfig {
    let prompts = config.prompts.as_mut().expect("prompts config");
    let provider = prompts.providers.first_mut().expect("first provider");
    let PromptProviderConfig::Inline { items } = provider else {
        panic!("expected inline provider");
    };
    items.first_mut().expect("first prompt item")
}

fn set_single_message_content(config: &mut McpConfig, content: Value) {
    let item = first_inline_provider_item_mut(config);
    item.template.messages = vec![PromptTemplateMessageConfig {
        role: PromptMessageRoleConfig::Assistant,
        content,
    }];
}

#[derive(Default)]
struct PromptPluginState {
    list_calls: AtomicUsize,
    get_calls: AtomicUsize,
    last_config: Mutex<Option<Value>>,
    last_args: Mutex<Option<Value>>,
}

struct TestPromptPlugin {
    state: Arc<PromptPluginState>,
}

impl TestPromptPlugin {
    const fn new(state: Arc<PromptPluginState>) -> Self {
        Self { state }
    }
}

struct BadCompletionPromptPlugin;

#[async_trait::async_trait]
impl PromptPlugin for BadCompletionPromptPlugin {
    fn name(&self) -> &'static str {
        "prompt.plugin"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        Ok(vec![PromptEntry {
            prompt: Prompt::new("prompt.bad", Option::<String>::None, None),
            arguments_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                }
            }),
            completions: Some(HashMap::from([(
                String::from("unknown"),
                String::from("provider"),
            )])),
        }])
    }

    async fn get(
        &self,
        _name: &str,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<rmcp::model::GetPromptResult, McpError> {
        unreachable!("get should not be called in this test")
    }
}

#[async_trait::async_trait]
impl PromptPlugin for TestPromptPlugin {
    fn name(&self) -> &'static str {
        "prompt.plugin"
    }

    async fn list(&self, params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        self.state.list_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_config.lock().expect("lock") = Some(params.config);

        let mut prompt = Prompt::new("prompt.duplicate", Some("from-plugin"), None);
        prompt.title = Some("Plugin Prompt".to_owned());
        Ok(vec![PromptEntry {
            prompt,
            arguments_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Input text" }
                },
                "required": ["text"]
            }),
            completions: Some(HashMap::from([(
                String::from("text"),
                String::from("prompt_values"),
            )])),
        }])
    }

    async fn get(
        &self,
        _name: &str,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<rmcp::model::GetPromptResult, McpError> {
        self.state.get_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_args.lock().expect("lock") = Some(args.clone());

        Ok(
            rmcp::model::GetPromptResult::new(vec![PromptMessage::new_text(
                PromptMessageRole::Assistant,
                format!("plugin:{args}"),
            )])
            .with_description("from-plugin"),
        )
    }
}

#[test]
fn get_info_exposes_prompt_capability_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let caps = engine.get_info().capabilities;
    let prompts_cap = caps.prompts.expect("prompts capability must be present");
    assert_eq!(
        prompts_cap.list_changed,
        Some(true),
        "list_changed must be Some(true) when notify_list_changed=true"
    );
}

#[test]
#[cfg(feature = "client_logging")]
#[allow(
    clippy::too_many_lines,
    reason = "multi-combo capability test requires many assertions"
)]
fn get_info_covers_prompt_and_logging_capability_combinations_fixture() {
    let base: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");

    let mut logging_only = base.config.clone();
    logging_only.prompts = None;
    logging_only.client_logging = Some(rust_mcp_core::config::ClientLoggingConfig::default());
    let logging_only_caps = Engine::new(logging_only)
        .expect("engine should build")
        .get_info()
        .capabilities;
    assert!(
        logging_only_caps.logging.is_some(),
        "logging capability must be present when logging config is set"
    );
    assert!(
        logging_only_caps.prompts.is_none(),
        "prompts capability must be absent when prompts config is None"
    );

    let mut prompts_only = base.config.clone();
    prompts_only.client_logging = None;
    prompts_only
        .prompts
        .as_mut()
        .expect("prompts config")
        .notify_list_changed = false;
    let prompts_only_caps = Engine::new(prompts_only)
        .expect("engine should build")
        .get_info()
        .capabilities;
    assert!(
        prompts_only_caps.logging.is_none(),
        "logging capability must be absent when logging config is None"
    );
    let prompts_only_prompts_cap = prompts_only_caps
        .prompts
        .expect("prompts capability must be present when prompts config is set");
    assert_eq!(
        prompts_only_prompts_cap.list_changed, None,
        "list_changed must be None when notify_list_changed=false"
    );

    let mut prompts_and_logging = base.config.clone();
    prompts_and_logging.client_logging =
        Some(rust_mcp_core::config::ClientLoggingConfig::default());
    prompts_and_logging
        .prompts
        .as_mut()
        .expect("prompts config")
        .notify_list_changed = false;
    let prompts_and_logging_caps = Engine::new(prompts_and_logging)
        .expect("engine should build")
        .get_info()
        .capabilities;
    assert!(
        prompts_and_logging_caps.logging.is_some(),
        "logging capability must be present when logging config is set"
    );
    let both_prompts_cap = prompts_and_logging_caps
        .prompts
        .expect("prompts capability must be present when prompts config is set");
    assert_eq!(
        both_prompts_cap.list_changed, None,
        "list_changed must be None when notify_list_changed=false"
    );

    let mut prompts_logging_and_list_changed = base.config;
    prompts_logging_and_list_changed.client_logging =
        Some(rust_mcp_core::config::ClientLoggingConfig::default());
    prompts_logging_and_list_changed
        .prompts
        .as_mut()
        .expect("prompts config")
        .notify_list_changed = true;
    let prompts_logging_and_list_changed_caps = Engine::new(prompts_logging_and_list_changed)
        .expect("engine should build")
        .get_info()
        .capabilities;
    assert!(
        prompts_logging_and_list_changed_caps.logging.is_some(),
        "logging capability must be present when logging config is set"
    );
    let all_prompts_cap = prompts_logging_and_list_changed_caps
        .prompts
        .expect("prompts capability must be present when prompts config is set");
    assert_eq!(
        all_prompts_cap.list_changed,
        Some(true),
        "list_changed must be Some(true) when notify_list_changed=true"
    );
}

#[tokio::test]
async fn list_and_get_inline_prompt_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect("list prompts");
    assert_eq!(list.prompts.len(), 1);
    let prompt = &list.prompts[0];
    assert_eq!(prompt.name, "prompt.inline");
    let arguments = prompt
        .arguments
        .as_ref()
        .expect("arguments should be derived");
    assert!(arguments
        .iter()
        .any(|arg| arg.name == "text" && arg.required == Some(true)));

    let get_result = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect("get prompt");

    assert_eq!(get_result.messages.len(), 6);
    assert!(matches!(
        get_result.messages[0].content,
        PromptMessageContent::Text { .. }
    ));
    assert!(matches!(
        get_result.messages[1].content,
        PromptMessageContent::Text { .. }
    ));
    assert!(matches!(
        get_result.messages[2].content,
        PromptMessageContent::Image { .. }
    ));
    assert!(matches!(
        get_result.messages[3].content,
        PromptMessageContent::Resource { .. }
    ));
    assert!(matches!(
        get_result.messages[4].content,
        PromptMessageContent::Resource { .. }
    ));
    assert!(matches!(
        get_result.messages[5].content,
        PromptMessageContent::ResourceLink { .. }
    ));

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_rejects_invalid_args_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(Map::new());
            request
        },
        request_context(&service),
    )
    .await
    .expect_err("missing required args should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
    assert!(
        error
            .message
            .contains("prompt arguments validation failed for 'prompt.inline'"),
        "error must mention prompt name and validation failure, got '{}'",
        error.message
    );
    assert!(
        error.message.contains("text"),
        "error must mention the missing required field 'text', got '{}'",
        error.message
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_returns_empty_when_prompts_disabled_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    fixture.config.prompts = None;
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect("list prompts should succeed");
    assert!(list.prompts.is_empty());
    assert!(list.next_cursor.is_none());

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_honors_prompt_pagination_fixture() {
    let fixture: PromptConfigFixture = load_config_fixture("prompts/prompts_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let first_page = ServerHandler::list_prompts(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(None)),
        request_context(&service),
    )
    .await
    .expect("first page");
    assert_eq!(first_page.prompts.len(), 1);
    assert_eq!(
        first_page.prompts[0].name, "prompt.alpha",
        "first page must contain prompt.alpha"
    );
    assert!(first_page.next_cursor.is_some());

    let second_page = ServerHandler::list_prompts(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(first_page.next_cursor)),
        request_context(&service),
    )
    .await
    .expect("second page");
    assert_eq!(second_page.prompts.len(), 1);
    assert_eq!(
        second_page.prompts[0].name, "prompt.bravo",
        "second page must contain prompt.bravo"
    );
    assert!(second_page.next_cursor.is_none());

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_page_size_zero_disables_prompt_pagination_fixture() {
    let mut fixture: PromptConfigFixture =
        load_config_fixture("prompts/prompts_pagination_fixture");
    fixture
        .config
        .prompts
        .as_mut()
        .expect("prompts config")
        .pagination
        .as_mut()
        .expect("prompt pagination")
        .page_size = 0;
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("not-used".to_owned()))),
        request_context(&service),
    )
    .await
    .expect("pagination disabled should ignore cursor");

    assert_eq!(list.prompts.len(), 2);
    assert_eq!(
        list.prompts[0].name, "prompt.alpha",
        "first prompt must be prompt.alpha"
    );
    assert_eq!(
        list.prompts[1].name, "prompt.bravo",
        "second prompt must be prompt.bravo"
    );
    assert!(list.next_cursor.is_none());

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_rejects_invalid_cursor_fixture() {
    let fixture: PromptConfigFixture = load_config_fixture("prompts/prompts_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_prompts(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("invalid-cursor".to_owned()))),
        request_context(&service),
    )
    .await
    .expect_err("invalid cursor should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
    assert_eq!(
        error.message, "invalid cursor",
        "error message must be the exact cursor validation message"
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_uses_global_pagination_fixture() {
    let fixture: PromptConfigFixture =
        load_config_fixture("prompts/prompts_global_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(None)),
        request_context(&service),
    )
    .await
    .expect("list prompts");

    assert_eq!(list.prompts.len(), 1);
    assert_eq!(
        list.prompts[0].name, "prompt.alpha",
        "first page must contain prompt.alpha when using global pagination"
    );
    assert!(list.next_cursor.is_some());

    let _ = service.close().await;
}

#[tokio::test]
async fn plugin_prompt_provider_merges_config_and_last_wins_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_plugin_fixture");
    let state = Arc::new(PromptPluginState::default());

    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_prompt(TestPromptPlugin::new(Arc::clone(&state)))
            .expect("register prompt plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect("list prompts");
    assert_eq!(list.prompts.len(), 2);
    assert_eq!(list.prompts[0].name, "prompt.duplicate");
    assert_eq!(list.prompts[1].name, "prompt.duplicate");

    let get_result = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect("get prompt");

    assert_eq!(get_result.description.as_deref(), Some("from-plugin"));
    // Prompt providers are now catalog-backed; list + get share one resolved snapshot.
    assert_eq!(state.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.get_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *state.last_config.lock().expect("lock"),
        Some(serde_json::json!({"region": "us", "timeout": 2}))
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn plugin_prompt_invalid_args_do_not_call_get_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_plugin_fixture");
    let state = Arc::new(PromptPluginState::default());

    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_prompt(TestPromptPlugin::new(Arc::clone(&state)))
            .expect("register prompt plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(Map::new());
            request
        },
        request_context(&service),
    )
    .await
    .expect_err("invalid args");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
    assert!(
        error
            .message
            .contains("prompt arguments validation failed for 'prompt.duplicate'"),
        "error must mention prompt name and validation failure, got '{}'",
        error.message
    );
    assert!(
        error.message.contains("text"),
        "error must mention the missing required field 'text', got '{}'",
        error.message
    );
    assert_eq!(state.get_calls.load(Ordering::SeqCst), 0);

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_rejects_missing_prompt_name_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new("prompt.missing".to_owned());
            request.arguments = Some(Map::new());
            request
        },
        request_context(&service),
    )
    .await
    .expect_err("missing prompt should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
    assert!(error.message.contains("prompt 'prompt.missing' not found"));

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_rejects_non_object_arguments_schema_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let item = first_inline_provider_item_mut(&mut fixture.config);
    item.arguments_schema = Value::String("not-an-object".to_owned());

    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect_err("invalid arguments_schema should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32600));
    assert_eq!(
        error.message, "prompt arguments_schema must be an object",
        "error message must exactly match schema type validation"
    );

    let _ = service.close().await;
}

#[tokio::test]
#[cfg(feature = "completion")]
async fn list_prompts_rejects_completion_keys_not_in_schema_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let item = first_inline_provider_item_mut(&mut fixture.config);
    item.completions = Some(HashMap::from([(
        String::from("missing_key"),
        String::from("prompt_values"),
    )]));

    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect_err("invalid completion key should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32600));
    assert_eq!(
        error.message,
        "prompt 'prompt.inline' completions key 'missing_key' is not defined in arguments_schema.properties",
        "error must include entity name and offending key"
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_rejects_invalid_inline_content_shapes_fixture() {
    let invalid_cases = vec![
        (
            json!({"text": "missing type"}),
            "prompt message content object requires type",
        ),
        (json!({"type": "text"}), "text content requires text"),
        (
            json!({"type": "image", "mime_type": "image/png"}),
            "image content requires data",
        ),
        (
            json!({"type": "image", "data": "YWJj"}),
            "image content requires mime_type",
        ),
        (json!({"type": "resource"}), "resource content requires uri"),
        (
            json!({"type": "resource", "uri": "resource://a"}),
            "resource content requires text or blob",
        ),
        (
            json!({"type": "resource_link"}),
            "resource_link content requires uri",
        ),
        (
            json!({"type": "not_supported"}),
            "unsupported prompt message content type: not_supported",
        ),
        (
            Value::Number(serde_json::Number::from(42)),
            "prompt message content must be a string or object",
        ),
    ];

    for (content, expected_message) in invalid_cases {
        let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
        set_single_message_content(&mut fixture.config, content);

        let engine = Engine::new(fixture.config).expect("engine should build");
        let (server_io, _client_io) = tokio::io::duplex(2048);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let error = ServerHandler::get_prompt(
            service.service(),
            {
                let mut request = GetPromptRequestParams::new(fixture.name);
                request.arguments = Some(fixture.args);
                request
            },
            request_context(&service),
        )
        .await
        .expect_err("invalid prompt content should fail");
        assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
        assert_eq!(
            error.message, expected_message,
            "error message must exactly match expected validation message"
        );

        let _ = service.close().await;
    }
}

#[tokio::test]
async fn get_prompt_rejects_invalid_inline_content_metadata_fixture() {
    let invalid_cases = vec![
        (
            json!({"type":"image","data":"YWJj","mime_type":"image/png","_meta":"bad"}),
            "invalid image _meta: invalid type: string \"bad\", expected a map",
        ),
        (
            json!({"type":"image","data":"YWJj","mime_type":"image/png","annotations":"bad"}),
            "invalid image annotations: invalid type: string \"bad\", expected struct Annotations",
        ),
        (
            json!({"type":"resource","uri":"resource://memo","text":"ok","content_meta":"bad"}),
            "invalid resource content_meta: invalid type: string \"bad\", expected a map",
        ),
        (
            json!({"type":"resource","uri":"resource://memo","text":"ok","_meta":"bad"}),
            "invalid resource _meta: invalid type: string \"bad\", expected a map",
        ),
        (
            json!({"type":"resource","uri":"resource://memo","text":"ok","annotations":"bad"}),
            "invalid resource annotations: invalid type: string \"bad\", expected struct Annotations",
        ),
        (
            json!({"type":"resource_link","uri":"file:///a","_meta":"bad"}),
            "invalid resource_link _meta: invalid type: string \"bad\", expected a map",
        ),
        (
            json!({"type":"resource_link","uri":"file:///a","icons":"bad"}),
            "invalid resource_link icons: invalid type: string \"bad\", expected a sequence",
        ),
        (
            json!({"type":"resource_link","uri":"file:///a","annotations":"bad"}),
            "invalid resource_link annotations: invalid type: string \"bad\", expected struct Annotations",
        ),
    ];

    for (content, expected_message) in invalid_cases {
        let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
        set_single_message_content(&mut fixture.config, content);

        let engine = Engine::new(fixture.config).expect("engine should build");
        let (server_io, _client_io) = tokio::io::duplex(2048);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let error = ServerHandler::get_prompt(
            service.service(),
            {
                let mut request = GetPromptRequestParams::new(fixture.name);
                request.arguments = Some(fixture.args);
                request
            },
            request_context(&service),
        )
        .await
        .expect_err("invalid prompt content metadata should fail");
        assert_eq!(error.code, rmcp::model::ErrorCode(-32602));
        assert_eq!(
            error.message, expected_message,
            "error message must exactly match expected validation message"
        );

        let _ = service.close().await;
    }
}

#[tokio::test]
async fn list_and_get_prompts_return_cancelled_error_when_context_cancelled_fixture() {
    let fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list_error =
        ServerHandler::list_prompts(service.service(), None, cancelled_request_context(&service))
            .await
            .expect_err("cancelled list should error");
    assert_eq!(list_error.code, rmcp::model::ErrorCode(-32000));
    assert_eq!(
        list_error.message, "request cancelled",
        "list cancellation must use exact cancellation message"
    );
    assert!(
        list_error.data.is_none(),
        "cancelled list error must have no data payload"
    );

    let get_error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        cancelled_request_context(&service),
    )
    .await
    .expect_err("cancelled get should error");
    assert_eq!(get_error.code, rmcp::model::ErrorCode(-32000));
    assert_eq!(
        get_error.message, "request cancelled",
        "get cancellation must use exact cancellation message"
    );
    assert!(
        get_error.data.is_none(),
        "cancelled get error must have no data payload"
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_handles_empty_arguments_schema_properties_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let item = first_inline_provider_item_mut(&mut fixture.config);
    item.arguments_schema = json!({"type": "object"});

    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect("list prompts");
    assert_eq!(list.prompts.len(), 1);
    assert!(list.prompts[0].arguments.is_none());

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_rejects_invalid_arguments_schema_definition_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let item = first_inline_provider_item_mut(&mut fixture.config);
    item.arguments_schema = json!({
        "type": "object",
        "properties": {
            "text": {
                "type": 1
            }
        }
    });

    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect_err("invalid schema definition should fail");
    assert!(error
        .message
        .contains("invalid arguments_schema for prompt 'prompt.inline'"));

    let _ = service.close().await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "typed content test requires many field-level assertions"
)]
async fn get_prompt_supports_typed_content_optional_fields_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    let item = first_inline_provider_item_mut(&mut fixture.config);
    item.template.messages = vec![
        PromptTemplateMessageConfig {
            role: PromptMessageRoleConfig::Assistant,
            content: json!({
                "type": "image",
                "data": "YWJj",
                "mimeType": "image/png",
                "_meta": { "source": "unit" },
                "annotations": { "audience": ["assistant"] }
            }),
        },
        PromptTemplateMessageConfig {
            role: PromptMessageRoleConfig::Assistant,
            content: json!({
                "type": "resource",
                "uri": "resource://memo",
                "mimeType": "text/plain",
                "text": "hello",
                "content_meta": { "etag": "123" },
                "_meta": { "owner": "tests" },
                "annotations": { "priority": 0.1 }
            }),
        },
        PromptTemplateMessageConfig {
            role: PromptMessageRoleConfig::Assistant,
            content: json!({
                "type": "resource_link",
                "uri": "file:///notes.txt",
                "name": "notes",
                "title": "Notes",
                "description": "test link",
                "mimeType": "text/plain",
                "size": 123,
                "_meta": { "k": "v" },
                "icons": [
                    {
                        "src": "https://example.com/icon.png",
                        "mimeType": "image/png"
                    }
                ],
                "annotations": { "audience": ["user"] }
            }),
        },
    ];

    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect("typed content should succeed");
    assert_eq!(result.messages.len(), 3);

    // Image: verify data, mimeType, _meta, annotations (image is #[serde(flatten)])
    let image_json =
        serde_json::to_value(&result.messages[0].content).expect("image content serializes");
    assert_eq!(image_json["type"], "image");
    assert_eq!(image_json["data"], "YWJj");
    assert_eq!(image_json["mimeType"], "image/png");

    // Resource: verify uri, mimeType, text, annotations
    // PromptMessageContent::Resource { resource: EmbeddedResource } is NOT flattened,
    // and RawEmbeddedResource.resource (ResourceContents) is also not flattened.
    let resource_json =
        serde_json::to_value(&result.messages[1].content).expect("resource content serializes");
    assert_eq!(resource_json["type"], "resource");
    assert_eq!(
        resource_json["resource"]["resource"]["uri"],
        "resource://memo"
    );
    assert_eq!(
        resource_json["resource"]["resource"]["mimeType"],
        "text/plain"
    );
    assert_eq!(resource_json["resource"]["resource"]["text"], "hello");

    // ResourceLink: verify uri, name, title, description, mimeType, size, icons, annotations
    // (link is #[serde(flatten)] so fields are at top level)
    let link_json = serde_json::to_value(&result.messages[2].content)
        .expect("resource_link content serializes");
    assert_eq!(link_json["type"], "resource_link");
    assert_eq!(link_json["uri"], "file:///notes.txt");
    assert_eq!(link_json["name"], "notes");
    assert_eq!(link_json["title"], "Notes");
    assert_eq!(link_json["description"], "test link");
    assert_eq!(link_json["mimeType"], "text/plain");
    assert_eq!(link_json["size"], 123);

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_resource_link_uses_default_name_when_missing_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    set_single_message_content(
        &mut fixture.config,
        json!({
            "type": "resource_link",
            "uri": "file:///only-uri"
        }),
    );
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect("resource_link with uri only should be valid");

    assert_eq!(result.messages.len(), 1);
    let link_json = serde_json::to_value(&result.messages[0].content)
        .expect("resource_link content serializes");
    assert_eq!(link_json["type"], "resource_link");
    assert_eq!(link_json["uri"], "file:///only-uri");
    assert_eq!(
        link_json["name"], "resource",
        "default name must be 'resource' when name is not provided"
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn get_prompt_rejects_resource_link_size_overflow_fixture() {
    let mut fixture: PromptGetFixture = load_get_fixture("prompts/prompts_inline_fixture");
    set_single_message_content(
        &mut fixture.config,
        json!({
            "type": "resource_link",
            "uri": "file:///big",
            "size": 4_294_967_296_u64
        }),
    );
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::get_prompt(
        service.service(),
        {
            let mut request = GetPromptRequestParams::new(fixture.name);
            request.arguments = Some(fixture.args);
            request
        },
        request_context(&service),
    )
    .await
    .expect_err("resource_link size overflow should fail");

    assert!(
        error
            .message
            .contains("resource_link size must be <= 4294967295"),
        "expected overflow message, got '{}'",
        error.message
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn list_prompts_rejects_plugin_completion_keys_not_in_schema_fixture() {
    let fixture: PromptConfigFixture = load_config_fixture("prompts/prompts_plugin_fixture");
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_prompt(BadCompletionPromptPlugin)
            .expect("register prompt plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(2048);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect_err("invalid plugin completion key should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32600));
    assert_eq!(
        error.message,
        "prompt 'prompt.bad' completions key 'unknown' is not defined in arguments_schema.properties",
        "error must include prompt name and offending key"
    );

    let _ = service.close().await;
}

#[test]
fn rejects_unallowlisted_prompt_provider_fixture() {
    let fixture: PromptConfigFixture =
        load_config_fixture("prompts/prompts_plugin_unallowlisted_fixture");
    let state = Arc::new(PromptPluginState::default());
    let error = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_prompt(TestPromptPlugin::new(state))
            .expect("register prompt plugin"),
        list_refresh_handle: None,
    })
    .expect_err("unallowlisted plugin should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32600));
    assert!(
        error.message.contains("prompt plugin not allowlisted"),
        "error must mention allowlist failure, got '{}'",
        error.message
    );
    assert!(
        error.message.contains("prompt.plugin"),
        "error must mention the plugin name, got '{}'",
        error.message
    );
}

#[test]
fn rejects_allowlisted_prompt_plugin_without_registry_fixture() {
    let fixture: PromptConfigFixture =
        load_config_fixture("prompts/prompts_plugin_not_registered_fixture");
    let error = Engine::new(fixture.config).expect_err("missing registry plugin should fail");
    assert_eq!(error.code, rmcp::model::ErrorCode(-32600));
    assert!(
        error.message.contains("prompt plugin not registered"),
        "error must mention registration failure, got '{}'",
        error.message
    );
    assert!(
        error.message.contains("prompt.plugin"),
        "error must mention the plugin name, got '{}'",
        error.message
    );
}
