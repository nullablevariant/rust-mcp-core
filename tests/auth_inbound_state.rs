#![cfg(feature = "auth")]

#[allow(dead_code)]
mod auth_common;

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::Utc;
use httpmock::{Method::GET, Method::POST, MockServer};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rust_mcp_core::{
    build_auth_state, oauth_router, AuthActivation, AuthStateParams, PluginRegistry,
};
use serde_json::json;
use tower::ServiceExt;

use rust_mcp_core::config::IntrospectionClientAuthMethod;

use auth_common::{
    build_hs256_jwks, build_hs256_token, build_hs256_token_with_claims, build_invalid_header_token,
    build_jwt_provider_config, build_opaque_provider_config, load_fixture, oauth_state,
    run_protected_request, run_simple_auth_fixture, AllFallbackFixture, AllowAllPlugin,
    ConfigCheckPlugin, InvalidJwtClaimsFixture, JwtErrorCasesFixture, JwtFixture,
    NormalizePathFixture, OpaqueErrorCasesFixture, OpaqueFixture, ProviderSelectionFixture,
    ResourceMetadataFixture, ResourceMissingFixture, SimpleAuthFixture, EXAMPLE_RESOURCE,
    EXAMPLE_RESOURCE_METADATA_URL,
};

#[tokio::test]
async fn auth_none_allows_fixture() {
    run_simple_auth_fixture("auth/auth_none_allows_fixture").await;
}

#[tokio::test]
async fn auth_bearer_accept_fixture() {
    run_simple_auth_fixture("auth/auth_bearer_accept_fixture").await;
}

#[tokio::test]
async fn auth_bearer_reject_fixture() {
    run_simple_auth_fixture("auth/auth_bearer_reject_fixture").await;
}

#[tokio::test]
async fn auth_oauth_missing_token_fixture() {
    run_simple_auth_fixture("auth/auth_oauth_missing_token_fixture").await;
}

#[tokio::test]
async fn auth_oauth_jwt_accept_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwt_accept_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });
    server.mock(|when, then| {
        when.method(GET).path("/discovery");
        then.status(200).json_body(json!({
            "jwks_uri": format!("{}/jwks", server.base_url()),
            "issuer": fixture.token_issuer
        }));
    });

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        kid,
        secret,
    );

    let mut provider = build_jwt_provider_config(
        "jwt",
        None,
        None,
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );
    provider
        .as_jwks_mut()
        .expect("jwt provider must be jwks")
        .discovery_url = Some(format!("{}/discovery", server.base_url()));
    provider
        .as_jwks_mut()
        .expect("jwt provider must be jwks")
        .clock_skew_sec = Some(30);

    let other = build_jwt_provider_config(
        "other",
        None,
        Some("http://other.example.com".to_owned()),
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        resource_metadata_url: Some(EXAMPLE_RESOURCE_METADATA_URL.to_owned()),
        providers: vec![provider, other],
        ..Default::default()
    });

    let (status, header) =
        run_protected_request(Arc::clone(&state), Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    assert!(
        header.is_none(),
        "successful JWT auth should not produce WWW-Authenticate header, got: {header:?}"
    );

    let (status_cached, header_cached) =
        run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status_cached.as_u16(), fixture.expected_status);
    assert!(
        header_cached.is_none(),
        "cached JWT auth should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_oauth_jwt_bad_scope_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwt_bad_scope_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        kid,
        secret,
    );

    let provider = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = oauth_state(vec![provider]);

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("403 should include WWW-Authenticate header");
    assert!(
        header_val.contains("error=\"insufficient_scope\""),
        "403 challenge should indicate insufficient_scope, got: {header_val}"
    );
    assert!(
        header_val.contains("scope=\"mcp.read\""),
        "403 challenge should include required scope, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_jwt_missing_iss_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwt_missing_iss_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });

    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(kid.to_owned());
    let claims = json!({
        "aud": fixture.token_audience,
        "scope": fixture.token_scope,
        "exp": (Utc::now().timestamp() + 3600)
    });
    let token = encode(
        &header,
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("token should encode");

    let provider = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );
    let other = build_jwt_provider_config(
        "other",
        None,
        Some("http://other.example.com".to_owned()),
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider, other],
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("401 should include WWW-Authenticate header");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_opaque_accept_fixture() {
    let fixture: OpaqueFixture = load_fixture("auth/auth_oauth_opaque_accept_fixture");
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(POST).path("/introspect");
        then.status(200).json_body(json!({
            "active": true,
            "scope": fixture.token_scope,
            "aud": fixture.audience,
            "iss": fixture.issuer
        }));
    });

    let mut provider = build_opaque_provider_config(
        "opaque",
        Some(format!("{}/introspect", server.base_url())),
        Some(fixture.issuer.clone()),
        vec![fixture.audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );
    let introspection = provider
        .as_introspection_mut()
        .expect("opaque provider must be introspection");
    introspection.client_id = Some("client".to_owned());
    introspection.client_secret = Some("secret".to_owned());

    let jwt_stub = build_jwt_provider_config(
        "jwt",
        None,
        Some("http://issuer.example.com".to_owned()),
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![jwt_stub, provider],
        ..Default::default()
    });

    let (status, _) = run_protected_request(state, Some("Bearer opaque-token".to_owned())).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
}

#[tokio::test]
async fn auth_oauth_introspection_basic_uses_basic_auth() {
    let server = MockServer::start();
    let encoded = STANDARD.encode("client:secret");

    server.mock(|when, then| {
        when.method(POST)
            .path("/introspect")
            .header("Authorization", format!("Basic {encoded}"))
            .body_includes("token=opaque-token");
        then.status(200).json_body(json!({
            "active": true,
            "scope": "mcp.read",
            "aud": "mcp",
            "iss": "http://issuer.example.com"
        }));
    });

    let mut provider = build_opaque_provider_config(
        "introspection",
        Some(format!("{}/introspect", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec!["mcp".to_owned()],
        vec!["mcp.read".to_owned()],
        HashMap::new(),
    );
    let introspection = provider
        .as_introspection_mut()
        .expect("opaque provider must be introspection");
    introspection.client_id = Some("client".to_owned());
    introspection.client_secret = Some("secret".to_owned());

    let state = oauth_state(vec![provider]);

    let (status, header) =
        run_protected_request(state, Some("Bearer opaque-token".to_owned())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        header.is_none(),
        "successful basic introspection should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_oauth_introspection_post_uses_form_params() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(POST)
            .path("/introspect")
            .body_includes("token=opaque-token")
            .body_includes("client_id=client")
            .body_includes("client_secret=secret");
        then.status(200).json_body(json!({
            "active": true,
            "scope": "mcp.read",
            "aud": "mcp",
            "iss": "http://issuer.example.com"
        }));
    });

    let mut provider = build_opaque_provider_config(
        "introspection",
        Some(format!("{}/introspect", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec!["mcp".to_owned()],
        vec!["mcp.read".to_owned()],
        HashMap::new(),
    );
    let introspection = provider
        .as_introspection_mut()
        .expect("opaque provider must be introspection");
    introspection.client_id = Some("client".to_owned());
    introspection.client_secret = Some("secret".to_owned());
    introspection.auth_method = IntrospectionClientAuthMethod::Post;

    let state = oauth_state(vec![provider]);

    let (status, header) =
        run_protected_request(state, Some("Bearer opaque-token".to_owned())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        header.is_none(),
        "successful post introspection should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_oauth_introspection_none_uses_token_only() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(POST)
            .path("/introspect")
            .body_includes("token=opaque-token")
            .body_excludes("client_id=")
            .body_excludes("client_secret=");
        then.status(200).json_body(json!({
            "active": true,
            "scope": "mcp.read",
            "aud": "mcp",
            "iss": "http://issuer.example.com"
        }));
    });

    let mut provider = build_opaque_provider_config(
        "introspection",
        Some(format!("{}/introspect", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec!["mcp".to_owned()],
        vec!["mcp.read".to_owned()],
        HashMap::new(),
    );
    provider
        .as_introspection_mut()
        .expect("opaque provider must be introspection")
        .auth_method = IntrospectionClientAuthMethod::None;

    let state = oauth_state(vec![provider]);

    let (status, header) =
        run_protected_request(state, Some("Bearer opaque-token".to_owned())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        header.is_none(),
        "successful none-method introspection should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_all_fallback_to_oauth_fixture() {
    let fixture: AllFallbackFixture = load_fixture("auth/auth_all_fallback_to_oauth_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        kid,
        secret,
    );

    let provider = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::BearerAndOauth,
        bearer_token: Some(fixture.bearer_token.clone()),
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider],
        ..Default::default()
    });

    let (status, _) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
}

#[tokio::test]
async fn auth_all_bearer_accept_fixture() {
    run_simple_auth_fixture("auth/auth_all_bearer_accept_fixture").await;
}

#[tokio::test]
async fn auth_oauth_no_providers_fixture() {
    run_simple_auth_fixture("auth/auth_oauth_no_providers_fixture").await;
}

#[tokio::test]
async fn auth_oauth_invalid_jwt_claims_no_iss_fixture() {
    let fixture: InvalidJwtClaimsFixture =
        load_fixture("auth/auth_oauth_invalid_jwt_claims_no_iss_fixture");
    let providers = vec![
        build_jwt_provider_config(
            "p1",
            None,
            Some("http://issuer.example.com".to_owned()),
            Vec::new(),
            Vec::new(),
            HashMap::new(),
        ),
        build_jwt_provider_config(
            "p2",
            None,
            Some("http://issuer.other.com".to_owned()),
            Vec::new(),
            Vec::new(),
            HashMap::new(),
        ),
    ];

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers,
        ..Default::default()
    });

    let (status, _) = run_protected_request(state, Some(fixture.auth_header)).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
}

#[tokio::test]
async fn auth_oauth_default_provider_fixture() {
    let fixture: ProviderSelectionFixture =
        load_fixture("auth/auth_oauth_default_provider_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });

    let claims = json!({
        "iss": "http://issuer.example.com",
        "aud": fixture.token_audience,
        "scope": fixture.token_scope,
        "exp": (Utc::now().timestamp() + 3600)
    });
    let token = build_hs256_token_with_claims(&claims, Some(kid), secret);

    let provider_default = build_jwt_provider_config(
        "p2",
        Some(format!("{}/jwks", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let providers = vec![
        build_jwt_provider_config(
            "p1",
            None,
            Some("http://issuer.other.com".to_owned()),
            Vec::new(),
            Vec::new(),
            HashMap::new(),
        ),
        provider_default,
    ];

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers,
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    assert!(
        header.is_none(),
        "successful default provider auth should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_oauth_desired_type_jwt_fixture() {
    let fixture: ProviderSelectionFixture =
        load_fixture("auth/auth_oauth_desired_type_jwt_fixture");
    let server = MockServer::start();
    let secret = "secret";
    let kid = "kid-1";
    let jwks = build_hs256_jwks(secret, kid);

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).json_body(jwks.clone());
    });

    let claims = json!({
        "iss": "http://issuer.example.com",
        "aud": fixture.token_audience,
        "scope": fixture.token_scope,
        "exp": (Utc::now().timestamp() + 3600)
    });
    let token = build_hs256_token_with_claims(&claims, Some(kid), secret);

    let provider_jwt = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some("http://issuer.example.com".to_owned()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let provider_opaque = build_opaque_provider_config(
        "opaque",
        Some("http://example.com/introspect".to_owned()),
        None,
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider_jwt, provider_opaque],
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    assert!(
        header.is_none(),
        "successful desired-type JWT auth should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_oauth_jwks_missing_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwks_missing_fixture");
    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        "kid-1",
        "secret",
    );

    let provider = build_jwt_provider_config(
        "jwt",
        None,
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = oauth_state(vec![provider]);

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("401 from missing JWKS should include WWW-Authenticate");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn auth_oauth_jwt_error_cases_fixture() {
    let fixture: JwtErrorCasesFixture = load_fixture("auth/auth_oauth_jwt_error_cases_fixture");
    for case in fixture.cases {
        let server = MockServer::start();
        let issuer = "https://issuer.example.com";
        let audience = "api://mcp";
        let secret = "secret";
        let kid = "kid-1";
        let mut required_claims = HashMap::new();

        let (token, jwks) = match case.name.as_str() {
            "invalid_header" => {
                let claims = json!({
                    "iss": issuer,
                    "aud": audience,
                    "scope": "mcp",
                    "exp": (Utc::now().timestamp() + 3600)
                });
                (
                    build_invalid_header_token(&claims),
                    build_hs256_jwks(secret, kid),
                )
            }
            "no_matching_kid" => {
                let claims = json!({
                    "iss": issuer,
                    "aud": audience,
                    "scope": "mcp",
                    "exp": (Utc::now().timestamp() + 3600)
                });
                (
                    build_hs256_token_with_claims(&claims, Some("kid-missing"), secret),
                    build_hs256_jwks(secret, kid),
                )
            }
            "invalid_jwk" => {
                let claims = json!({
                    "iss": issuer,
                    "aud": audience,
                    "scope": "mcp",
                    "exp": (Utc::now().timestamp() + 3600)
                });
                let token = build_hs256_token_with_claims(&claims, Some("bad-kid"), secret);
                let jwks = json!({
                    "keys": [
                        {"kty": "oct", "kid": "bad-kid", "alg": "HS256", "k": "@@@"}
                    ]
                });
                (token, jwks)
            }
            "decode_error" => {
                let claims = json!({
                    "iss": issuer,
                    "aud": audience,
                    "scope": "mcp",
                    "exp": (Utc::now().timestamp() + 3600)
                });
                let token = build_hs256_token_with_claims(&claims, Some(kid), "right-secret");
                let jwks = build_hs256_jwks("wrong-secret", kid);
                (token, jwks)
            }
            "required_claims_mismatch" => {
                required_claims.insert("azp".to_owned(), "client".to_owned());
                let claims = json!({
                    "iss": issuer,
                    "aud": audience,
                    "scope": "mcp",
                    "azp": "other",
                    "exp": (Utc::now().timestamp() + 3600)
                });
                (
                    build_hs256_token_with_claims(&claims, Some(kid), secret),
                    build_hs256_jwks(secret, kid),
                )
            }
            other => panic!("unknown jwt error case: {other}"),
        };

        let jwks_mock = server.mock(|when, then| {
            when.method(GET).path("/jwks");
            then.status(200).json_body(jwks);
        });

        let provider = build_jwt_provider_config(
            "jwt",
            Some(format!("{}/jwks", server.base_url())),
            Some(issuer.to_owned()),
            vec![audience.to_owned()],
            Vec::new(),
            required_claims,
        );

        let state = oauth_state(vec![provider]);

        let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
        assert_eq!(status.as_u16(), 401, "case {}", case.name);
        let header_val = header.unwrap_or_else(|| {
            panic!(
                "case {}: 401 should include WWW-Authenticate header",
                case.name
            )
        });
        assert!(
            header_val.starts_with("Bearer"),
            "case {}: challenge should start with Bearer, got: {header_val}",
            case.name
        );
        // JWT validation currently always fetches JWKS for these error classes.
        jwks_mock.assert_calls(1);
        match case.name.as_str() {
            "invalid_header"
            | "no_matching_kid"
            | "invalid_jwk"
            | "decode_error"
            | "required_claims_mismatch" => {
                assert!(
                    !header_val.contains("error=\"insufficient_scope\""),
                    "case {} should remain a 401 invalid-token class challenge, got: {header_val}",
                    case.name
                );
            }
            other => panic!("unknown jwt error case: {other}"),
        }
    }
}

#[tokio::test]
async fn auth_oauth_opaque_missing_url_fixture() {
    let fixture: SimpleAuthFixture = load_fixture("auth/auth_oauth_opaque_missing_url_fixture");
    let provider =
        build_opaque_provider_config("opaque", None, None, Vec::new(), Vec::new(), HashMap::new());
    let state = oauth_state(vec![provider]);

    let (status, _) = run_protected_request(state, fixture.auth_header).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
}

#[tokio::test]
async fn auth_oauth_opaque_request_error_fixture() {
    let fixture: SimpleAuthFixture = load_fixture("auth/auth_oauth_opaque_request_error_fixture");
    let provider = build_opaque_provider_config(
        "opaque",
        Some("http://[::1".to_owned()),
        None,
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );
    let state = oauth_state(vec![provider]);

    let (status, _) = run_protected_request(state, fixture.auth_header).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn auth_oauth_opaque_error_cases_fixture() {
    let fixture: OpaqueErrorCasesFixture =
        load_fixture("auth/auth_oauth_opaque_error_cases_fixture");
    let server = MockServer::start();

    for case in fixture.cases {
        let path = format!("/{}", case.name);
        let (status, body, provider) = match case.name.as_str() {
            "status_error" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    Vec::new(),
                    Vec::new(),
                    HashMap::new(),
                );
                (500, json!({"active": true}), provider)
            }
            "json_error" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    Vec::new(),
                    Vec::new(),
                    HashMap::new(),
                );
                (
                    200,
                    serde_json::Value::String("not-json".to_owned()),
                    provider,
                )
            }
            "inactive" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    Vec::new(),
                    Vec::new(),
                    HashMap::new(),
                );
                (200, json!({"active": false}), provider)
            }
            "issuer_mismatch" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    Some("https://issuer.example.com".to_owned()),
                    Vec::new(),
                    Vec::new(),
                    HashMap::new(),
                );
                (
                    200,
                    json!({"active": true, "iss": "https://other.example.com"}),
                    provider,
                )
            }
            "audience_mismatch" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    vec!["api://mcp".to_owned()],
                    Vec::new(),
                    HashMap::new(),
                );
                (200, json!({"active": true, "aud": "other"}), provider)
            }
            "claims_mismatch" => {
                let mut required_claims = HashMap::new();
                required_claims.insert("azp".to_owned(), "client".to_owned());
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    Vec::new(),
                    Vec::new(),
                    required_claims,
                );
                (200, json!({"active": true, "azp": "other"}), provider)
            }
            "scopes_mismatch" => {
                let provider = build_opaque_provider_config(
                    "opaque",
                    Some(format!("{}{}", server.base_url(), path)),
                    None,
                    Vec::new(),
                    vec!["mcp".to_owned()],
                    HashMap::new(),
                );
                (200, json!({"active": true, "scope": "other"}), provider)
            }
            other => panic!("unknown opaque error case: {other}"),
        };

        let introspection_mock = server.mock(|when, then| {
            when.method(POST).path(path);
            if matches!(body, serde_json::Value::String(_)) {
                then.status(status).body(body.as_str().unwrap());
            } else {
                then.status(status).json_body(body.clone());
            }
        });

        let state = oauth_state(vec![provider]);

        let (status, header) =
            run_protected_request(state, Some("Bearer opaque-token".to_owned())).await;
        let expected_status = if case.name == "scopes_mismatch" {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(status, expected_status, "case {}", case.name);
        let header_val = header.unwrap_or_else(|| {
            panic!(
                "case {}: {expected_status} should include WWW-Authenticate header",
                case.name
            )
        });
        assert!(
            header_val.starts_with("Bearer"),
            "case {}: challenge should start with Bearer, got: {header_val}",
            case.name
        );
        introspection_mock.assert_calls(1);
        if case.name == "scopes_mismatch" {
            assert!(
                header_val.contains("error=\"insufficient_scope\""),
                "case scopes_mismatch: 403 challenge should indicate insufficient_scope, got: {header_val}"
            );
            assert!(
                header_val.contains("scope=\"mcp\""),
                "case scopes_mismatch: 403 challenge should include required scope, got: {header_val}"
            );
        } else {
            assert!(
                !header_val.contains("error=\"insufficient_scope\""),
                "case {} should not be tagged insufficient_scope, got: {header_val}",
                case.name
            );
        }
    }
}

#[tokio::test]
async fn auth_oauth_plugin_not_found_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_plugin_not_found_fixture");
    let token = build_hs256_token_with_claims(
        &json!({"sub": "user-123", "scope": fixture.token_scope}),
        None,
        "unused",
    );
    let registry = PluginRegistry::new().register_auth(AllowAllPlugin).unwrap();
    let plugin_name = fixture.plugin.expect("fixture plugin");
    let provider = rust_mcp_core::config::AuthProviderConfig::plugin("plugin", plugin_name);

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider],
        plugins: Some(Arc::new(registry)),
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val =
        header.expect("401 from plugin-not-found should include WWW-Authenticate header");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_plugin_missing_registry_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_plugin_missing_registry_fixture");
    let token = build_hs256_token_with_claims(
        &json!({"sub": "user-123", "scope": fixture.token_scope}),
        None,
        "unused",
    );
    let plugin_name = fixture.plugin.expect("fixture plugin");
    let provider = rust_mcp_core::config::AuthProviderConfig::plugin("plugin", plugin_name);

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider],
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val =
        header.expect("401 from plugin-missing-registry should include WWW-Authenticate header");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_plugin_accept_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_plugin_accept_fixture");
    let token = build_hs256_token_with_claims(
        &json!({"sub": "user-123", "scope": fixture.token_scope}),
        None,
        "unused",
    );
    let plugin_name = fixture.plugin.expect("fixture plugin");
    let provider = rust_mcp_core::config::AuthProviderConfig::plugin("plugin", plugin_name);
    let registry = PluginRegistry::new().register_auth(AllowAllPlugin).unwrap();

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider],
        plugins: Some(Arc::new(registry)),
        ..Default::default()
    });

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    assert!(
        header.is_none(),
        "successful plugin-accept auth should not produce WWW-Authenticate header"
    );
}

#[tokio::test]
async fn auth_resource_metadata_fixture() {
    let fixture: ResourceMetadataFixture = load_fixture("auth/auth_resource_metadata_fixture");
    let provider = build_jwt_provider_config(
        "jwt",
        None,
        Some(fixture.issuer.clone()),
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(fixture.resource.clone()),
        resource_metadata_url: Some(EXAMPLE_RESOURCE_METADATA_URL.to_owned()),
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
        Some(fixture.resource.as_str())
    );
    let servers = value
        .get("authorization_servers")
        .and_then(|v| v.as_array())
        .expect("authorization_servers should be present when issuer is configured");
    assert_eq!(
        servers.len(),
        1,
        "should have exactly one authorization server"
    );
    assert_eq!(
        servers[0].as_str(),
        Some(fixture.issuer.as_str()),
        "authorization_servers[0] should match configured issuer"
    );
    // No oauth_client_metadata_document_url was configured, so it should be absent
    assert!(
        value.get("oauth_client_metadata_document_url").is_none(),
        "oauth_client_metadata_document_url should be absent when not configured"
    );
}

#[tokio::test]
async fn auth_resource_metadata_missing_fixture() {
    let fixture: ResourceMissingFixture =
        load_fixture("auth/auth_resource_metadata_missing_fixture");
    let provider = build_jwt_provider_config(
        "jwt",
        None,
        Some("http://issuer.example.com".to_owned()),
        Vec::new(),
        Vec::new(),
        HashMap::new(),
    );

    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource_metadata_url: Some(EXAMPLE_RESOURCE_METADATA_URL.to_owned()),
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
    assert_eq!(
        response.status().as_u16(),
        fixture.expected_status,
        "missing resource should return 501 Not Implemented"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        body.is_empty(),
        "501 response body should be empty, got {} bytes",
        body.len()
    );
}

#[tokio::test]
async fn auth_oauth_jwks_status_error_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwks_status_error_fixture");
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(500).body("error");
    });

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        "kid-1",
        "secret",
    );

    let provider = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = oauth_state(vec![provider]);

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("401 from JWKS status error should include WWW-Authenticate");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_jwks_json_error_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwks_json_error_fixture");
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/jwks");
        then.status(200).body("not-json");
    });

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        "kid-1",
        "secret",
    );

    let provider = build_jwt_provider_config(
        "jwt",
        Some(format!("{}/jwks", server.base_url())),
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = oauth_state(vec![provider]);

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("401 from JWKS JSON error should include WWW-Authenticate");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_oauth_jwks_request_error_fixture() {
    let fixture: JwtFixture = load_fixture("auth/auth_oauth_jwks_status_error_fixture");

    let token = build_hs256_token(
        &fixture.token_issuer,
        &fixture.token_audience,
        &fixture.token_scope,
        "kid-1",
        "secret",
    );

    let provider = build_jwt_provider_config(
        "jwt",
        Some("http://127.0.0.1:1/jwks".to_owned()),
        Some(fixture.token_issuer.clone()),
        vec![fixture.token_audience.clone()],
        fixture.required_scopes.clone().unwrap_or_default(),
        HashMap::new(),
    );

    let state = oauth_state(vec![provider]);

    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    let header_val = header.expect("401 from JWKS request error should include WWW-Authenticate");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[tokio::test]
async fn auth_plugin_config_passthrough() {
    let token = "opaque-token";
    let provider = rust_mcp_core::config::AuthProviderConfig::plugin("plugin", "config-check");
    // allow: true -> should pass
    let registry = PluginRegistry::new()
        .register_auth(ConfigCheckPlugin)
        .unwrap();
    let mut plugin_configs = HashMap::new();
    plugin_configs.insert("config-check".to_owned(), json!({"allow": true}));
    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider.clone()],
        plugins: Some(Arc::new(registry)),
        auth_plugin_configs: plugin_configs,
        ..Default::default()
    });
    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        header.is_none(),
        "successful config-check plugin auth should not produce WWW-Authenticate header"
    );

    // allow: false -> should reject
    let registry = PluginRegistry::new()
        .register_auth(ConfigCheckPlugin)
        .unwrap();
    let mut plugin_configs = HashMap::new();
    plugin_configs.insert("config-check".to_owned(), json!({"allow": false}));
    let state = build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers: vec![provider],
        plugins: Some(Arc::new(registry)),
        auth_plugin_configs: plugin_configs,
        ..Default::default()
    });
    let (status, header) = run_protected_request(state, Some(format!("Bearer {token}"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let header_val =
        header.expect("401 from config-check rejection should include WWW-Authenticate header");
    assert!(
        header_val.starts_with("Bearer"),
        "challenge should start with Bearer, got: {header_val}"
    );
}

#[test]
fn normalize_endpoint_path_fixture() {
    let fixture: NormalizePathFixture = load_fixture("auth/auth_normalize_endpoint_path_fixture");
    for case in fixture.cases {
        assert_eq!(
            rust_mcp_core::normalize_endpoint_path(&case.input),
            case.expected
        );
    }
}

#[test]
fn normalize_endpoint_path_cases() {
    use rust_mcp_core::normalize_endpoint_path;

    assert_eq!(normalize_endpoint_path(""), "/mcp");
    assert_eq!(normalize_endpoint_path("/"), "/");
    assert_eq!(normalize_endpoint_path("mcp/"), "/mcp");
    assert_eq!(normalize_endpoint_path("/mcp/"), "/mcp");
    assert_eq!(normalize_endpoint_path(" /foo/bar/ "), "/foo/bar");
}
