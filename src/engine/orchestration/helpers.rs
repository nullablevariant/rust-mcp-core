//! Engine helper methods for plugin context construction and upstream resolution.
use std::{collections::HashSet, sync::Arc};

use rmcp::ErrorData as McpError;

#[cfg(any(feature = "prompts", feature = "resources"))]
use crate::config::PaginationConfig;
#[cfg(feature = "completion")]
use crate::plugins::CompletionPlugin;
#[cfg(feature = "prompts")]
use crate::plugins::PromptPlugin;
#[cfg(feature = "resources")]
use crate::plugins::ResourcePlugin;
use crate::{
    config::McpConfig,
    plugins::{PluginLookup, PluginRef, PluginRegistry, PluginType, ToolPlugin},
};

pub(super) fn build_plugin_allowlist(
    config: &McpConfig,
    plugin_type: PluginType,
) -> HashSet<String> {
    config
        .plugins
        .iter()
        .filter(|plugin| plugin.plugin_type == plugin_type)
        .map(|plugin| plugin.name.clone())
        .collect()
}

// Fails fast at startup if any allowlisted plugin name is missing from the
// registry — catches typos and missing .register_*() calls.
pub(super) fn validate_allowlist_registered(
    plugins: &PluginRegistry,
    allowlist: &HashSet<String>,
    plugin_type: PluginType,
) -> Result<(), McpError> {
    let plugin_label = plugin_type_label(plugin_type);
    for plugin_name in allowlist {
        if plugins.get_plugin(plugin_type, plugin_name).is_none() {
            return Err(McpError::invalid_request(
                format!("{plugin_label} plugin not registered: {plugin_name}"),
                None,
            ));
        }
    }
    Ok(())
}

// Inverse of validate_allowlist_registered: warns about plugins that are
// registered but not referenced in config — likely a forgotten config entry.
pub(super) fn warn_registered_not_allowlisted(
    plugins: &PluginRegistry,
    allowlist: &HashSet<String>,
    plugin_type: PluginType,
) {
    let plugin_label = plugin_type_label(plugin_type);
    for (name, ptype) in plugins.names() {
        if ptype == plugin_type && !allowlist.contains(&name) {
            tracing::warn!(
                "{} plugin registered but not allowlisted: {}",
                plugin_label,
                name
            );
        }
    }
}

// Resolves page size: feature-specific pagination overrides global, and
// page_size=0 disables pagination (returns None).
#[cfg(any(feature = "prompts", feature = "resources"))]
pub(super) fn page_size(
    feature_pagination: Option<&PaginationConfig>,
    global: &McpConfig,
) -> Option<usize> {
    feature_pagination
        .map(|pagination| pagination.page_size)
        .or_else(|| {
            global
                .pagination
                .as_ref()
                .map(|pagination| pagination.page_size)
        })
        .filter(|page_size| *page_size > 0)
        .and_then(|page_size| usize::try_from(page_size).ok())
}

pub(super) fn lookup_tool_plugin(
    plugins: &PluginRegistry,
    plugin_name: &str,
) -> Result<Arc<dyn ToolPlugin>, McpError> {
    let plugin_ref = plugins
        .get_plugin(PluginType::Tool, plugin_name)
        .ok_or_else(|| {
            McpError::invalid_request(format!("tool plugin not registered: {plugin_name}"), None)
        })?;

    let PluginRef::Tool(plugin) = plugin_ref else {
        return Err(McpError::invalid_request(
            format!("plugin type mismatch for tool plugin: {plugin_name}"),
            None,
        ));
    };

    Ok(plugin)
}

#[cfg(feature = "prompts")]
pub(super) fn lookup_prompt_plugin(
    plugins: &PluginRegistry,
    plugin_name: &str,
) -> Result<Arc<dyn PromptPlugin>, McpError> {
    let plugin_ref = plugins
        .get_plugin(PluginType::Prompt, plugin_name)
        .ok_or_else(|| {
            McpError::invalid_request(format!("prompt plugin not registered: {plugin_name}"), None)
        })?;

    let PluginRef::Prompt(plugin) = plugin_ref else {
        return Err(McpError::invalid_request(
            format!("plugin type mismatch for prompt plugin: {plugin_name}"),
            None,
        ));
    };

    Ok(plugin)
}

#[cfg(feature = "resources")]
pub(super) fn lookup_resource_plugin(
    plugins: &PluginRegistry,
    plugin_name: &str,
) -> Result<Arc<dyn ResourcePlugin>, McpError> {
    let plugin_ref = plugins
        .get_plugin(PluginType::Resource, plugin_name)
        .ok_or_else(|| {
            McpError::invalid_request(
                format!("resource plugin not registered: {plugin_name}"),
                None,
            )
        })?;

    let PluginRef::Resource(plugin) = plugin_ref else {
        return Err(McpError::invalid_request(
            format!("plugin type mismatch for resource plugin: {plugin_name}"),
            None,
        ));
    };

    Ok(plugin)
}

#[cfg(feature = "completion")]
pub(super) fn lookup_completion_plugin(
    plugins: &PluginRegistry,
    plugin_name: &str,
) -> Result<Arc<dyn CompletionPlugin>, McpError> {
    let plugin_ref = plugins
        .get_plugin(PluginType::Completion, plugin_name)
        .ok_or_else(|| {
            McpError::invalid_request(
                format!("completion plugin not registered: {plugin_name}"),
                None,
            )
        })?;

    let PluginRef::Completion(plugin) = plugin_ref else {
        return Err(McpError::invalid_request(
            format!("plugin type mismatch for completion plugin: {plugin_name}"),
            None,
        ));
    };

    Ok(plugin)
}

const fn plugin_type_label(plugin_type: PluginType) -> &'static str {
    match plugin_type {
        PluginType::Tool => "tool",
        PluginType::Auth => "auth",
        PluginType::HttpRouter => "http_router",
        PluginType::Completion => "completion",
        PluginType::Prompt => "prompt",
        PluginType::Resource => "resource",
    }
}

#[cfg(test)]
// Inline tests are required because these helper functions are scoped to the
// `engine::orchestration` module and are not reachable from integration tests.
mod tests {
    #[cfg(any(feature = "prompts", feature = "resources", feature = "completion"))]
    use std::{collections::HashMap, sync::Arc};

    #[cfg(feature = "completion")]
    use super::lookup_completion_plugin;
    #[cfg(feature = "prompts")]
    use super::lookup_prompt_plugin;
    #[cfg(feature = "resources")]
    use super::lookup_resource_plugin;
    #[cfg(any(feature = "prompts", feature = "resources"))]
    use super::page_size;
    use super::{build_plugin_allowlist, lookup_tool_plugin, validate_allowlist_registered};
    #[cfg(any(feature = "prompts", feature = "resources"))]
    use crate::config::PaginationConfig;
    #[cfg(any(feature = "prompts", feature = "resources", feature = "completion"))]
    use crate::http::client::default_http_client;
    #[cfg(any(feature = "prompts", feature = "resources", feature = "completion"))]
    use crate::plugins::PluginContext;
    use crate::plugins::{PluginCallParams, PluginRegistry, PluginType, ToolPlugin};
    use crate::{config::PluginConfig, inline_test_fixtures::base_config};
    use rmcp::model::ErrorCode;
    #[cfg(feature = "completion")]
    use rmcp::model::{ArgumentInfo, CompleteRequestParams, Reference};
    use rmcp::ErrorData as McpError;
    use serde_json::Value;

    struct TestToolPlugin;

    #[async_trait::async_trait]
    impl ToolPlugin for TestToolPlugin {
        fn name(&self) -> &'static str {
            "tool.lookup"
        }

        async fn call(
            &self,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<rmcp::model::CallToolResult, McpError> {
            Ok(rmcp::model::CallToolResult::success(vec![]))
        }
    }

    #[cfg(feature = "prompts")]
    struct TestPromptPlugin;

    #[cfg(feature = "prompts")]
    #[async_trait::async_trait]
    impl crate::plugins::PromptPlugin for TestPromptPlugin {
        fn name(&self) -> &'static str {
            "prompt.lookup"
        }

        async fn list(
            &self,
            _params: PluginCallParams,
        ) -> Result<Vec<crate::plugins::PromptEntry>, McpError> {
            Ok(Vec::new())
        }

        async fn get(
            &self,
            _name: &str,
            _args: Value,
            _params: PluginCallParams,
        ) -> Result<rmcp::model::GetPromptResult, McpError> {
            Ok(rmcp::model::GetPromptResult::new(Vec::new()))
        }
    }

    #[cfg(feature = "resources")]
    struct TestResourcePlugin;

    #[cfg(feature = "resources")]
    #[async_trait::async_trait]
    impl crate::plugins::ResourcePlugin for TestResourcePlugin {
        fn name(&self) -> &'static str {
            "resource.lookup"
        }

        async fn list(
            &self,
            _params: PluginCallParams,
        ) -> Result<Vec<crate::plugins::ResourceEntry>, McpError> {
            Ok(Vec::new())
        }

        async fn read(
            &self,
            _uri: &str,
            _params: PluginCallParams,
        ) -> Result<rmcp::model::ReadResourceResult, McpError> {
            Ok(rmcp::model::ReadResourceResult::new(Vec::new()))
        }

        async fn subscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
            Ok(())
        }

        async fn unsubscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
            Ok(())
        }
    }

    #[cfg(feature = "completion")]
    struct TestCompletionPlugin;

    #[cfg(feature = "completion")]
    #[async_trait::async_trait]
    impl crate::plugins::CompletionPlugin for TestCompletionPlugin {
        fn name(&self) -> &'static str {
            "completion.lookup"
        }

        async fn complete(
            &self,
            _req: &rmcp::model::CompleteRequestParams,
            _params: PluginCallParams,
        ) -> Result<rmcp::model::CompletionInfo, McpError> {
            Ok(rmcp::model::CompletionInfo {
                values: Vec::new(),
                total: Some(0),
                has_more: Some(false),
            })
        }
    }

    #[test]
    fn build_plugin_allowlist_filters_by_plugin_type() {
        let mut config = base_config();
        config.plugins = vec![
            PluginConfig {
                name: "tool.allowlisted".to_owned(),
                plugin_type: PluginType::Tool,
                targets: None,
                config: None,
            },
            PluginConfig {
                name: "prompt.allowlisted".to_owned(),
                plugin_type: PluginType::Prompt,
                targets: None,
                config: None,
            },
        ];

        let tool_allowlist = build_plugin_allowlist(&config, PluginType::Tool);
        let expected: std::collections::HashSet<String> =
            ["tool.allowlisted".to_owned()].into_iter().collect();
        assert_eq!(
            tool_allowlist, expected,
            "tool allowlist must contain exactly the tool-type entries"
        );

        let prompt_allowlist = build_plugin_allowlist(&config, PluginType::Prompt);
        let expected_prompt: std::collections::HashSet<String> =
            ["prompt.allowlisted".to_owned()].into_iter().collect();
        assert_eq!(
            prompt_allowlist, expected_prompt,
            "prompt allowlist must contain exactly the prompt-type entries"
        );
    }

    #[test]
    fn validate_allowlist_registered_errors_when_plugin_missing() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "tool.missing".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }];
        let allowlist = build_plugin_allowlist(&config, PluginType::Tool);
        let registry = PluginRegistry::default();

        let err = validate_allowlist_registered(&registry, &allowlist, PluginType::Tool)
            .expect_err("missing plugin should error");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
        assert!(
            err.message.contains("tool plugin not registered"),
            "error must mention plugin type"
        );
        assert!(
            err.message.contains("tool.missing"),
            "error must mention plugin name"
        );
    }

    #[test]
    fn validate_allowlist_registered_accepts_registered_plugin() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "tool.lookup".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: None,
        }];
        let allowlist = build_plugin_allowlist(&config, PluginType::Tool);
        let registry = PluginRegistry::new()
            .register_tool(TestToolPlugin)
            .expect("tool registration");

        validate_allowlist_registered(&registry, &allowlist, PluginType::Tool)
            .expect("registered plugin should validate");

        // Post-validation: confirm the plugin is actually accessible via lookup.
        let plugin = lookup_tool_plugin(&registry, "tool.lookup")
            .expect("validated plugin must be lookupable");
        assert_eq!(
            plugin.name(),
            "tool.lookup",
            "looked-up plugin must match registered name"
        );
    }

    #[test]
    fn lookup_tool_plugin_returns_registered_plugin() {
        let registry = PluginRegistry::new()
            .register_tool(TestToolPlugin)
            .expect("tool registration");

        let plugin = lookup_tool_plugin(&registry, "tool.lookup").expect("plugin lookup");
        assert_eq!(plugin.name(), "tool.lookup");

        // Verify the returned Arc points to the correct implementation by
        // confirming the name is exactly the registered TestToolPlugin's name.
        // A second lookup must return the same name, proving registry stability.
        let plugin2 = lookup_tool_plugin(&registry, "tool.lookup").expect("second lookup");
        assert_eq!(
            plugin2.name(),
            plugin.name(),
            "repeated lookup must return same plugin"
        );
    }

    #[test]
    fn lookup_tool_plugin_returns_error_for_missing_plugin() {
        let registry = PluginRegistry::default();
        let Err(err) = lookup_tool_plugin(&registry, "tool.lookup") else {
            panic!("missing plugin should error")
        };
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
        assert!(
            err.message.contains("tool plugin not registered"),
            "error must mention plugin type"
        );
        assert!(
            err.message.contains("tool.lookup"),
            "error must mention plugin name"
        );
    }

    #[cfg(feature = "prompts")]
    #[tokio::test]
    async fn lookup_prompt_plugin_returns_registered_plugin() {
        let registry = PluginRegistry::new()
            .register_prompt(TestPromptPlugin)
            .expect("prompt registration");

        let plugin = lookup_prompt_plugin(&registry, "prompt.lookup").expect("plugin lookup");
        assert_eq!(plugin.name(), "prompt.lookup");
        // Verify contract: a second lookup returns the same plugin identity.
        let plugin2 = lookup_prompt_plugin(&registry, "prompt.lookup").expect("second lookup");
        assert_eq!(
            plugin2.name(),
            plugin.name(),
            "repeated lookup must return same plugin"
        );

        let context = test_plugin_context();
        let list_result = plugin
            .list(PluginCallParams {
                config: Value::Null,
                ctx: context.clone(),
            })
            .await
            .expect("list");
        assert!(
            list_result.is_empty(),
            "test prompt plugin list must return empty vec"
        );
        let get_result = plugin
            .get(
                "prompt.lookup",
                Value::Null,
                PluginCallParams {
                    config: Value::Null,
                    ctx: context,
                },
            )
            .await
            .expect("get");
        assert!(
            get_result.messages.is_empty(),
            "test prompt plugin get must return empty messages"
        );
    }

    #[cfg(feature = "resources")]
    #[tokio::test]
    async fn lookup_resource_plugin_returns_registered_plugin() {
        let registry = PluginRegistry::new()
            .register_resource(TestResourcePlugin)
            .expect("resource registration");

        let plugin = lookup_resource_plugin(&registry, "resource.lookup").expect("plugin lookup");
        assert_eq!(plugin.name(), "resource.lookup");

        // Verify contract: repeated lookup returns same plugin identity.
        let plugin2 = lookup_resource_plugin(&registry, "resource.lookup").expect("second lookup");
        assert_eq!(
            plugin2.name(),
            plugin.name(),
            "repeated lookup must return same plugin"
        );

        let context = test_plugin_context();
        let list_result = plugin
            .list(PluginCallParams {
                config: Value::Null,
                ctx: context.clone(),
            })
            .await
            .expect("list");
        assert!(
            list_result.is_empty(),
            "test resource plugin list must return empty vec"
        );
        let read_result = plugin
            .read(
                "resource://lookup",
                PluginCallParams {
                    config: Value::Null,
                    ctx: context.clone(),
                },
            )
            .await
            .expect("read");
        assert!(
            read_result.contents.is_empty(),
            "test resource plugin read must return empty contents"
        );
        plugin
            .subscribe(
                "resource://lookup",
                PluginCallParams {
                    config: Value::Null,
                    ctx: context.clone(),
                },
            )
            .await
            .expect("subscribe");
        plugin
            .unsubscribe(
                "resource://lookup",
                PluginCallParams {
                    config: Value::Null,
                    ctx: context,
                },
            )
            .await
            .expect("unsubscribe");
    }

    #[cfg(feature = "completion")]
    #[tokio::test]
    async fn lookup_completion_plugin_returns_registered_plugin() {
        let registry = PluginRegistry::new()
            .register_completion(TestCompletionPlugin)
            .expect("completion registration");

        let plugin =
            lookup_completion_plugin(&registry, "completion.lookup").expect("plugin lookup");
        assert_eq!(plugin.name(), "completion.lookup");

        // Verify contract: repeated lookup returns same plugin identity.
        let plugin2 =
            lookup_completion_plugin(&registry, "completion.lookup").expect("second lookup");
        assert_eq!(
            plugin2.name(),
            plugin.name(),
            "repeated lookup must return same plugin"
        );

        let request = CompleteRequestParams::new(
            Reference::for_prompt("prompt.lookup"),
            ArgumentInfo {
                name: "value".to_owned(),
                value: "a".to_owned(),
            },
        );
        let result = plugin
            .complete(
                &request,
                PluginCallParams {
                    config: Value::Null,
                    ctx: test_plugin_context(),
                },
            )
            .await
            .expect("complete");
        assert!(
            result.values.is_empty(),
            "test completion plugin must return empty values"
        );
        assert_eq!(
            result.total,
            Some(0),
            "test completion plugin total must be 0"
        );
        assert_eq!(
            result.has_more,
            Some(false),
            "test completion plugin has_more must be false"
        );
    }

    #[cfg(any(feature = "prompts", feature = "resources"))]
    #[test]
    fn page_size_prefers_feature_specific_then_global_and_ignores_zero() {
        let mut config = base_config();
        config.pagination = Some(PaginationConfig { page_size: 25 });

        assert_eq!(page_size(None, &config), Some(25));
        assert_eq!(
            page_size(Some(&PaginationConfig { page_size: 10 }), &config),
            Some(10)
        );
        assert_eq!(
            page_size(Some(&PaginationConfig { page_size: 0 }), &config),
            None
        );

        // No global + no feature pagination: must return None deterministically.
        let config_no_pagination = base_config();
        assert_eq!(
            page_size(None, &config_no_pagination),
            None,
            "absent global + absent feature pagination must return None"
        );

        // Feature-specific set but global absent: feature value must be used.
        assert_eq!(
            page_size(
                Some(&PaginationConfig { page_size: 42 }),
                &config_no_pagination
            ),
            Some(42),
            "feature pagination must work even without global pagination"
        );

        // Feature-specific zero with no global: still None.
        assert_eq!(
            page_size(
                Some(&PaginationConfig { page_size: 0 }),
                &config_no_pagination
            ),
            None,
            "zero feature pagination with absent global must return None"
        );
    }

    #[cfg(any(feature = "prompts", feature = "resources", feature = "completion"))]
    fn test_plugin_context() -> PluginContext {
        PluginContext::new(None, Arc::new(HashMap::new()), default_http_client())
    }
}
