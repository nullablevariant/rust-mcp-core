use std::{collections::HashMap, path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rust_mcp_core::{
    auth_middleware, build_auth_state, AuthActivation, AuthPlugin, AuthPluginDecision,
    AuthPluginValidateParams, AuthStateParams,
};
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt;

use rust_mcp_core::config::{AuthProviderConfig, IntrospectionClientAuthMethod};

pub(crate) const EXAMPLE_RESOURCE: &str = "http://example.com/mcp";
pub(crate) const EXAMPLE_RESOURCE_METADATA_URL: &str =
    "http://example.com/.well-known/oauth-protected-resource";

#[derive(Deserialize)]
pub(crate) struct SimpleAuthFixture {
    pub mode: String,
    pub bearer_token: Option<String>,
    pub auth_header: Option<String>,
    pub expected_status: u16,
    pub expect_header: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct JwtFixture {
    pub expected_status: u16,
    pub token_issuer: String,
    pub token_audience: String,
    pub token_scope: String,
    pub required_scopes: Option<Vec<String>>,
    pub plugin: Option<String>,
    pub allow_missing_iss: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct OpaqueFixture {
    pub expected_status: u16,
    pub token_scope: String,
    pub required_scopes: Option<Vec<String>>,
    pub issuer: String,
    pub audience: String,
    pub default_provider: Option<String>,
    pub allow_missing_iss: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct ResourceMetadataFixture {
    pub resource: String,
    pub issuer: String,
}

#[derive(Deserialize)]
pub(crate) struct NormalizePathFixture {
    pub cases: Vec<NormalizePathCase>,
}

#[derive(Deserialize)]
pub(crate) struct NormalizePathCase {
    pub input: String,
    pub expected: String,
}

#[derive(Deserialize)]
pub(crate) struct InvalidJwtClaimsFixture {
    pub auth_header: String,
    pub expected_status: u16,
    pub allow_missing_iss: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct ProviderSelectionFixture {
    pub expected_status: u16,
    pub token_audience: String,
    pub token_scope: String,
    pub required_scopes: Option<Vec<String>>,
    pub default_provider: Option<String>,
    pub allow_missing_iss: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct AllFallbackFixture {
    pub expected_status: u16,
    pub bearer_token: String,
    pub token_issuer: String,
    pub token_audience: String,
    pub token_scope: String,
    pub required_scopes: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct JwtErrorCasesFixture {
    pub cases: Vec<JwtErrorCase>,
}

#[derive(Deserialize)]
pub(crate) struct JwtErrorCase {
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct OpaqueErrorCasesFixture {
    pub cases: Vec<OpaqueErrorCase>,
}

#[derive(Deserialize)]
pub(crate) struct OpaqueErrorCase {
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct DiscoveryStatusFixture {
    pub resource: String,
}

#[derive(Deserialize)]
pub(crate) struct ResourceMissingFixture {
    pub expected_status: u16,
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub(crate) fn load_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

pub(crate) struct AuthModeFlags {
    pub(crate) bearer_enabled: bool,
    pub(crate) oauth_enabled: bool,
}

pub(crate) fn parse_auth_mode(mode: &str) -> AuthModeFlags {
    match mode {
        "none" => AuthModeFlags {
            bearer_enabled: false,
            oauth_enabled: false,
        },
        "bearer" | "static" => AuthModeFlags {
            bearer_enabled: true,
            oauth_enabled: false,
        },
        "oauth" => AuthModeFlags {
            bearer_enabled: false,
            oauth_enabled: true,
        },
        _ => AuthModeFlags {
            bearer_enabled: true,
            oauth_enabled: true,
        },
    }
}

pub(crate) async fn run_protected_request(
    state: Arc<rust_mcp_core::AuthState>,
    auth_header: Option<String>,
) -> (StatusCode, Option<String>) {
    let app = Router::new()
        .route("/protected", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            auth_middleware,
        ));

    let mut request_builder = Request::builder().uri("/protected");
    if let Some(header) = auth_header {
        request_builder = request_builder.header("Authorization", header);
    }

    let response = app
        .oneshot(
            request_builder
                .body(Body::empty())
                .expect("request body should build"),
        )
        .await
        .expect("request should execute");
    let header_value = response.headers().get("WWW-Authenticate").map(|value| {
        value
            .to_str()
            .expect("WWW-Authenticate should be valid header text")
            .to_owned()
    });
    (response.status(), header_value)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_hs256_token(
    issuer: &str,
    audience: &str,
    scope: &str,
    kid: &str,
    secret: &str,
) -> String {
    let claims = json!({
        "iss": issuer,
        "aud": audience,
        "scope": scope,
        "exp": (Utc::now().timestamp() + 3600)
    });
    build_hs256_token_with_claims(&claims, Some(kid), secret)
}

pub(crate) fn build_hs256_token_with_claims(
    claims: &serde_json::Value,
    kid: Option<&str>,
    secret: &str,
) -> String {
    let mut header = Header::new(Algorithm::HS256);
    header.kid = kid.map(std::borrow::ToOwned::to_owned);
    encode(
        &header,
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("token should encode")
}

pub(crate) fn build_hs256_jwks(secret: &str, kid: &str) -> serde_json::Value {
    let encoded = URL_SAFE_NO_PAD.encode(secret.as_bytes());
    json!({
        "keys": [
            {
                "kty": "oct",
                "kid": kid,
                "alg": "HS256",
                "k": encoded
            }
        ]
    })
}

pub(crate) fn build_invalid_header_token(claims: &serde_json::Value) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims to vec"));
    format!("!!.{payload}.sig")
}

pub(crate) fn oauth_state(providers: Vec<AuthProviderConfig>) -> Arc<rust_mcp_core::AuthState> {
    build_auth_state(AuthStateParams {
        activation: AuthActivation::OauthOnly,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        providers,
        ..Default::default()
    })
}

pub(crate) async fn run_simple_auth_fixture(fixture_name: &str) {
    let fixture: SimpleAuthFixture = load_fixture(fixture_name);
    let mode = parse_auth_mode(&fixture.mode);
    let activation = match (mode.bearer_enabled, mode.oauth_enabled) {
        (false, false) => AuthActivation::None,
        (true, false) => AuthActivation::BearerOnly,
        (false, true) => AuthActivation::OauthOnly,
        (true, true) => AuthActivation::BearerAndOauth,
    };
    let state = build_auth_state(AuthStateParams {
        activation,
        bearer_token: fixture.bearer_token,
        resource: Some(EXAMPLE_RESOURCE.to_owned()),
        resource_metadata_url: Some(EXAMPLE_RESOURCE_METADATA_URL.to_owned()),
        ..Default::default()
    });
    let (status, header) = run_protected_request(state, fixture.auth_header).await;
    assert_eq!(status.as_u16(), fixture.expected_status);
    if let Some(ref expected_header_name) = fixture.expect_header {
        let header_val = header
            .as_ref()
            .unwrap_or_else(|| panic!("expected {expected_header_name} header but it was absent"));
        assert!(
            header_val.starts_with("Bearer"),
            "WWW-Authenticate should start with 'Bearer', got: {header_val}"
        );
        if mode.oauth_enabled {
            assert!(
                header_val.contains("resource_metadata="),
                "OAuth WWW-Authenticate should include resource_metadata, got: {header_val}"
            );
        }
    } else if !mode.oauth_enabled || fixture.expected_status == 200 {
        // Success in non-OAuth modes or mode=none should not produce WWW-Authenticate
        assert!(
            header.is_none(),
            "expected no WWW-Authenticate header for success/none mode, got: {header:?}"
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_jwt_provider_config(
    name: &str,
    jwks_url: Option<String>,
    issuer: Option<String>,
    audiences: Vec<String>,
    required_scopes: Vec<String>,
    required_claims: HashMap<String, String>,
) -> AuthProviderConfig {
    let mut provider = AuthProviderConfig::jwks(name);
    let jwks = provider.as_jwks_mut().expect("jwks provider");
    jwks.jwks_url = jwks_url;
    jwks.issuer = issuer;
    jwks.audiences = audiences;
    jwks.required_scopes = required_scopes;
    jwks.required_claims = required_claims;
    jwks.algorithms = vec!["HS256".to_owned()];
    provider
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_opaque_provider_config(
    name: &str,
    introspection_url: Option<String>,
    issuer: Option<String>,
    audiences: Vec<String>,
    required_scopes: Vec<String>,
    required_claims: HashMap<String, String>,
) -> AuthProviderConfig {
    let mut provider =
        AuthProviderConfig::introspection(name, introspection_url.unwrap_or_default());
    let introspection = provider
        .as_introspection_mut()
        .expect("introspection provider");
    introspection.issuer = issuer;
    introspection.audiences = audiences;
    introspection.required_scopes = required_scopes;
    introspection.required_claims = required_claims;
    introspection.auth_method = IntrospectionClientAuthMethod::Basic;
    provider
}

pub(crate) struct AllowAllPlugin;

#[async_trait::async_trait]
impl AuthPlugin for AllowAllPlugin {
    fn name(&self) -> &'static str {
        "allow-all"
    }

    async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        AuthPluginDecision::Accept
    }
}

pub(crate) struct ConfigCheckPlugin;

#[async_trait::async_trait]
impl AuthPlugin for ConfigCheckPlugin {
    fn name(&self) -> &'static str {
        "config-check"
    }

    async fn validate(&self, params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        if params
            .config
            .get("allow")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            AuthPluginDecision::Accept
        } else {
            AuthPluginDecision::Reject
        }
    }
}
