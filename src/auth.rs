//! Authentication domains and inbound auth re-exports.

pub(crate) mod inbound;
#[doc(hidden)]
pub mod oauth;
#[cfg(feature = "http_tools")]
pub(crate) mod outbound;

#[doc(hidden)]
pub use inbound::normalize_endpoint_path;
#[cfg(feature = "streamable_http")]
#[doc(hidden)]
pub use inbound::{auth_middleware, oauth_router};
#[doc(hidden)]
pub use inbound::{
    build_auth_state, build_auth_state_from_config, build_auth_state_with_plugins, AuthActivation,
    AuthState, AuthStateParams,
};
