#![cfg(all(feature = "resources", feature = "completion"))]

mod engine_common;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use engine_common::fixture_path;
use rmcp::{
    model::{
        AnnotateAble, ErrorCode, Extensions, Meta, NumberOrString, PaginatedRequestParams,
        ReadResourceRequestParams, ResourceContents, SubscribeRequestParams,
        UnsubscribeRequestParams,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    engine::{Engine, EngineConfig},
    plugins::{PluginCallParams, PluginRegistry, ResourceEntry, ResourcePlugin},
    CANCELLED_ERROR_CODE,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct ResourceFixture {
    config: rust_mcp_core::config::McpConfig,
}

#[derive(Deserialize)]
struct ResourceReadFixture {
    config: rust_mcp_core::config::McpConfig,
    uri: String,
}

#[derive(Deserialize)]
struct ResourceReadFixturePayload {
    uri: String,
}

fn load_resource_fixture(name: &str) -> ResourceFixture {
    ResourceFixture {
        config: engine_common::load_config_fixture(name).config,
    }
}

fn load_resource_read_fixture(name: &str) -> ResourceReadFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    let parsed: ResourceReadFixturePayload =
        serde_yaml::from_str(&raw).expect("fixture should parse");
    let companion_name = format!("{name}_config");
    let config_fixture_name = if fixture_path(&companion_name).exists() {
        companion_name
    } else {
        name.to_owned()
    };
    ResourceReadFixture {
        config: engine_common::load_config_fixture(&config_fixture_name).config,
        uri: parsed.uri,
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

fn unsubscribe_request(uri: impl Into<String>) -> UnsubscribeRequestParams {
    let uri = uri.into();
    serde_json::from_value(json!({ "uri": uri })).expect("unsubscribe params")
}

fn assert_error_message_contains(error: &McpError, token: &str) {
    assert!(
        error.message.contains(token),
        "expected error message '{}' to contain '{}'",
        error.message,
        token
    );
}

fn assert_resource_not_found(error: &McpError, expected_uri: &str) {
    assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
    assert_eq!(error.message, "Resource not found");
    assert_eq!(error.data, Some(json!({ "uri": expected_uri })));
}

fn assert_invalid_cursor(error: &McpError) {
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "invalid cursor");
    assert_eq!(error.data, None);
}

fn assert_cancelled(error: &McpError) {
    assert_eq!(error.code, CANCELLED_ERROR_CODE);
    assert_eq!(error.message, "request cancelled");
    assert_eq!(error.data, None);
}

fn assert_invalid_template_args(error: &McpError, template_uri: &str) {
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_error_message_contains(error, "resource template arguments validation failed");
    assert_error_message_contains(error, template_uri);
    assert_error_message_contains(error, "name");
    assert_error_message_contains(error, "integer");
}

fn expect_engine_new_invalid_request(config: rust_mcp_core::config::McpConfig, token: &str) {
    let error = Engine::new(config).expect_err("engine construction should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_error_message_contains(&error, token);
}

#[derive(Default)]
struct ResourcePluginState {
    list_calls: AtomicUsize,
    read_calls: AtomicUsize,
    subscribe_calls: AtomicUsize,
    unsubscribe_calls: AtomicUsize,
    last_config: Mutex<Option<Value>>,
    last_uri: Mutex<Option<String>>,
}

struct TestResourcePlugin {
    state: Arc<ResourcePluginState>,
    list_entries: Vec<ResourceEntry>,
}

impl TestResourcePlugin {
    const fn new(state: Arc<ResourcePluginState>, list_entries: Vec<ResourceEntry>) -> Self {
        Self {
            state,
            list_entries,
        }
    }
}

#[async_trait::async_trait]
impl ResourcePlugin for TestResourcePlugin {
    fn name(&self) -> &'static str {
        "resource.plugin"
    }

    async fn list(&self, params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        let config = params.config;
        self.state.list_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_config.lock().expect("lock") = Some(config);
        Ok(self.list_entries.clone())
    }

    async fn read(
        &self,
        uri: &str,
        params: PluginCallParams,
    ) -> Result<rmcp::model::ReadResourceResult, McpError> {
        let config = params.config;
        self.state.read_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_config.lock().expect("lock") = Some(config);
        *self.state.last_uri.lock().expect("lock") = Some(uri.to_owned());
        Ok(rmcp::model::ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: uri.to_owned(),
                mime_type: Some("text/plain".to_owned()),
                text: "plugin".to_owned(),
                meta: None,
            },
        ]))
    }

    async fn subscribe(&self, uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        self.state.subscribe_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_uri.lock().expect("lock") = Some(uri.to_owned());
        Ok(())
    }

    async fn unsubscribe(&self, uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        self.state.unsubscribe_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_uri.lock().expect("lock") = Some(uri.to_owned());
        Ok(())
    }
}

#[test]
fn get_info_exposes_resources_capability_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let caps = engine.get_info().capabilities;
    let resources = caps.resources.expect("resources capability");
    assert_eq!(resources.list_changed, Some(true));
    assert_eq!(resources.subscribe, None);
}

#[tokio::test]
async fn list_resources_inline_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("resources list");

    assert_eq!(result.resources.len(), 2);
    assert_eq!(
        result
            .resources
            .iter()
            .map(|resource| (resource.uri.as_str(), resource.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("resource://docs/terms", "terms"),
            ("resource://docs/blob", "blob"),
        ]
    );
    assert!(result.next_cursor.is_none());
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_inline_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("templates list");

    assert_eq!(result.resource_templates.len(), 1);
    assert_eq!(
        result.resource_templates[0].uri_template,
        "resource://docs/{path}"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_inline_text_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new(fixture.uri),
        request_context(&service),
    )
    .await
    .expect("read resource");

    assert_eq!(result.contents.len(), 1);
    assert!(matches!(
        &result.contents[0],
        ResourceContents::TextResourceContents { text, .. } if text == "# Terms"
    ));
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_inline_blob_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/blob".to_owned()),
        request_context(&service),
    )
    .await
    .expect("read blob resource");

    assert!(matches!(
        &result.contents[0],
        ResourceContents::BlobResourceContents { blob, .. } if blob == "AA=="
    ));
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resources_pagination_fixture() {
    let fixture: ResourceFixture = load_resource_fixture("resources/resources_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let page1 = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("page 1");
    assert_eq!(page1.resources.len(), 1);
    assert_eq!(page1.resources[0].uri, "resource://page/one");
    assert_eq!(page1.resources[0].name, "one");
    assert!(page1.next_cursor.is_some());

    let page2 = ServerHandler::list_resources(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(page1.next_cursor)),
        request_context(&service),
    )
    .await
    .expect("page 2");
    assert_eq!(page2.resources.len(), 1);
    assert_eq!(page2.resources[0].uri, "resource://page/two");
    assert_eq!(page2.resources[0].name, "two");
    assert!(page2.next_cursor.is_none());
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resources_rejects_invalid_cursor_fixture() {
    let fixture: ResourceFixture = load_resource_fixture("resources/resources_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_resources(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("bad".to_owned()))),
        request_context(&service),
    )
    .await
    .expect_err("invalid cursor should fail");
    assert_invalid_cursor(&error);
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resources_uses_global_pagination_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_global_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let page1 = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("page 1");
    assert_eq!(page1.resources.len(), 1);
    assert_eq!(page1.resources[0].uri, "resource://global/one");
    assert_eq!(page1.resources[0].name, "one");
    assert!(page1.next_cursor.is_some());
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_uses_global_pagination_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_global_pagination_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    *templates = Some(vec![
        rust_mcp_core::config::ResourceTemplateConfig {
            uri_template: "resource://global/{id}".to_owned(),
            name: "global-template-1".to_owned(),
            title: None,
            description: None,
            mime_type: None,
            icons: None,
            annotations: None,
            arguments_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
            completions: None,
        },
        rust_mcp_core::config::ResourceTemplateConfig {
            uri_template: "resource://global/{name}".to_owned(),
            name: "global-template-2".to_owned(),
            title: None,
            description: None,
            mime_type: None,
            icons: None,
            annotations: None,
            arguments_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            completions: None,
        },
    ]);

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let page1 =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("page 1");
    assert_eq!(page1.resource_templates.len(), 1);
    assert_eq!(
        page1.resource_templates[0].uri_template,
        "resource://global/{id}"
    );
    assert_eq!(page1.resource_templates[0].name, "global-template-1");
    assert!(page1.next_cursor.is_some());
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resources_pagination_disabled_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_pagination_disabled_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("resources list");
    assert_eq!(result.resources.len(), 2);
    assert!(result.next_cursor.is_none());
    let _ = service.close().await;
}

#[tokio::test]
async fn plugin_resources_read_template_and_merge_config_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(
        Arc::clone(&state),
        vec![ResourceEntry {
            resource: rmcp::model::RawResource::new("resource://plugin/listed", "listed")
                .no_annotation(),
        }],
    );
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("resource list");
    assert_eq!(list.resources.len(), 1);

    let read = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new(fixture.uri),
        request_context(&service),
    )
    .await
    .expect("read resource");

    assert!(matches!(
        &read.contents[0],
        ResourceContents::TextResourceContents { text, .. } if text == "plugin"
    ));
    // Resource providers are now catalog-backed; list + read share one resolved snapshot.
    assert_eq!(state.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.read_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *state.last_uri.lock().expect("lock"),
        Some("resource://plugin/readme.md".to_owned())
    );

    let last_config = state
        .last_config
        .lock()
        .expect("lock")
        .clone()
        .expect("config");
    assert_eq!(last_config.get("region"), Some(&json!("us")));
    assert_eq!(last_config.get("timeout"), Some(&json!(2)));
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_plugin_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(Arc::clone(&state), Vec::new());
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let result =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("templates list");
    assert_eq!(result.resource_templates.len(), 1);
    assert_eq!(
        result.resource_templates[0].uri_template,
        "resource://plugin/{path}"
    );
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_plugin_listed_uri_uses_direct_entry_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(
        Arc::clone(&state),
        vec![ResourceEntry {
            resource: rmcp::model::RawResource::new("resource://plugin/listed", "listed")
                .no_annotation(),
        }],
    );
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let read = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://plugin/listed".to_owned()),
        request_context(&service),
    )
    .await
    .expect("read direct resource");

    assert!(matches!(
        &read.contents[0],
        ResourceContents::TextResourceContents { uri, text, .. }
            if uri == "resource://plugin/listed" && text == "plugin"
    ));
    assert_eq!(state.read_calls.load(Ordering::SeqCst), 1);
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_and_unsubscribe_direct_plugin_resource_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(
        Arc::clone(&state),
        vec![ResourceEntry {
            resource: rmcp::model::RawResource::new("resource://plugin/listed", "listed")
                .no_annotation(),
        }],
    );
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://plugin/listed".to_owned()),
        request_context(&service),
    )
    .await
    .expect("subscribe");

    ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://plugin/listed".to_owned()),
        request_context(&service),
    )
    .await
    .expect("unsubscribe");

    assert_eq!(state.subscribe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.unsubscribe_calls.load(Ordering::SeqCst), 1);
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_and_unsubscribe_route_to_plugin_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(Arc::clone(&state), Vec::new());
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://plugin/a.md".to_owned()),
        request_context(&service),
    )
    .await
    .expect("subscribe");

    ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://plugin/a.md".to_owned()),
        request_context(&service),
    )
    .await
    .expect("unsubscribe");

    assert_eq!(state.subscribe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.unsubscribe_calls.load(Ordering::SeqCst), 1);
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_disabled_returns_method_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("subscribe should fail");

    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(error.message, "resources/subscribe");
    assert_eq!(error.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn unsubscribe_disabled_returns_method_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("unsubscribe should fail");

    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(error.message, "resources/unsubscribe");
    assert_eq!(error.data, None);
    let _ = service.close().await;
}

#[tokio::test]
async fn read_inline_template_only_returns_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new(fixture.uri),
        request_context(&service),
    )
    .await
    .expect_err("read should fail");

    assert_resource_not_found(&error, "resource://inline/test");
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_and_unsubscribe_inline_template_are_noop_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect("subscribe");

    ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect("unsubscribe");

    let read_error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("inline template read remains unresolved");
    assert_resource_not_found(&read_error, "resource://inline/test");
    let _ = service.close().await;
}

#[tokio::test]
async fn cancelled_resource_requests_return_cancelled_error_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list_error =
        ServerHandler::list_resources(service.service(), None, cancelled_request_context(&service))
            .await
            .expect_err("list should be cancelled");
    assert_cancelled(&list_error);

    let templates_error = ServerHandler::list_resource_templates(
        service.service(),
        None,
        cancelled_request_context(&service),
    )
    .await
    .expect_err("templates should be cancelled");
    assert_cancelled(&templates_error);

    let read_error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/terms".to_owned()),
        cancelled_request_context(&service),
    )
    .await
    .expect_err("read should be cancelled");
    assert_cancelled(&read_error);

    let subscribe_error = ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://docs/terms".to_owned()),
        cancelled_request_context(&service),
    )
    .await
    .expect_err("subscribe should be cancelled");
    assert_cancelled(&subscribe_error);

    let unsubscribe_error = ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://docs/terms".to_owned()),
        cancelled_request_context(&service),
    )
    .await
    .expect_err("unsubscribe should be cancelled");
    assert_cancelled(&unsubscribe_error);
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_without_inline_content_returns_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    let item = items
        .as_mut()
        .expect("items")
        .first_mut()
        .expect("first item");
    item.content = None;

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("read should fail");
    assert_resource_not_found(&error, "resource://docs/terms");
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_and_unsubscribe_inline_resource_are_noop_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect("subscribe");
    ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect("unsubscribe");

    let read = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect("read inline resource should remain available");
    assert_eq!(read.contents.len(), 1);
    assert!(matches!(
        &read.contents[0],
        ResourceContents::TextResourceContents { text, .. } if text == "# Terms"
    ));
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_unknown_uri_returns_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://unknown/nope".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("subscribe should fail");
    assert_resource_not_found(&error, "resource://unknown/nope");
    let _ = service.close().await;
}

#[tokio::test]
async fn unsubscribe_unknown_uri_returns_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://unknown/nope".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("unsubscribe should fail");
    assert_resource_not_found(&error, "resource://unknown/nope");
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_unknown_uri_returns_not_found_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://not/found".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("read should fail");
    assert_resource_not_found(&error, "resource://not/found");
    let _ = service.close().await;
}

#[tokio::test]
async fn read_template_args_validation_error_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.arguments_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "integer" }
        },
        "required": ["name"]
    });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("read should fail");
    assert_invalid_template_args(&error, "resource://inline/{name}");
    let _ = service.close().await;
}

#[tokio::test]
async fn subscribe_template_args_validation_error_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.arguments_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "integer" }
        },
        "required": ["name"]
    });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("subscribe should fail");
    assert_invalid_template_args(&error, "resource://inline/{name}");
    let _ = service.close().await;
}

#[tokio::test]
async fn unsubscribe_template_args_validation_error_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let mut config = fixture.config;
    config
        .resources
        .as_mut()
        .expect("resources")
        .clients_can_subscribe = true;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.arguments_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "integer" }
        },
        "required": ["name"]
    });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::unsubscribe(
        service.service(),
        unsubscribe_request("resource://inline/test".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("unsubscribe should fail");
    assert_invalid_template_args(&error, "resource://inline/{name}");
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_supports_pagination_fixture() {
    let fixture: ResourceFixture = load_resource_fixture("resources/resources_pagination_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    templates
        .as_mut()
        .expect("templates")
        .push(rust_mcp_core::config::ResourceTemplateConfig {
            uri_template: "resource://page/{other}".to_owned(),
            name: "page-template-two".to_owned(),
            title: None,
            description: None,
            mime_type: None,
            icons: None,
            annotations: None,
            arguments_schema: json!({
                "type": "object",
                "properties": {
                    "other": { "type": "string" }
                },
                "required": ["other"]
            }),
            completions: None,
        });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let page1 =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("page 1");
    assert_eq!(page1.resource_templates.len(), 1);
    assert_eq!(
        page1.resource_templates[0].uri_template,
        "resource://page/{id}"
    );
    assert_eq!(page1.resource_templates[0].name, "page-template");
    assert!(page1.next_cursor.is_some());

    let page2 = ServerHandler::list_resource_templates(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(page1.next_cursor)),
        request_context(&service),
    )
    .await
    .expect("page 2");
    assert_eq!(page2.resource_templates.len(), 1);
    assert_eq!(
        page2.resource_templates[0].uri_template,
        "resource://page/{other}"
    );
    assert_eq!(page2.resource_templates[0].name, "page-template-two");
    assert!(page2.next_cursor.is_none());
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_rejects_invalid_cursor_fixture() {
    let fixture: ResourceFixture = load_resource_fixture("resources/resources_pagination_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::list_resource_templates(
        service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("bad".to_owned()))),
        request_context(&service),
    )
    .await
    .expect_err("invalid cursor should fail");
    assert_invalid_cursor(&error);
    let _ = service.close().await;
}

#[test]
fn rejects_inline_provider_without_items_or_templates_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, templates } = provider
    else {
        panic!("expected inline provider");
    };
    *items = None;
    *templates = None;

    expect_engine_new_invalid_request(
        config,
        "resource inline provider requires items and/or templates",
    );
}

#[test]
fn rejects_resource_content_without_text_or_blob_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    let item = items
        .as_mut()
        .expect("items")
        .first_mut()
        .expect("first item");
    item.content = Some(rust_mcp_core::config::ResourceContentConfig {
        text: None,
        blob: None,
    });

    expect_engine_new_invalid_request(config, "content must include text or blob");
}

#[test]
fn rejects_template_with_empty_placeholder_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.uri_template = "resource://docs/{}".to_owned();

    expect_engine_new_invalid_request(config, "invalid placeholder");
}

#[test]
fn rejects_template_with_unbalanced_brace_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.uri_template = "resource://docs/path}".to_owned();

    expect_engine_new_invalid_request(config, "unbalanced braces");
}

#[test]
fn rejects_template_with_invalid_url_after_placeholder_rewrite_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.uri_template = "://docs/{path}".to_owned();

    expect_engine_new_invalid_request(config, "invalid resource uri_template");
}

#[test]
fn rejects_resource_annotation_priority_out_of_range_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    let item = items.as_mut().expect("items").first_mut().expect("item");
    item.annotations.as_mut().expect("annotations").priority = Some(1.2);

    expect_engine_new_invalid_request(
        config,
        "resource annotations.priority must be between 0.0 and 1.0",
    );
}

#[test]
fn rejects_resource_annotation_last_modified_format_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    let item = items.as_mut().expect("items").first_mut().expect("item");
    item.annotations
        .as_mut()
        .expect("annotations")
        .last_modified = Some("not-a-date".to_owned());

    expect_engine_new_invalid_request(config, "invalid resource annotations.last_modified");
}

#[tokio::test]
async fn empty_template_name_uses_default_name_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template")
        .name = String::new();

    let engine = Engine::new(config).expect("engine");
    let templates = engine
        .list_resource_templates_for_refresh()
        .await
        .expect("templates");
    assert_eq!(templates[0].name, "resource-template");
}

#[test]
fn get_info_exposes_resource_subscribe_when_enabled_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let state = Arc::new(ResourcePluginState::default());
    let engine = Engine::from_config(EngineConfig {
        config: fixture.config,
        plugins: PluginRegistry::new()
            .register_resource(TestResourcePlugin::new(Arc::clone(&state), Vec::new()))
            .expect("register"),
        list_refresh_handle: None,
    })
    .expect("engine");
    let resources = engine
        .get_info()
        .capabilities
        .resources
        .expect("resources capability");
    assert_eq!(resources.list_changed, Some(true));
    assert_eq!(resources.subscribe, Some(true));
}

#[test]
fn rejects_resource_plugin_not_allowlisted_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_plugin_not_allowlisted_fixture");
    expect_engine_new_invalid_request(fixture.config, "resource plugin not allowlisted");
}

#[test]
fn rejects_resource_plugin_not_registered_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_plugin_not_registered_fixture");
    expect_engine_new_invalid_request(fixture.config, "resource plugin not registered");
}

#[test]
fn rejects_invalid_resource_uri_fixture() {
    let fixture: ResourceFixture = load_resource_fixture("resources/resources_invalid_uri_fixture");
    expect_engine_new_invalid_request(fixture.config, "invalid resource uri");
}

#[test]
fn rejects_invalid_template_uri_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_invalid_template_uri_fixture");
    expect_engine_new_invalid_request(fixture.config, "invalid resource uri_template");
}

#[test]
fn rejects_invalid_template_schema_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_invalid_template_schema_fixture");
    expect_engine_new_invalid_request(
        fixture.config,
        "invalid arguments_schema for resource template 'resource://docs/{path}'",
    );
}

#[test]
fn rejects_invalid_template_completion_key_fixture() {
    let fixture: ResourceFixture =
        load_resource_fixture("resources/resources_invalid_template_completion_fixture");
    expect_engine_new_invalid_request(
        fixture.config,
        "completions key 'unknown' is not defined in arguments_schema.properties",
    );
}

#[tokio::test]
async fn list_resources_prefers_last_duplicate_uri_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    items
        .as_mut()
        .expect("items")
        .push(rust_mcp_core::config::ResourceItemConfig {
            uri: "resource://docs/terms".to_owned(),
            name: "terms-v2".to_owned(),
            title: None,
            description: None,
            mime_type: Some("text/plain".to_owned()),
            size: None,
            icons: None,
            annotations: None,
            content: Some(rust_mcp_core::config::ResourceContentConfig {
                text: Some("second".to_owned()),
                blob: None,
            }),
        });
    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let read = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect("read");
    assert!(matches!(
        &read.contents[0],
        ResourceContents::TextResourceContents { text, .. } if text == "second"
    ));
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resource_templates_warns_on_duplicates_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_template_only_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { templates, .. } = provider else {
        panic!("expected inline provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.arguments_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "integer" }
        },
        "required": ["name"]
    });
    templates
        .as_mut()
        .expect("templates")
        .push(rust_mcp_core::config::ResourceTemplateConfig {
            uri_template: "resource://inline/{name}".to_owned(),
            name: "inline-template-two".to_owned(),
            title: None,
            description: None,
            mime_type: None,
            icons: None,
            annotations: None,
            arguments_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }),
            completions: None,
        });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let list =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("list");
    assert_eq!(list.resource_templates.len(), 2);
    assert_eq!(
        list.resource_templates[1].uri_template,
        "resource://inline/{name}"
    );
    assert_eq!(list.resource_templates[1].name, "inline-template-two");

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://inline/value".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("inline template read should fail");
    assert_resource_not_found(&error, "resource://inline/value");
    let _ = service.close().await;
}

#[tokio::test]
async fn read_resource_template_with_literal_suffix_matching_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_plugin_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Plugin { templates, .. } = provider else {
        panic!("expected plugin provider");
    };
    let template = templates
        .as_mut()
        .expect("templates")
        .first_mut()
        .expect("template");
    template.uri_template = "resource://plugin/{path}/raw".to_owned();

    let state = Arc::new(ResourcePluginState::default());
    let plugin = TestResourcePlugin::new(Arc::clone(&state), Vec::new());
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::new()
            .register_resource(plugin)
            .expect("register plugin"),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let read = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://plugin/readme/raw".to_owned()),
        request_context(&service),
    )
    .await
    .expect("matching literal suffix read should pass");
    assert_eq!(read.contents.len(), 1);
    assert!(matches!(
        &read.contents[0],
        ResourceContents::TextResourceContents { uri, text, .. }
            if uri == "resource://plugin/readme/raw" && text == "plugin"
    ));
    assert_eq!(state.read_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *state.last_uri.lock().expect("lock"),
        Some("resource://plugin/readme/raw".to_owned())
    );

    let error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://plugin/readme/raw/extra".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("uri with extra suffix should not match");
    assert_resource_not_found(&error, "resource://plugin/readme/raw/extra");
    let _ = service.close().await;
}

#[tokio::test]
async fn list_resources_maps_assistant_audience_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    let resources = config.resources.as_mut().expect("resources");
    let provider = resources.providers.first_mut().expect("provider");
    let rust_mcp_core::config::ResourceProviderConfig::Inline { items, .. } = provider else {
        panic!("expected inline provider");
    };
    let item = items.as_mut().expect("items").first_mut().expect("item");
    item.annotations = Some(rust_mcp_core::config::ResourceAnnotationsConfig {
        audience: Some(vec![
            rust_mcp_core::config::ResourceAudienceConfig::Assistant,
        ]),
        priority: Some(0.5),
        last_modified: Some("2026-02-24T00:00:00Z".to_owned()),
    });

    let engine = Engine::new(config).expect("engine should build");
    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);
    let list = ServerHandler::list_resources(service.service(), None, request_context(&service))
        .await
        .expect("list");
    assert_eq!(list.resources.len(), 2);
    let terms = list
        .resources
        .iter()
        .find(|resource| resource.uri == "resource://docs/terms")
        .expect("terms resource should exist");
    let annotations = terms
        .annotations
        .as_ref()
        .expect("annotations should exist");
    assert_eq!(
        annotations.audience,
        Some(vec![rmcp::model::Role::Assistant])
    );
    assert_eq!(annotations.priority, Some(0.5));
    let last_modified = annotations
        .last_modified
        .expect("last_modified timestamp should exist");
    assert_eq!(last_modified.to_rfc3339(), "2026-02-24T00:00:00+00:00");
    let _ = service.close().await;
}

#[tokio::test]
async fn resources_disabled_returns_empty_lists_fixture() {
    let fixture: ResourceReadFixture =
        load_resource_read_fixture("resources/resources_inline_fixture");
    let mut config = fixture.config;
    config.resources = None;
    config.tools = None;
    let engine = Engine::new(config).expect("engine should build");
    let caps = engine.get_info().capabilities;
    assert!(caps.resources.is_none());

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut service = rmcp::service::serve_directly(engine, server_io, None);

    let resources =
        ServerHandler::list_resources(service.service(), None, request_context(&service))
            .await
            .expect("list resources");
    assert!(resources.resources.is_empty());

    let templates =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect("list templates");
    assert!(templates.resource_templates.is_empty());

    let read_error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("read should fail without resources");
    assert_resource_not_found(&read_error, "resource://docs/terms");

    let subscribe_error = ServerHandler::subscribe(
        service.service(),
        SubscribeRequestParams::new("resource://docs/terms".to_owned()),
        request_context(&service),
    )
    .await
    .expect_err("subscribe should be unavailable when resources are disabled");
    assert_eq!(subscribe_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(subscribe_error.message, "resources/subscribe");
    assert_eq!(subscribe_error.data, None);
    let _ = service.close().await;
}
