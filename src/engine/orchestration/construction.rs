//! Engine construction from config: tool building, plugin wiring, and validation.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rmcp::{
    handler::server::tool_name_validation::validate_and_warn_tool_name, ErrorData as McpError,
};
#[cfg(any(feature = "prompts", feature = "resources"))]
use tokio::sync::RwLock;
use tracing::debug;

use crate::{config::ExecuteType, http::client::default_http_client, plugins::PluginType};

#[cfg(feature = "prompts")]
use crate::config::PromptProviderConfig;
#[cfg(feature = "resources")]
use crate::config::ResourceProviderConfig;

#[cfg(feature = "completion")]
use super::super::completion::{build_completion_sources, validate_completion_source_references};
#[cfg(feature = "resources")]
use super::super::resources::{
    map_resource_annotations, validate_resource_template_schema, validate_resource_template_uri,
    validate_resource_uri,
};
#[cfg(feature = "tasks_utility")]
use super::super::tasks::TaskStore;
use super::super::{
    client_notifications::ClientNotificationHub, feature_validation::validate_engine_feature_flags,
    tool_builders::build_tool_attr, SchemaValidatorCache,
};
use super::{
    helpers::{
        build_plugin_allowlist, validate_allowlist_registered, warn_registered_not_allowlisted,
    },
    Engine, EngineConfig,
};
use crate::config::{McpConfig, ToolConfig};

impl Engine {
    // Validates config, checks that every plugin referenced in config is both
    // allowlisted and registered, validates prompt/resource/completion providers,
    // builds tool definitions, and wires up logging/progress/task state.
    // This is the single construction path — all Engine instances go through here.
    pub fn from_config(engine_config: EngineConfig) -> Result<Self, McpError> {
        let config = Arc::new(engine_config.config);
        validate_engine_feature_flags(config.as_ref())?;
        let plugins = engine_config.plugins;
        let schema_validator_cache = SchemaValidatorCache::default();
        let allowlists = PluginAllowlists::from_config(config.as_ref());

        allowlists.validate_registered(&plugins)?;

        #[cfg(any(feature = "prompts", feature = "resources"))]
        validate_provider_plugins(
            config.as_ref(),
            &allowlists.prompt,
            &allowlists.resource,
            &schema_validator_cache,
        )?;

        #[cfg(not(any(feature = "prompts", feature = "resources")))]
        validate_provider_plugins(
            config.as_ref(),
            &allowlists.prompt,
            &allowlists.resource,
            &schema_validator_cache,
        );

        #[cfg(feature = "completion")]
        let completion_sources = if config.completion_active() {
            let completion_sources =
                build_completion_sources(config.completion_providers(), &allowlists.completion)?;
            #[cfg(any(feature = "prompts", feature = "resources"))]
            validate_completion_source_references(config.as_ref(), &completion_sources)?;
            #[cfg(not(any(feature = "prompts", feature = "resources")))]
            validate_completion_source_references(config.as_ref(), &completion_sources);
            completion_sources
        } else {
            HashMap::new()
        };

        allowlists.warn_unallowlisted(&plugins);

        if matches!(
            config
                .pagination
                .as_ref()
                .map(|pagination| pagination.page_size),
            Some(0)
        ) {
            debug!("pagination.page_size=0 disables pagination");
        }

        let http_client = default_http_client();
        #[cfg(feature = "http_tools")]
        let outbound_token_manager =
            crate::auth::outbound::token_manager::OutboundTokenManager::new(Arc::clone(
                &http_client,
            ));
        let upstreams = Arc::new(config.upstreams.clone());

        let client_logging = build_client_logging_state(config.as_ref());
        let progress_state = build_progress_state(config.as_ref());

        let (tools, tool_map) = build_tool_map(config.tools_items(), &allowlists.tool)?;

        Ok(Self {
            config,
            tools,
            tool_map,
            http_client,
            #[cfg(feature = "http_tools")]
            outbound_token_manager,
            plugins,
            upstreams,
            #[cfg(feature = "completion")]
            completion_sources,
            client_logging,
            progress_state,
            #[cfg(feature = "tasks_utility")]
            task_store: TaskStore::new(),
            schema_validator_cache,
            #[cfg(feature = "prompts")]
            prompt_catalog: Arc::new(RwLock::new(None)),
            #[cfg(feature = "resources")]
            resource_catalog: Arc::new(RwLock::new(None)),
            list_refresh_handle: engine_config.list_refresh_handle,
            notification_hub: ClientNotificationHub::default(),
        })
    }
}

struct PluginAllowlists {
    tool: HashSet<String>,
    prompt: HashSet<String>,
    resource: HashSet<String>,
    completion: HashSet<String>,
}

impl PluginAllowlists {
    fn from_config(config: &McpConfig) -> Self {
        Self {
            tool: build_plugin_allowlist(config, PluginType::Tool),
            prompt: build_plugin_allowlist(config, PluginType::Prompt),
            resource: build_plugin_allowlist(config, PluginType::Resource),
            completion: build_plugin_allowlist(config, PluginType::Completion),
        }
    }

    fn validate_registered(
        &self,
        plugins: &crate::plugins::PluginRegistry,
    ) -> Result<(), McpError> {
        validate_allowlist_registered(plugins, &self.tool, PluginType::Tool)?;
        validate_allowlist_registered(plugins, &self.prompt, PluginType::Prompt)?;
        validate_allowlist_registered(plugins, &self.resource, PluginType::Resource)?;
        validate_allowlist_registered(plugins, &self.completion, PluginType::Completion)?;
        Ok(())
    }

    fn warn_unallowlisted(&self, plugins: &crate::plugins::PluginRegistry) {
        warn_registered_not_allowlisted(plugins, &self.tool, PluginType::Tool);
        warn_registered_not_allowlisted(plugins, &self.prompt, PluginType::Prompt);
        warn_registered_not_allowlisted(plugins, &self.resource, PluginType::Resource);
        warn_registered_not_allowlisted(plugins, &self.completion, PluginType::Completion);
    }
}

fn build_client_logging_state(config: &McpConfig) -> crate::plugins::ClientLoggingState {
    #[cfg(feature = "client_logging")]
    {
        crate::plugins::ClientLoggingState::new(
            config.client_logging_active(),
            config.client_logging_level(),
        )
    }
    #[cfg(not(feature = "client_logging"))]
    {
        crate::plugins::ClientLoggingState::new(false, config.client_logging_level())
    }
}

fn build_progress_state(config: &McpConfig) -> crate::plugins::ProgressState {
    #[cfg(feature = "progress_utility")]
    {
        crate::plugins::ProgressState::new(config.progress_active(), config.progress_interval_ms())
    }
    #[cfg(not(feature = "progress_utility"))]
    {
        crate::plugins::ProgressState::new(false, config.progress_interval_ms())
    }
}

// Validates that every prompt and resource plugin referenced in providers is
// present in the corresponding allowlist. Resource inline providers are also
// checked for structural completeness (items and/or templates present, content
// has text or blob). Called once during engine construction.
#[cfg(any(feature = "prompts", feature = "resources"))]
fn validate_provider_plugins(
    config: &McpConfig,
    prompt_allowlist: &std::collections::HashSet<String>,
    resource_allowlist: &std::collections::HashSet<String>,
    schema_validator_cache: &SchemaValidatorCache,
) -> Result<(), McpError> {
    #[cfg(feature = "prompts")]
    if config.prompts_active() {
        let prompts = config.prompts.as_ref().expect("active prompts config");
        for provider in &prompts.providers {
            if let PromptProviderConfig::Plugin { plugin, .. } = provider {
                if !prompt_allowlist.contains(plugin) {
                    return Err(McpError::invalid_request(
                        format!("prompt plugin not allowlisted: {plugin}"),
                        None,
                    ));
                }
            }
        }
    }

    #[cfg(feature = "resources")]
    validate_resource_providers(config, resource_allowlist, schema_validator_cache)?;

    #[cfg(not(feature = "prompts"))]
    let _ = prompt_allowlist;
    #[cfg(not(feature = "resources"))]
    let _ = (resource_allowlist, schema_validator_cache);

    Ok(())
}

#[cfg(not(any(feature = "prompts", feature = "resources")))]
const fn validate_provider_plugins(
    _config: &McpConfig,
    _prompt_allowlist: &std::collections::HashSet<String>,
    _resource_allowlist: &std::collections::HashSet<String>,
    _schema_validator_cache: &SchemaValidatorCache,
) {
}

// Validates all resource providers: inline providers must have items or
// templates, each item must have text or blob content, and plugin providers
// must be allowlisted.
#[cfg(feature = "resources")]
fn validate_resource_providers(
    config: &McpConfig,
    resource_allowlist: &std::collections::HashSet<String>,
    schema_validator_cache: &SchemaValidatorCache,
) -> Result<(), McpError> {
    if !config.resources_active() {
        return Ok(());
    }
    let resources = config.resources.as_ref().expect("active resources config");
    for provider in &resources.providers {
        match provider {
            ResourceProviderConfig::Inline { items, templates } => {
                validate_inline_resource_provider(
                    items.as_ref(),
                    templates.as_ref(),
                    schema_validator_cache,
                )?;
            }
            ResourceProviderConfig::Plugin {
                plugin, templates, ..
            } => {
                if !resource_allowlist.contains(plugin) {
                    return Err(McpError::invalid_request(
                        format!("resource plugin not allowlisted: {plugin}"),
                        None,
                    ));
                }
                if let Some(templates) = templates.as_ref() {
                    for template in templates {
                        validate_resource_template_uri(&template.uri_template)?;
                        validate_resource_template_schema(template, schema_validator_cache)?;
                        let _ = map_resource_annotations(template.annotations.as_ref())?;
                    }
                }
            }
        }
    }
    Ok(())
}

// Checks that an inline resource provider has at least one item or template,
// validates each item's URI and content, and validates each template's URI
// and schema.
#[cfg(feature = "resources")]
fn validate_inline_resource_provider(
    items: Option<&Vec<crate::config::ResourceItemConfig>>,
    templates: Option<&Vec<crate::config::ResourceTemplateConfig>>,
    schema_validator_cache: &SchemaValidatorCache,
) -> Result<(), McpError> {
    if items.is_none_or(Vec::is_empty) && templates.is_none_or(Vec::is_empty) {
        return Err(McpError::invalid_request(
            "resource inline provider requires items and/or templates".to_owned(),
            None,
        ));
    }
    if let Some(items) = items {
        for item in items {
            validate_resource_uri(&item.uri)?;
            let _ = map_resource_annotations(item.annotations.as_ref())?;
            if let Some(content) = item.content.as_ref() {
                if content.text.is_none() && content.blob.is_none() {
                    return Err(McpError::invalid_request(
                        format!("resource '{}' content must include text or blob", item.uri),
                        None,
                    ));
                }
            }
        }
    }
    if let Some(templates) = templates {
        for template in templates {
            validate_resource_template_uri(&template.uri_template)?;
            validate_resource_template_schema(template, schema_validator_cache)?;
            let _ = map_resource_annotations(template.annotations.as_ref())?;
        }
    }
    Ok(())
}

// Ordered tool list paired with the name-keyed map, returned by `build_tool_map`.
type ToolMapResult = (Vec<rmcp::model::Tool>, HashMap<String, ToolConfig>);

// Builds the ordered tool list and name-keyed tool map. Validates each tool
// name and checks that plugin tools reference an allowlisted plugin.
fn build_tool_map(
    tool_configs: &[ToolConfig],
    tool_allowlist: &std::collections::HashSet<String>,
) -> Result<ToolMapResult, McpError> {
    let mut tools = Vec::with_capacity(tool_configs.len());
    let mut tool_map = HashMap::new();

    for tool_cfg in tool_configs {
        if !validate_and_warn_tool_name(&tool_cfg.name) {
            return Err(McpError::invalid_request(
                format!("invalid tool name: {}", tool_cfg.name),
                None,
            ));
        }

        if tool_cfg.execute.execute_type() == ExecuteType::Plugin {
            let plugin_name = &tool_cfg
                .execute
                .as_plugin()
                .ok_or_else(|| {
                    McpError::invalid_request("plugin tool requires plugin".to_owned(), None)
                })?
                .plugin;
            if plugin_name.is_empty() {
                return Err(McpError::invalid_request(
                    "plugin tool requires plugin".to_owned(),
                    None,
                ));
            }
            if !tool_allowlist.contains(plugin_name) {
                return Err(McpError::invalid_request(
                    format!("tool plugin not allowlisted: {plugin_name}"),
                    None,
                ));
            }
        }

        let tool_attr = build_tool_attr(tool_cfg)?;
        tools.push(tool_attr);
        tool_map.insert(tool_cfg.name.clone(), tool_cfg.clone());
    }

    Ok((tools, tool_map))
}
