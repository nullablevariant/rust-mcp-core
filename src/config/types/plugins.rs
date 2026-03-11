//! Plugin declaration config types.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plugins::PluginType;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    #[serde(default)]
    pub targets: Option<Vec<PluginTargetConfig>>,
    #[serde(default)]
    pub config: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginTargetConfig {
    #[serde(rename = "type")]
    pub target_type: HttpRouterTargetType,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HttpRouterTargetType {
    Wrap,
    Route,
}
