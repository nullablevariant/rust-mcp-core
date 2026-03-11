//! Auth middleware and OAuth protected resource metadata endpoint.

use std::sync::Arc;

use serde_json::Value;

use super::authorization::{AuthDecision, AuthState};

pub(crate) fn oauth_resource_metadata_endpoint_path(endpoint_path: &str) -> String {
    if endpoint_path == "/" {
        crate::http::OAUTH_METADATA_PATH.to_owned()
    } else {
        format!("{}{}", crate::http::OAUTH_METADATA_PATH, endpoint_path)
    }
}

#[cfg(feature = "streamable_http")]
use axum::{
    body::Body,
    extract::State,
    http::{header::WWW_AUTHENTICATE, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
#[cfg(feature = "streamable_http")]
use serde_json::Map;

// Axum middleware that validates bearer tokens against the configured [`AuthState`].
//
// Returns `401 Unauthorized` or `403 Forbidden` with a `WWW-Authenticate`
// header when validation fails. Apply to routes via `axum::middleware::from_fn_with_state`.
//
// Requires the `streamable_http` feature.
#[cfg(feature = "streamable_http")]
#[doc(hidden)]
pub async fn auth_middleware(
    State(state): State<Arc<AuthState>>,
    headers: http::HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response {
    match state.authorize(&headers).await {
        AuthDecision::Allow => next.run(request).await,
        AuthDecision::Unauthorized { scope } => {
            let mut response = StatusCode::UNAUTHORIZED.into_response();
            if let Some(header) = state.www_authenticate(scope.as_deref(), false) {
                // www_authenticate() only emits RFC-compatible challenge strings,
                // so parsing into HeaderValue must succeed.
                let value = header.parse().expect("valid WWW-Authenticate header value");
                response.headers_mut().insert(WWW_AUTHENTICATE, value);
            }
            response
        }
        AuthDecision::Forbidden { scope } => {
            let mut response = StatusCode::FORBIDDEN.into_response();
            if let Some(header) = state.www_authenticate(scope.as_deref(), true) {
                // www_authenticate() only emits RFC-compatible challenge strings,
                // so parsing into HeaderValue must succeed.
                let value = header.parse().expect("valid WWW-Authenticate header value");
                response.headers_mut().insert(WWW_AUTHENTICATE, value);
            }
            response
        }
    }
}

// Build an Axum [`Router`] serving the `/.well-known/oauth-protected-resource` metadata endpoint.
//
// Mounts the metadata handler at both the canonical well-known path and
// the endpoint-specific path (e.g., `/.well-known/oauth-protected-resource/mcp`).
//
// Requires the `streamable_http` feature.
#[cfg(feature = "streamable_http")]
#[doc(hidden)]
pub fn oauth_router(state: Arc<AuthState>) -> Router {
    let endpoint_metadata_path = oauth_resource_metadata_endpoint_path(state.endpoint_path());
    let mut router = Router::new().route(
        crate::http::OAUTH_METADATA_PATH,
        get(oauth_resource_metadata),
    );
    if endpoint_metadata_path != crate::http::OAUTH_METADATA_PATH {
        router = router.route(&endpoint_metadata_path, get(oauth_resource_metadata));
    }
    router.with_state(state)
}

#[cfg(feature = "streamable_http")]
async fn oauth_resource_metadata(State(state): State<Arc<AuthState>>) -> impl IntoResponse {
    let Some(resource) = state.resource() else {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    };
    let mut payload = Map::new();
    payload.insert("resource".to_owned(), Value::String(resource.to_owned()));

    let mut issuers = Vec::new();
    for provider in state.providers() {
        if let Some(issuer) = provider.issuer(state.http_client()).await {
            issuers.push(Value::String(issuer));
        }
    }
    if !issuers.is_empty() {
        payload.insert("authorization_servers".to_owned(), Value::Array(issuers));
    }
    if let Some(document_url) = state.oauth_client_metadata_document_url() {
        payload.insert(
            "oauth_client_metadata_document_url".to_owned(),
            Value::String(document_url.to_owned()),
        );
    }

    Json(Value::Object(payload)).into_response()
}

#[cfg(test)]
// Inline tests here cover private OAuth metadata handler behavior and route
// wiring branches that are not directly reachable from integration tests
// without exposing additional internals.
mod tests {
    use super::{oauth_resource_metadata, oauth_resource_metadata_endpoint_path, oauth_router};
    use crate::auth::inbound::authorization::AuthStateInit;
    use crate::auth::inbound::provider::AuthProvider;
    use crate::auth::oauth::token_exchange::HttpTokenExchanger;
    use crate::auth::{AuthActivation, AuthState};
    use crate::http::client::ReqwestHttpClient;
    use crate::inline_test_fixtures::base_provider;
    use axum::extract::State;
    use axum::http::{Request, StatusCode};
    use axum::{body::Body, response::IntoResponse};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[test]
    fn oauth_resource_metadata_endpoint_path_cases() {
        assert_eq!(
            oauth_resource_metadata_endpoint_path("/"),
            "/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            oauth_resource_metadata_endpoint_path("/mcp"),
            "/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            oauth_resource_metadata_endpoint_path("/custom/"),
            "/.well-known/oauth-protected-resource/custom/"
        );
        assert_eq!(
            oauth_resource_metadata_endpoint_path("/custom/v1"),
            "/.well-known/oauth-protected-resource/custom/v1"
        );
    }

    #[tokio::test]
    async fn oauth_resource_metadata_includes_client_metadata_document_url() {
        let state = Arc::new(AuthState::new(AuthStateInit {
            activation: AuthActivation::OauthOnly,
            endpoint_path: "/mcp".to_owned(),
            bearer_token: None,
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers: vec![{
                let mut provider = base_provider();
                provider
                    .as_jwks_mut()
                    .expect("base_provider must be jwks")
                    .issuer = Some("https://issuer.example.com".to_owned());
                AuthProvider::new(provider, None, None)
            }],
            plugins: None,
            auth_plugin_configs: HashMap::new(),
            scope_challenges_enabled: true,
            oauth_client_metadata_document_url: Some(
                "https://client.example.com/metadata.json".to_owned(),
            ),
            http: Arc::new(ReqwestHttpClient::default()),
            token_exchanger: Arc::new(HttpTokenExchanger),
            outbound_timeout_ms: None,
            outbound_max_response_bytes: None,
        }));

        let response = oauth_resource_metadata(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let payload: Value = serde_json::from_slice(&body).expect("json payload");
        assert_eq!(
            payload.get("resource"),
            Some(&Value::String("http://example.com/mcp".to_owned()))
        );
        assert_eq!(
            payload.get("authorization_servers"),
            Some(&Value::Array(vec![Value::String(
                "https://issuer.example.com".to_owned()
            )]))
        );
        assert_eq!(
            payload.get("oauth_client_metadata_document_url"),
            Some(&Value::String(
                "https://client.example.com/metadata.json".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn oauth_router_mounts_endpoint_specific_metadata_route() {
        let state = Arc::new(AuthState::new(AuthStateInit {
            activation: AuthActivation::OauthOnly,
            endpoint_path: "/custom".to_owned(),
            bearer_token: None,
            resource: Some("http://example.com/custom".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource/custom".to_owned(),
            ),
            providers: vec![AuthProvider::new(base_provider(), None, None)],
            plugins: None,
            auth_plugin_configs: HashMap::new(),
            scope_challenges_enabled: true,
            oauth_client_metadata_document_url: None,
            http: Arc::new(ReqwestHttpClient::default()),
            token_exchanger: Arc::new(HttpTokenExchanger),
            outbound_timeout_ms: None,
            outbound_max_response_bytes: None,
        }));

        let app = oauth_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource/custom")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let canonical_response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(canonical_response.status(), StatusCode::OK);
    }
}
