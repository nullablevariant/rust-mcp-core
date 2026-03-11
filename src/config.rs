//! Configuration loading, JSON schema validation, and all config type definitions.

pub(crate) mod feature_validation;
#[doc(hidden)]
pub mod json_schema;
pub mod loader;
#[doc(hidden)]
pub mod types;

#[doc(hidden)]
pub use json_schema::config_schema;
pub use loader::*;
#[doc(hidden)]
pub use types::*;
