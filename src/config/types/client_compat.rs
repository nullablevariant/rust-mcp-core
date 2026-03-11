//! Client-compatibility policy controls for configuration linting.

use serde::{Deserialize, Serialize};

const fn default_top_level_combinators_policy() -> TopLevelCombinatorsPolicy {
    TopLevelCombinatorsPolicy::Warn
}

/// Server policy for compatibility lint checks against known client constraints.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClientCompatConfig {
    #[serde(default)]
    pub input_schema: InputSchemaCompatConfig,
}

/// Compatibility policy for tool input schema authoring patterns.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct InputSchemaCompatConfig {
    #[serde(default = "default_top_level_combinators_policy")]
    pub top_level_combinators: TopLevelCombinatorsPolicy,
}

/// Enforcement level for top-level JSON Schema combinators.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TopLevelCombinatorsPolicy {
    Off,
    #[default]
    Warn,
    Error,
}
