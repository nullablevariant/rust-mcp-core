//! Handler implementations for prompts/list and prompts/get requests.
use crate::plugins::{PluginCallParams, PluginLookup};
use rmcp::{
    model::{GetPromptRequestParams, GetPromptResult, ListPromptsResult, PaginatedRequestParams},
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde_json::json;

use super::super::orchestration::Engine;
use crate::errors::cancelled_error;

impl Engine {
    fn validate_prompt_args(
        &self,
        schema: &serde_json::Value,
        args: &serde_json::Map<String, serde_json::Value>,
        prompt_name: &str,
    ) -> Result<(), McpError> {
        crate::engine::schema_argument_validation::validate_schema_args(
            &crate::engine::schema_argument_validation::ValidateSchemaArgsParams {
                schema,
                args,
                entity_label: "prompt",
                entity_name: prompt_name,
                schema_validator_cache: &self.schema_validator_cache,
            },
        )
    }

    pub(super) async fn handle_list_prompts_request(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let request_cursor = request.and_then(|params| params.cursor);
        let _ = context;
        let catalog = self.prompt_catalog().await?;
        let prompt_items = catalog.prompts();

        if let Some(page_size) = self.prompt_page_size() {
            let (prompts, next_cursor) = crate::engine::pagination::paginate_items(
                &prompt_items,
                request_cursor,
                page_size,
            )?;
            Ok(ListPromptsResult {
                prompts,
                next_cursor,
                ..Default::default()
            })
        } else {
            Ok(ListPromptsResult {
                prompts: prompt_items,
                ..Default::default()
            })
        }
    }

    pub(super) async fn handle_get_prompt_request(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let args = request.arguments.unwrap_or_default();
        let catalog = self.prompt_catalog().await?;

        let prompt_name = request.name;
        let Some(selected) = catalog.by_name(&prompt_name).cloned() else {
            let available_prompts = catalog
                .entries()
                .iter()
                .map(|entry| entry.prompt.name.clone())
                .collect::<Vec<_>>();
            return Err(McpError::invalid_params(
                format!("prompt '{prompt_name}' not found"),
                Some(json!({ "available_prompts": available_prompts })),
            ));
        };

        self.validate_prompt_args(&selected.arguments_schema, &args, &prompt_name)?;
        match selected.source {
            crate::engine::prompts::PromptSource::Inline { messages } => {
                let rendered = crate::engine::prompts::render_prompt_messages(
                    &messages,
                    &serde_json::Value::Object(args),
                )?;
                let mut result = GetPromptResult::new(rendered);
                if let Some(description) = selected.prompt.description.clone() {
                    result = result.with_description(description);
                }
                Ok(result)
            }
            crate::engine::prompts::PromptSource::Plugin {
                plugin_name,
                config,
            } => {
                let plugin_ref = self
                    .plugins
                    .get_plugin(crate::plugins::PluginType::Prompt, &plugin_name)
                    .ok_or_else(|| {
                        McpError::invalid_request(
                            format!("prompt plugin not registered: {plugin_name}"),
                            None,
                        )
                    })?;
                let crate::plugins::PluginRef::Prompt(prompt_plugin) = plugin_ref else {
                    return Err(McpError::invalid_request(
                        format!("plugin type mismatch for prompt plugin: {plugin_name}"),
                        None,
                    ));
                };
                prompt_plugin
                    .get(
                        &prompt_name,
                        serde_json::Value::Object(args),
                        PluginCallParams {
                            config,
                            ctx: self.build_plugin_context(Some(context)),
                        },
                    )
                    .await
            }
        }
    }
}
