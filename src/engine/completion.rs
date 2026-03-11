//! Completion source resolution and argument autocompletion dispatch.
use std::collections::{HashMap, HashSet};

use rmcp::{model::CompletionInfo, ErrorData as McpError};
use serde_json::Value;

use crate::config::{CompletionProviderConfig, McpConfig};

#[cfg(feature = "prompts")]
use crate::config::PromptProviderConfig;

#[cfg(feature = "resources")]
use crate::config::ResourceProviderConfig;

pub(super) fn build_completion_sources(
    sources: &[CompletionProviderConfig],
    completion_allowlist: &HashSet<String>,
) -> Result<HashMap<String, CompletionProviderConfig>, McpError> {
    let mut by_name = HashMap::new();
    for source in sources {
        let source_name = match source {
            CompletionProviderConfig::Inline { name, .. } => name,
            CompletionProviderConfig::Plugin { name, plugin, .. } => {
                if !completion_allowlist.contains(plugin) {
                    return Err(McpError::invalid_request(
                        format!("completion plugin not allowlisted: {plugin}"),
                        None,
                    ));
                }
                name
            }
        };
        if by_name
            .insert(source_name.clone(), source.clone())
            .is_some()
        {
            return Err(McpError::invalid_request(
                format!("duplicate completion provider '{source_name}'"),
                None,
            ));
        }
    }
    Ok(by_name)
}

#[cfg(any(feature = "prompts", feature = "resources"))]
pub(super) fn validate_completion_source_references(
    config: &McpConfig,
    completion_sources: &HashMap<String, CompletionProviderConfig>,
) -> Result<(), McpError> {
    #[cfg(feature = "prompts")]
    if config.prompts_active() {
        let prompts = config.prompts.as_ref().expect("active prompts config");
        for provider in &prompts.providers {
            if let PromptProviderConfig::Inline { items } = provider {
                for item in items {
                    validate_completion_source_mapping(
                        item.completions.as_ref(),
                        completion_sources,
                        &format!("prompt '{}'", item.name),
                    )?;
                }
            }
        }
    }

    #[cfg(feature = "resources")]
    if config.resources_active() {
        let resources = config.resources.as_ref().expect("active resources config");
        for provider in &resources.providers {
            let templates = match provider {
                ResourceProviderConfig::Inline { templates, .. }
                | ResourceProviderConfig::Plugin { templates, .. } => templates.as_ref(),
            };
            let Some(templates) = templates else {
                continue;
            };
            for template in templates {
                validate_completion_source_mapping(
                    template.completions.as_ref(),
                    completion_sources,
                    &format!("resource template '{}'", template.uri_template),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(not(any(feature = "prompts", feature = "resources")))]
pub(super) const fn validate_completion_source_references(
    _config: &McpConfig,
    _completion_sources: &HashMap<String, CompletionProviderConfig>,
) {
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn validate_completion_source_mapping(
    completions: Option<&HashMap<String, String>>,
    completion_sources: &HashMap<String, CompletionProviderConfig>,
    owner: &str,
) -> Result<(), McpError> {
    let Some(completions) = completions else {
        return Ok(());
    };
    for source_name in completions.values() {
        if !completion_sources.contains_key(source_name) {
            return Err(McpError::invalid_request(
                format!("{owner} references unknown completion provider '{source_name}'"),
                None,
            ));
        }
    }
    Ok(())
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn validate_completion_argument_exists(
    schema: &Value,
    argument_name: &str,
    owner: &str,
) -> Result<(), McpError> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            McpError::invalid_request(
                format!("{owner} arguments_schema.properties must be an object"),
                None,
            )
        })?;
    if properties.contains_key(argument_name) {
        Ok(())
    } else {
        Err(McpError::invalid_params(
            format!(
                "{owner} argument '{argument_name}' is not defined in arguments_schema.properties"
            ),
            None,
        ))
    }
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn complete_inline_values(
    values: &[String],
    prefix: &str,
) -> Result<CompletionInfo, McpError> {
    let mut matches = values
        .iter()
        .filter(|candidate| candidate.starts_with(prefix))
        .cloned()
        .collect::<Vec<_>>();
    let total = u32::try_from(matches.len()).map_err(|_| {
        McpError::internal_error("completion result count exceeds u32::MAX".to_owned(), None)
    })?;
    let has_more = matches.len() > CompletionInfo::MAX_VALUES;
    if has_more {
        matches.truncate(CompletionInfo::MAX_VALUES);
    }
    completion_info_from_values_with_total(matches, Some(total), has_more)
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn completion_info_from_values(values: Vec<String>) -> Result<CompletionInfo, McpError> {
    completion_info_from_values_with_total(values, None, false)
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn completion_info_from_values_with_total(
    values: Vec<String>,
    total: Option<u32>,
    has_more: bool,
) -> Result<CompletionInfo, McpError> {
    CompletionInfo::with_pagination(values, total, has_more).map_err(|err| {
        McpError::internal_error(format!("invalid completion response: {err}"), None)
    })
}

#[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
pub(super) fn validate_completion_info(
    completion: CompletionInfo,
) -> Result<CompletionInfo, McpError> {
    completion.validate().map(|()| completion).map_err(|err| {
        McpError::internal_error(format!("invalid completion response: {err}"), None)
    })
}
