//! Prompt provider and prompt item configuration types.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{IconConfig, PaginationConfig};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notify_list_changed: bool,
    #[serde(default)]
    pub pagination: Option<PaginationConfig>,
    pub providers: Vec<PromptProviderConfig>,
}

impl PromptsConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptProviderConfig {
    Inline {
        items: Vec<PromptItemConfig>,
    },
    Plugin {
        plugin: String,
        #[serde(default)]
        config: Option<Value>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptItemConfig {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icons: Option<Vec<IconConfig>>,
    pub arguments_schema: Value,
    pub template: PromptTemplateConfig,
    #[serde(default)]
    pub completions: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptTemplateConfig {
    pub messages: Vec<PromptTemplateMessageConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptTemplateMessageConfig {
    pub role: PromptMessageRoleConfig,
    pub content: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PromptMessageRoleConfig {
    User,
    Assistant,
}
