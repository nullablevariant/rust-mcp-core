//! Core hardening configuration types for error exposure, logging, and response limits.

use serde::{Deserialize, Serialize};

const fn default_expose_internal_details() -> bool {
    false
}

/// Controls client-facing error detail exposure.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ErrorExposureConfig {
    /// When true, internal error details may be exposed in client-facing errors.
    #[serde(default = "default_expose_internal_details")]
    pub expose_internal_details: bool,
}

impl Default for ErrorExposureConfig {
    fn default() -> Self {
        Self {
            expose_internal_details: default_expose_internal_details(),
        }
    }
}

/// Optional caps for MCP tool response payload channels.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ResponseLimitsConfig {
    #[serde(default)]
    pub text_bytes: Option<u64>,
    #[serde(default)]
    pub structured_bytes: Option<u64>,
    #[serde(default)]
    pub binary_bytes: Option<u64>,
    #[serde(default)]
    pub other_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
}
