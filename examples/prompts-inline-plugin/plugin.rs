use std::collections::HashMap;

use async_trait::async_trait;
use rust_mcp_core::mcp::{GetPromptResult, McpError, Prompt, PromptMessage, PromptMessageRole};
use rust_mcp_core::{PluginCallParams, PromptEntry, PromptPlugin};
use serde_json::{json, Value};

pub(crate) struct PromptCatalogPlugin;

#[async_trait]
impl PromptPlugin for PromptCatalogPlugin {
    fn name(&self) -> &'static str {
        "prompts.catalog"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        let mut prompt = Prompt::new(
            "catalog.quickstart",
            Some("Explain how to use the catalog"),
            None,
        );
        prompt.title = Some("Catalog quickstart".to_owned());
        Ok(vec![PromptEntry {
            prompt,
            arguments_schema: json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string" }
                },
                "required": ["topic"]
            }),
            completions: Some(HashMap::from([(
                "topic".to_owned(),
                "catalog_topics".to_owned(),
            )])),
        }])
    }

    async fn get(
        &self,
        name: &str,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::Assistant,
            format!("Prompt args: {args}"),
        )])
        .with_description(format!("Dynamic prompt '{name}'")))
    }
}
