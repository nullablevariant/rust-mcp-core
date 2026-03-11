#![cfg(feature = "prompts")]

mod e2e_common;

use e2e_common::{build_engine, make_minimal_config, spawn_e2e, SmokeTestClient};
use rmcp::model::GetPromptRequestParams;
use rust_mcp_core::{
    config::{
        PromptItemConfig, PromptProviderConfig, PromptTemplateConfig, PromptTemplateMessageConfig,
        PromptsConfig,
    },
    plugins::PluginRegistry,
};
use serde_json::json;

#[tokio::test]
async fn e2e_prompts_list_and_get() {
    let mut config = make_minimal_config();
    config.prompts = Some(PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Inline {
            items: vec![PromptItemConfig {
                name: "greet".to_owned(),
                title: None,
                description: Some("A greeting prompt".to_owned()),
                icons: None,
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    },
                    "required": ["name"]
                }),
                template: PromptTemplateConfig {
                    messages: vec![PromptTemplateMessageConfig {
                        role: rust_mcp_core::config::PromptMessageRoleConfig::Assistant,
                        content: json!("Hello ${name}!"),
                    }],
                },
                completions: None,
            }],
        }],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let prompts = client_service
        .peer()
        .list_prompts(None)
        .await
        .expect("list_prompts");
    assert_eq!(
        prompts.prompts.len(),
        1,
        "should have exactly 1 prompt listed"
    );
    assert_eq!(
        prompts.prompts[0].name, "greet",
        "prompt name should be 'greet'"
    );
    assert_eq!(
        prompts.prompts[0].description.as_deref(),
        Some("A greeting prompt"),
        "prompt description should match"
    );

    let result = client_service
        .peer()
        .get_prompt({
            let mut request = GetPromptRequestParams::new("greet");
            request.arguments = Some(json!({"name": "World"}).as_object().unwrap().clone());
            request
        })
        .await
        .expect("get_prompt greet");

    assert_eq!(
        result.messages.len(),
        1,
        "prompt should have exactly 1 message"
    );
    let message = &result.messages[0];
    assert_eq!(
        message.role,
        rmcp::model::PromptMessageRole::Assistant,
        "message role should be Assistant"
    );
    let message_text = match &message.content {
        rmcp::model::PromptMessageContent::Text { text } => text.as_str(),
        other => panic!("expected text content, got: {other:?}"),
    };
    assert_eq!(
        message_text, "Hello World!",
        "prompt should render exact greeting text"
    );
}
