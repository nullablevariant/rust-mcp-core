//! Axum router composition: MCP endpoint, auth, and HTTP router plugins.
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header::HeaderName, HeaderMap};
#[cfg(feature = "http_hardening")]
use axum::response::IntoResponse;
use axum::{middleware, Router};
#[cfg(not(feature = "http_hardening"))]
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use serde_json::Value;
use tracing::warn;

use crate::config::{
    HttpRouterTargetType, McpConfig, PluginTargetConfig, ProtocolVersionNegotiationMode,
};
use crate::mcp::ProtocolVersion;
use crate::plugins::http_router::{
    AuthSummary, HttpRouterOp, HttpRouterTarget, RouterTransform, RuntimeContext,
};
use crate::plugins::{PluginLookup, PluginRef, PluginRegistry, PluginType};
use crate::utils::normalize_endpoint_path;
#[cfg(feature = "auth")]
use crate::{auth_middleware, oauth_router};
use crate::{AuthState, McpError};
#[cfg(feature = "http_hardening")]
use tower_http::catch_panic::CatchPanicLayer;
#[cfg(feature = "http_hardening")]
use tower_http::limit::RequestBodyLimitLayer;
#[cfg(feature = "http_hardening")]
use tower_http::sensitive_headers::{
    SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer,
};

use super::policy::{streamable_http_route_guard, StreamableHttpRoutePolicy};
use super::server::build_streamable_http_config;
use super::OAUTH_METADATA_PATH;
#[cfg(feature = "http_hardening")]
use crate::http::hardening::{
    apply_inbound_rate_limit_layer, build_session_creation_rate_limiter,
    build_streamable_http_session_manager, is_session_creation_request,
};

use crate::Engine;

const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
#[cfg(feature = "http_hardening")]
const SENSITIVE_REQUEST_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "mcp-session-id",
];
#[cfg(feature = "http_hardening")]
const SENSITIVE_RESPONSE_HEADERS: &[&str] = &["set-cookie"];

#[derive(Clone)]
pub(crate) struct RouterSegment {
    router: Router,
    layers: Vec<RouterTransform>,
}

impl RouterSegment {
    pub(crate) fn new(router: Router) -> Self {
        Self {
            router,
            layers: Vec::new(),
        }
    }

    pub(crate) fn add_layer(&mut self, layer: RouterTransform) {
        self.layers.push(layer);
    }

    pub(crate) fn build(self) -> Router {
        apply_layers(self.router, &self.layers)
    }
}

pub(crate) fn apply_layers(mut router: Router, layers: &[RouterTransform]) -> Router {
    for layer in layers.iter().rev() {
        router = layer(router);
    }
    router
}

// Normalizes a plugin target path: ensures leading slash, strips trailing slash,
// and allows the special "*" wildcard (meaning all routes).
pub(crate) fn normalize_target_path(path: &str) -> Result<String, McpError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(McpError::invalid_request(
            "http_router_plugins target path cannot be empty".to_owned(),
            None,
        ));
    }
    if trimmed == "*" {
        return Ok("*".to_owned());
    }
    if trimmed == "/" {
        return Ok("/".to_owned());
    }
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.starts_with('/') {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("/{trimmed}"))
    }
}

// Validates and normalizes plugin targets. Route targets cannot use "*"
// (only wrap targets can apply globally).
pub(crate) fn normalize_targets(
    targets: &[PluginTargetConfig],
) -> Result<Vec<HttpRouterTarget>, McpError> {
    if targets.is_empty() {
        return Err(McpError::invalid_request(
            "http_router plugins target list cannot be empty".to_owned(),
            None,
        ));
    }

    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        let path = normalize_target_path(&target.path)?;
        if target.target_type == HttpRouterTargetType::Route && path == "*" {
            return Err(McpError::invalid_request(
                "http_router plugins route targets cannot use '*'".to_owned(),
                None,
            ));
        }
        out.push(HttpRouterTarget {
            target_type: target.target_type,
            path,
        });
    }
    Ok(out)
}

// Builds an AuthSummary snapshot for router plugins so they can make
// routing decisions based on effective auth activation without accessing
// AuthState directly.
pub(crate) fn build_auth_summary(config: &McpConfig, auth_state: &AuthState) -> AuthSummary {
    let auth_cfg = config.server.auth.as_ref();
    let endpoint_path = normalize_endpoint_path(&config.server.endpoint_path);
    let public_base = auth_cfg
        .and_then(crate::config::AuthConfig::oauth_public_url)
        .map_or_else(
            || format!("http://{}:{}", config.server.host, config.server.port),
            std::borrow::ToOwned::to_owned,
        );
    let resource_url = auth_cfg
        .and_then(crate::config::AuthConfig::oauth_resource)
        .map_or_else(
            || format!("{public_base}{endpoint_path}"),
            std::borrow::ToOwned::to_owned,
        );
    let resource_url = if config.server.auth_active() {
        Some(resource_url)
    } else {
        None
    };
    AuthSummary {
        auth_enabled: config.server.auth_active(),
        oauth_enabled: auth_state.oauth_enabled(),
        resource_url,
    }
}

pub(crate) fn validate_http_router_plugins_allowlist(
    config: &McpConfig,
    registry: &PluginRegistry,
) {
    let allowlist: std::collections::HashSet<String> = config
        .plugins
        .iter()
        .filter(|p| p.plugin_type == PluginType::HttpRouter)
        .map(|p| p.name.clone())
        .collect();

    for (name, ptype) in registry.names() {
        if ptype == PluginType::HttpRouter && !allowlist.contains(&name) {
            warn!(
                "http router plugin registered but not allowlisted: {}",
                name
            );
        }
    }
}

pub(crate) fn engine_handler_factory(
    engine: Arc<Engine>,
) -> impl Fn() -> Result<Engine, std::io::Error> + Clone {
    // rmcp session service materializes an owned handler via this factory.
    move || Ok((*engine).clone())
}

fn normalize_protocol_version_header(
    mode: ProtocolVersionNegotiationMode,
    headers: &mut HeaderMap,
) {
    if mode != ProtocolVersionNegotiationMode::Negotiate {
        return;
    }

    let header_name = HeaderName::from_static(MCP_PROTOCOL_VERSION_HEADER);
    let Some(value) = headers.get(&header_name) else {
        return;
    };
    let Some(version) = value.to_str().ok().map(str::to_owned) else {
        headers.remove(&header_name);
        warn!("dropping invalid MCP-Protocol-Version header value");
        return;
    };
    let is_known = ProtocolVersion::KNOWN_VERSIONS
        .iter()
        .any(|known| known.as_str() == version);
    if !is_known {
        headers.remove(&header_name);
        warn!(version, "dropping unsupported MCP-Protocol-Version header");
    }
}

async fn protocol_version_negotiation_middleware(
    mode: ProtocolVersionNegotiationMode,
    mut request: Request<axum::body::Body>,
    next: middleware::Next,
) -> axum::response::Response {
    normalize_protocol_version_header(mode, request.headers_mut());
    next.run(request).await
}

#[cfg(feature = "http_hardening")]
fn parse_sensitive_header_names(headers: &[&str]) -> Result<Arc<[HeaderName]>, McpError> {
    let mut parsed = Vec::with_capacity(headers.len());
    for header in headers {
        parsed.push(HeaderName::from_bytes(header.as_bytes()).map_err(|error| {
            McpError::invalid_request(
                format!("invalid hardening header name '{header}': {error}"),
                None,
            )
        })?);
    }
    Ok(parsed.into())
}

#[cfg(feature = "http_hardening")]
fn to_request_body_limit_bytes(max_request_bytes: u64) -> usize {
    usize::try_from(max_request_bytes).unwrap_or(usize::MAX)
}

#[cfg(feature = "http_hardening")]
fn apply_streamable_http_hardening_layers(
    segment: &mut RouterSegment,
    hardening: &crate::config::StreamableHttpHardeningConfig,
) -> Result<(), McpError> {
    if hardening.catch_panics {
        segment.add_layer(Arc::new(|router: Router| {
            router.layer(CatchPanicLayer::new())
        }));
    }

    if hardening.sanitize_sensitive_headers {
        let request_headers = parse_sensitive_header_names(SENSITIVE_REQUEST_HEADERS)?;
        segment.add_layer(Arc::new(move |router: Router| {
            router.layer(SetSensitiveRequestHeadersLayer::from_shared(Arc::clone(
                &request_headers,
            )))
        }));

        let response_headers = parse_sensitive_header_names(SENSITIVE_RESPONSE_HEADERS)?;
        segment.add_layer(Arc::new(move |router: Router| {
            router.layer(SetSensitiveResponseHeadersLayer::from_shared(Arc::clone(
                &response_headers,
            )))
        }));
    }

    Ok(())
}

#[cfg(feature = "http_hardening")]
fn apply_session_creation_rate_limit_layer(
    mut mcp_router: Router,
    hardening: &crate::config::StreamableHttpHardeningConfig,
) -> Router {
    if let Some(limiter) = build_session_creation_rate_limiter(hardening) {
        mcp_router = mcp_router.layer(middleware::from_fn(
            move |_headers: HeaderMap, request: Request, next: middleware::Next| {
                let limiter = Arc::clone(&limiter);
                async move {
                    if is_session_creation_request(&request) {
                        let per_ip_key = limiter.extract_key(&request);
                        if !limiter.allow_with_key(per_ip_key).await {
                            return (
                                axum::http::StatusCode::TOO_MANY_REQUESTS,
                                "Too Many Requests: session creation rate limit exceeded",
                            )
                                .into_response();
                        }
                    }
                    next.run(request).await
                }
            },
        ));
    }
    mcp_router
}

#[cfg(feature = "http_hardening")]
fn build_mcp_router(
    engine: Arc<Engine>,
    config: &McpConfig,
    auth_state: &Arc<AuthState>,
) -> Result<Router, McpError> {
    let endpoint_path = normalize_endpoint_path(&config.server.endpoint_path);
    let policy = StreamableHttpRoutePolicy::from_config(config);
    let streamable_config = build_streamable_http_config(&config.server.transport);
    #[cfg(feature = "http_hardening")]
    let hardening = config
        .server
        .transport
        .streamable_http
        .effective_hardening();
    #[cfg(feature = "http_hardening")]
    let session_manager =
        build_streamable_http_session_manager(&config.server.transport.streamable_http);
    #[cfg(not(feature = "http_hardening"))]
    let session_manager = LocalSessionManager::default().into();
    let service = StreamableHttpService::new(
        engine_handler_factory(engine),
        session_manager,
        streamable_config,
    );

    let mut mcp_router = Router::new().nest_service(&endpoint_path, service);
    #[cfg(feature = "http_hardening")]
    {
        mcp_router = apply_inbound_rate_limit_layer(mcp_router, &hardening)?;
        mcp_router = apply_session_creation_rate_limit_layer(mcp_router, &hardening);
        mcp_router = mcp_router.layer(RequestBodyLimitLayer::new(to_request_body_limit_bytes(
            hardening.max_request_bytes,
        )));
    }
    let negotiation_mode = config
        .server
        .transport
        .streamable_http
        .protocol_version_negotiation
        .mode;
    mcp_router = mcp_router.layer(middleware::from_fn(
        move |_headers: HeaderMap, request: Request, next: middleware::Next| {
            protocol_version_negotiation_middleware(negotiation_mode, request, next)
        },
    ));
    mcp_router = mcp_router.layer(middleware::from_fn(
        move |_headers: HeaderMap, request: Request, next: middleware::Next| {
            streamable_http_route_guard(policy.clone(), request, next)
        },
    ));
    #[cfg(not(feature = "auth"))]
    let _ = auth_state;
    #[cfg(feature = "auth")]
    if config.server.auth_active() {
        let state = Arc::clone(auth_state);
        mcp_router = mcp_router.layer(middleware::from_fn_with_state(state, auth_middleware));
    }
    Ok(mcp_router)
}

#[cfg(not(feature = "http_hardening"))]
fn build_mcp_router(
    engine: Arc<Engine>,
    config: &McpConfig,
    auth_state: &Arc<AuthState>,
) -> Router {
    let endpoint_path = normalize_endpoint_path(&config.server.endpoint_path);
    let policy = StreamableHttpRoutePolicy::from_config(config);
    let streamable_config = build_streamable_http_config(&config.server.transport);
    let session_manager = LocalSessionManager::default().into();
    let service = StreamableHttpService::new(
        engine_handler_factory(engine),
        session_manager,
        streamable_config,
    );

    let mut mcp_router = Router::new().nest_service(&endpoint_path, service);
    let negotiation_mode = config
        .server
        .transport
        .streamable_http
        .protocol_version_negotiation
        .mode;
    mcp_router = mcp_router.layer(middleware::from_fn(
        move |_headers: HeaderMap, request: Request, next: middleware::Next| {
            protocol_version_negotiation_middleware(negotiation_mode, request, next)
        },
    ));
    mcp_router = mcp_router.layer(middleware::from_fn(
        move |_headers: HeaderMap, request: Request, next: middleware::Next| {
            streamable_http_route_guard(policy.clone(), request, next)
        },
    ));
    #[cfg(not(feature = "auth"))]
    let _ = auth_state;
    #[cfg(feature = "auth")]
    if config.server.auth_active() {
        let state = Arc::clone(auth_state);
        mcp_router = mcp_router.layer(middleware::from_fn_with_state(state, auth_middleware));
    }
    mcp_router
}

// Assembles the full Axum router: MCP endpoint (with route guard + auth
// middleware), OAuth metadata endpoint, and HTTP router plugin routes/layers.
// Plugin routes are collision-checked against reserved paths (MCP endpoint,
// OAuth metadata). Wrap targets with "*" apply to all segments.
pub(crate) fn build_streamable_http_router(
    engine: Arc<Engine>,
    auth_state: &Arc<AuthState>,
    plugins: &Arc<PluginRegistry>,
) -> Result<Router, McpError> {
    let config = Arc::clone(&engine.config);
    let config = config.as_ref();
    let endpoint_path = normalize_endpoint_path(&config.server.endpoint_path);
    #[cfg(feature = "http_hardening")]
    let hardening = config
        .server
        .transport
        .streamable_http
        .effective_hardening();
    #[cfg(feature = "http_hardening")]
    let mcp_router = build_mcp_router(engine, config, auth_state)?;
    #[cfg(not(feature = "http_hardening"))]
    let mcp_router = build_mcp_router(engine, config, auth_state);

    #[allow(unused_mut)]
    let mut app_router = Router::new();
    #[cfg(feature = "auth")]
    if auth_state.oauth_enabled() {
        app_router = app_router.merge(oauth_router(Arc::clone(auth_state)));
    }

    let router_plugins: Vec<_> = config
        .plugins
        .iter()
        .filter(|p| p.plugin_type == PluginType::HttpRouter)
        .collect();

    let auth_summary = build_auth_summary(config, auth_state);
    let auth_state_for_ctx = if config.server.auth_active() {
        Some(Arc::clone(auth_state))
    } else {
        None
    };
    let ctx = RuntimeContext::new(auth_summary, auth_state_for_ctx);

    let mut state = PluginRouteState::new(mcp_router, app_router, &endpoint_path, auth_state);
    #[cfg(feature = "http_hardening")]
    apply_streamable_http_hardening_layers(&mut state.mcp_segment, &hardening)?;
    if router_plugins.is_empty() {
        return Ok(state.assemble());
    }
    validate_http_router_plugins_allowlist(config, plugins);
    state.process_plugins(plugins, &router_plugins, &ctx)?;
    Ok(state.assemble())
}

// Holds mutable router segment state during HTTP router plugin processing.
struct PluginRouteState {
    mcp_segment: RouterSegment,
    app_segment: RouterSegment,
    plugin_segments: std::collections::HashMap<String, RouterSegment>,
    plugin_route_order: Vec<String>,
    plugin_routes: std::collections::HashSet<String>,
    reserved_routes: std::collections::HashSet<String>,
    endpoint_path: String,
}

impl PluginRouteState {
    fn new(
        mcp_router: Router,
        app_router: Router,
        endpoint_path: &str,
        auth_state: &Arc<AuthState>,
    ) -> Self {
        let mut reserved_routes: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        reserved_routes.insert(endpoint_path.to_owned());
        if auth_state.oauth_enabled() {
            reserved_routes.insert(OAUTH_METADATA_PATH.to_owned());
            if endpoint_path != "/" {
                reserved_routes.insert(format!("{OAUTH_METADATA_PATH}{endpoint_path}"));
            }
        }
        Self {
            mcp_segment: RouterSegment::new(mcp_router),
            app_segment: RouterSegment::new(app_router),
            plugin_segments: std::collections::HashMap::new(),
            plugin_route_order: Vec::new(),
            plugin_routes: std::collections::HashSet::new(),
            reserved_routes,
            endpoint_path: endpoint_path.to_owned(),
        }
    }

    // Iterates all plugin configurations, resolves ops, and applies route/wrap
    // targets to the appropriate router segments.
    fn process_plugins(
        &mut self,
        plugins: &Arc<PluginRegistry>,
        router_plugins: &[&crate::config::PluginConfig],
        ctx: &RuntimeContext,
    ) -> Result<(), McpError> {
        for plugin_cfg in router_plugins {
            let targets_cfg = plugin_cfg.targets.as_deref().unwrap_or(&[]);
            if targets_cfg.is_empty() {
                return Err(McpError::invalid_request(
                    format!("http_router plugin {} requires targets", plugin_cfg.name),
                    None,
                ));
            }
            let targets = normalize_targets(targets_cfg)?;
            let Some(PluginRef::HttpRouter(plugin)) =
                plugins.get_plugin(PluginType::HttpRouter, &plugin_cfg.name)
            else {
                return Err(McpError::invalid_request(
                    format!("http router plugin not registered: {}", plugin_cfg.name),
                    None,
                ));
            };
            let merged_config = plugin_cfg.config.clone().unwrap_or(Value::Null);
            let ops = plugin.apply(ctx, &targets, &merged_config)?;
            if ops.len() != targets.len() {
                return Err(McpError::invalid_request(
                    format!(
                        "http router plugin {} returned {} ops for {} targets",
                        plugin_cfg.name,
                        ops.len(),
                        targets.len()
                    ),
                    None,
                ));
            }
            self.apply_ops(&plugin_cfg.name, &targets, ops)?;
        }
        Ok(())
    }

    // Applies the resolved ops for one plugin to the segment state.
    fn apply_ops(
        &mut self,
        plugin_name: &str,
        targets: &[HttpRouterTarget],
        ops: Vec<HttpRouterOp>,
    ) -> Result<(), McpError> {
        for (index, (target, op)) in targets.iter().zip(ops).enumerate() {
            if target.target_type == HttpRouterTargetType::Route {
                if self.reserved_routes.contains(&target.path)
                    || self.plugin_routes.contains(&target.path)
                {
                    return Err(McpError::invalid_request(
                        format!(
                            "http router plugin {} target {} collides with existing route: {}",
                            plugin_name, index, target.path
                        ),
                        None,
                    ));
                }

                let router = match op {
                    HttpRouterOp::Route(router) => router,
                    HttpRouterOp::Wrap(_) => {
                        return Err(McpError::invalid_request(
                            format!(
                                "http router plugin {plugin_name} target {index} expected route op"
                            ),
                            None,
                        ))
                    }
                };

                let target_path = target.path.clone();
                self.plugin_routes.insert(target_path.clone());
                self.plugin_route_order.push(target_path.clone());
                self.plugin_segments
                    .insert(target_path, RouterSegment::new(router));
            } else {
                if target.path != "*"
                    && target.path != self.endpoint_path
                    && !self.plugin_routes.contains(&target.path)
                {
                    return Err(McpError::invalid_request(
                        format!(
                            "http router plugin {} target {} wraps unknown path: {}",
                            plugin_name, index, target.path
                        ),
                        None,
                    ));
                }

                let layer = match op {
                    HttpRouterOp::Wrap(layer) => layer,
                    HttpRouterOp::Route(_) => {
                        return Err(McpError::invalid_request(
                            format!(
                                "http router plugin {plugin_name} target {index} expected wrap op"
                            ),
                            None,
                        ))
                    }
                };

                if target.path == "*" {
                    self.mcp_segment.add_layer(Arc::clone(&layer));
                    self.app_segment.add_layer(Arc::clone(&layer));
                    for segment in self.plugin_segments.values_mut() {
                        segment.add_layer(Arc::clone(&layer));
                    }
                } else if target.path == self.endpoint_path {
                    self.mcp_segment.add_layer(layer);
                } else {
                    let segment = self.plugin_segments.get_mut(&target.path).ok_or_else(|| {
                        McpError::internal_error(
                            format!(
                                "http router plugin {} target {} missing route segment for path {}",
                                plugin_name, index, target.path
                            ),
                            None,
                        )
                    })?;
                    segment.add_layer(layer);
                }
            }
        }
        Ok(())
    }

    // Builds the final merged router from all accumulated segments.
    fn assemble(mut self) -> Router {
        let mut app = self.app_segment.build().merge(self.mcp_segment.build());
        for path in self.plugin_route_order {
            if let Some(segment) = self.plugin_segments.remove(&path) {
                app = app.nest(&path, segment.build());
            }
        }
        app
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "auth")]
    use super::build_streamable_http_router;
    use super::{
        build_auth_summary, engine_handler_factory, normalize_protocol_version_header,
        normalize_target_path, normalize_targets, MCP_PROTOCOL_VERSION_HEADER,
    };
    #[cfg(feature = "http_hardening")]
    use crate::config::StreamableHttpHardeningConfig;
    use crate::config::{HttpRouterTargetType, PluginConfig, ProtocolVersionNegotiationMode};
    #[cfg(feature = "auth")]
    use crate::inline_test_fixtures::base_provider;
    use crate::inline_test_fixtures::{
        base_config, build_router, route_target, router_router_plugin_config, wrap_target,
        AuthWrapPlugin, ConfigCheckPlugin, MismatchedOpsPlugin, TestPlugin, WrongRouteOpPlugin,
        WrongWrapOpPlugin,
    };
    use crate::mcp::ProtocolVersion;
    #[cfg(feature = "http_hardening")]
    use crate::plugins::http_router::HttpRouterOp;
    use crate::plugins::http_router::{
        AuthSummary, HttpRouterPlugin, HttpRouterTarget, RuntimeContext,
    };
    use crate::plugins::{PluginRegistry, PluginType};
    use crate::McpError;
    use crate::{build_auth_state_with_plugins, EngineConfig};
    use axum::body::{to_bytes, Body};
    #[cfg(feature = "auth")]
    use axum::http::header::WWW_AUTHENTICATE;
    #[cfg(feature = "http_hardening")]
    use axum::http::StatusCode;
    use axum::http::{HeaderMap, HeaderValue, Request};
    #[cfg(feature = "http_hardening")]
    use axum::response::Response;
    use rmcp::model::ErrorCode;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[cfg(feature = "http_hardening")]
    struct SensitiveHeaderProbePlugin;

    #[cfg(feature = "http_hardening")]
    impl HttpRouterPlugin for SensitiveHeaderProbePlugin {
        fn name(&self) -> &'static str {
            "sensitive_probe"
        }

        fn apply(
            &self,
            _ctx: &RuntimeContext,
            targets: &[HttpRouterTarget],
            _config: &serde_json::Value,
        ) -> Result<Vec<HttpRouterOp>, McpError> {
            let mut ops = Vec::with_capacity(targets.len());
            for _target in targets {
                ops.push(HttpRouterOp::Wrap(Arc::new(|router: axum::Router| {
                    router.layer(axum::middleware::from_fn(
                        |request: Request<Body>, _next: axum::middleware::Next| async move {
                            let auth_sensitive = request
                                .headers()
                                .get("authorization")
                                .is_some_and(HeaderValue::is_sensitive);
                            let session_sensitive = request
                                .headers()
                                .get("mcp-session-id")
                                .is_some_and(HeaderValue::is_sensitive);
                            let cookie_sensitive = request
                                .headers()
                                .get("cookie")
                                .is_some_and(HeaderValue::is_sensitive);
                            Response::builder()
                                .status(StatusCode::NO_CONTENT)
                                .header(
                                    "x-auth-sensitive",
                                    if auth_sensitive { "true" } else { "false" },
                                )
                                .header(
                                    "x-session-sensitive",
                                    if session_sensitive { "true" } else { "false" },
                                )
                                .header(
                                    "x-cookie-sensitive",
                                    if cookie_sensitive { "true" } else { "false" },
                                )
                                .header("set-cookie", "session=secret")
                                .header("www-authenticate", "Bearer realm=\"mcp\"")
                                .header("x-control", "ok")
                                .body(Body::empty())
                                .expect("response")
                        },
                    ))
                })));
            }
            Ok(ops)
        }
    }

    #[cfg(feature = "http_hardening")]
    fn initialize_request_body() -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"}
            }
        })
        .to_string()
    }

    #[cfg(feature = "http_hardening")]
    fn initialized_notification_body() -> String {
        json!({
            "jsonrpc":"2.0",
            "method":"notifications/initialized",
            "params":{}
        })
        .to_string()
    }

    fn assert_invalid_request_contains(result: Result<axum::Router, McpError>, token: &str) {
        let error = result.expect_err("router build should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(
            error.message.contains(token),
            "expected token `{token}` in message `{}`",
            error.message
        );
        assert!(error.data.is_none());
    }

    async fn response_body_text(response: axum::response::Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        String::from_utf8(body.to_vec()).expect("utf8 body")
    }

    #[test]
    fn normalize_targets_rejects_empty() {
        let error = normalize_targets(&[]).expect_err("empty targets should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "http_router plugins target list cannot be empty"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn normalize_targets_rejects_route_star() {
        let targets = vec![route_target("*")];
        let error = normalize_targets(&targets).expect_err("route * should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "http_router plugins route targets cannot use '*'"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn normalize_target_path_rejects_empty() {
        let error = normalize_target_path("   ").expect_err("empty path should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "http_router_plugins target path cannot be empty"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn normalize_target_path_accepts_root() {
        let result = normalize_target_path("/").expect("root");
        assert_eq!(result, "/");
    }

    #[test]
    fn normalize_target_path_prefixes_missing_slash() {
        let result = normalize_target_path("health/").expect("path");
        assert_eq!(result, "/health");
    }

    #[test]
    fn protocol_version_negotiate_preserves_known_header() {
        let known_version = ProtocolVersion::KNOWN_VERSIONS
            .first()
            .expect("known version")
            .as_str();
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_str(known_version).expect("header value"),
        );

        normalize_protocol_version_header(ProtocolVersionNegotiationMode::Negotiate, &mut headers);

        assert_eq!(
            headers
                .get(MCP_PROTOCOL_VERSION_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some(known_version)
        );
    }

    #[test]
    fn protocol_version_negotiate_drops_unknown_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static("9999-01-01"),
        );

        normalize_protocol_version_header(ProtocolVersionNegotiationMode::Negotiate, &mut headers);

        assert!(headers.get(MCP_PROTOCOL_VERSION_HEADER).is_none());
    }

    #[test]
    fn protocol_version_negotiate_keeps_missing_header_absent() {
        let mut headers = HeaderMap::new();

        normalize_protocol_version_header(ProtocolVersionNegotiationMode::Negotiate, &mut headers);

        assert!(headers.get(MCP_PROTOCOL_VERSION_HEADER).is_none());
    }

    #[test]
    fn protocol_version_strict_preserves_unknown_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static("9999-01-01"),
        );

        normalize_protocol_version_header(ProtocolVersionNegotiationMode::Strict, &mut headers);

        assert_eq!(
            headers
                .get(MCP_PROTOCOL_VERSION_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some("9999-01-01")
        );
    }

    #[test]
    #[cfg(feature = "auth")]
    fn build_auth_summary_respects_resource_override() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![crate::config::AuthProviderConfig::bearer(
            "static", "secret",
        )];
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: Some("https://example.com".to_owned()),
            resource: "https://example.com/custom".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        let auth_state = build_auth_state_with_plugins(&config, None).expect("auth state");
        let summary = build_auth_summary(&config, &auth_state);
        assert_eq!(
            summary.resource_url.as_deref(),
            Some("https://example.com/custom")
        );
        assert!(!summary.oauth_enabled);
    }

    #[test]
    fn build_auth_summary_clears_resource_for_none_mode() {
        let config = base_config();
        let auth_state = build_auth_state_with_plugins(&config, None).expect("auth state");
        let summary = build_auth_summary(&config, &auth_state);
        assert!(summary.resource_url.is_none());
    }

    #[test]
    fn auth_wrap_plugin_requires_auth_enabled() {
        let ctx = RuntimeContext::new(
            AuthSummary {
                auth_enabled: false,
                oauth_enabled: false,
                resource_url: None,
            },
            None,
        );
        let plugin = AuthWrapPlugin;
        let error = plugin
            .apply(
                &ctx,
                &[HttpRouterTarget {
                    target_type: HttpRouterTargetType::Wrap,
                    path: "/mcp".to_owned(),
                }],
                &Value::Null,
            )
            .expect_err("auth wrap should reject when auth disabled");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "auth_wrap requested with auth disabled");
        assert!(error.data.is_none());
    }

    #[tokio::test]
    async fn router_plugin_config_passed_to_plugin() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "config_check".to_owned(),
            plugin_type: PluginType::HttpRouter,
            targets: Some(vec![route_target("/merge")]),
            config: Some(json!({
                "base": true,
                "limit": 120,
                "mode": "override"
            })),
        }];
        let registry = PluginRegistry::new()
            .register_http_router(ConfigCheckPlugin)
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let request: Request<Body> = Request::builder()
            .uri("/merge")
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(response_body_text(response).await, "ok");
    }

    #[test]
    fn router_plugins_require_non_empty_targets() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "route".to_owned(),
            plugin_type: PluginType::HttpRouter,
            targets: Some(Vec::new()),
            config: None,
        }];

        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "route",
                label: "r",
            })
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "http_router plugin route requires targets",
        );
    }

    #[test]
    fn router_plugins_reject_unregistered_plugin_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "missing",
            vec![route_target("/missing")],
        )];
        assert_invalid_request_contains(
            build_router(&config, PluginRegistry::new()),
            "http router plugin not registered: missing",
        );
    }

    #[test]
    fn router_plugins_reject_mismatched_ops_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "mismatch",
            vec![route_target("/health")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(MismatchedOpsPlugin)
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "http router plugin mismatch returned 0 ops for 1 targets",
        );
    }

    #[test]
    fn router_plugins_reject_route_collision_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "route",
            vec![route_target("/mcp")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "route",
                label: "r",
            })
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "collides with existing route: /mcp",
        );
    }

    #[test]
    fn router_plugins_reject_wrong_route_op_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "wrong_route",
            vec![route_target("/health")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(WrongRouteOpPlugin)
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "target 0 expected route op",
        );
    }

    #[test]
    fn router_plugins_reject_wrap_unknown_path_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "wrap",
            vec![wrap_target("/unknown")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "wrap",
                label: "w",
            })
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "target 0 wraps unknown path: /unknown",
        );
    }

    #[test]
    fn router_plugins_reject_wrong_wrap_op_in_direct_builder() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "wrong_wrap",
            vec![wrap_target("/mcp")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(WrongWrapOpPlugin)
            .expect("registry");
        assert_invalid_request_contains(
            build_router(&config, registry),
            "target 0 expected wrap op",
        );
    }

    #[tokio::test]
    #[cfg(feature = "auth")]
    async fn router_plugins_wrap_order_outermost_first() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![base_provider()];
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "http://127.0.0.1:3000/mcp".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        config.plugins = vec![
            router_router_plugin_config("wrap_a", vec![wrap_target("*")]),
            router_router_plugin_config("wrap_b", vec![wrap_target("*")]),
        ];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "wrap_a",
                label: "A",
            })
            .unwrap()
            .register_http_router(TestPlugin {
                name: "wrap_b",
                label: "B",
            })
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let request: Request<Body> = Request::builder()
            .uri(super::OAUTH_METADATA_PATH)
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("response");
        let header = response
            .headers()
            .get("x-order")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(header, "BA");
    }

    #[tokio::test]
    async fn router_plugins_wrap_applies_to_plugin_route() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "route_wrap",
            vec![route_target("/health"), wrap_target("/health")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "route_wrap",
                label: "H",
            })
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let request: Request<Body> = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("response");
        let header = response
            .headers()
            .get("x-order")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(header, "H");

        let mcp_response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mcp")
                    .header("accept", "text/event-stream")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert!(
            mcp_response.headers().get("x-order").is_none(),
            "health-only wrap should not affect /mcp"
        );
    }

    #[tokio::test]
    async fn router_plugins_wrap_star_applies_to_existing_plugin_route() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "route_wrap",
            vec![route_target("/health"), wrap_target("*")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "route_wrap",
                label: "S",
            })
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let request: Request<Body> = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        let header = response
            .headers()
            .get("x-order")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(header, "S");
    }

    #[tokio::test]
    async fn mcp_endpoint_router_is_mounted() {
        let config = base_config();
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"}
            }
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("accept", "application/json, text/event-stream")
                    .header("content-type", "application/json")
                    .body(Body::from(initialize.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        let body = response_body_text(response).await;
        assert!(body.contains("data:"));
        assert!(body.contains("\"result\""));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_request_body_cap_rejects_oversized_payload() {
        let config = base_config();
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let payload = vec![b'x'; 1_048_577];
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("content-length", payload.len().to_string())
                    .body(Body::from(payload))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok());
        assert_eq!(content_type, Some("text/plain; charset=utf-8"));
        let body = response_body_text(response).await;
        assert!(body.contains("length limit exceeded"));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_catch_panics_maps_plugin_wrap_panic_to_500() {
        let mut segment = super::RouterSegment::new(
            axum::Router::new().route("/mcp", axum::routing::post(|| async { StatusCode::OK })),
        );
        super::apply_streamable_http_hardening_layers(
            &mut segment,
            &StreamableHttpHardeningConfig::default(),
        )
        .expect("hardening");
        segment.add_layer(Arc::new(|router: axum::Router| {
            router.layer(axum::middleware::from_fn(
                |request: Request<Body>, next: axum::middleware::Next| async move {
                    assert!(
                        !request.headers().contains_key("x-hardening-panic"),
                        "hardening panic probe"
                    );
                    next.run(request).await
                },
            ))
        }));
        let router = segment.build();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-hardening-panic", "1")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        let body = response_body_text(response).await;
        assert!(body.contains("panicked"));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_sensitive_headers_marks_request_and_response_values() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "sensitive_probe",
            vec![wrap_target("/mcp")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(SensitiveHeaderProbePlugin)
            .expect("registry");
        let router = build_router(&config, registry).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("authorization", "Bearer top-secret")
                    .header("cookie", "session=abc")
                    .header("mcp-session-id", "session-123")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("x-auth-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        assert_eq!(
            response
                .headers()
                .get("x-session-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        assert_eq!(
            response
                .headers()
                .get("x-cookie-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        assert!(response
            .headers()
            .get("set-cookie")
            .is_some_and(HeaderValue::is_sensitive));
        assert!(response
            .headers()
            .get("x-control")
            .is_some_and(|value| !value.is_sensitive()));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_sensitive_headers_can_be_disabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            sanitize_sensitive_headers: false,
            ..StreamableHttpHardeningConfig::default()
        });
        config.plugins = vec![router_router_plugin_config(
            "sensitive_probe",
            vec![wrap_target("/mcp")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(SensitiveHeaderProbePlugin)
            .expect("registry");
        let router = build_router(&config, registry).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("authorization", "Bearer top-secret")
                    .header("cookie", "session=abc")
                    .header("mcp-session-id", "session-123")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(
            response
                .headers()
                .get("x-auth-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("false")
        );
        assert_eq!(
            response
                .headers()
                .get("x-session-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("false")
        );
        assert_eq!(
            response
                .headers()
                .get("x-cookie-sensitive")
                .and_then(|value| value.to_str().ok()),
            Some("false")
        );
        assert!(response
            .headers()
            .get("set-cookie")
            .is_some_and(|value| !value.is_sensitive()));
        assert!(response
            .headers()
            .get("x-control")
            .is_some_and(|value| !value.is_sensitive()));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_session_max_sessions_rejects_second_initialize() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                max_sessions: Some(1),
                ..Default::default()
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        assert!(first.headers().get("mcp-session-id").is_some());

        let second = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(second.headers().get("mcp-session-id").is_none());
        let body = response_body_text(second).await;
        assert!(body.contains("max"));
        assert!(body.contains("session"));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_session_max_lifetime_expires_session_between_requests() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                max_lifetime_secs: Some(1),
                ..Default::default()
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let initialize_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize_request_body()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let session_id = initialize_response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .expect("session id");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let follow_up = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("mcp-session-id", session_id)
                    .body(Body::from(initialized_notification_body()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(follow_up.status(), StatusCode::NOT_FOUND);
        assert!(follow_up.headers().get("mcp-session-id").is_none());
        let body = response_body_text(follow_up).await;
        assert!(body.contains("Session not found"));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_session_creation_rate_global_rejects_burst_initializes() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                creation_rate: Some(crate::config::StreamableHttpRateLimitConfig {
                    enabled: Some(true),
                    global: Some(crate::config::StreamableHttpRateBucketConfig {
                        capacity: 1,
                        refill_per_sec: 1,
                    }),
                    per_ip: None,
                }),
                ..Default::default()
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        assert!(first.headers().get("mcp-session-id").is_some());

        let second = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(second.headers().get("mcp-session-id").is_none());
        let body = response_body_text(second).await;
        assert!(body.contains("session creation rate limit exceeded"));
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_session_creation_rate_disabled_allows_burst_initializes() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                creation_rate: Some(crate::config::StreamableHttpRateLimitConfig {
                    enabled: Some(false),
                    global: Some(crate::config::StreamableHttpRateBucketConfig {
                        capacity: 1,
                        refill_per_sec: 1,
                    }),
                    per_ip: None,
                }),
                ..Default::default()
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        assert!(first.headers().get("mcp-session-id").is_some());

        let second = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::OK);
        assert!(second.headers().get("mcp-session-id").is_some());
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_inbound_rate_limit_global_rejects_second_request() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            rate_limit: Some(crate::config::StreamableHttpRateLimitConfig {
                enabled: Some(true),
                global: Some(crate::config::StreamableHttpRateBucketConfig {
                    capacity: 1,
                    refill_per_sec: 1,
                }),
                per_ip: None,
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);

        let second = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        let second_body = response_body_text(second).await;
        assert!(second_body.contains("Too Many Requests"));

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let third = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(third.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_inbound_rate_limit_disabled_allows_second_request() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            rate_limit: Some(crate::config::StreamableHttpRateLimitConfig {
                enabled: Some(false),
                global: Some(crate::config::StreamableHttpRateBucketConfig {
                    capacity: 1,
                    refill_per_sec: 1,
                }),
                per_ip: None,
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);

        let second = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .body(Body::from(initialize))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[cfg(feature = "http_hardening")]
    async fn hardening_inbound_rate_limit_per_ip_isolated_by_x_forwarded_for() {
        let mut config = base_config();
        config.server.transport.streamable_http.hardening = Some(StreamableHttpHardeningConfig {
            rate_limit: Some(crate::config::StreamableHttpRateLimitConfig {
                enabled: Some(true),
                global: None,
                per_ip: Some(crate::config::StreamableHttpPerIpRateBucketConfig {
                    capacity: 1,
                    refill_per_sec: 1,
                    key_source: crate::config::StreamableHttpRateLimitKeySource::XForwardedFor,
                }),
            }),
            ..StreamableHttpHardeningConfig::default()
        });
        let router = build_router(&config, PluginRegistry::new()).expect("router");
        let initialize = initialize_request_body();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);

        let second = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("x-forwarded-for", "10.0.0.2")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second.status(), StatusCode::OK);

        let third = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::from(initialize.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
        let third_body = response_body_text(third).await;
        assert!(third_body.contains("Too Many Requests"));

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let after_refill = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::from(initialize))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(after_refill.status(), StatusCode::OK);
    }

    #[test]
    fn engine_handler_factory_returns_clone() {
        let config = base_config();
        let engine = crate::Engine::from_config(EngineConfig {
            config,
            plugins: PluginRegistry::default(),
            list_refresh_handle: None,
        })
        .expect("engine");
        let factory = engine_handler_factory(Arc::new(engine));
        let cloned = factory().expect("cloned engine");
        assert!(cloned.list_tools().is_empty());
    }

    #[tokio::test]
    async fn router_plugins_wrap_endpoint_path_builds() {
        let mut config = base_config();
        config.plugins = vec![router_router_plugin_config(
            "wrap",
            vec![wrap_target("/mcp")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(TestPlugin {
                name: "wrap",
                label: "E",
            })
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_ne!(response.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get("x-order")
                .and_then(|v| v.to_str().ok()),
            Some("E")
        );
    }

    #[tokio::test]
    #[cfg(feature = "auth")]
    async fn build_streamable_http_router_applies_auth_layer() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![crate::config::AuthProviderConfig::bearer(
            "static", "secret",
        )];
        auth.enabled = Some(true);
        let engine = crate::Engine::from_config(EngineConfig {
            config: config.clone(),
            plugins: PluginRegistry::default(),
            list_refresh_handle: None,
        })
        .expect("engine should build");
        let auth_state = build_auth_state_with_plugins(&config, None).expect("auth state builds");
        let router = build_streamable_http_router(
            Arc::new(engine),
            &auth_state,
            &Arc::new(PluginRegistry::default()),
        )
        .expect("router");

        let unauthorized = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), axum::http::StatusCode::UNAUTHORIZED);

        let authorized = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_ne!(authorized.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[cfg(feature = "auth")]
    async fn router_plugins_auth_wrap_enforces_bearer() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![crate::config::AuthProviderConfig::bearer(
            "static", "secret",
        )];
        auth.enabled = Some(true);
        config.plugins = vec![router_router_plugin_config(
            "auth_wrap",
            vec![route_target("/health"), wrap_target("/health")],
        )];
        let registry = PluginRegistry::new()
            .register_http_router(AuthWrapPlugin)
            .unwrap();
        let router = build_router(&config, registry).expect("router");
        let request: Request<Body> = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), 401);
        assert!(!response.headers().contains_key(WWW_AUTHENTICATE));

        let request: Request<Body> = Request::builder()
            .uri("/health")
            .header("Authorization", "Bearer secret")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), 200);
        assert!(!response.headers().contains_key(WWW_AUTHENTICATE));
    }
}
