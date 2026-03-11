//! Core authorization logic: token extraction, JWT/introspection validation, and auth decisions.

use super::claims::{audiences_match_required, claims_match_required, scopes_match_required};
use super::provider::AuthProvider;
use super::token::{
    decode_jwt_claims, extract_bearer_token, parse_algorithms, select_jwk, token_looks_like_jwt,
};

use std::{collections::HashMap, fmt, sync::Arc};

use http::HeaderMap;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::auth::oauth::token_exchange::{IntrospectionExchangeParams, TokenExchanger};
use crate::http::client::SharedHttpClient;
use crate::plugins::{
    AuthPluginDecision, AuthPluginValidateParams, PluginLookup, PluginRef, PluginRegistry,
    PluginType,
};

#[derive(Clone, Debug)]
pub(super) enum AuthDecision {
    Allow,
    Unauthorized { scope: Option<String> },
    Forbidden { scope: Option<String> },
}

#[derive(Clone, Debug)]
enum ValidationOutcome {
    Valid,
    Invalid,
    InsufficientScope(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthActivation {
    None,
    BearerOnly,
    OauthOnly,
    BearerAndOauth,
}

#[derive(Clone)]
pub(crate) struct AuthStateInit {
    pub(crate) activation: AuthActivation,
    pub(crate) endpoint_path: String,
    pub(crate) bearer_token: Option<String>,
    pub(crate) resource: Option<String>,
    pub(crate) resource_metadata_url: Option<String>,
    pub(crate) providers: Vec<AuthProvider>,
    pub(crate) plugins: Option<Arc<PluginRegistry>>,
    pub(crate) auth_plugin_configs: HashMap<String, Value>,
    pub(crate) scope_challenges_enabled: bool,
    pub(crate) oauth_client_metadata_document_url: Option<String>,
    pub(crate) http: SharedHttpClient,
    pub(crate) token_exchanger: Arc<dyn TokenExchanger>,
    pub(crate) outbound_timeout_ms: Option<u64>,
    pub(crate) outbound_max_response_bytes: Option<u64>,
}

// Server-side authentication and authorization state.
//
// Holds configured auth activation, token validation providers, scope enforcement
// settings, and plugin references. Created via [`build_auth_state`](crate::build_auth_state),
// [`build_auth_state_from_config`](crate::build_auth_state_from_config), or
// [`build_auth_state_with_plugins`](crate::build_auth_state_with_plugins).
//
// Used internally by [`auth_middleware`](crate::auth_middleware) and the
// OAuth metadata endpoint.
#[derive(Clone)]
#[doc(hidden)]
pub struct AuthState {
    pub activation: AuthActivation,
    endpoint_path: String,
    bearer_token: Option<String>,
    resource: Option<String>,
    resource_metadata_url: Option<String>,
    providers: Vec<AuthProvider>,
    plugins: Option<Arc<PluginRegistry>>,
    auth_plugin_configs: HashMap<String, Value>,
    scope_challenges_enabled: bool,
    oauth_client_metadata_document_url: Option<String>,
    http: SharedHttpClient,
    token_exchanger: Arc<dyn TokenExchanger>,
    outbound_timeout_ms: Option<u64>,
    outbound_max_response_bytes: Option<u64>,
}

// Manual Debug impl because SharedHttpClient (Arc<dyn HttpClient>) does not
// implement Debug — the trait object has no Debug bound.
impl fmt::Debug for AuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthState")
            .field("activation", &self.activation)
            .field("endpoint_path", &self.endpoint_path)
            .field("resource", &self.resource)
            .field("scope_challenges_enabled", &self.scope_challenges_enabled)
            .finish_non_exhaustive()
    }
}

// Parameters passed to [`AuthState::run_plugin`].
struct RunPluginParams<'a> {
    token: &'a str,
    claims: &'a Value,
    headers: &'a HeaderMap,
    provider: &'a AuthProvider,
}

struct TokenProfile<'a> {
    token: &'a str,
    is_jwt: bool,
    claims: Option<Value>,
    token_iss: Option<String>,
}

impl AuthState {
    pub(crate) fn new(init: AuthStateInit) -> Self {
        Self {
            activation: init.activation,
            endpoint_path: init.endpoint_path,
            bearer_token: init.bearer_token,
            resource: init.resource,
            resource_metadata_url: init.resource_metadata_url,
            providers: init.providers,
            plugins: init.plugins,
            auth_plugin_configs: init.auth_plugin_configs,
            scope_challenges_enabled: init.scope_challenges_enabled,
            oauth_client_metadata_document_url: init.oauth_client_metadata_document_url,
            http: init.http,
            token_exchanger: init.token_exchanger,
            outbound_timeout_ms: init.outbound_timeout_ms,
            outbound_max_response_bytes: init.outbound_max_response_bytes,
        }
    }

    #[doc(hidden)]
    pub const fn oauth_enabled(&self) -> bool {
        matches!(
            self.activation,
            AuthActivation::OauthOnly | AuthActivation::BearerAndOauth
        )
    }

    #[doc(hidden)]
    pub const fn auth_enabled(&self) -> bool {
        !matches!(self.activation, AuthActivation::None)
    }

    pub(super) async fn authorize(&self, headers: &HeaderMap) -> AuthDecision {
        match self.activation {
            AuthActivation::None => AuthDecision::Allow,
            AuthActivation::BearerOnly => {
                if self.check_bearer(headers) {
                    AuthDecision::Allow
                } else {
                    AuthDecision::Unauthorized { scope: None }
                }
            }
            AuthActivation::OauthOnly => self.check_oauth(headers).await,
            AuthActivation::BearerAndOauth => {
                if self.check_bearer(headers) {
                    AuthDecision::Allow
                } else {
                    self.check_oauth(headers).await
                }
            }
        }
    }

    #[doc(hidden)]
    pub fn www_authenticate(
        &self,
        scope: Option<&str>,
        insufficient_scope: bool,
    ) -> Option<String> {
        if !self.oauth_enabled() {
            return None;
        }
        let mut parts = Vec::new();
        if insufficient_scope {
            parts.push("error=\"insufficient_scope\"".to_owned());
        }
        if self.scope_challenges_enabled {
            if let Some(scope) = scope {
                parts.push(format!("scope=\"{scope}\""));
            }
        }
        if let Some(resource_metadata) = self.resource_metadata_url.as_ref() {
            parts.push(format!("resource_metadata=\"{resource_metadata}\""));
        }
        if parts.is_empty() {
            Some("Bearer".to_owned())
        } else {
            Some(format!("Bearer {}", parts.join(", ")))
        }
    }

    pub(super) fn endpoint_path(&self) -> &str {
        &self.endpoint_path
    }

    pub(super) fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    pub(super) fn providers(&self) -> &[AuthProvider] {
        &self.providers
    }

    pub(super) fn oauth_client_metadata_document_url(&self) -> Option<&str> {
        self.oauth_client_metadata_document_url.as_deref()
    }

    pub(super) fn http_client(&self) -> &dyn crate::http::client::HttpClient {
        self.http.as_ref()
    }

    // Constant-time comparison to prevent timing side-channels on static bearer tokens.
    fn check_bearer(&self, headers: &HeaderMap) -> bool {
        match (&self.bearer_token, extract_bearer_token(headers)) {
            (Some(expected), Some(actual)) => expected.as_bytes().ct_eq(actual.as_bytes()).into(),
            _ => false,
        }
    }

    // Extracts the bearer token and evaluates configured providers in order.
    // Candidate providers either authenticate, reject, or abstain/continue.
    async fn check_oauth(&self, headers: &HeaderMap) -> AuthDecision {
        let token = extract_bearer_token(headers);
        let Some(token) = token else {
            return self.default_unauthorized();
        };
        let profile = Self::token_profile(&token);
        for provider in &self.providers {
            if let Some(decision) = self.evaluate_provider(provider, &profile, headers).await {
                return decision;
            }
        }
        self.default_unauthorized()
    }

    fn default_unauthorized(&self) -> AuthDecision {
        AuthDecision::Unauthorized {
            scope: self.default_scope_challenge(),
        }
    }

    fn token_profile(token: &str) -> TokenProfile<'_> {
        let is_jwt = token_looks_like_jwt(token);
        let claims = if is_jwt {
            decode_jwt_claims(token)
        } else {
            None
        };
        let token_iss = claims
            .as_ref()
            .and_then(|parsed| parsed.get("iss"))
            .and_then(Value::as_str)
            .map(std::borrow::ToOwned::to_owned);
        TokenProfile {
            token,
            is_jwt,
            claims,
            token_iss,
        }
    }

    async fn evaluate_provider(
        &self,
        provider: &AuthProvider,
        profile: &TokenProfile<'_>,
        headers: &HeaderMap,
    ) -> Option<AuthDecision> {
        if provider.config.is_bearer() {
            return Self::bearer_decision(provider, profile.token);
        }
        if provider.config.is_jwks() {
            return self.jwks_decision(provider, profile).await;
        }
        if provider.config.is_introspection() {
            return self.introspection_decision(provider, profile).await;
        }
        if provider.config.is_plugin() {
            return self.plugin_decision(provider, profile, headers).await;
        }
        None
    }

    fn bearer_decision(provider: &AuthProvider, token: &str) -> Option<AuthDecision> {
        provider
            .config
            .bearer_token()
            .is_some_and(|expected| expected.as_bytes().ct_eq(token.as_bytes()).into())
            .then_some(AuthDecision::Allow)
    }

    async fn jwks_decision(
        &self,
        provider: &AuthProvider,
        profile: &TokenProfile<'_>,
    ) -> Option<AuthDecision> {
        if !profile.is_jwt {
            return None;
        }
        let iss = profile.token_iss.as_deref()?;
        let provider_issuer = provider.issuer(self.http.as_ref()).await;
        if provider_issuer.as_deref() != Some(iss) {
            return None;
        }
        Some(Self::decision_from_validation(
            self.validate_jwt(profile.token, provider).await,
            provider,
            self,
        ))
    }

    async fn introspection_decision(
        &self,
        provider: &AuthProvider,
        profile: &TokenProfile<'_>,
    ) -> Option<AuthDecision> {
        if profile.is_jwt {
            if let Some(iss) = profile.token_iss.as_deref() {
                let provider_issuer = provider.issuer(self.http.as_ref()).await;
                if provider_issuer.as_deref() != Some(iss) {
                    return None;
                }
            } else if !provider.config.allow_missing_iss() {
                return None;
            }
        }
        Some(Self::decision_from_validation(
            self.validate_opaque(profile.token, provider).await,
            provider,
            self,
        ))
    }

    async fn plugin_decision(
        &self,
        provider: &AuthProvider,
        profile: &TokenProfile<'_>,
        headers: &HeaderMap,
    ) -> Option<AuthDecision> {
        let claim_payload = profile.claims.as_ref().unwrap_or(&Value::Null);
        let decision = self
            .run_plugin(RunPluginParams {
                token: profile.token,
                claims: claim_payload,
                headers,
                provider,
            })
            .await;
        match decision {
            AuthPluginDecision::Accept => Some(AuthDecision::Allow),
            AuthPluginDecision::Reject => Some(AuthDecision::Unauthorized {
                scope: self.scope_for_provider(provider),
            }),
            AuthPluginDecision::Abstain => None,
        }
    }

    fn decision_from_validation(
        outcome: ValidationOutcome,
        provider: &AuthProvider,
        state: &Self,
    ) -> AuthDecision {
        match outcome {
            ValidationOutcome::Valid => AuthDecision::Allow,
            ValidationOutcome::Invalid => AuthDecision::Unauthorized {
                scope: state.scope_for_provider(provider),
            },
            ValidationOutcome::InsufficientScope(scopes) => AuthDecision::Forbidden {
                scope: state.scope_challenge(Some(&scopes)),
            },
        }
    }

    pub(crate) fn scope_challenge(&self, scopes: Option<&[String]>) -> Option<String> {
        if !self.scope_challenges_enabled {
            return None;
        }
        let scopes = scopes?;
        if scopes.is_empty() {
            return None;
        }
        Some(scopes.join(" "))
    }

    pub(crate) fn default_scope_challenge(&self) -> Option<String> {
        if !self.scope_challenges_enabled {
            return None;
        }
        self.providers
            .iter()
            .find(|provider| !provider.config.required_scopes().is_empty())
            .and_then(|provider| self.scope_challenge(Some(provider.config.required_scopes())))
    }

    fn scope_for_provider(&self, provider: &AuthProvider) -> Option<String> {
        self.scope_challenge(Some(provider.config.required_scopes()))
    }

    // Validates a JWT: fetches JWKS, selects the signing key by kid, verifies
    // signature/expiry/issuer/audience, then checks required claims and scopes.
    async fn validate_jwt(&self, token: &str, provider: &AuthProvider) -> ValidationOutcome {
        let Some(jwks) = provider.jwks(self.http.as_ref()).await else {
            return ValidationOutcome::Invalid;
        };
        let Ok(header) = decode_header(token) else {
            return ValidationOutcome::Invalid;
        };
        let jwk = select_jwk(&jwks, header.kid.as_deref());
        let Some(jwk) = jwk else {
            return ValidationOutcome::Invalid;
        };
        let Ok(key) = DecodingKey::from_jwk(jwk) else {
            return ValidationOutcome::Invalid;
        };

        let mut validation = Validation::new(header.alg);
        let algs = parse_algorithms(&provider.config);
        if !algs.is_empty() {
            validation.algorithms = algs;
        }
        if let Some(issuer) = provider.issuer(self.http.as_ref()).await {
            validation.set_issuer(&[issuer]);
        }
        if !provider.config.audiences().is_empty() {
            validation.set_audience(provider.config.audiences());
        }
        if let Some(leeway) = provider.config.clock_skew_sec() {
            validation.leeway = leeway;
        }

        let Ok(data) = decode::<Value>(token, &key, &validation) else {
            return ValidationOutcome::Invalid;
        };
        let claims = data.claims;

        if !claims_match_required(&claims, provider.config.required_claims()) {
            return ValidationOutcome::Invalid;
        }
        if !scopes_match_required(&claims, provider.config.required_scopes()) {
            return ValidationOutcome::InsufficientScope(
                provider.config.required_scopes().to_vec(),
            );
        }

        ValidationOutcome::Valid
    }

    // Validates an opaque token via RFC 7662 introspection: sends the token to
    // the provider's introspection endpoint using the configured client auth
    // method (basic, post, or none), then checks active=true, issuer, audience,
    // claims, and scopes.
    async fn validate_opaque(&self, token: &str, provider: &AuthProvider) -> ValidationOutcome {
        let Some(url) = provider.config.introspection_url() else {
            return ValidationOutcome::Invalid;
        };

        let Ok(payload) = self
            .token_exchanger
            .introspect(
                Arc::clone(&self.http),
                IntrospectionExchangeParams {
                    token,
                    url,
                    client_auth_method: provider.config.introspection_client_auth_method(),
                    client_id: provider.config.introspection_client_id(),
                    client_secret: provider.config.introspection_client_secret(),
                    timeout_ms: self.outbound_timeout_ms,
                    max_response_bytes: self.outbound_max_response_bytes,
                },
            )
            .await
        else {
            return ValidationOutcome::Invalid;
        };
        if !payload
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return ValidationOutcome::Invalid;
        }

        if let Some(issuer) = provider.issuer(self.http.as_ref()).await {
            let token_iss = payload.get("iss").and_then(|v| v.as_str());
            if token_iss != Some(issuer.as_str()) {
                return ValidationOutcome::Invalid;
            }
        }

        if !audiences_match_required(&payload, provider.config.audiences()) {
            return ValidationOutcome::Invalid;
        }
        if !claims_match_required(&payload, provider.config.required_claims()) {
            return ValidationOutcome::Invalid;
        }
        if !scopes_match_required(&payload, provider.config.required_scopes()) {
            return ValidationOutcome::InsufficientScope(
                provider.config.required_scopes().to_vec(),
            );
        }

        ValidationOutcome::Valid
    }

    // Runs a plugin provider and returns its explicit decision.
    async fn run_plugin(&self, params: RunPluginParams<'_>) -> AuthPluginDecision {
        let plugin_name = params.provider.config.plugin_name();
        let Some(plugin_name) = plugin_name else {
            return AuthPluginDecision::Abstain;
        };
        let Some(registry) = self.plugins.as_ref() else {
            return AuthPluginDecision::Reject;
        };
        let Some(PluginRef::Auth(plugin)) = registry.get_plugin(PluginType::Auth, plugin_name)
        else {
            return AuthPluginDecision::Reject;
        };
        let null_config = Value::Null;
        let config = self
            .auth_plugin_configs
            .get(plugin_name)
            .unwrap_or(&null_config);
        plugin
            .validate(AuthPluginValidateParams {
                token: params.token,
                claims: params.claims,
                headers: params.headers,
                config,
            })
            .await
    }
}

#[cfg(test)]
// Inline tests here cover private AuthState auth-path helpers and routing
// branches that are not reachable from integration tests without changing visibility.
mod tests {
    use super::{AuthActivation, AuthDecision, AuthState, AuthStateInit};
    use crate::auth::inbound::provider::AuthProvider;
    use crate::auth::oauth::token_exchange::HttpTokenExchanger;
    use crate::config::AuthProviderConfig;
    use crate::http::client::ReqwestHttpClient;
    use crate::inline_test_fixtures::base_provider;
    use crate::plugins::{
        AuthPlugin, AuthPluginDecision, AuthPluginValidateParams, PluginRegistry,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use http::HeaderMap;
    use serde_json::Value;
    use std::{collections::HashMap, sync::Arc};

    fn jwt_with_claims(claims: &Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("a.{payload}.c")
    }

    fn parse_bearer_challenge_params(challenge: &str) -> HashMap<String, String> {
        let raw = challenge
            .strip_prefix("Bearer ")
            .expect("bearer challenge prefix");
        raw.split(", ")
            .map(|entry| {
                let mut key_value = entry.splitn(2, '=');
                let key = key_value.next().expect("key").to_owned();
                let raw_value = key_value.next().expect("value");
                let value = raw_value.trim_matches('"').to_owned();
                (key, value)
            })
            .collect()
    }

    fn base_state_init() -> AuthStateInit {
        AuthStateInit {
            activation: AuthActivation::OauthOnly,
            endpoint_path: "/mcp".to_owned(),
            bearer_token: None,
            resource: None,
            resource_metadata_url: None,
            providers: Vec::new(),
            plugins: None,
            auth_plugin_configs: HashMap::new(),
            scope_challenges_enabled: true,
            oauth_client_metadata_document_url: None,
            http: Arc::new(ReqwestHttpClient::default()),
            token_exchanger: Arc::new(HttpTokenExchanger),
            outbound_timeout_ms: None,
            outbound_max_response_bytes: None,
        }
    }

    #[test]
    fn check_bearer_matches_and_rejects_non_matching_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            "Bearer secret-token".parse().expect("header"),
        );
        let state = AuthState::new(AuthStateInit {
            activation: AuthActivation::BearerOnly,
            bearer_token: Some("secret-token".to_owned()),
            ..base_state_init()
        });
        assert!(state.check_bearer(&headers));

        let mut bad_headers = HeaderMap::new();
        bad_headers.insert(
            "Authorization",
            "Bearer wrong-token".parse().expect("header"),
        );
        assert!(!state.check_bearer(&bad_headers));

        let missing_headers = HeaderMap::new();
        assert!(!state.check_bearer(&missing_headers));

        let mut non_bearer_headers = HeaderMap::new();
        non_bearer_headers.insert(
            "Authorization",
            "Basic dXNlcjpwYXNz".parse().expect("header"),
        );
        assert!(!state.check_bearer(&non_bearer_headers));

        let mut empty_bearer_headers = HeaderMap::new();
        empty_bearer_headers.insert("Authorization", "Bearer ".parse().expect("header"));
        assert!(!state.check_bearer(&empty_bearer_headers));
    }

    struct AcceptAuthPlugin;

    #[async_trait::async_trait]
    impl AuthPlugin for AcceptAuthPlugin {
        fn name(&self) -> &'static str {
            "accept-auth"
        }

        async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
            AuthPluginDecision::Accept
        }
    }

    struct DenyAuthPlugin;

    #[async_trait::async_trait]
    impl AuthPlugin for DenyAuthPlugin {
        fn name(&self) -> &'static str {
            "deny-auth"
        }

        async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
            AuthPluginDecision::Reject
        }
    }

    struct AbstainAuthPlugin;

    #[async_trait::async_trait]
    impl AuthPlugin for AbstainAuthPlugin {
        fn name(&self) -> &'static str {
            "abstain-auth"
        }

        async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
            AuthPluginDecision::Abstain
        }
    }

    #[tokio::test]
    async fn ordered_auth_falls_back_after_bearer_mismatch_to_plugin_accept() {
        let providers = vec![
            AuthProvider::new(
                AuthProviderConfig::bearer("static", "expected-token"),
                None,
                None,
            ),
            AuthProvider::new(
                AuthProviderConfig::plugin("plugin-provider", "accept-auth"),
                None,
                None,
            ),
        ];
        let plugins = PluginRegistry::new()
            .register_auth(AcceptAuthPlugin)
            .expect("register accept plugin");
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers,
            plugins: Some(Arc::new(plugins)),
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    "Bearer opaque-token".parse().expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Allow));
    }

    #[tokio::test]
    async fn plugin_provider_returns_unauthorized_when_plugin_denies() {
        let provider = AuthProviderConfig::plugin("plugin-provider", "deny-auth");
        let provider = AuthProvider::new(provider, None, None);
        let plugins = PluginRegistry::new()
            .register_auth(DenyAuthPlugin)
            .expect("register deny plugin");
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers: vec![provider.clone()],
            plugins: Some(Arc::new(plugins)),
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    "Bearer opaque-token".parse().expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn plugin_provider_abstain_continues_to_next_provider() {
        let providers = vec![
            AuthProvider::new(
                AuthProviderConfig::plugin("abstain", "abstain-auth"),
                None,
                None,
            ),
            AuthProvider::new(
                AuthProviderConfig::plugin("accept", "accept-auth"),
                None,
                None,
            ),
        ];
        let plugins = PluginRegistry::new()
            .register_auth(AbstainAuthPlugin)
            .expect("register abstain plugin")
            .register_auth(AcceptAuthPlugin)
            .expect("register accept plugin");
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers,
            plugins: Some(Arc::new(plugins)),
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    "Bearer opaque-token".parse().expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Allow));
    }

    #[tokio::test]
    async fn plugin_provider_abstain_without_followup_returns_unauthorized() {
        let providers = vec![AuthProvider::new(
            AuthProviderConfig::plugin("abstain", "abstain-auth"),
            None,
            None,
        )];
        let plugins = PluginRegistry::new()
            .register_auth(AbstainAuthPlugin)
            .expect("register abstain plugin");
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers,
            plugins: Some(Arc::new(plugins)),
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    "Bearer opaque-token".parse().expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn introspection_jwt_with_issuer_mismatch_returns_unauthorized() {
        let mut provider =
            AuthProviderConfig::introspection("opaque", "https://introspect.example");
        provider
            .as_introspection_mut()
            .expect("introspection provider")
            .issuer = Some("https://issuer.example.com".to_owned());
        let providers = vec![AuthProvider::new(provider, None, None)];
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers,
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    format!(
                        "Bearer {}",
                        jwt_with_claims(&serde_json::json!({"iss":"https://other.example.com"}))
                    )
                    .parse()
                    .expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn introspection_jwt_without_iss_falls_through_when_missing_iss_disallowed() {
        let introspection = AuthProvider::new(
            AuthProviderConfig::introspection("opaque", "https://introspect.example"),
            None,
            None,
        );
        let plugin = AuthProvider::new(
            AuthProviderConfig::plugin("accept", "accept-auth"),
            None,
            None,
        );
        let plugins = PluginRegistry::new()
            .register_auth(AcceptAuthPlugin)
            .expect("register accept plugin");
        let state = AuthState::new(AuthStateInit {
            resource: Some("http://example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "http://example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers: vec![introspection, plugin],
            plugins: Some(Arc::new(plugins)),
            ..base_state_init()
        });

        let outcome = state
            .check_oauth(&{
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Authorization",
                    format!(
                        "Bearer {}",
                        jwt_with_claims(&serde_json::json!({"sub":"u"}))
                    )
                    .parse()
                    .expect("auth header"),
                );
                headers
            })
            .await;
        assert!(matches!(outcome, AuthDecision::Allow));
    }

    #[test]
    fn auth_state_debug_includes_core_fields() {
        let state = AuthState::new(base_state_init());
        let rendered = format!("{state:?}");
        assert!(rendered.contains("AuthState"));
        assert!(rendered.contains("endpoint_path"));
    }

    #[test]
    fn scope_challenge_disabled_omits_scope_from_header() {
        let mut provider = base_provider();
        provider
            .as_jwks_mut()
            .expect("base provider must be jwks")
            .required_scopes = vec!["files:read".to_owned()];
        let state = AuthState::new(AuthStateInit {
            resource: Some("https://api.example.com/mcp".to_owned()),
            resource_metadata_url: Some(
                "https://api.example.com/.well-known/oauth-protected-resource".to_owned(),
            ),
            providers: vec![AuthProvider::new(provider, None, None)],
            scope_challenges_enabled: false,
            ..base_state_init()
        });

        assert!(state
            .scope_challenge(Some(&["files:read".to_owned()]))
            .is_none());
        assert!(state.default_scope_challenge().is_none());
        let header = state
            .www_authenticate(Some("files:read"), true)
            .expect("header should exist");
        let params = parse_bearer_challenge_params(&header);
        assert_eq!(params.get("error"), Some(&"insufficient_scope".to_owned()));
        assert_eq!(
            params.get("resource_metadata"),
            Some(&"https://api.example.com/.well-known/oauth-protected-resource".to_owned())
        );
        assert!(!params.contains_key("scope"));
        assert!(!params.contains_key("resource"));
    }
}
