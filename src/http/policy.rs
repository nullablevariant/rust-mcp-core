//! Route access policy enforcement for streamable HTTP transport.
use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header::ALLOW, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::config::StreamableHttpSessionMode;

use super::{LAST_EVENT_ID_HEADER, MCP_SESSION_ID_HEADER};

const SESSION_REMAP_BODY_MAX_BYTES: usize = 4096;

#[derive(Clone)]
pub(crate) struct StreamableHttpRoutePolicy {
    pub(crate) enable_get_stream: bool,
    pub(crate) enable_sse_resumption: bool,
    pub(crate) allow_delete_session: bool,
    pub(crate) session_mode: StreamableHttpSessionMode,
}

impl StreamableHttpRoutePolicy {
    pub(crate) const fn from_config(config: &crate::config::McpConfig) -> Self {
        let transport = &config.server.transport.streamable_http;
        Self {
            enable_get_stream: transport.enable_get_stream,
            enable_sse_resumption: transport.enable_sse_resumption,
            allow_delete_session: transport.allow_delete_session,
            session_mode: transport.session_mode,
        }
    }
}

pub(crate) async fn streamable_http_route_guard(
    policy: StreamableHttpRoutePolicy,
    request: Request<Body>,
    next: Next,
) -> Response {
    let enable_get_stream = policy.enable_get_stream;
    let enable_sse_resumption = policy.enable_sse_resumption;
    let allow_delete_session = policy.allow_delete_session;
    let session_mode = policy.session_mode;
    let method = request.method().clone();

    if method == Method::GET && !enable_get_stream {
        let allow = if allow_delete_session {
            "DELETE, POST"
        } else {
            "POST"
        };
        return method_not_allowed_response(allow);
    }
    if method == Method::DELETE && !allow_delete_session {
        let allow = if enable_get_stream {
            "GET, POST"
        } else {
            "POST"
        };
        return method_not_allowed_response(allow);
    }
    if method == Method::GET
        && !enable_sse_resumption
        && request.headers().contains_key(LAST_EVENT_ID_HEADER)
    {
        return (
            StatusCode::BAD_REQUEST,
            "SSE resumption is disabled for this server",
        )
            .into_response();
    }
    if session_mode == StreamableHttpSessionMode::Required
        && matches!(method, Method::GET | Method::DELETE)
        && !request.headers().contains_key(MCP_SESSION_ID_HEADER)
    {
        return (StatusCode::BAD_REQUEST, "MCP-Session-Id header is required").into_response();
    }

    let response = next.run(request).await;
    remap_session_status(response).await
}

pub(crate) fn method_not_allowed_response(allow: &str) -> Response {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header(ALLOW, allow)
        .body(Body::from("Method Not Allowed"))
        .expect("valid method not allowed response")
}

pub(crate) async fn remap_session_status(response: Response) -> Response {
    if response.status() != StatusCode::UNAUTHORIZED {
        return response;
    }

    let (parts, body) = response.into_parts();
    let Ok(body_bytes) = to_bytes(body, SESSION_REMAP_BODY_MAX_BYTES).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let remapped = classify_session_status(&body_bytes);
    if remapped == StatusCode::UNAUTHORIZED {
        Response::from_parts(parts, Body::from(body_bytes))
    } else {
        (remapped, body_bytes).into_response()
    }
}

// rmcp currently returns session errors as plain text. Keep this mapping centralized so
// a future rmcp structured error code can replace message matching in one place.
fn classify_session_status(body_bytes: &[u8]) -> StatusCode {
    let text = String::from_utf8_lossy(body_bytes);
    if text.contains("Session ID is required") {
        return StatusCode::BAD_REQUEST;
    }
    if text.contains("Session not found") {
        return StatusCode::NOT_FOUND;
    }
    StatusCode::UNAUTHORIZED
}

#[cfg(test)]
mod tests {
    use super::{remap_session_status, SESSION_REMAP_BODY_MAX_BYTES};
    use crate::config::StreamableHttpSessionMode;
    use crate::http::{LAST_EVENT_ID_HEADER, MCP_SESSION_ID_HEADER};
    use crate::inline_test_fixtures::{base_config, build_router};
    use crate::plugins::PluginRegistry;
    use axum::body::{to_bytes, Body};
    use axum::http::header::ALLOW;
    use axum::http::{Request, StatusCode};
    use axum::response::IntoResponse;
    use std::collections::BTreeSet;
    use tower::ServiceExt;

    async fn response_body_text(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    fn parse_allow_set(response: &axum::response::Response) -> BTreeSet<String> {
        response
            .headers()
            .get(ALLOW)
            .and_then(|value| value.to_str().ok())
            .map(|allow| {
                allow
                    .split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn transport_guard_rejects_get_when_disabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.enable_get_stream = false;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
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
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            parse_allow_set(&response),
            BTreeSet::from([String::from("POST")])
        );
        assert_eq!(response_body_text(response).await, "Method Not Allowed");
    }

    #[tokio::test]
    async fn transport_guard_rejects_delete_when_disabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.allow_delete_session = false;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            parse_allow_set(&response),
            BTreeSet::from([String::from("GET"), String::from("POST")])
        );
        assert_eq!(response_body_text(response).await, "Method Not Allowed");
    }

    #[tokio::test]
    async fn transport_guard_rejects_delete_when_get_also_disabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.enable_get_stream = false;
        config.server.transport.streamable_http.allow_delete_session = false;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            parse_allow_set(&response),
            BTreeSet::from([String::from("POST")])
        );
        assert_eq!(response_body_text(response).await, "Method Not Allowed");
    }

    #[tokio::test]
    async fn transport_guard_rejects_get_with_delete_allowlist_when_enabled() {
        let mut config = base_config();
        config.server.transport.streamable_http.enable_get_stream = false;
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::Optional;
        config.server.transport.streamable_http.allow_delete_session = true;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
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
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            parse_allow_set(&response),
            BTreeSet::from([String::from("DELETE"), String::from("POST")])
        );
        assert_eq!(response_body_text(response).await, "Method Not Allowed");
    }

    #[tokio::test]
    async fn transport_guard_rejects_last_event_id_when_resumption_disabled() {
        let config = base_config();
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mcp")
                    .header("accept", "text/event-stream")
                    .header(LAST_EVENT_ID_HEADER, "5")
                    .header(MCP_SESSION_ID_HEADER, "missing")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_body_text(response).await,
            "SSE resumption is disabled for this server"
        );
    }

    #[tokio::test]
    async fn transport_guard_maps_missing_session_id_to_bad_request() {
        let config = base_config();
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
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
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body_text(response).await;
        assert!(body.contains("Session ID is required"));
    }

    #[tokio::test]
    async fn transport_guard_maps_unknown_session_id_to_not_found() {
        let config = base_config();
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mcp")
                    .header("accept", "text/event-stream")
                    .header(MCP_SESSION_ID_HEADER, "missing-session")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_body_text(response).await;
        assert!(body.contains("Session not found"));
    }

    #[tokio::test]
    async fn transport_guard_required_session_mode_requires_header_on_get() {
        let mut config = base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::Required;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
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
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_body_text(response).await,
            "MCP-Session-Id header is required"
        );
    }

    #[tokio::test]
    async fn transport_guard_required_session_mode_requires_header_on_delete() {
        let mut config = base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::Required;
        config.server.transport.streamable_http.allow_delete_session = true;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_body_text(response).await,
            "MCP-Session-Id header is required"
        );
    }

    #[tokio::test]
    async fn transport_guard_allows_last_event_id_when_resumption_enabled() {
        let mut config = base_config();
        config
            .server
            .transport
            .streamable_http
            .enable_sse_resumption = true;
        let router = build_router(&config, PluginRegistry::new()).expect("router");

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mcp")
                    .header("accept", "text/event-stream")
                    .header(LAST_EVENT_ID_HEADER, "5")
                    .header(MCP_SESSION_ID_HEADER, "missing-session")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn remap_session_status_preserves_non_session_unauthorized() {
        let response = remap_session_status(
            (StatusCode::UNAUTHORIZED, "plain unauthorized error").into_response(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_body_text(response).await,
            "plain unauthorized error"
        );
    }

    #[tokio::test]
    async fn remap_session_status_preserves_non_unauthorized_responses() {
        let response = remap_session_status((StatusCode::OK, "ok").into_response()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_body_text(response).await, "ok");
    }

    #[tokio::test]
    async fn remap_session_status_handles_oversized_unauthorized_body() {
        let oversized = "x".repeat(SESSION_REMAP_BODY_MAX_BYTES + 1);
        let response =
            remap_session_status((StatusCode::UNAUTHORIZED, oversized.clone()).into_response())
                .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        assert!(body.is_empty());
    }
}
