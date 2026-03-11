#![cfg(feature = "auth")]

#[allow(dead_code)]
mod auth_common;

use std::sync::Arc;

use rust_mcp_core::{build_auth_state_from_config, build_auth_state_with_plugins, PluginRegistry};

use auth_common::{
    build_hs256_jwks, build_hs256_token, fixture_path, run_protected_request, AllowAllPlugin,
};

#[tokio::test]
async fn build_auth_state_from_config_fixture() {
    use axum::{body::Body, http::Request};
    use rust_mcp_core::oauth_router;
    use tower::ServiceExt;

    let config =
        rust_mcp_core::load_mcp_config_from_path(fixture_path("auth/auth_from_config_fixture"))
            .expect("config should load");
    let state = build_auth_state_from_config(&config).expect("auth state should build");
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
        Some("http://example.com/custom")
    );
    let authorization_servers = value
        .get("authorization_servers")
        .and_then(serde_json::Value::as_array)
        .expect("authorization_servers array should be present");
    assert_eq!(authorization_servers.len(), 1);
    assert_eq!(
        authorization_servers
            .first()
            .and_then(serde_json::Value::as_str),
        Some("https://issuer.example.com")
    );
}

#[test]
fn auth_absent_builds_disabled_state_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_from_config_auth_absent_fixture",
    ))
    .expect("config should load");
    let state = build_auth_state_from_config(&config).expect("auth state should build");
    assert!(
        !state.auth_enabled() && !state.oauth_enabled(),
        "auth should be disabled when server.auth is absent"
    );
}

#[test]
fn auth_disabled_allows_non_empty_provider_list_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_from_config_auth_disabled_with_providers_fixture",
    ))
    .expect("config should load");
    let state = build_auth_state_from_config(&config).expect("auth state should build");
    assert!(
        !state.auth_enabled() && !state.oauth_enabled(),
        "auth.enabled=false should disable auth even when providers are configured"
    );
}

#[test]
fn auth_disabled_allows_empty_provider_list_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_from_config_auth_disabled_empty_providers_fixture",
    ))
    .expect("config should load");
    let state = build_auth_state_from_config(&config).expect("auth state should build");
    assert!(
        !state.auth_enabled() && !state.oauth_enabled(),
        "auth.enabled=false with empty providers should remain disabled"
    );
}

#[tokio::test]
#[cfg(feature = "streamable_http")]
async fn ordered_provider_chain_falls_back_to_plugin_fixture() {
    use axum::{
        body::Body,
        http::{header::AUTHORIZATION, Request, StatusCode},
        middleware::from_fn_with_state,
        routing::get,
        Router,
    };
    use rust_mcp_core::auth_middleware;
    use tower::ServiceExt;

    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_from_config_ordered_bearer_then_plugin_fixture",
    ))
    .expect("config should load");
    let registry = Arc::new(PluginRegistry::new().register_auth(AllowAllPlugin).unwrap());
    let state =
        build_auth_state_with_plugins(&config, Some(registry)).expect("auth state should build");
    let app = Router::new()
        .route("/mcp", get(|| async { StatusCode::OK }))
        .layer(from_fn_with_state(state, auth_middleware));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header(AUTHORIZATION, "Bearer not-the-static-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "request should fall through bearer mismatch and be accepted by plugin provider"
    );
}

#[test]
fn auth_plugins_missing_registry_returns_error_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_plugins_missing_registry_fixture",
    ))
    .expect("config should load");
    let err = build_auth_state_from_config(&config).expect_err("should fail without registry");
    assert!(
        err.message
            .contains("auth plugins configured but no registry provided"),
        "expected missing registry error, got: {}",
        err.message
    );
}

#[test]
fn auth_plugins_unknown_returns_error_fixture() {
    let config =
        rust_mcp_core::load_mcp_config_from_path(fixture_path("auth/auth_plugins_unknown_fixture"))
            .expect("config should load");
    let registry = Arc::new(PluginRegistry::new());
    let err = build_auth_state_with_plugins(&config, Some(registry))
        .expect_err("should fail for unknown plugin");
    assert!(
        err.message.contains("auth plugin not registered"),
        "expected unknown plugin error, got: {}",
        err.message
    );
    assert!(
        err.message.contains("allow-all"),
        "error should name the unregistered plugin 'allow-all', got: {}",
        err.message
    );
}

#[test]
fn auth_provider_plugin_not_allowlisted_returns_error_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_provider_plugin_not_allowlisted_fixture",
    ))
    .expect("config should load");
    let registry = Arc::new(PluginRegistry::new().register_auth(AllowAllPlugin).unwrap());
    let err = build_auth_state_with_plugins(&config, Some(registry))
        .expect_err("should fail for provider plugin not allowlisted");
    assert!(
        err.message.contains("auth provider plugin not allowlisted"),
        "expected provider plugin allowlist error, got: {}",
        err.message
    );
    assert!(
        err.message.contains("other-plugin"),
        "error should name the non-allowlisted provider plugin 'other-plugin', got: {}",
        err.message
    );
}

#[test]
fn auth_plugins_warns_on_extra_registered_plugins_fixture() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_plugins_warn_extra_registry_fixture",
    ))
    .expect("config should load");
    let registry = Arc::new(PluginRegistry::new().register_auth(AllowAllPlugin).unwrap());
    let state = build_auth_state_with_plugins(&config, Some(registry))
        .expect("extra registered plugins should not cause failure");
    assert!(
        !state.auth_enabled() && !state.oauth_enabled(),
        "expected auth disabled from fixture"
    );
}

#[test]
fn auth_mode_none_warns_when_settings_present() {
    let config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_mode_none_warns_settings_fixture",
    ))
    .expect("config should load");
    let state = build_auth_state_with_plugins(&config, None).expect("auth state");
    assert!(!state.auth_enabled() && !state.oauth_enabled());
}

async fn assert_authorized_without_challenge(
    state: Arc<rust_mcp_core::AuthState>,
    auth_header: String,
    reason: &str,
) {
    let (status, header) = run_protected_request(state, Some(auth_header)).await;
    assert_eq!(status.as_u16(), 200, "{reason}");
    assert!(
        header.is_none(),
        "successful auth should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn ordered_provider_chain_supports_bearer_then_jwks_then_introspection_fixture() {
    use httpmock::prelude::*;
    use serde_json::json;

    let secret = "phase5-jwks-secret";
    let kid = "phase5-kid";
    let server = MockServer::start();
    let jwks_mock = server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(build_hs256_jwks(secret, kid));
    });
    let introspection_mock = server.mock(|when, then| {
        when.method(POST)
            .path("/introspect")
            .header_exists("authorization")
            .body_includes("token=opaque-good");
        then.status(200).json_body(json!({
            "active": true,
            "scope": "mcp.read",
            "aud": "api://mcp",
            "iss": "https://issuer.opaque.example.com"
        }));
    });

    let mut config = rust_mcp_core::load_mcp_config_from_path(fixture_path(
        "auth/auth_from_config_ordered_bearer_jwks_introspection_fixture",
    ))
    .expect("config should load");
    let auth = config.server.auth_mut_or_insert();
    let jwt_provider = auth
        .providers
        .iter_mut()
        .find_map(rust_mcp_core::config::AuthProviderConfig::as_jwks_mut)
        .expect("fixture should include jwks provider");
    jwt_provider.jwks_url = Some(format!("{}/jwks", server.base_url()));
    let opaque_provider = auth
        .providers
        .iter_mut()
        .find_map(rust_mcp_core::config::AuthProviderConfig::as_introspection_mut)
        .expect("fixture should include introspection provider");
    opaque_provider.introspection_url = format!("{}/introspect", server.base_url());

    let state = build_auth_state_from_config(&config).expect("auth state should build");
    assert_authorized_without_challenge(
        Arc::clone(&state),
        "Bearer expected-token".to_owned(),
        "matching bearer provider should authenticate before oauth providers",
    )
    .await;

    let jwt_token = build_hs256_token(
        "https://issuer.jwt.example.com",
        "api://mcp",
        "mcp.read",
        kid,
        secret,
    );
    assert_authorized_without_challenge(
        Arc::clone(&state),
        format!("Bearer {jwt_token}"),
        "jwt provider should authenticate when bearer token does not match static token",
    )
    .await;

    assert_authorized_without_challenge(
        state,
        "Bearer opaque-good".to_owned(),
        "introspection provider should authenticate opaque token after bearer/jwks miss",
    )
    .await;

    jwks_mock.assert_calls(1);
    introspection_mock.assert_calls(1);
}
