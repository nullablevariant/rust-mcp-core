//! Tool configuration types: definitions, execution, input/output schemas.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{IconConfig, OutboundRetryConfig};

const fn default_tool_cancellable() -> bool {
    true
}

const fn default_tools_notify_list_changed() -> bool {
    false
}

const fn default_task_support() -> TaskSupport {
    TaskSupport::Forbidden
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default = "default_tools_notify_list_changed")]
    pub notify_list_changed: bool,
    pub items: Vec<ToolConfig>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            notify_list_changed: default_tools_notify_list_changed(),
            items: Vec::new(),
        }
    }
}

impl ToolsConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolConfig {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    #[serde(default = "default_tool_cancellable")]
    pub cancellable: bool,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub annotations: Option<Value>,
    #[serde(default)]
    pub icons: Option<Vec<IconConfig>>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
    pub execute: ExecuteConfig,
    #[serde(default)]
    pub response: Option<ResponseConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ExecuteConfig {
    Http(ExecuteHttpConfig),
    Plugin(ExecutePluginConfig),
}

pub use crate::mcp::TaskSupport;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteHttpConfig {
    pub upstream: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: Option<Value>,
    #[serde(default)]
    pub headers: Option<Value>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub retry: Option<OutboundRetryConfig>,
    #[serde(default = "default_task_support")]
    pub task_support: TaskSupport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutePluginConfig {
    pub plugin: String,
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default = "default_task_support")]
    pub task_support: TaskSupport,
}

impl ExecuteConfig {
    #[must_use]
    pub const fn execute_type(&self) -> ExecuteType {
        match self {
            Self::Http(_) => ExecuteType::Http,
            Self::Plugin(_) => ExecuteType::Plugin,
        }
    }

    #[must_use]
    pub const fn task_support(&self) -> TaskSupport {
        match self {
            Self::Http(config) => config.task_support,
            Self::Plugin(config) => config.task_support,
        }
    }

    #[must_use]
    pub const fn as_http(&self) -> Option<&ExecuteHttpConfig> {
        match self {
            Self::Http(config) => Some(config),
            Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub const fn as_plugin(&self) -> Option<&ExecutePluginConfig> {
        match self {
            Self::Plugin(config) => Some(config),
            Self::Http(_) => None,
        }
    }

    pub const fn as_http_mut(&mut self) -> Option<&mut ExecuteHttpConfig> {
        match self {
            Self::Http(config) => Some(config),
            Self::Plugin(_) => None,
        }
    }

    pub const fn as_plugin_mut(&mut self) -> Option<&mut ExecutePluginConfig> {
        match self {
            Self::Plugin(config) => Some(config),
            Self::Http(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecuteType {
    Http,
    Plugin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ResponseConfig {
    Structured(ResponseStructuredConfig),
    Content(ResponseContentConfig),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseStructuredConfig {
    #[serde(default)]
    pub template: Option<Value>,
    #[serde(default)]
    pub fallback: Option<ContentFallback>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseContentConfig {
    pub items: Vec<Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentFallback {
    Text,
    JsonText,
}
