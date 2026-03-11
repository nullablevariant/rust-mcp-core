#![cfg(feature = "streamable_http")]

use async_trait::async_trait;
use rmcp::{
    model::{
        AnnotateAble, CallToolResult, CompleteRequestParams, CompletionInfo, GetPromptResult,
        Prompt, ReadResourceResult, ResourceContents,
    },
    ErrorData as McpError,
};
use rust_mcp_core::{
    plugins::http_router::{HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RuntimeContext},
    plugins::{
        AuthPlugin, AuthPluginDecision, AuthPluginValidateParams, CompletionPlugin,
        PluginCallParams, PluginLookup, PluginRef, PluginRegistry, PluginType, PromptEntry,
        PromptPlugin, RegistryEvent, ResourceEntry, ResourcePlugin, ToolPlugin,
    },
};
use serde_json::Value;
use tokio::time::{timeout, Duration};

use rust_mcp_core::plugins::http_router::AuthSummary;

struct EchoPlugin;

#[async_trait]
impl ToolPlugin for EchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(args))
    }
}

struct DuplicateToolPlugin;

#[async_trait]
impl ToolPlugin for DuplicateToolPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(Value::Null))
    }
}

struct DuplicateAuthPlugin;

#[async_trait]
impl AuthPlugin for DuplicateAuthPlugin {
    fn name(&self) -> &'static str {
        "plugin.echo"
    }

    async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        AuthPluginDecision::Accept
    }
}

struct CompletionEchoPlugin;
#[async_trait]
impl CompletionPlugin for CompletionEchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.completion"
    }

    async fn complete(
        &self,
        req: &CompleteRequestParams,
        _params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError> {
        CompletionInfo::with_all_values(vec![req.argument.value.clone()])
            .map_err(|err| McpError::internal_error(err, None))
    }
}

struct PromptEchoPlugin;
#[async_trait]
impl PromptPlugin for PromptEchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.prompt"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        Ok(vec![PromptEntry {
            prompt: Prompt::new("prompt.example", Some("example"), None),
            arguments_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }),
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
                rmcp::model::PromptMessageRole::Assistant,
                "ok",
            )])
            .with_description("example"),
        )
    }
}

struct ResourceEchoPlugin;
#[async_trait]
impl ResourcePlugin for ResourceEchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.resource"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        Ok(vec![ResourceEntry {
            resource: rmcp::model::RawResource::new("resource://echo/1", "echo").no_annotation(),
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

struct AuthNamedPlugin;
#[async_trait]
impl AuthPlugin for AuthNamedPlugin {
    fn name(&self) -> &'static str {
        "plugin.auth"
    }

    async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        AuthPluginDecision::Accept
    }
}

struct HttpRouterEchoPlugin;
impl HttpRouterPlugin for HttpRouterEchoPlugin {
    fn name(&self) -> &'static str {
        "plugin.router"
    }

    fn apply(
        &self,
        _ctx: &RuntimeContext,
        _targets: &[HttpRouterTarget],
        _config: &Value,
    ) -> Result<Vec<HttpRouterOp>, McpError> {
        Ok(Vec::new())
    }
}

#[test]
fn register_rejects_duplicate_within_type() {
    let err = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .unwrap()
        .register_tool(DuplicateToolPlugin)
        .expect_err("duplicate name within same type should fail");
    assert_eq!(err.message, "duplicate plugin name: plugin.echo");
}

#[test]
fn register_rejects_duplicate_across_types() {
    let err = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .unwrap()
        .register_auth(DuplicateAuthPlugin)
        .expect_err("duplicate name across types should fail");
    assert_eq!(err.message, "duplicate plugin name: plugin.echo");
}

#[test]
fn registers_completion_prompt_and_resource_plugins() {
    let registry = PluginRegistry::new()
        .register_completion(CompletionEchoPlugin)
        .unwrap()
        .register_prompt(PromptEchoPlugin)
        .unwrap()
        .register_resource(ResourceEchoPlugin)
        .unwrap();

    assert!(matches!(
        registry.get_plugin(PluginType::Completion, "plugin.completion"),
        Some(PluginRef::Completion(_))
    ));
    assert!(matches!(
        registry.get_plugin(PluginType::Prompt, "plugin.prompt"),
        Some(PluginRef::Prompt(_))
    ));
    assert!(matches!(
        registry.get_plugin(PluginType::Resource, "plugin.resource"),
        Some(PluginRef::Resource(_))
    ));
}

#[test]
fn plugin_lookup_names_includes_all_registered_types() {
    let registry = PluginRegistry::new()
        .register_completion(CompletionEchoPlugin)
        .unwrap()
        .register_prompt(PromptEchoPlugin)
        .unwrap()
        .register_resource(ResourceEchoPlugin)
        .unwrap();
    let mut names = registry.names();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(names.len(), 3);
    assert_eq!(
        names,
        vec![
            ("plugin.completion".to_owned(), PluginType::Completion),
            ("plugin.prompt".to_owned(), PluginType::Prompt),
            ("plugin.resource".to_owned(), PluginType::Resource),
        ]
    );
}

#[test]
fn plugin_lookup_supports_tool_auth_and_http_router() {
    let registry = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .unwrap()
        .register_auth(AuthNamedPlugin)
        .unwrap()
        .register_http_router(HttpRouterEchoPlugin)
        .unwrap();

    assert!(matches!(
        registry.get_plugin(PluginType::Tool, "plugin.echo"),
        Some(PluginRef::Tool(_))
    ));
    assert!(matches!(
        registry.get_plugin(PluginType::Auth, "plugin.auth"),
        Some(PluginRef::Auth(_))
    ));
    assert!(registry
        .get_plugin(PluginType::Auth, "plugin.echo")
        .is_none());
    assert!(matches!(
        registry.get_plugin(PluginType::HttpRouter, "plugin.router"),
        Some(PluginRef::HttpRouter(_))
    ));
}

#[test]
fn plugin_lookup_names_includes_tool_auth_http_router_types() {
    let registry = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .unwrap()
        .register_auth(AuthNamedPlugin)
        .unwrap()
        .register_http_router(HttpRouterEchoPlugin)
        .unwrap();

    let mut names = registry.names();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(names.len(), 3);
    assert_eq!(
        names,
        vec![
            ("plugin.auth".to_owned(), PluginType::Auth),
            ("plugin.echo".to_owned(), PluginType::Tool),
            ("plugin.router".to_owned(), PluginType::HttpRouter),
        ]
    );
}

#[tokio::test]
async fn registry_emits_tool_changed_for_register_replace_and_unregister() {
    let registry = PluginRegistry::new();
    let mut events = registry.subscribe_events();

    let registry = registry.register_tool(EchoPlugin).expect("register tool");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ToolChanged
    );

    let registry = registry
        .replace_tool(DuplicateToolPlugin)
        .expect("replace tool");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ToolChanged
    );

    let _registry = registry
        .unregister_tool("plugin.echo")
        .expect("unregister tool");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ToolChanged
    );

    // Assert no additional unexpected events
    assert!(
        timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err(),
        "no additional events should be emitted after the expected sequence"
    );
}

#[tokio::test]
async fn registry_emits_prompt_and_resource_events() {
    let registry = PluginRegistry::new();
    let mut events = registry.subscribe_events();

    let registry = registry
        .register_prompt(PromptEchoPlugin)
        .expect("register prompt");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::PromptChanged
    );

    let registry = registry
        .replace_prompt(PromptEchoPlugin)
        .expect("replace prompt");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::PromptChanged
    );

    let registry = registry
        .register_resource(ResourceEchoPlugin)
        .expect("register resource");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ResourceChanged
    );

    let registry = registry
        .replace_resource(ResourceEchoPlugin)
        .expect("replace resource");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ResourceChanged
    );

    let _registry = registry
        .unregister_resource("plugin.resource")
        .expect("unregister resource");
    assert_eq!(
        events.recv().await.expect("event"),
        RegistryEvent::ResourceChanged
    );

    // Assert no additional unexpected events
    assert!(
        timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err(),
        "no additional events should be emitted after the expected sequence"
    );
}

#[tokio::test]
async fn registry_does_not_emit_events_for_non_list_plugin_types() {
    let registry = PluginRegistry::new();
    let mut events = registry.subscribe_events();

    let _registry = registry
        .register_auth(AuthNamedPlugin)
        .expect("register auth");
    assert!(timeout(Duration::from_millis(50), events.recv())
        .await
        .is_err());
}

#[test]
fn replace_tool_rejects_name_owned_by_different_plugin_type() {
    let err = PluginRegistry::new()
        .register_auth(DuplicateAuthPlugin)
        .unwrap()
        .replace_tool(EchoPlugin)
        .expect_err("replace should fail when name belongs to different type");
    assert_eq!(
        err.message,
        "cannot replace tool plugin plugin.echo: name is registered as a different plugin type"
    );
}

#[test]
fn unregister_tool_frees_name_for_reuse_by_other_type() {
    let registry = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .expect("register tool")
        .unregister_tool("plugin.echo")
        .expect("unregister tool")
        .register_auth(DuplicateAuthPlugin)
        .expect("re-register under different type should succeed");
    // Verify the name is now registered as Auth, not Tool
    assert!(
        registry
            .get_plugin(PluginType::Tool, "plugin.echo")
            .is_none(),
        "tool lookup should be empty after unregister"
    );
    assert!(
        matches!(
            registry.get_plugin(PluginType::Auth, "plugin.echo"),
            Some(PluginRef::Auth(_))
        ),
        "auth lookup should return the re-registered plugin"
    );
    assert_eq!(registry.names().len(), 1);
}

#[test]
fn replace_tool_keeps_name_reserved_against_other_type() {
    let err = PluginRegistry::new()
        .register_tool(EchoPlugin)
        .expect("register tool")
        .replace_tool(DuplicateToolPlugin)
        .expect("replace tool")
        .register_auth(DuplicateAuthPlugin)
        .expect_err("name should still be reserved after replace");
    assert_eq!(err.message, "duplicate plugin name: plugin.echo");
}

#[test]
fn unregister_prompt_rejects_missing_plugin() {
    let err = PluginRegistry::new()
        .unregister_prompt("missing.prompt")
        .expect_err("unregister of non-existent plugin should fail");
    assert_eq!(err.message, "prompt plugin not registered: missing.prompt");
}

#[test]
fn http_router_plugin_apply_accepts_empty_targets() {
    let plugin = HttpRouterEchoPlugin;
    let ops = plugin
        .apply(
            &RuntimeContext::new(
                AuthSummary {
                    auth_enabled: false,
                    oauth_enabled: false,
                    resource_url: None,
                },
                None,
            ),
            &[],
            &Value::Null,
        )
        .expect("apply with empty targets should succeed");
    assert_eq!(ops.len(), 0, "empty targets should produce zero ops");
}
