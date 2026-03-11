//! Config-driven MCP server framework built on the official rmcp SDK.
//!
//! Define tools, auth, prompts, resources, and HTTP behavior in YAML/JSON
//! configuration. The library handles execution, validation, and MCP protocol
//! compliance with minimal Rust code.
//!
//! ## Consumer-facing entry points
//! - Config loading: [`load_mcp_config`], [`load_mcp_config_from_path`]
//! - Runtime startup: [`runtime::build_runtime`], [`runtime::run_from_config`], [`runtime::Runtime`]
//! - Plugin integration: [`PluginRegistry`] and plugin traits in [`plugins`]
//! - Outbound HTTP for plugins/custom integrations: [`HttpClient`], [`SharedHttpClient`],
//!   [`OutboundHttpRequest`], [`OutboundHttpResponse`], [`default_http_client`]
//! - MCP model facade: [`mcp`] (re-exported rmcp model types)
//!
//! ## Feature-gated public API
//! - `streamable_http`: HTTP router plugin API in [`plugins::http_router`]
//! - `auth` or `http_tools`: [`ReqwestHttpClient`]
//! - `client_features`: client-callback model types in [`mcp`] (sampling/elicitation/roots)

#![recursion_limit = "256"]

#[cfg(feature = "auth")]
#[doc(hidden)]
pub mod auth;
#[cfg(not(feature = "auth"))]
#[path = "auth/disabled.rs"]
#[doc(hidden)]
pub mod auth;
pub mod config;
#[doc(hidden)]
pub mod engine;
#[doc(hidden)]
pub mod errors;
pub(crate) mod http;
pub(crate) mod log_safety;
pub mod mcp;
pub mod plugins;
pub(crate) mod rmcp_internal;
pub mod runtime;
pub(crate) mod utils;

#[cfg(test)]
pub(crate) mod inline_test_fixtures;

#[cfg(feature = "auth")]
#[doc(hidden)]
pub use auth::{auth_middleware, oauth_router};
#[doc(hidden)]
pub use auth::{
    build_auth_state, build_auth_state_from_config, build_auth_state_with_plugins,
    normalize_endpoint_path, AuthActivation, AuthState, AuthStateParams,
};
pub use config::McpConfig;
pub use config::{load_mcp_config, load_mcp_config_from_path};
#[doc(hidden)]
pub use config::{
    ClientElicitationConfig, ClientFeaturesConfig, ClientRootsConfig, ClientSamplingConfig,
    ElicitationMode,
};
#[doc(hidden)]
pub use engine::{Engine, EngineConfig};
#[doc(hidden)]
pub use errors::{
    cancelled_error, cancelled_tool_result, CANCELLED_ERROR_CODE, CANCELLED_ERROR_MESSAGE,
};
#[cfg(any(feature = "auth", feature = "http_tools"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "auth", feature = "http_tools"))))]
pub use http::client::ReqwestHttpClient;
pub use http::client::{
    default_http_client, HttpClient, OutboundHttpRequest, OutboundHttpResponse, SharedHttpClient,
};
pub use mcp::McpError;
#[cfg(feature = "streamable_http")]
#[cfg_attr(docsrs, doc(cfg(feature = "streamable_http")))]
pub use plugins::http_router::{
    AuthSummary, HttpRouterOp, HttpRouterPlugin, HttpRouterTarget, RouterTransform, RuntimeContext,
};
pub use plugins::{
    AuthPlugin, AuthPluginDecision, AuthPluginValidateParams, CompletionPlugin, ListFeature,
    LogChannel, LogEventParams, LogResult, PluginAccessToken, PluginCallParams, PluginContext,
    PluginRegistry, PluginSendAuthMode, PluginSendOptions, PluginType, PromptEntry, PromptPlugin,
    ResourceEntry, ResourcePlugin, ToolPlugin,
};
#[doc(hidden)]
pub use plugins::{PluginLookup, PluginRef};
