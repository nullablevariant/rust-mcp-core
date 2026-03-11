//! Shared config types used across multiple config sections.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IconConfig {
    pub src: String,
    pub mime_type: Option<String>,
    #[serde(default)]
    pub sizes: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaginationConfig {
    pub page_size: u64,
}
