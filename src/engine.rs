//! MCP server engine: tool dispatch, handler implementations, and ServerHandler trait.

mod client_notifications;
#[cfg(feature = "completion")]
mod completion;
mod feature_validation;
mod handler;
#[cfg(feature = "http_tools")]
#[doc(hidden)]
pub mod http_executor;
mod orchestration;
mod pagination;
#[cfg(feature = "prompts")]
mod prompts;
#[cfg(feature = "resources")]
mod resources;
#[cfg(any(feature = "prompts", feature = "resources"))]
mod schema_argument_validation;
mod schema_validator_cache;
#[cfg(feature = "tasks_utility")]
pub(crate) mod tasks;
#[doc(hidden)]
pub mod templating;
mod tool_builders;
#[doc(hidden)]
pub mod tool_response;
mod value_helpers;

pub(crate) use client_notifications::ClientNotificationHub;
#[doc(hidden)]
pub use orchestration::{Engine, EngineConfig};
pub(crate) use schema_validator_cache::SchemaValidatorCache;
