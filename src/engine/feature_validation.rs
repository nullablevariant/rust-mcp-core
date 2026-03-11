//! Engine construction-time feature validation.

#[cfg(all(
    not(feature = "completion"),
    any(feature = "prompts", feature = "resources")
))]
use std::collections::HashMap;

use rmcp::ErrorData as McpError;

use crate::config::feature_validation::shared_feature_validation_error;
use crate::config::McpConfig;
#[cfg(all(not(feature = "completion"), feature = "prompts"))]
use crate::config::PromptProviderConfig;
#[cfg(feature = "tasks_utility")]
use crate::config::TaskSupport;
#[cfg(all(not(feature = "completion"), feature = "resources"))]
use crate::config::{ResourceProviderConfig, ResourceTemplateConfig};
#[cfg(not(feature = "completion"))]
use crate::plugins::PluginType;

// Checks that disabled compile-time features don't have corresponding config
// entries. Catches mistakes like configuring completion providers without
// enabling the completion feature flag.
pub(super) fn validate_engine_feature_flags(config: &McpConfig) -> Result<(), McpError> {
    if let Some(error) = shared_feature_validation_error(config) {
        return Err(error);
    }

    #[cfg(feature = "completion")]
    {
        if config.completion_active() && config.completion_providers().is_empty() {
            return Err(McpError::invalid_request(
            "completion config is active but completion.providers is empty; set completion.enabled=false to disable completion"
                .to_owned(),
            None,
        ));
        }
    }

    #[cfg(not(feature = "completion"))]
    {
        if config.completion_active() {
            return Err(McpError::invalid_request(
                "completion feature disabled but completion config is active".to_owned(),
                None,
            ));
        }
        if config
            .plugins
            .iter()
            .any(|plugin| plugin.plugin_type == PluginType::Completion)
        {
            return Err(McpError::invalid_request(
                "completion feature disabled but plugins include type=completion".to_owned(),
                None,
            ));
        }
        if config_has_completion_mappings(config) {
            return Err(McpError::invalid_request(
                "completion feature disabled but prompt/resource completions are configured"
                    .to_owned(),
                None,
            ));
        }
    }

    #[cfg(feature = "tasks_utility")]
    {
        if !config.tasks_active()
            && config
                .tools_items()
                .iter()
                .any(|tool| tool.execute.task_support() != TaskSupport::Forbidden)
        {
            return Err(McpError::invalid_request(
                "tasks config is inactive but tools.items[].execute.task_support=optional|required is configured"
                    .to_owned(),
                None,
            ));
        }
    }

    Ok(())
}

// Scans prompt and resource providers for completion mappings — used only
// when the completion feature is disabled to catch invalid config.
#[cfg(all(
    not(feature = "completion"),
    any(feature = "prompts", feature = "resources")
))]
fn config_has_completion_mappings(config: &McpConfig) -> bool {
    #[cfg(feature = "prompts")]
    {
        let prompts_have_completions = config.prompts.as_ref().is_some_and(|prompts| {
            prompts.providers.iter().any(|provider| match provider {
                PromptProviderConfig::Inline { items } => items.iter().any(|item| {
                    item.completions
                        .as_ref()
                        .is_some_and(|m: &HashMap<String, String>| !m.is_empty())
                }),
                PromptProviderConfig::Plugin { .. } => false,
            })
        });
        if prompts_have_completions {
            return true;
        }
    }

    #[cfg(feature = "resources")]
    {
        if config.resources.as_ref().is_some_and(|resources| {
            resources.providers.iter().any(|provider| {
                let templates: Option<&Vec<ResourceTemplateConfig>> = match provider {
                    ResourceProviderConfig::Inline { templates, .. }
                    | ResourceProviderConfig::Plugin { templates, .. } => templates.as_ref(),
                };
                templates.is_some_and(|templates| {
                    templates.iter().any(|template| {
                        template
                            .completions
                            .as_ref()
                            .is_some_and(|m: &HashMap<String, String>| !m.is_empty())
                    })
                })
            })
        }) {
            return true;
        }
    }
    false
}

#[cfg(all(
    not(feature = "completion"),
    not(any(feature = "prompts", feature = "resources"))
))]
const fn config_has_completion_mappings(_config: &McpConfig) -> bool {
    false
}
