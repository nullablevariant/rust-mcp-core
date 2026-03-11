//! Prompt dispatch from engine to inline providers or prompt plugins.
use std::collections::HashMap;
use std::sync::Arc;

use rmcp::{model::Prompt, ErrorData as McpError};
use serde_json::Value;

use crate::{
    config::PromptProviderConfig,
    engine::{
        prompts::{derive_prompt_arguments, PromptCatalog, PromptSource, ResolvedPrompt},
        schema_argument_validation::validate_completion_keys,
        tool_builders::map_icons,
        value_helpers::merge_plugin_config,
    },
    plugins::{PluginCallParams, PluginType},
};

use super::{helpers, Engine};

impl Engine {
    pub(crate) fn prompt_page_size(&self) -> Option<usize> {
        helpers::page_size(
            self.config
                .prompts
                .as_ref()
                .and_then(|prompts| prompts.pagination.as_ref()),
            self.config.as_ref(),
        )
    }

    async fn build_prompt_catalog(&self) -> Result<PromptCatalog, McpError> {
        if !self.config.prompts_active() {
            return Ok(PromptCatalog::default());
        }
        let prompts_config = self.config.prompts.as_ref().expect("active prompts config");

        let mut prompts = Vec::new();
        for provider in &prompts_config.providers {
            match provider {
                PromptProviderConfig::Inline { items } => {
                    for item in items {
                        let arguments = derive_prompt_arguments(&item.arguments_schema)?;
                        self.validate_prompt_completions(
                            item.completions.as_ref(),
                            &item.arguments_schema,
                            &item.name,
                        )?;
                        let mut prompt =
                            Prompt::new(item.name.clone(), item.description.clone(), arguments);
                        prompt.title.clone_from(&item.title);
                        prompt.icons = map_icons(item.icons.as_deref());
                        prompts.push(ResolvedPrompt {
                            prompt,
                            arguments_schema: item.arguments_schema.clone(),
                            completions: item.completions.clone(),
                            source: PromptSource::Inline {
                                messages: item.template.messages.clone(),
                            },
                        });
                    }
                }
                PromptProviderConfig::Plugin {
                    plugin,
                    config: provider_config,
                } => {
                    let prompt_plugin = helpers::lookup_prompt_plugin(&self.plugins, plugin)?;
                    let merged_config = merge_plugin_config(
                        &self.config,
                        PluginType::Prompt,
                        plugin,
                        provider_config.as_ref(),
                    );
                    let entries = prompt_plugin
                        .list(PluginCallParams {
                            config: merged_config.clone(),
                            ctx: self.build_plugin_context(None),
                        })
                        .await?;
                    for mut entry in entries {
                        let arguments = derive_prompt_arguments(&entry.arguments_schema)?;
                        self.validate_prompt_completions(
                            entry.completions.as_ref(),
                            &entry.arguments_schema,
                            &entry.prompt.name,
                        )?;
                        entry.prompt.arguments = arguments;
                        prompts.push(ResolvedPrompt {
                            prompt: entry.prompt,
                            arguments_schema: entry.arguments_schema,
                            completions: entry.completions,
                            source: PromptSource::Plugin {
                                plugin_name: plugin.clone(),
                                config: merged_config.clone(),
                            },
                        });
                    }
                }
            }
        }

        crate::engine::prompts::warn_prompt_duplicates(&prompts);
        Ok(PromptCatalog::new(prompts))
    }

    pub(in crate::engine) async fn prompt_catalog(&self) -> Result<Arc<PromptCatalog>, McpError> {
        if let Some(existing) = self.prompt_catalog.read().await.as_ref() {
            return Ok(Arc::clone(existing));
        }

        let refreshed = self.refresh_prompt_catalog().await?;
        Ok(refreshed)
    }

    pub(in crate::engine) async fn refresh_prompt_catalog(
        &self,
    ) -> Result<Arc<PromptCatalog>, McpError> {
        let catalog = Arc::new(self.build_prompt_catalog().await?);
        let mut guard = self.prompt_catalog.write().await;
        *guard = Some(Arc::clone(&catalog));
        Ok(catalog)
    }

    // Validates completion key references for a prompt: checks that all completion
    // keys map to valid argument names in the schema, then (when completion feature
    // is enabled) checks that each completion value maps to a known completion provider.
    #[cfg(feature = "completion")]
    fn validate_prompt_completions(
        &self,
        completions: Option<&HashMap<String, String>>,
        schema: &Value,
        prompt_name: &str,
    ) -> Result<(), McpError> {
        validate_completion_keys(completions, schema, "prompt", prompt_name)?;
        if self.config.completion_active() {
            crate::engine::completion::validate_completion_source_mapping(
                completions,
                &self.completion_sources,
                &format!("prompt '{prompt_name}'"),
            )?;
        }
        Ok(())
    }

    #[cfg(not(feature = "completion"))]
    fn validate_prompt_completions(
        &self,
        completions: Option<&HashMap<String, String>>,
        schema: &Value,
        prompt_name: &str,
    ) -> Result<(), McpError> {
        validate_completion_keys(completions, schema, "prompt", prompt_name)?;
        if self.config.completion_active() {
            return Err(McpError::invalid_request(
                "completion feature disabled but completion config is active".to_owned(),
                None,
            ));
        }
        Ok(())
    }

    pub async fn list_prompts_for_refresh(&self) -> Result<Vec<Prompt>, McpError> {
        Ok(self.refresh_prompt_catalog().await?.prompts())
    }
}
