#![cfg(feature = "auth")]

#[allow(dead_code)]
mod auth_common;

use axum::{body::Body, http::Request};
use httpmock::{Method::GET, MockServer};
use rust_mcp_core::{build_auth_state, oauth_router, AuthActivation, AuthStateParams};
use tower::ServiceExt;

use rust_mcp_core::config::AuthProviderConfig;

use auth_common::{load_fixture, DiscoveryStatusFixture};

#[tokio::test]
async fn auth_oauth_discovery_status_error_fixture() {
    let fixture: DiscoveryStatusFixture =
        load_fixture("auth/auth_oauth_discovery_status_error_fixture");
    let server = MockServer::start();

    let discovery_mock = server.mock(|when, then| {
        when.method(GET).path("/discovery");
        then.status(500).body("error");
    });

    let mut provider = AuthProviderConfig::jwks("jwt");
    provider.as_jwks_mut().expect("jwks provider").discovery_url =
        Some(format!("{}/discovery", server.base_url()));

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(fixture.resource.clone()),
        resource_metadata_url: Some(
            "http://example.com/.well-known/oauth-protected-resource".to_owned(),
        ),
        providers: vec![provider],
        ..Default::default()
    });

    let app = oauth_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-protected-resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("resource").and_then(|v| v.as_str()),
        Some(fixture.resource.as_str()),
        "resource field must match fixture"
    );
    assert!(
        value.get("authorization_servers").is_none(),
        "authorization_servers must be absent when discovery fails with status error"
    );
    discovery_mock.assert_calls(1);
}

#[tokio::test]
async fn auth_oauth_discovery_json_error_fixture() {
    let fixture: DiscoveryStatusFixture =
        load_fixture("auth/auth_oauth_discovery_json_error_fixture");
    let server = MockServer::start();

    let discovery_mock = server.mock(|when, then| {
        when.method(GET).path("/discovery");
        then.status(200).body("not-json");
    });

    let mut provider = AuthProviderConfig::jwks("jwt");
    provider.as_jwks_mut().expect("jwks provider").discovery_url =
        Some(format!("{}/discovery", server.base_url()));

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(fixture.resource.clone()),
        resource_metadata_url: Some(
            "http://example.com/.well-known/oauth-protected-resource".to_owned(),
        ),
        providers: vec![provider],
        ..Default::default()
    });

    let app = oauth_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-protected-resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("resource").and_then(|v| v.as_str()),
        Some(fixture.resource.as_str()),
        "resource field must match fixture"
    );
    assert!(
        value.get("authorization_servers").is_none(),
        "authorization_servers must be absent when discovery returns invalid JSON"
    );
    discovery_mock.assert_calls(1);
}

#[tokio::test]
async fn auth_oauth_discovery_request_error_fixture() {
    let fixture: DiscoveryStatusFixture =
        load_fixture("auth/auth_oauth_discovery_status_error_fixture");

    let mut provider = AuthProviderConfig::jwks("jwt");
    provider.as_jwks_mut().expect("jwks provider").discovery_url =
        Some("http://127.0.0.1:1/discovery".to_owned());

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(fixture.resource.clone()),
        resource_metadata_url: Some(
            "http://example.com/.well-known/oauth-protected-resource".to_owned(),
        ),
        providers: vec![provider],
        ..Default::default()
    });

    let app = oauth_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-protected-resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value.get("resource").and_then(|v| v.as_str()),
        Some(fixture.resource.as_str()),
        "resource field must match fixture even on request error"
    );
    assert!(
        value.get("authorization_servers").is_none(),
        "authorization_servers must be absent when discovery request fails (connection refused)"
    );
}
