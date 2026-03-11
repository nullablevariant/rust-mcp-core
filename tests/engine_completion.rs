#![cfg(all(feature = "completion", feature = "prompts", feature = "resources"))]
mod engine_common;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use rmcp::{
    model::{
        ArgumentInfo, CompleteRequestParams, CompletionContext, CompletionInfo, ErrorCode,
        Extensions, Meta, NumberOrString, Prompt, Reference,
    },
    service::{RequestContext, RoleServer, RunningService},
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    config::{CompletionProviderConfig, McpConfig, PromptProviderConfig},
    engine::{Engine, EngineConfig},
    plugins::{CompletionPlugin, PluginCallParams, PluginRegistry, PromptEntry, PromptPlugin},
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn load_fixture(name: &str) -> McpConfig {
    engine_common::load_config_fixture(name).config
}

fn request_context(service: &RunningService<RoleServer, Engine>) -> RequestContext<RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

fn cancelled_request_context(
    service: &RunningService<RoleServer, Engine>,
) -> RequestContext<RoleServer> {
    let context = request_context(service);
    context.ct.cancel();
    context
}

fn build_engine(config: McpConfig, plugins: PluginRegistry) -> Engine {
    Engine::from_config(EngineConfig {
        config,
        plugins,
        list_refresh_handle: None,
    })
    .expect("engine should build")
}

#[derive(Default)]
struct CompletionPluginState {
    calls: AtomicUsize,
    last_config: Mutex<Option<Value>>,
    last_context_args: Mutex<Option<HashMap<String, String>>>,
}

struct TestCompletionPlugin {
    state: Arc<CompletionPluginState>,
}

impl TestCompletionPlugin {
    const fn new(state: Arc<CompletionPluginState>) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl CompletionPlugin for TestCompletionPlugin {
    fn name(&self) -> &'static str {
        "completion.plugin"
    }

    async fn complete(
        &self,
        req: &CompleteRequestParams,
        params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_config.lock().expect("lock") = Some(params.config);
        *self.state.last_context_args.lock().expect("lock") =
            req.context.as_ref().and_then(|ctx| ctx.arguments.clone());

        CompletionInfo::with_all_values(vec![
            format!("{}-one", req.argument.value),
            format!("{}-two", req.argument.value),
        ])
        .map_err(|err| McpError::internal_error(err, None))
    }
}

struct BadCompletionSourcePromptPlugin;

#[async_trait::async_trait]
impl PromptPlugin for BadCompletionSourcePromptPlugin {
    fn name(&self) -> &'static str {
        "prompt.plugin"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        Ok(vec![PromptEntry {
            prompt: Prompt::new("prompt.dynamic", Option::<String>::None, None),
            arguments_schema: json!({
                "type": "object",
                "properties": {
                    "country": { "type": "string" }
                },
                "required": ["country"]
            }),
            completions: Some(HashMap::from([(
                String::from("country"),
                String::from("missing_source"),
            )])),
        }])
    }

    async fn get(
        &self,
        _name: &str,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<rmcp::model::GetPromptResult, McpError> {
        unreachable!("get should not be called")
    }
}

struct OversizedCompletionPlugin;

#[async_trait::async_trait]
impl CompletionPlugin for OversizedCompletionPlugin {
    fn name(&self) -> &'static str {
        "completion.plugin"
    }

    async fn complete(
        &self,
        _req: &CompleteRequestParams,
        _params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError> {
        Ok(CompletionInfo {
            values: (0..105).map(|idx| format!("value-{idx}")).collect(),
            total: Some(105),
            has_more: Some(true),
        })
    }
}

fn prompt_complete_request(name: &str, argument_name: &str, value: &str) -> CompleteRequestParams {
    CompleteRequestParams::new(
        Reference::for_prompt(name),
        ArgumentInfo {
            name: argument_name.to_owned(),
            value: value.to_owned(),
        },
    )
}

fn resource_complete_request(
    uri_template: &str,
    argument_name: &str,
    value: &str,
) -> CompleteRequestParams {
    CompleteRequestParams::new(
        Reference::for_resource(uri_template),
        ArgumentInfo {
            name: argument_name.to_owned(),
            value: value.to_owned(),
        },
    )
}

#[test]
fn get_info_exposes_completion_capability_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = Engine::new(fixture).expect("engine should build");
    let info = engine.get_info();
    let completions = info
        .capabilities
        .completions
        .as_ref()
        .expect("completions capability should be present");
    assert!(
        completions.is_empty(),
        "completions capability should be an empty JSON object, got: {completions:?}"
    );
}

#[test]
fn get_info_omits_completion_capability_when_disabled_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;
    config.completion = Some(rust_mcp_core::config::CompletionConfig {
        enabled: Some(false),
        providers: vec![],
    });
    let engine = Engine::new(config).expect("engine should build");
    let info = engine.get_info();
    assert!(
        info.capabilities.completions.is_none(),
        "completions capability should be absent when disabled"
    );
    // Verify other capabilities are still present (prompts/resources from fixture)
    assert!(
        info.capabilities.tools.is_some(),
        "tools capability should still be present when completion is disabled"
    );
}

#[tokio::test]
async fn complete_prompt_argument_from_inline_source_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "country", "C"),
        request_context(&service),
    )
    .await
    .expect("completion request should succeed");

    assert_eq!(result.completion.values, vec!["CA"]);
    assert_eq!(result.completion.total, Some(1));
    assert_eq!(result.completion.has_more, Some(false));
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_prompt_returns_empty_for_unmapped_argument_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "region", "E"),
        request_context(&service),
    )
    .await
    .expect("completion request should succeed");

    assert!(result.completion.values.is_empty());
    assert_eq!(result.completion.total, None);
    assert_eq!(result.completion.has_more, Some(false));
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_resource_template_argument_from_inline_source_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::complete(
        service.service(),
        resource_complete_request("resource://docs/{path}", "path", "guide/"),
        request_context(&service),
    )
    .await
    .expect("completion request should succeed");

    assert_eq!(
        result.completion.values,
        vec!["guide/start", "guide/install"]
    );
    assert_eq!(result.completion.total, Some(2));
    assert_eq!(result.completion.has_more, Some(false));
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_rejects_unknown_prompt_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.missing", "country", "U"),
        request_context(&service),
    )
    .await
    .expect_err("unknown prompt should fail");

    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("prompt 'prompt.missing' not found"),
        "expected message to name missing prompt, got: {}",
        error.message
    );
    assert!(
        error.data.is_some(),
        "expected data with available_prompts list"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_rejects_unknown_prompt_argument_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "missing", "U"),
        request_context(&service),
    )
    .await
    .expect_err("unknown argument should fail");

    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error
            .message
            .contains("prompt 'prompt.travel' argument 'missing'"),
        "expected message to name prompt and missing argument, got: {}",
        error.message
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_returns_cancelled_error_when_request_context_cancelled_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "country", "U"),
        cancelled_request_context(&service),
    )
    .await
    .expect_err("cancelled request should fail");

    assert_eq!(error.code, rust_mcp_core::CANCELLED_ERROR_CODE);
    assert_eq!(error.message, "request cancelled");
    assert!(error.data.is_none(), "cancelled error should have no data");
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_disabled_returns_method_not_found_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;
    config.completion = Some(rust_mcp_core::config::CompletionConfig {
        enabled: Some(false),
        providers: vec![],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "country", "U"),
        request_context(&service),
    )
    .await
    .expect_err("completion should be disabled");

    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert!(
        error.message.contains("completion/complete"),
        "expected method-not-found message to reference completion/complete, got: {}",
        error.message
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_inline_source_caps_results_at_100_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    let source = config
        .completion
        .as_mut()
        .expect("completion should exist")
        .providers
        .iter_mut()
        .find_map(|source| match source {
            CompletionProviderConfig::Inline { name, values } if name == "countries" => {
                Some(values)
            }
            _ => None,
        })
        .expect("countries source should exist");
    source.clear();
    source.extend((0..150).map(|idx| format!("c{idx:03}")));

    let engine = build_engine(config, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.travel", "country", "c"),
        request_context(&service),
    )
    .await
    .expect("completion request should succeed");

    assert_eq!(result.completion.values.len(), 100);
    assert_eq!(result.completion.total, Some(150));
    assert_eq!(result.completion.has_more, Some(true));
    // Assert boundary values: first, last included, and first excluded element
    assert_eq!(
        result.completion.values.first().map(String::as_str),
        Some("c000"),
        "first element should be c000"
    );
    assert_eq!(
        result.completion.values.last().map(String::as_str),
        Some("c099"),
        "last included element should be c099 (100th item, index 99)"
    );
    assert!(
        !result.completion.values.contains(&String::from("c100")),
        "c100 should be excluded (beyond the 100-item cap)"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_rejects_unknown_resource_template_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        resource_complete_request("resource://docs/{missing}", "path", "guide"),
        request_context(&service),
    )
    .await
    .expect_err("unknown resource template should fail");

    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error
            .message
            .contains("resource template 'resource://docs/{missing}' not found"),
        "expected message to name missing resource template, got: {}",
        error.message
    );
    assert!(
        error.data.is_some(),
        "expected data with available_templates list"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_rejects_unknown_resource_template_argument_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let engine = build_engine(fixture, PluginRegistry::new());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        resource_complete_request("resource://docs/{path}", "missing", "guide"),
        request_context(&service),
    )
    .await
    .expect_err("unknown argument should fail");

    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error
            .message
            .contains("resource template 'resource://docs/{path}' argument 'missing'"),
        "expected message to name resource template and missing argument, got: {}",
        error.message
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn complete_plugin_source_merges_plugin_and_source_config_fixture() {
    let fixture = load_fixture("completion/completion_plugin_fixture");
    let state = Arc::new(CompletionPluginState::default());
    let registry = PluginRegistry::new()
        .register_completion(TestCompletionPlugin::new(Arc::clone(&state)))
        .expect("completion plugin should register");

    let engine = build_engine(fixture, registry);
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let mut request = prompt_complete_request("prompt.plugin", "value", "al");
    request.context = Some(CompletionContext::with_arguments(HashMap::from([(
        String::from("hint"),
        String::from("from-context"),
    )])));

    let result = ServerHandler::complete(service.service(), request, request_context(&service))
        .await
        .expect("completion request should succeed");

    assert_eq!(
        result.completion.values,
        vec![String::from("al-one"), String::from("al-two")]
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);

    let config = state
        .last_config
        .lock()
        .expect("lock")
        .clone()
        .expect("config should be captured");
    assert_eq!(config.get("default_level"), Some(&json!(1)));
    assert_eq!(config.get("source_level"), Some(&json!(2)));
    assert_eq!(config.get("unchanged"), Some(&json!(true)));

    let context_args = state
        .last_context_args
        .lock()
        .expect("lock")
        .clone()
        .expect("context args should be captured");
    assert_eq!(
        context_args.get("hint").map(String::as_str),
        Some("from-context")
    );

    let _ = service.close().await;
}

#[tokio::test]
async fn complete_rejects_invalid_plugin_completion_response_fixture() {
    let fixture = load_fixture("completion/completion_plugin_fixture");
    let registry = PluginRegistry::new()
        .register_completion(OversizedCompletionPlugin)
        .expect("completion plugin should register");

    let engine = build_engine(fixture, registry);
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        prompt_complete_request("prompt.plugin", "value", "a"),
        request_context(&service),
    )
    .await
    .expect_err("invalid plugin completion response should fail");

    assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.contains("invalid completion response"),
        "expected error message to contain 'invalid completion response', got: {}",
        error.message
    );
    assert!(
        error.message.contains("values"),
        "expected error message to reference 'values' field constraint, got: {}",
        error.message
    );
    let _ = service.close().await;
}

#[test]
fn engine_rejects_duplicate_completion_source_names_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    config
        .completion
        .as_mut()
        .expect("completion should exist")
        .providers
        .push(CompletionProviderConfig::Inline {
            name: "countries".to_owned(),
            values: vec!["US".to_owned()],
        });

    let Err(error) = Engine::new(config) else {
        panic!("duplicate completion provider should fail");
    };
    assert!(
        error.message.contains("duplicate completion provider"),
        "expected 'duplicate completion provider', got: {}",
        error.message
    );
    assert!(
        error.message.contains("'countries'"),
        "expected error to name the duplicate source 'countries', got: {}",
        error.message
    );
}

#[test]
fn engine_rejects_unknown_completion_source_reference_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    let prompts = config.prompts.as_mut().expect("prompts should exist");
    let PromptProviderConfig::Inline { items } = prompts
        .providers
        .first_mut()
        .expect("provider should exist")
    else {
        panic!("expected inline provider");
    };
    let item = items.first_mut().expect("item should exist");
    item.completions = Some(HashMap::from([(
        String::from("country"),
        String::from("missing_source"),
    )]));

    let Err(error) = Engine::new(config) else {
        panic!("unknown provider should fail");
    };
    assert!(
        error
            .message
            .contains("references unknown completion provider"),
        "expected 'references unknown completion provider', got: {}",
        error.message
    );
    assert!(
        error.message.contains("'missing_source'"),
        "expected error to name the missing source 'missing_source', got: {}",
        error.message
    );
    assert!(
        error.message.contains("prompt"),
        "expected error to reference the prompt owner context, got: {}",
        error.message
    );
}

#[test]
fn engine_rejects_plugin_completion_source_not_allowlisted_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    config
        .completion
        .as_mut()
        .expect("completion should exist")
        .providers = vec![CompletionProviderConfig::Plugin {
        name: "plugin_values".to_owned(),
        plugin: "completion.plugin".to_owned(),
        config: None,
    }];
    let prompts = config.prompts.as_mut().expect("prompts should exist");
    let PromptProviderConfig::Inline { items } = prompts
        .providers
        .first_mut()
        .expect("provider should exist")
    else {
        panic!("expected inline provider");
    };
    items[0].completions = Some(HashMap::from([(
        String::from("country"),
        String::from("plugin_values"),
    )]));

    let Err(error) = Engine::new(config) else {
        panic!("missing allowlist should fail");
    };
    assert!(
        error.message.contains("completion plugin not allowlisted"),
        "expected 'completion plugin not allowlisted', got: {}",
        error.message
    );
    assert!(
        error.message.contains("completion.plugin"),
        "expected error to name the plugin 'completion.plugin', got: {}",
        error.message
    );
}

#[test]
fn engine_rejects_unknown_resource_completion_source_reference_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    let resources = config.resources.as_mut().expect("resources should exist");
    let templates = match resources
        .providers
        .first_mut()
        .expect("provider should exist")
    {
        rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } => {
            templates.as_mut().expect("templates should exist")
        }
        rust_mcp_core::config::ResourceProviderConfig::Plugin { .. } => {
            panic!("expected inline provider")
        }
    };
    templates[0].completions = Some(HashMap::from([(
        String::from("path"),
        String::from("missing_source"),
    )]));

    let Err(error) = Engine::new(config) else {
        panic!("unknown provider should fail");
    };
    assert!(
        error
            .message
            .contains("references unknown completion provider"),
        "expected 'references unknown completion provider', got: {}",
        error.message
    );
    assert!(
        error.message.contains("'missing_source'"),
        "expected error to name the missing source 'missing_source', got: {}",
        error.message
    );
    assert!(
        error.message.contains("resource template"),
        "expected error to reference the resource template owner context, got: {}",
        error.message
    );
}

#[test]
fn engine_rejects_allowlisted_completion_plugin_when_registry_missing_fixture() {
    let fixture = load_fixture("completion/completion_plugin_fixture");
    let Err(error) = Engine::new(fixture) else {
        panic!("registry missing should fail");
    };
    assert!(
        error.message.contains("completion plugin not registered"),
        "expected 'completion plugin not registered', got: {}",
        error.message
    );
    assert!(
        error.message.contains("completion.plugin"),
        "expected error to name the plugin 'completion.plugin', got: {}",
        error.message
    );
}

#[tokio::test]
async fn prompt_plugin_completion_sources_are_validated_at_runtime_fixture() {
    let fixture = load_fixture("completion/completion_inline_fixture");
    let mut config = fixture;

    config.prompts = Some(rust_mcp_core::config::PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Plugin {
            plugin: "prompt.plugin".to_owned(),
            config: None,
        }],
    });
    config.plugins.push(rust_mcp_core::config::PluginConfig {
        name: "prompt.plugin".to_owned(),
        plugin_type: rust_mcp_core::plugins::PluginType::Prompt,
        targets: None,
        config: None,
    });

    let registry = PluginRegistry::new()
        .register_prompt(BadCompletionSourcePromptPlugin)
        .expect("prompt plugin should register");
    let engine = build_engine(config, registry);
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_prompts(service.service(), None, request_context(&service))
        .await
        .expect_err("plugin completion provider validation should fail");

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(
        error
            .message
            .contains("references unknown completion provider"),
        "expected 'references unknown completion provider', got: {}",
        error.message
    );
    assert!(
        error.message.contains("'missing_source'"),
        "expected error to name the missing source 'missing_source', got: {}",
        error.message
    );
    assert!(
        error.message.contains("prompt"),
        "expected error to reference prompt context, got: {}",
        error.message
    );
    let _ = service.close().await;
}
