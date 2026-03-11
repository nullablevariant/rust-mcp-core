//! HTTP router plugin trait and supporting types for custom Axum routes and middleware.

use std::fmt;
use std::sync::Arc;

#[cfg(feature = "auth")]
use axum::middleware;
use axum::Router;
use serde_json::Value;

#[cfg(feature = "auth")]
use crate::auth::auth_middleware;
use crate::auth::AuthState;
use crate::config::HttpRouterTargetType;
use crate::McpError;

/// A thread-safe function that wraps an Axum [`Router`] (e.g., to add middleware layers).
pub type RouterTransform = Arc<dyn Fn(Router) -> Router + Send + Sync>;

/// Summary of the server's auth configuration, passed to [`HttpRouterPlugin::apply`].
///
/// Plugins can inspect this to decide whether to enforce auth on their routes.
#[derive(Clone, Debug)]
pub struct AuthSummary {
    /// Whether auth middleware is effectively active.
    pub auth_enabled: bool,
    /// Whether OAuth validation is active (mode is `oauth` or `all`).
    pub oauth_enabled: bool,
    /// The OAuth protected resource URL, if configured.
    pub resource_url: Option<String>,
}

/// Runtime context passed to [`HttpRouterPlugin::apply`].
///
/// Provides auth configuration and an optional helper to wrap routes with
/// the server's auth middleware.
#[derive(Clone)]
pub struct RuntimeContext {
    auth: AuthSummary,
    #[cfg_attr(not(feature = "auth"), allow(dead_code))]
    auth_state: Option<Arc<AuthState>>,
}

impl RuntimeContext {
    /// Create a new runtime context with the given auth summary and state.
    pub const fn new(auth: AuthSummary, auth_state: Option<Arc<AuthState>>) -> Self {
        Self { auth, auth_state }
    }

    /// Access the auth configuration summary.
    pub const fn auth(&self) -> &AuthSummary {
        &self.auth
    }

    /// Returns a [`RouterTransform`] that applies the server's auth middleware,
    /// or `None` when auth is disabled.
    pub fn auth_wrap(&self) -> Option<RouterTransform> {
        if !self.auth.auth_enabled {
            return None;
        }
        #[cfg(feature = "auth")]
        {
            let state = self.auth_state.clone()?;
            Some(Arc::new(move |router: Router| {
                let state = Arc::clone(&state);
                router.layer(middleware::from_fn_with_state(state, auth_middleware))
            }))
        }
        #[cfg(not(feature = "auth"))]
        {
            None
        }
    }
}

// RuntimeContext contains `Option<Arc<AuthState>>` which may not implement Debug.
// Use finish_non_exhaustive to satisfy the Debug requirement without exposing internals.
impl fmt::Debug for RuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeContext")
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

/// A route target declared in config `plugins[].targets[]`.
///
/// Each target specifies a path and type (e.g., `route` or `wrap`) that the
/// plugin should handle.
#[derive(Clone, Debug)]
pub struct HttpRouterTarget {
    /// Whether this target adds a new route or wraps existing routes with middleware.
    pub target_type: HttpRouterTargetType,
    /// The URL path for this target (e.g., `/health`).
    pub path: String,
}

/// An operation returned by [`HttpRouterPlugin::apply`].
///
/// Each operation either adds middleware around existing routes or mounts
/// a new Axum [`Router`].
pub enum HttpRouterOp {
    /// Wrap the server's router with middleware (e.g., CORS, rate limiting).
    Wrap(RouterTransform),
    /// Mount an additional Axum router (e.g., a health check endpoint).
    Route(Router),
}

// HttpRouterOp contains RouterTransform (a dyn Fn closure) and Router, neither of which
// implement Debug. Use finish_non_exhaustive to satisfy the Debug requirement.
impl fmt::Debug for HttpRouterOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wrap(_) => f.debug_tuple("Wrap").finish_non_exhaustive(),
            Self::Route(_) => f.debug_tuple("Route").finish_non_exhaustive(),
        }
    }
}

/// Custom HTTP router plugin for adding Axum routes or middleware.
///
/// Implement this trait to extend the server's HTTP surface beyond the MCP
/// endpoint. Requires the `streamable_http` feature.
///
/// # Errors
///
/// Returns `McpError` if the plugin cannot construct its routes or middleware.
pub trait HttpRouterPlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// Build route/middleware operations from the given targets and config.
    fn apply(
        &self,
        ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        config: &Value,
    ) -> Result<Vec<HttpRouterOp>, McpError>;
}
