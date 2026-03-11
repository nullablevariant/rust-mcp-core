//! Completion configuration types for argument autocompletion.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompletionConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub providers: Vec<CompletionProviderConfig>,
}

impl CompletionConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionProviderConfig {
    Inline {
        name: String,
        values: Vec<String>,
    },
    Plugin {
        name: String,
        plugin: String,
        #[serde(default)]
        config: Option<Value>,
    },
}
