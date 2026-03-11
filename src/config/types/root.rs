//! Root config struct and top-level server section.

use std::collections::HashMap;

use rmcp::model::LoggingLevel;
use serde::{Deserialize, Serialize};

use super::upstream::{OutboundHttpConfig, UpstreamConfig};
use super::{
    AuthConfig, ClientCompatConfig, ClientFeaturesConfig, ClientLoggingConfig, CompletionConfig,
    CompletionProviderConfig, ErrorExposureConfig, IconConfig, PaginationConfig, PluginConfig,
    ProgressConfig, PromptsConfig, ResourcesConfig, ResponseLimitsConfig, TasksConfig, ToolConfig,
    ToolsConfig, TransportConfig,
};

fn default_server_host() -> String {
    "127.0.0.1".to_owned()
}

const fn default_server_port() -> u16 {
    3000
}

fn default_server_endpoint_path() -> String {
    "/mcp".to_owned()
}

fn default_server_log_level() -> String {
    "info".to_owned()
}

const fn default_log_payload_max_bytes() -> u64 {
    4096
}

/// Top-level configuration for an MCP server.
///
/// Deserialized from a YAML or JSON config file via [`load_mcp_config`](crate::load_mcp_config)
/// or [`load_mcp_config_from_path`](crate::load_mcp_config_from_path). Passed to
/// [`run_from_config`](crate::runtime::run_from_config) to start the server.
///
/// See `docs/CONFIG_SCHEMA.md` for the full field reference and validation rules.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpConfig {
    pub version: u32,
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub client_logging: Option<ClientLoggingConfig>,
    #[serde(default)]
    pub progress: Option<ProgressConfig>,
    #[serde(default)]
    pub prompts: Option<PromptsConfig>,
    #[serde(default)]
    pub resources: Option<ResourcesConfig>,
    #[serde(default)]
    pub completion: Option<CompletionConfig>,
    #[serde(default)]
    pub tasks: Option<TasksConfig>,
    #[serde(default)]
    pub client_features: ClientFeaturesConfig,
    #[serde(default)]
    pub pagination: Option<PaginationConfig>,
    #[serde(default)]
    pub upstreams: HashMap<String, UpstreamConfig>,
    #[serde(default)]
    pub outbound_http: Option<OutboundHttpConfig>,
    #[serde(default)]
    pub tools: Option<ToolsConfig>,
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

impl McpConfig {
    #[must_use]
    pub const fn client_logging_active(&self) -> bool {
        self.client_logging.is_some()
    }

    #[must_use]
    pub fn client_logging_level(&self) -> LoggingLevel {
        self.client_logging
            .as_ref()
            .map_or(LoggingLevel::Info, |logging| logging.level)
    }

    #[must_use]
    pub const fn progress_active(&self) -> bool {
        self.progress.is_some()
    }

    #[must_use]
    pub fn progress_interval_ms(&self) -> u64 {
        self.progress
            .as_ref()
            .map_or(250, |progress| progress.notification_interval_ms)
    }

    #[must_use]
    pub fn completion_active(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(CompletionConfig::is_active)
    }

    #[must_use]
    pub fn completion_providers(&self) -> &[CompletionProviderConfig] {
        self.completion
            .as_ref()
            .map_or(&[], |completion| completion.providers.as_slice())
    }

    #[must_use]
    pub fn tasks_active(&self) -> bool {
        self.tasks.as_ref().is_some_and(TasksConfig::is_active)
    }

    #[must_use]
    pub fn tasks_status_notifications_active(&self) -> bool {
        self.tasks
            .as_ref()
            .is_some_and(|tasks| tasks.is_active() && tasks.status_notifications)
    }

    #[must_use]
    pub fn tasks_list_active(&self) -> bool {
        self.tasks
            .as_ref()
            .is_some_and(|tasks| tasks.is_active() && tasks.capabilities.list)
    }

    #[must_use]
    pub fn tasks_cancel_active(&self) -> bool {
        self.tasks
            .as_ref()
            .is_some_and(|tasks| tasks.is_active() && tasks.capabilities.cancel)
    }

    #[must_use]
    pub fn client_roots_active(&self) -> bool {
        self.client_features.roots_active()
    }

    #[must_use]
    pub fn client_sampling_active(&self) -> bool {
        self.client_features.sampling_active()
    }

    #[must_use]
    pub fn client_sampling_allow_tools(&self) -> bool {
        self.client_features.sampling_allow_tools()
    }

    #[must_use]
    pub fn client_elicitation_active(&self) -> bool {
        self.client_features.elicitation_active()
    }

    #[must_use]
    pub fn client_elicitation_mode(&self) -> Option<super::ElicitationMode> {
        self.client_features.elicitation_mode()
    }

    #[must_use]
    pub fn tools_notify_list_changed(&self) -> bool {
        self.tools
            .as_ref()
            .is_some_and(|tools| tools.is_active() && tools.notify_list_changed)
    }

    #[must_use]
    pub fn tools_items(&self) -> &[ToolConfig] {
        self.tools
            .as_ref()
            .filter(|tools| tools.is_active())
            .map_or(&[], |tools| tools.items.as_slice())
    }

    #[must_use]
    pub fn tools_active(&self) -> bool {
        self.tools.as_ref().is_some_and(ToolsConfig::is_active)
    }

    #[must_use]
    pub fn prompts_active(&self) -> bool {
        self.prompts.as_ref().is_some_and(PromptsConfig::is_active)
    }

    #[must_use]
    pub fn resources_active(&self) -> bool {
        self.resources
            .as_ref()
            .is_some_and(ResourcesConfig::is_active)
    }

    pub fn tools_items_mut(&mut self) -> &mut Vec<ToolConfig> {
        &mut self.tools.get_or_insert_with(ToolsConfig::default).items
    }

    pub fn set_tools_notify_list_changed(&mut self, enabled: bool) {
        self.tools
            .get_or_insert_with(ToolsConfig::default)
            .notify_list_changed = enabled;
    }

    pub fn set_tools_items(&mut self, items: Vec<ToolConfig>) {
        let tools_enabled = self.tools.as_ref().and_then(|tools| tools.enabled);
        let notify_list_changed = self.tools_notify_list_changed();
        self.tools = Some(ToolsConfig {
            enabled: tools_enabled,
            notify_list_changed,
            items,
        });
    }

    #[must_use]
    pub const fn expose_internal_error_details(&self) -> bool {
        self.server.errors.expose_internal_details
    }

    #[must_use]
    pub const fn log_payload_max_bytes(&self) -> u64 {
        self.server.logging.log_payload_max_bytes
    }

    #[must_use]
    pub const fn response_limits(&self) -> Option<ResponseLimitsConfig> {
        self.server.response_limits
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerInfoConfig {
    pub name: Option<String>,
    pub version: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub website_url: Option<String>,
    pub icons: Option<Vec<IconConfig>>,
    #[serde(default)]
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerLoggingConfig {
    #[serde(default = "default_server_log_level")]
    pub level: String,
    #[serde(default = "default_log_payload_max_bytes")]
    pub log_payload_max_bytes: u64,
}

impl Default for ServerLoggingConfig {
    fn default() -> Self {
        Self {
            level: default_server_log_level(),
            log_payload_max_bytes: default_log_payload_max_bytes(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
    #[serde(default = "default_server_endpoint_path")]
    pub endpoint_path: String,
    #[serde(default)]
    pub transport: TransportConfig,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub errors: ErrorExposureConfig,
    #[serde(default)]
    pub logging: ServerLoggingConfig,
    #[serde(default)]
    pub response_limits: Option<ResponseLimitsConfig>,
    #[serde(default)]
    pub client_compat: ClientCompatConfig,
    #[serde(default)]
    pub info: Option<ServerInfoConfig>,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            host: default_server_host(),
            port: default_server_port(),
            endpoint_path: default_server_endpoint_path(),
            transport: TransportConfig::default(),
            auth: None,
            errors: ErrorExposureConfig::default(),
            logging: ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: ClientCompatConfig::default(),
            info: None,
        }
    }
}

impl ServerSection {
    #[must_use]
    pub fn auth_enabled(&self) -> bool {
        self.auth.as_ref().is_some_and(AuthConfig::is_enabled)
    }

    #[must_use]
    pub fn auth_active(&self) -> bool {
        self.auth.as_ref().is_some_and(AuthConfig::is_active)
    }

    #[must_use]
    pub fn auth_oauth_enabled(&self) -> bool {
        self.auth
            .as_ref()
            .is_some_and(|auth| auth.is_enabled() && auth.oauth.is_some())
    }

    pub fn auth_mut_or_insert(&mut self) -> &mut AuthConfig {
        self.auth.get_or_insert_with(AuthConfig::default)
    }
}
