//! Inbound authentication and authorization: bearer tokens, JWT/JWKS, OAuth introspection, and middleware.

pub(crate) mod authorization;
mod claims;
mod factory;
pub(crate) mod middleware;
pub(crate) mod provider;
mod token;

#[doc(hidden)]
pub use crate::utils::normalize_endpoint_path;
#[doc(hidden)]
pub use authorization::{AuthActivation, AuthState};
#[doc(hidden)]
pub use factory::{
    build_auth_state, build_auth_state_from_config, build_auth_state_with_plugins, AuthStateParams,
};
#[cfg(feature = "streamable_http")]
#[doc(hidden)]
pub use middleware::{auth_middleware, oauth_router};
