//! HTTP layer: outbound client, Axum server, router composition, and route policies.

pub(crate) mod client;
#[cfg(feature = "http_hardening")]
pub(crate) mod hardening;
pub(crate) mod outbound_pipeline;
#[cfg(feature = "streamable_http")]
pub(crate) mod policy;
#[cfg(feature = "streamable_http")]
pub(crate) mod router;
#[cfg(feature = "streamable_http")]
pub(crate) mod server;

#[cfg(feature = "streamable_http")]
pub(crate) const OAUTH_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
#[cfg(feature = "streamable_http")]
pub(crate) const MCP_SESSION_ID_HEADER: &str = "MCP-Session-Id";
#[cfg(feature = "streamable_http")]
pub(crate) const LAST_EVENT_ID_HEADER: &str = "Last-Event-Id";
