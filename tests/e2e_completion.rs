#![cfg(all(feature = "completion", feature = "prompts"))]

mod e2e_common;

use e2e_common::{build_engine, make_minimal_config, spawn_e2e, SmokeTestClient};
use rmcp::model::{ArgumentInfo, CompleteRequestParams, CompletionInfo, Reference};
use rust_mcp_core::{
    config::{
        CompletionConfig, CompletionProviderConfig, PluginConfig, PromptItemConfig,
        PromptProviderConfig, PromptTemplateConfig, PromptTemplateMessageConfig, PromptsConfig,
    },
    plugins::{CompletionPlugin, PluginCallParams, PluginRegistry},
};
use serde_json::json;
use std::collections::HashMap;

struct PrefixCompletionPlugin;

#[async_trait::async_trait]
impl CompletionPlugin for PrefixCompletionPlugin {
    fn name(&self) -> &'static str {
        "completion.plugin"
    }

    async fn complete(
        &self,
        req: &CompleteRequestParams,
        _params: PluginCallParams,
    ) -> Result<CompletionInfo, rmcp::ErrorData> {
        CompletionInfo::with_all_values(vec![
            format!("{}-one", req.argument.value),
            format!("{}-two", req.argument.value),
        ])
        .map_err(|message| rmcp::ErrorData::internal_error(message, None))
    }
}

#[tokio::test]
async fn e2e_completion_inline() {
    let mut config = make_minimal_config();
    config.completion = Some(CompletionConfig {
        enabled: Some(true),
        providers: vec![CompletionProviderConfig::Inline {
            name: "countries".to_owned(),
            values: vec![
                "US".to_owned(),
                "UK".to_owned(),
                "UA".to_owned(),
                "FR".to_owned(),
            ],
        }],
    });
    config.prompts = Some(PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Inline {
            items: vec![PromptItemConfig {
                name: "country_prompt".to_owned(),
                title: None,
                description: None,
                icons: None,
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "country": { "type": "string" }
                    },
                    "required": ["country"]
                }),
                template: PromptTemplateConfig {
                    messages: vec![PromptTemplateMessageConfig {
                        role: rust_mcp_core::config::PromptMessageRoleConfig::User,
                        content: json!("Country: ${country}"),
                    }],
                },
                completions: Some(HashMap::from([(
                    "country".to_owned(),
                    "countries".to_owned(),
                )])),
            }],
        }],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .complete(CompleteRequestParams::new(
            Reference::for_prompt("country_prompt"),
            ArgumentInfo {
                name: "country".to_owned(),
                value: "U".to_owned(),
            },
        ))
        .await
        .expect("complete");

    let mut values = result.completion.values.clone();
    values.sort();
    assert_eq!(
        values,
        vec!["UA".to_owned(), "UK".to_owned(), "US".to_owned()],
        "completion for prefix 'U' should return exactly UA, UK, US (sorted)"
    );
    assert_eq!(
        result.completion.values.len(),
        3,
        "should return exactly 3 completions for prefix 'U'"
    );
}

#[tokio::test]
async fn e2e_completion_plugin_provider() {
    let mut config = make_minimal_config();
    config.completion = Some(CompletionConfig {
        enabled: Some(true),
        providers: vec![CompletionProviderConfig::Plugin {
            name: "dynamic".to_owned(),
            plugin: "completion.plugin".to_owned(),
            config: None,
        }],
    });
    config.prompts = Some(PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Inline {
            items: vec![PromptItemConfig {
                name: "dynamic_prompt".to_owned(),
                title: None,
                description: None,
                icons: None,
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "country": { "type": "string" }
                    },
                    "required": ["country"]
                }),
                template: PromptTemplateConfig {
                    messages: vec![PromptTemplateMessageConfig {
                        role: rust_mcp_core::config::PromptMessageRoleConfig::User,
                        content: json!("Country: ${country}"),
                    }],
                },
                completions: Some(HashMap::from([(
                    "country".to_owned(),
                    "dynamic".to_owned(),
                )])),
            }],
        }],
    });
    config.plugins.push(PluginConfig {
        name: "completion.plugin".to_owned(),
        plugin_type: rust_mcp_core::plugins::PluginType::Completion,
        targets: None,
        config: None,
    });

    let plugins = PluginRegistry::new()
        .register_completion(PrefixCompletionPlugin)
        .expect("completion plugin should register");
    let engine = build_engine(config, plugins);
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .complete(CompleteRequestParams::new(
            Reference::for_prompt("dynamic_prompt"),
            ArgumentInfo {
                name: "country".to_owned(),
                value: "U".to_owned(),
            },
        ))
        .await
        .expect("complete");

    assert_eq!(
        result.completion.values,
        vec!["U-one".to_owned(), "U-two".to_owned()],
        "plugin completion provider should return exact plugin values"
    );
}
