//! Configuration types for logging, progress, tasks, and client feature capabilities.

use rmcp::model::LoggingLevel;
use serde::{Deserialize, Serialize};

const fn default_client_logging_level() -> LoggingLevel {
    LoggingLevel::Info
}

const fn default_progress_notification_interval_ms() -> u64 {
    250
}

const fn default_tasks_capabilities_list() -> bool {
    true
}

const fn default_tasks_capabilities_cancel() -> bool {
    true
}

const fn default_tasks_status_notifications() -> bool {
    false
}

const fn default_elicitation_mode() -> ElicitationMode {
    ElicitationMode::Form
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientLoggingConfig {
    #[serde(default = "default_client_logging_level")]
    pub level: LoggingLevel,
}

impl Default for ClientLoggingConfig {
    fn default() -> Self {
        Self {
            level: default_client_logging_level(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressConfig {
    #[serde(default = "default_progress_notification_interval_ms")]
    pub notification_interval_ms: u64,
}

impl Default for ProgressConfig {
    fn default() -> Self {
        Self {
            notification_interval_ms: default_progress_notification_interval_ms(),
        }
    }
}

/// Sub-struct grouping the per-capability toggles for the tasks utility feature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskCapabilities {
    /// Enables `tasks/list` when tasks config is active.
    #[serde(default = "default_tasks_capabilities_list")]
    pub list: bool,
    /// Enables `tasks/cancel` when tasks config is active.
    #[serde(default = "default_tasks_capabilities_cancel")]
    pub cancel: bool,
}

impl Default for TaskCapabilities {
    fn default() -> Self {
        Self {
            list: default_tasks_capabilities_list(),
            cancel: default_tasks_capabilities_cancel(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TasksConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Per-capability toggles for listing and cancelling tasks.
    #[serde(default)]
    pub capabilities: TaskCapabilities,
    #[serde(default = "default_tasks_status_notifications")]
    pub status_notifications: bool,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            capabilities: TaskCapabilities::default(),
            status_notifications: default_tasks_status_notifications(),
        }
    }
}

impl TasksConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

/// Configuration for server-initiated client feature capabilities.
///
/// Controls which client features (roots, sampling, elicitation) the server
/// is allowed to invoke via [`PluginContext`](crate::PluginContext).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClientFeaturesConfig {
    /// Configuration for the `roots/list` client capability.
    #[serde(default)]
    pub roots: Option<ClientRootsConfig>,
    /// Configuration for the `sampling/createMessage` client capability.
    #[serde(default)]
    pub sampling: Option<ClientSamplingConfig>,
    /// Configuration for the `elicitation/create` client capability.
    #[serde(default)]
    pub elicitation: Option<ClientElicitationConfig>,
}

impl ClientFeaturesConfig {
    #[must_use]
    pub fn roots_active(&self) -> bool {
        self.roots
            .as_ref()
            .is_some_and(ClientRootsConfig::is_active)
    }

    #[must_use]
    pub fn sampling_active(&self) -> bool {
        self.sampling
            .as_ref()
            .is_some_and(ClientSamplingConfig::is_active)
    }

    #[must_use]
    pub fn sampling_allow_tools(&self) -> bool {
        self.sampling
            .as_ref()
            .is_some_and(|sampling| sampling.is_active() && sampling.allow_tools)
    }

    #[must_use]
    pub fn elicitation_active(&self) -> bool {
        self.elicitation
            .as_ref()
            .is_some_and(ClientElicitationConfig::is_active)
    }

    #[must_use]
    pub fn elicitation_mode(&self) -> Option<ElicitationMode> {
        self.elicitation
            .as_ref()
            .filter(|config| config.is_active())
            .map(|config| config.mode)
    }
}

/// Configuration for the `roots/list` client capability.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientRootsConfig {
    pub enabled: bool,
}

impl ClientRootsConfig {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.enabled
    }
}

/// Configuration for the `sampling/createMessage` client capability.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClientSamplingConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Whether tool use is permitted in sampling requests.
    #[serde(default)]
    pub allow_tools: bool,
}

impl ClientSamplingConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}

/// Controls which elicitation modes the server is allowed to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationMode {
    /// Form-based structured input only.
    Form,
    /// URL-based out-of-band interaction only (e.g., OAuth flows).
    Url,
    /// Both form and URL modes are allowed.
    Both,
}

/// Configuration for the `elicitation/create` client capability.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientElicitationConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Which elicitation modes are allowed (default: `form`).
    #[serde(default = "default_elicitation_mode")]
    pub mode: ElicitationMode,
}

impl Default for ClientElicitationConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            mode: default_elicitation_mode(),
        }
    }
}

impl ClientElicitationConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }
}
