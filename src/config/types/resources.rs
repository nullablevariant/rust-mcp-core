//! Resource provider, template, and subscription configuration types.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{IconConfig, PaginationConfig};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourcesConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notify_list_changed: bool,
    #[serde(default)]
    pub clients_can_subscribe: bool,
    #[serde(default)]
    pub pagination: Option<PaginationConfig>,
    pub providers: Vec<ResourceProviderConfig>,
}

impl ResourcesConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceProviderConfig {
    Inline {
        #[serde(default)]
        items: Option<Vec<ResourceItemConfig>>,
        #[serde(default)]
        templates: Option<Vec<ResourceTemplateConfig>>,
    },
    Plugin {
        plugin: String,
        #[serde(default)]
        config: Option<Value>,
        #[serde(default)]
        templates: Option<Vec<ResourceTemplateConfig>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceItemConfig {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size: Option<u32>,
    #[serde(default)]
    pub icons: Option<Vec<IconConfig>>,
    #[serde(default)]
    pub annotations: Option<ResourceAnnotationsConfig>,
    #[serde(default)]
    pub content: Option<ResourceContentConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceContentConfig {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub blob: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceTemplateConfig {
    pub uri_template: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub icons: Option<Vec<IconConfig>>,
    #[serde(default)]
    pub annotations: Option<ResourceAnnotationsConfig>,
    pub arguments_schema: Value,
    #[serde(default)]
    pub completions: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceAnnotationsConfig {
    #[serde(default)]
    pub audience: Option<Vec<ResourceAudienceConfig>>,
    #[serde(default)]
    pub priority: Option<f32>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceAudienceConfig {
    User,
    Assistant,
}
