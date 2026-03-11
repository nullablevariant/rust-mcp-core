//! Cached list responses for tools, prompts, and resources with refresh support.
use serde_json::Value;
use tokio::sync::Mutex as TokioMutex;

use crate::config::McpConfig;
use crate::plugins::ListFeature;
use crate::{Engine, McpError};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ListCache {
    pub(super) tools: Vec<u8>,
    pub(super) prompts: Vec<u8>,
    pub(super) resources: Vec<u8>,
    pub(super) resource_templates: Vec<u8>,
}

pub(super) struct RefreshLocks {
    pub(super) tools: TokioMutex<()>,
    pub(super) prompts: TokioMutex<()>,
    pub(super) resources: TokioMutex<()>,
}

impl Default for RefreshLocks {
    fn default() -> Self {
        Self {
            tools: TokioMutex::new(()),
            prompts: TokioMutex::new(()),
            resources: TokioMutex::new(()),
        }
    }
}

pub(super) async fn build_list_cache(engine: &Engine) -> Result<ListCache, McpError> {
    Ok(ListCache {
        tools: serialize_payload(&tools_list_payload(engine)?)?,
        prompts: serialize_payload(&prompt_list_payload(engine).await?)?,
        resources: serialize_payload(&resource_list_payload(engine).await?)?,
        resource_templates: serialize_payload(&resource_templates_list_payload(engine).await?)?,
    })
}

pub(super) fn serialize_payload(payload: &Value) -> Result<Vec<u8>, McpError> {
    serde_json::to_vec(payload).map_err(|error| {
        McpError::internal_error(format!("failed to serialize list payload: {error}"), None)
    })
}

pub(super) fn tools_list_payload(engine: &Engine) -> Result<Value, McpError> {
    serde_json::to_value(engine.list_tools()).map_err(|error| {
        McpError::internal_error(format!("failed to serialize tools list: {error}"), None)
    })
}

pub(super) async fn prompt_list_payload(engine: &Engine) -> Result<Value, McpError> {
    #[cfg(feature = "prompts")]
    {
        serde_json::to_value(engine.list_prompts_for_refresh().await?).map_err(|error| {
            McpError::internal_error(format!("failed to serialize prompts list: {error}"), None)
        })
    }
    #[cfg(not(feature = "prompts"))]
    {
        let _ = engine;
        Ok(serde_json::Value::Array(Vec::new()))
    }
}

pub(super) async fn resource_list_payload(engine: &Engine) -> Result<Value, McpError> {
    #[cfg(feature = "resources")]
    {
        serde_json::to_value(engine.list_resources_for_refresh().await?).map_err(|error| {
            McpError::internal_error(format!("failed to serialize resources list: {error}"), None)
        })
    }
    #[cfg(not(feature = "resources"))]
    {
        let _ = engine;
        Ok(serde_json::Value::Array(Vec::new()))
    }
}

pub(super) async fn resource_templates_list_payload(engine: &Engine) -> Result<Value, McpError> {
    #[cfg(feature = "resources")]
    {
        serde_json::to_value(engine.list_resource_templates_for_refresh().await?).map_err(|error| {
            McpError::internal_error(
                format!("failed to serialize resource templates list: {error}"),
                None,
            )
        })
    }
    #[cfg(not(feature = "resources"))]
    {
        let _ = engine;
        Ok(serde_json::Value::Array(Vec::new()))
    }
}

pub(super) fn list_changed_enabled(config: &McpConfig, feature: ListFeature) -> bool {
    match feature {
        ListFeature::Prompts => config
            .prompts
            .as_ref()
            .is_some_and(|prompts| prompts.notify_list_changed),
        ListFeature::Resources => config
            .resources
            .as_ref()
            .is_some_and(|resources| resources.notify_list_changed),
        ListFeature::Tools => config.tools_notify_list_changed(),
    }
}

pub(super) const fn list_feature_label(feature: ListFeature) -> &'static str {
    match feature {
        ListFeature::Tools => "tools/list_changed",
        ListFeature::Prompts => "prompts/list_changed",
        ListFeature::Resources => "resources/list_changed",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "prompts")]
    use super::prompt_list_payload;
    #[cfg(feature = "http_tools")]
    use super::{build_list_cache, tools_list_payload};
    use super::{list_changed_enabled, list_feature_label, serialize_payload};
    #[cfg(feature = "resources")]
    use super::{resource_list_payload, resource_templates_list_payload};
    use crate::config::McpConfig;
    #[cfg(feature = "prompts")]
    use crate::config::{
        PromptItemConfig, PromptMessageRoleConfig, PromptProviderConfig, PromptTemplateConfig,
        PromptTemplateMessageConfig, PromptsConfig,
    };
    #[cfg(feature = "resources")]
    use crate::config::{ResourceProviderConfig, ResourceTemplateConfig, ResourcesConfig};
    #[cfg(any(feature = "http_tools", feature = "prompts", feature = "resources"))]
    use crate::engine::EngineConfig;
    use crate::inline_test_fixtures::stdio_base_config;
    #[cfg(feature = "http_tools")]
    use crate::inline_test_fixtures::stdio_http_tool_config;
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    use crate::inline_test_fixtures::{read_frame, request_context};
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    use crate::mcp::{ClientCapabilities, Implementation, InitializeRequestParams};
    use crate::plugins::ListFeature;
    #[cfg(any(feature = "http_tools", feature = "prompts", feature = "resources"))]
    use crate::plugins::PluginRegistry;
    #[cfg(any(feature = "http_tools", feature = "prompts", feature = "resources"))]
    use crate::Engine;
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    use rmcp::ServerHandler;
    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    use serde::Deserialize;
    use serde_json::{json, Value};

    fn minimal_config() -> McpConfig {
        stdio_base_config()
    }

    #[cfg(feature = "http_tools")]
    fn config_with_http_tool() -> McpConfig {
        stdio_http_tool_config()
    }

    #[cfg(any(feature = "http_tools", feature = "prompts", feature = "resources"))]
    fn build_engine(config: McpConfig) -> Engine {
        Engine::from_config(EngineConfig {
            config,
            plugins: PluginRegistry::new(),
            list_refresh_handle: None,
        })
        .expect("engine should build")
    }

    #[cfg(feature = "prompts")]
    fn config_with_inline_prompt() -> McpConfig {
        let mut config = minimal_config();
        config.prompts = Some(PromptsConfig {
            enabled: None,
            notify_list_changed: true,
            pagination: None,
            providers: vec![PromptProviderConfig::Inline {
                items: vec![PromptItemConfig {
                    name: "prompt.inline".to_owned(),
                    title: None,
                    description: Some("inline prompt".to_owned()),
                    icons: None,
                    arguments_schema: json!({"type":"object"}),
                    template: PromptTemplateConfig {
                        messages: vec![PromptTemplateMessageConfig {
                            role: PromptMessageRoleConfig::User,
                            content: Value::String("hello".to_owned()),
                        }],
                    },
                    completions: None,
                }],
            }],
        });
        config
    }

    #[cfg(feature = "resources")]
    fn config_with_inline_resource() -> McpConfig {
        let mut config = minimal_config();
        config.resources = Some(ResourcesConfig {
            enabled: None,
            notify_list_changed: true,
            clients_can_subscribe: false,
            pagination: None,
            providers: vec![ResourceProviderConfig::Inline {
                items: None,
                templates: Some(vec![ResourceTemplateConfig {
                    uri_template: "resource://inline/{name}".to_owned(),
                    name: "resource.inline".to_owned(),
                    title: None,
                    description: Some("inline resource template".to_owned()),
                    mime_type: Some("text/plain".to_owned()),
                    icons: None,
                    annotations: None,
                    arguments_schema: json!({"type":"object"}),
                    completions: None,
                }]),
            }],
        });
        config
    }

    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    fn parse_jsonrpc_notification(frame: &str) -> Value {
        let json_start = frame.find('{').expect("frame must contain JSON object");
        let json_str = &frame[json_start..];
        let mut deserializer = serde_json::Deserializer::from_str(json_str);
        Value::deserialize(&mut deserializer).expect("frame must contain valid JSON")
    }

    #[test]
    fn serialize_payload_produces_deterministic_bytes() {
        let payload_a = json!({"b": 2, "a": 1});
        let payload_b = json!({"a": 1, "b": 2});
        let payload_c = json!({"a": 1, "b": 3});

        let bytes_a = serialize_payload(&payload_a).expect("serialize payload a");
        let bytes_b = serialize_payload(&payload_b).expect("serialize payload b");
        let bytes_c = serialize_payload(&payload_c).expect("serialize payload c");

        assert_eq!(
            bytes_a, bytes_b,
            "equivalent payloads must serialize identically"
        );
        assert_ne!(
            bytes_a, bytes_c,
            "different payloads must produce different serialized bytes"
        );

        let roundtrip: Value = serde_json::from_slice(&bytes_a).expect("deserialize");
        assert_eq!(roundtrip, payload_b);
    }

    #[test]
    fn list_changed_enabled_tools_true_when_configured() {
        let mut config = minimal_config();
        config.set_tools_notify_list_changed(true);
        assert!(list_changed_enabled(&config, ListFeature::Tools));
    }

    #[test]
    fn list_changed_enabled_tools_false_by_default() {
        let config = minimal_config();
        assert!(!list_changed_enabled(&config, ListFeature::Tools));
    }

    #[test]
    fn list_changed_enabled_prompts_false_when_no_prompts() {
        let config = minimal_config();
        assert!(!list_changed_enabled(&config, ListFeature::Prompts));
    }

    #[test]
    #[cfg(feature = "prompts")]
    fn list_changed_enabled_prompts_true_when_configured() {
        let config = config_with_inline_prompt();
        assert!(list_changed_enabled(&config, ListFeature::Prompts));
    }

    #[test]
    fn list_changed_enabled_resources_false_when_no_resources() {
        let config = minimal_config();
        assert!(!list_changed_enabled(&config, ListFeature::Resources));
    }

    #[test]
    #[cfg(feature = "resources")]
    fn list_changed_enabled_resources_true_when_configured() {
        let config = config_with_inline_resource();
        assert!(list_changed_enabled(&config, ListFeature::Resources));
    }

    #[test]
    fn list_feature_label_returns_correct_strings() {
        assert_eq!(list_feature_label(ListFeature::Tools), "tools/list_changed");
        assert_eq!(
            list_feature_label(ListFeature::Prompts),
            "prompts/list_changed"
        );
        assert_eq!(
            list_feature_label(ListFeature::Resources),
            "resources/list_changed"
        );
    }

    #[cfg(feature = "http_tools")]
    #[tokio::test]
    async fn build_list_cache_produces_non_empty_tools() {
        let engine = build_engine(config_with_http_tool());
        let cache = build_list_cache(&engine).await.expect("build_list_cache");
        assert!(!cache.tools.is_empty());
        let expected = serialize_payload(&tools_list_payload(&engine).expect("tools payload"))
            .expect("serialize expected payload");
        assert_eq!(cache.tools, expected);
    }

    #[cfg(feature = "http_tools")]
    #[test]
    fn tools_list_payload_returns_serializable_json() {
        let engine = build_engine(config_with_http_tool());
        let payload = tools_list_payload(&engine).expect("tools_list_payload");
        assert!(payload.is_array());
        let tools = payload.as_array().expect("tools payload should be array");
        assert!(
            tools
                .iter()
                .any(|entry| entry.get("name").and_then(Value::as_str) == Some("noop")),
            "tools payload should include the configured noop tool"
        );
    }

    #[cfg(feature = "prompts")]
    #[tokio::test]
    async fn prompt_list_payload_returns_expected_inline_item() {
        let engine = build_engine(config_with_inline_prompt());
        let payload = prompt_list_payload(&engine)
            .await
            .expect("prompt_list_payload");
        let prompts = payload.as_array().expect("prompt payload should be array");
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0].get("name").and_then(Value::as_str),
            Some("prompt.inline")
        );
    }

    #[cfg(feature = "resources")]
    #[tokio::test]
    async fn resource_list_payload_returns_empty_when_no_inline_items() {
        let engine = build_engine(config_with_inline_resource());
        let payload = resource_list_payload(&engine)
            .await
            .expect("resource_list_payload");
        let resources = payload
            .as_array()
            .expect("resource payload should be array");
        assert!(
            resources.is_empty(),
            "resource payload should be empty without inline items"
        );
    }

    #[cfg(feature = "resources")]
    #[tokio::test]
    async fn resource_templates_list_payload_returns_expected_inline_template() {
        let engine = build_engine(config_with_inline_resource());
        let payload = resource_templates_list_payload(&engine)
            .await
            .expect("resource_templates_list_payload");
        let templates = payload
            .as_array()
            .expect("resource templates payload should be array");
        assert_eq!(templates.len(), 1);
        assert_eq!(
            templates[0].get("name").and_then(Value::as_str),
            Some("resource.inline")
        );
        assert_eq!(
            templates[0].get("uriTemplate").and_then(Value::as_str),
            Some("resource://inline/{name}")
        );
    }

    #[cfg(feature = "http_tools")]
    #[tokio::test]
    async fn notify_list_changed_methods_return_zero_without_peers() {
        let engine = build_engine(minimal_config());
        assert_eq!(engine.notify_tools_list_changed().await, 0);
        assert_eq!(engine.notify_prompts_list_changed().await, 0);
        assert_eq!(engine.notify_resources_list_changed().await, 0);
    }

    #[cfg(all(feature = "streamable_http", feature = "http_tools"))]
    #[tokio::test]
    async fn notify_list_changed_methods_return_positive_with_observed_peer() {
        let engine = build_engine(minimal_config());
        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly(engine.clone(), server_io, None);

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

        let sent = engine.notify_tools_list_changed().await;
        assert_eq!(sent, 1, "exactly one observed peer should be notified");

        let frame = read_frame(&mut client_io)
            .await
            .expect("tools list_changed frame");
        let notification = parse_jsonrpc_notification(&frame);
        assert_eq!(
            notification.get("method").and_then(Value::as_str),
            Some("notifications/tools/list_changed")
        );
        assert!(
            read_frame(&mut client_io).await.is_none(),
            "only one list_changed frame should be emitted"
        );

        let _ = service.close().await;
    }
}
