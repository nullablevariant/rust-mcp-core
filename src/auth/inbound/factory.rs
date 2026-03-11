//! Auth state construction and validation from config or explicit parameters.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rmcp::ErrorData as McpError;
use serde_json::Value;
use tracing::warn;

use super::authorization::{AuthActivation, AuthState, AuthStateInit};
use super::middleware::oauth_resource_metadata_endpoint_path;
use super::provider::AuthProvider;
use crate::auth::oauth::token_exchange::HttpTokenExchanger;
use crate::config::{AuthConfig, AuthProviderConfig, IntrospectionClientAuthMethod, McpConfig};
use crate::http::client::default_http_client;
use crate::plugins::{PluginLookup, PluginRegistry, PluginType};
use crate::utils::normalize_endpoint_path;

// Parameters for constructing an [`AuthState`] via [`build_auth_state`].
//
// Use `Default::default()` and override only the fields you need.
// Most fields default to disabled/empty, with `endpoint_path` defaulting to `"/mcp"`.
//
// # Examples
//
// ```rust
// use rust_mcp_core::{AuthStateParams, build_auth_state};
//
// let state = build_auth_state(AuthStateParams {
//     activation: rust_mcp_core::AuthActivation::BearerOnly,
//     bearer_token: Some("my-secret-token".into()),
//     ..Default::default()
// });
// # let _ = state;
// ```
#[derive(Debug)]
#[doc(hidden)]
pub struct AuthStateParams {
    pub activation: AuthActivation,
    pub bearer_token: Option<String>,
    pub resource: Option<String>,
    pub resource_metadata_url: Option<String>,
    pub providers: Vec<AuthProviderConfig>,
    pub plugins: Option<Arc<PluginRegistry>>,
    pub auth_plugin_configs: HashMap<String, Value>,
    pub endpoint_path: String,
    pub scope_challenges_enabled: bool,
    pub oauth_client_metadata_document_url: Option<String>,
    pub outbound_timeout_ms: Option<u64>,
    pub outbound_max_response_bytes: Option<u64>,
    pub server_log_payload_max_bytes: u64,
}

impl Default for AuthStateParams {
    fn default() -> Self {
        Self {
            activation: AuthActivation::None,
            bearer_token: None,
            resource: None,
            resource_metadata_url: None,
            providers: Vec::new(),
            plugins: None,
            auth_plugin_configs: HashMap::new(),
            endpoint_path: "/mcp".to_owned(),
            scope_challenges_enabled: true,
            oauth_client_metadata_document_url: None,
            outbound_timeout_ms: None,
            outbound_max_response_bytes: None,
            server_log_payload_max_bytes: 4096,
        }
    }
}

// Construct an [`AuthState`] from explicit parameters.
//
// This is the low-level constructor for full control over auth configuration.
// For config-driven usage, prefer [`build_auth_state_from_config`] or
// [`build_auth_state_with_plugins`].
//
// # Arguments
//
// * `params` — [`AuthStateParams`] with auth activation, tokens, providers, and options
//
// # Returns
//
// A shared [`AuthState`] ready for use with the auth middleware.
#[doc(hidden)]
pub fn build_auth_state(params: AuthStateParams) -> Arc<AuthState> {
    let endpoint_path = normalize_endpoint_path(&params.endpoint_path);
    let providers = params
        .providers
        .into_iter()
        .map(|provider| {
            AuthProvider::new(
                provider,
                params.outbound_timeout_ms,
                params.outbound_max_response_bytes,
            )
            .with_server_log_payload_max_bytes(params.server_log_payload_max_bytes)
        })
        .collect();

    Arc::new(AuthState::new(AuthStateInit {
        activation: params.activation,
        endpoint_path,
        bearer_token: params.bearer_token,
        resource: params.resource,
        resource_metadata_url: params.resource_metadata_url,
        providers,
        plugins: params.plugins,
        auth_plugin_configs: params.auth_plugin_configs,
        scope_challenges_enabled: params.scope_challenges_enabled,
        oauth_client_metadata_document_url: params.oauth_client_metadata_document_url,
        http: default_http_client(),
        token_exchanger: Arc::new(HttpTokenExchanger),
        outbound_timeout_ms: params.outbound_timeout_ms,
        outbound_max_response_bytes: params.outbound_max_response_bytes,
    }))
}

// Construct an [`AuthState`] from an [`McpConfig`], without plugin support.
//
// Validates auth settings, derives the resource URL and metadata URL from
// config, and builds the auth state. Equivalent to calling
// [`build_auth_state_with_plugins`] with `plugins: None`.
//
// # Errors
//
// Returns `McpError` if auth settings are invalid (e.g., oauth-capable providers without
// a resource URL).
#[doc(hidden)]
pub fn build_auth_state_from_config(config: &McpConfig) -> Result<Arc<AuthState>, McpError> {
    build_auth_state_with_plugins(config, None)
}

// Construct an [`AuthState`] from an [`McpConfig`] with optional plugin support.
//
// Validates auth settings and plugin references, then builds the auth state.
// Auth plugins declared in config `plugins[]` must be registered in the
// provided [`PluginRegistry`].
//
// # Arguments
//
// * `config` — the parsed MCP configuration
// * `plugins` — optional plugin registry containing auth plugins
//
// # Errors
//
// Returns `McpError` if auth settings are invalid or referenced auth plugins
// are not registered in the registry.
#[doc(hidden)]
pub fn build_auth_state_with_plugins(
    config: &McpConfig,
    plugins: Option<Arc<PluginRegistry>>,
) -> Result<Arc<AuthState>, McpError> {
    validate_auth_settings(config)?;
    validate_auth_plugins(config, plugins.as_deref())?;
    warn_unused_auth_settings(config);
    warn_introspection_client_auth(config);
    let endpoint_path = normalize_endpoint_path(&config.server.endpoint_path);
    let auth_cfg = config.server.auth.as_ref();
    let auth_enabled = auth_cfg.is_some_and(AuthConfig::is_enabled);
    let providers = auth_cfg.map_or_else(Vec::new, |auth| auth.providers.clone());
    let has_bearer = providers.iter().any(AuthProviderConfig::is_bearer);
    let has_oauth = providers.iter().any(|provider| !provider.is_bearer());
    let activation = if !auth_enabled {
        AuthActivation::None
    } else if has_bearer && has_oauth {
        AuthActivation::BearerAndOauth
    } else if has_bearer {
        AuthActivation::BearerOnly
    } else {
        AuthActivation::OauthOnly
    };
    let oauth_cfg = auth_cfg.and_then(|auth| auth.oauth.as_ref());
    let public_base = oauth_cfg
        .and_then(|oauth| oauth.public_url.clone())
        .unwrap_or_else(|| format!("http://{}:{}", config.server.host, config.server.port));
    let metadata_path = oauth_resource_metadata_endpoint_path(&endpoint_path);
    let resource_metadata_url = oauth_cfg.map(|_| format!("{public_base}{metadata_path}"));

    let auth_plugin_configs: HashMap<String, Value> = config
        .plugins
        .iter()
        .filter(|p| p.plugin_type == PluginType::Auth)
        .filter_map(|p| p.config.clone().map(|c| (p.name.clone(), c)))
        .collect();

    Ok(build_auth_state(AuthStateParams {
        activation,
        endpoint_path: config.server.endpoint_path.clone(),
        bearer_token: providers
            .iter()
            .find_map(AuthProviderConfig::bearer_token)
            .map(std::borrow::ToOwned::to_owned),
        resource: oauth_cfg.map(|oauth| oauth.resource.clone()),
        resource_metadata_url,
        providers,
        plugins,
        auth_plugin_configs,
        scope_challenges_enabled: auth_cfg.is_none_or(AuthConfig::scope_challenges_enabled),
        oauth_client_metadata_document_url: auth_cfg
            .and_then(AuthConfig::oauth_client_metadata_document_url)
            .map(std::borrow::ToOwned::to_owned),
        outbound_timeout_ms: config.outbound_http.as_ref().and_then(|cfg| cfg.timeout_ms),
        outbound_max_response_bytes: config
            .outbound_http
            .as_ref()
            .and_then(|cfg| cfg.max_response_bytes),
        server_log_payload_max_bytes: config.log_payload_max_bytes(),
    }))
}

// Warns when auth is disabled but auth-related fields are populated.
fn warn_unused_auth_settings(config: &McpConfig) {
    let Some(auth) = config.server.auth.as_ref() else {
        return;
    };

    if auth.is_enabled() {
        return;
    }

    if !auth.providers.is_empty() || auth.oauth.is_some() {
        warn!("auth disabled/auth.enabled=false ignores configured auth settings");
    }
}

// Enforces auth invariants for enabled auth configuration.
fn validate_auth_settings(config: &McpConfig) -> Result<(), McpError> {
    let Some(auth) = config.server.auth.as_ref() else {
        return Ok(());
    };

    if !auth.is_enabled() {
        return Ok(());
    }

    if auth.providers.is_empty() {
        return Err(McpError::invalid_request(
            "server.auth.providers must be non-empty when auth is enabled".to_owned(),
            None,
        ));
    }

    let mut seen_names = HashSet::new();
    let mut seen_issuers = HashMap::new();
    let mut allow_missing_iss_count = 0_u32;
    let mut bearer_provider_count = 0_u32;
    for provider in &auth.providers {
        let provider_name = provider.name();
        if !seen_names.insert(provider_name.to_owned()) {
            return Err(McpError::invalid_request(
                format!("duplicate auth provider name: {provider_name}"),
                None,
            ));
        }

        if provider.is_bearer() {
            bearer_provider_count += 1;
        }

        if provider.allow_missing_iss() {
            allow_missing_iss_count += 1;
            if allow_missing_iss_count > 1 {
                return Err(McpError::invalid_request(
                    "at most one auth provider may set allow_missing_iss=true".to_owned(),
                    None,
                ));
            }
        }

        if provider.is_jwks() && provider.issuer().is_none() && provider.discovery_url().is_none() {
            return Err(McpError::invalid_request(
                format!("auth jwks provider '{provider_name}' must set issuer or discovery_url"),
                None,
            ));
        }

        if let Some(issuer) = provider.issuer() {
            let normalized_issuer = issuer.trim().to_ascii_lowercase();
            if normalized_issuer.is_empty() {
                return Err(McpError::invalid_request(
                    format!("auth provider '{provider_name}' has an empty issuer"),
                    None,
                ));
            }
            if let Some(existing_provider) =
                seen_issuers.insert(normalized_issuer, provider_name.to_owned())
            {
                return Err(McpError::invalid_request(
                    format!(
                        "duplicate auth issuer ownership between providers '{existing_provider}' and '{provider_name}'"
                    ),
                    None,
                ));
            }
        }
    }

    if bearer_provider_count > 1 {
        return Err(McpError::invalid_request(
            "at most one auth bearer provider is allowed".to_owned(),
            None,
        ));
    }

    let has_oauth_capable_provider = auth.providers.iter().any(AuthProviderConfig::oauth_capable);

    if has_oauth_capable_provider && auth.oauth_resource().is_none() {
        return Err(McpError::invalid_request(
            "server.auth.oauth.resource is required when oauth-capable providers are configured"
                .to_owned(),
            None,
        ));
    }

    Ok(())
}

// Warns about insecure introspection setups: client_auth_method=none (no auth
// at all) or basic/post without a client_secret.
fn warn_introspection_client_auth(config: &McpConfig) {
    let Some(auth) = config.server.auth.as_ref() else {
        return;
    };
    if !auth.is_active() {
        return;
    }

    for provider in &auth.providers {
        if !provider.is_introspection() {
            continue;
        }
        warn_introspection_provider_auth(provider);
    }
}

// Emits warnings for a single introspection provider's auth method configuration.
fn warn_introspection_provider_auth(provider: &AuthProviderConfig) {
    let method = match provider.introspection_client_auth_method() {
        IntrospectionClientAuthMethod::Basic => Some("basic"),
        IntrospectionClientAuthMethod::Post => Some("post"),
        IntrospectionClientAuthMethod::None => {
            warn!(
                "introspection provider '{}' uses client_auth_method=none",
                provider.name()
            );
            None
        }
    };
    if let Some(method) = method {
        if provider.introspection_client_secret().is_none() {
            warn!(
                "introspection provider '{}' uses '{}' auth without client_secret",
                provider.name(),
                method
            );
        }
    }
}

// Validates auth plugin wiring: every plugin in config.plugins[] must be
// registered in the registry, and per-provider plugins must be allowlisted.
// Also warns about registered plugins that aren't in the allowlist.
fn validate_auth_plugins(
    config: &McpConfig,
    registry: Option<&PluginRegistry>,
) -> Result<(), McpError> {
    let allowlist: HashSet<String> = config
        .plugins
        .iter()
        .filter(|p| p.plugin_type == PluginType::Auth)
        .map(|p| p.name.clone())
        .collect();

    if !allowlist.is_empty() && registry.is_none() {
        return Err(McpError::invalid_request(
            "auth plugins configured but no registry provided".to_owned(),
            None,
        ));
    }

    if let Some(registry) = registry {
        for plugin_name in &allowlist {
            if registry.get_plugin(PluginType::Auth, plugin_name).is_none() {
                return Err(McpError::invalid_request(
                    format!("auth plugin not registered: {plugin_name}"),
                    None,
                ));
            }
        }

        for (name, ptype) in registry.names() {
            if ptype == PluginType::Auth && !allowlist.contains(&name) {
                warn!("auth plugin registered but not allowlisted: {name}");
            }
        }
    }

    let providers = config
        .server
        .auth
        .as_ref()
        .map_or(&[][..], |auth| auth.providers.as_slice());
    for provider in providers {
        if let Some(plugin_name) = provider.plugin_name() {
            if !allowlist.contains(plugin_name) {
                return Err(McpError::invalid_request(
                    format!("auth provider plugin not allowlisted: {plugin_name}"),
                    None,
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_auth_state_with_plugins, validate_auth_plugins, validate_auth_settings,
        warn_introspection_client_auth,
    };
    use crate::config::IntrospectionClientAuthMethod;
    use crate::inline_test_fixtures::{base_config, base_provider};
    use crate::mcp::ErrorCode;
    use crate::plugins::registry::PluginLookup;
    use crate::plugins::{
        AuthPlugin, AuthPluginDecision, AuthPluginValidateParams, PluginRegistry, PluginType,
    };
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::Level;

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture_warn_logs<F>(operation: F) -> String
    where
        F: FnOnce(),
    {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_buffer = Arc::clone(&buffer);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::WARN)
            .without_time()
            .with_ansi(false)
            .with_writer(move || LogWriter(Arc::clone(&writer_buffer)))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        operation();

        let bytes = buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn warn_introspection_client_auth_allows_config() {
        let mut config = base_config();

        let mut provider = crate::config::AuthProviderConfig::introspection(
            "test",
            "http://example.com/introspect",
        );
        let introspection = provider
            .as_introspection_mut()
            .expect("introspection provider");
        introspection.client_id = Some("client".to_owned());
        introspection.client_secret = None;
        introspection.auth_method = IntrospectionClientAuthMethod::Basic;

        let mut provider_none = provider.clone();
        provider_none
            .as_introspection_mut()
            .expect("introspection provider")
            .auth_method = IntrospectionClientAuthMethod::None;

        let mut provider_post = provider.clone();
        provider_post
            .as_introspection_mut()
            .expect("introspection provider")
            .auth_method = IntrospectionClientAuthMethod::Post;

        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![provider, provider_post, provider_none];

        let logs = capture_warn_logs(|| warn_introspection_client_auth(&config));
        assert!(
            logs.contains("introspection provider 'test' uses 'basic' auth without client_secret")
        );
        assert!(
            logs.contains("introspection provider 'test' uses 'post' auth without client_secret")
        );
        assert!(logs.contains("introspection provider 'test' uses client_auth_method=none"));
    }

    #[test]
    fn warn_missing_secret_skips_non_introspection_providers() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![base_provider()];
        let logs = capture_warn_logs(|| warn_introspection_client_auth(&config));
        assert!(logs.is_empty());
    }

    #[test]
    fn warn_missing_secret_skips_when_secret_present() {
        let mut config = base_config();
        let mut provider = crate::config::AuthProviderConfig::introspection(
            "test",
            "http://example.com/introspect",
        );
        let introspection = provider
            .as_introspection_mut()
            .expect("introspection provider");
        introspection.auth_method = IntrospectionClientAuthMethod::Basic;
        introspection.client_secret = Some("secret".to_owned());
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![provider];
        let logs = capture_warn_logs(|| warn_introspection_client_auth(&config));
        assert!(logs.is_empty());
    }

    #[test]
    fn validate_auth_settings_rejects_duplicate_provider_names() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "https://resource.example".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        auth.providers = vec![
            crate::config::AuthProviderConfig::bearer("dup", "first"),
            crate::config::AuthProviderConfig::bearer("dup", "second"),
        ];

        let error =
            validate_auth_settings(&config).expect_err("duplicate provider names must fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "duplicate auth provider name: dup");
        assert!(error.data.is_none());
    }

    #[test]
    fn validate_auth_settings_rejects_duplicate_issuer_ownership() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "https://resource.example".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        let mut first = crate::config::AuthProviderConfig::jwks("first");
        first.as_jwks_mut().expect("jwks").issuer = Some("https://issuer.example".to_owned());
        let mut second = crate::config::AuthProviderConfig::introspection(
            "second",
            "https://issuer.example/introspect",
        );
        second.as_introspection_mut().expect("introspection").issuer =
            Some("https://issuer.example".to_owned());
        auth.providers = vec![first, second];

        let error =
            validate_auth_settings(&config).expect_err("duplicate issuer ownership must fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "duplicate auth issuer ownership between providers 'first' and 'second'"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn validate_auth_settings_rejects_multiple_allow_missing_iss_providers() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "https://resource.example".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        let mut first = crate::config::AuthProviderConfig::introspection(
            "first",
            "https://issuer.example/introspect",
        );
        first
            .as_introspection_mut()
            .expect("introspection")
            .allow_missing_iss = true;
        let mut second = crate::config::AuthProviderConfig::plugin("second", "auth.plugin");
        second.as_plugin_mut().expect("plugin").allow_missing_iss = true;
        auth.providers = vec![first, second];

        let error =
            validate_auth_settings(&config).expect_err("multiple allow_missing_iss must fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "at most one auth provider may set allow_missing_iss=true"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn validate_auth_settings_rejects_jwks_without_issuer_or_discovery() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "https://resource.example".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        auth.providers = vec![crate::config::AuthProviderConfig::jwks("jwks")];

        let error = validate_auth_settings(&config)
            .expect_err("jwks provider without issuer/discovery must fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "auth jwks provider 'jwks' must set issuer or discovery_url"
        );
        assert!(error.data.is_none());
    }

    #[test]
    fn validate_auth_settings_rejects_multiple_bearer_providers() {
        let mut config = base_config();
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![
            crate::config::AuthProviderConfig::bearer("one", "token-a"),
            crate::config::AuthProviderConfig::bearer("two", "token-b"),
        ];

        let error =
            validate_auth_settings(&config).expect_err("multiple bearer providers must fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "at most one auth bearer provider is allowed");
        assert!(error.data.is_none());
    }

    #[test]
    fn build_auth_state_uses_per_provider_discovery_options() {
        let mut config = base_config();
        let mut provider = crate::config::AuthProviderConfig::jwks("jwks");
        let jwks = provider.as_jwks_mut().expect("jwks provider");
        jwks.discovery_url =
            Some("https://issuer.example/.well-known/openid-configuration".to_owned());
        jwks.enable_oidc_discovery = false;
        jwks.allow_well_known_fallback = false;

        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.oauth = Some(crate::config::AuthOauthConfig {
            public_url: None,
            resource: "https://resource.example".to_owned(),
            client_metadata_document_url: None,
            scope_in_challenges: true,
        });
        auth.providers = vec![provider];

        let state = build_auth_state_with_plugins(&config, None).expect("auth state should build");
        let runtime_provider = state.providers().first().expect("provider");
        assert_eq!(runtime_provider.config.name(), "jwks");
        assert!(!runtime_provider.discovery_options.enable_oidc);
        assert!(!runtime_provider.discovery_options.allow_fallback);
    }

    struct NamedAuthPlugin(&'static str);

    #[async_trait::async_trait]
    impl crate::plugins::AuthPlugin for NamedAuthPlugin {
        fn name(&self) -> &str {
            self.0
        }

        async fn validate(&self, _params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
            AuthPluginDecision::Accept
        }
    }

    #[tokio::test]
    async fn validate_auth_plugins_accepts_allowlisted_provider_plugins() {
        let mut config = base_config();
        config.plugins = vec![crate::config::PluginConfig {
            name: "provider.plugin".to_owned(),
            plugin_type: PluginType::Auth,
            targets: None,
            config: None,
        }];
        let provider = crate::config::AuthProviderConfig::plugin("provider", "provider.plugin");
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![provider];
        assert_eq!(
            config
                .server
                .auth
                .as_ref()
                .expect("auth config should be present")
                .providers
                .first()
                .and_then(crate::config::AuthProviderConfig::plugin_name),
            Some("provider.plugin")
        );
        let allowlisted: HashSet<&str> = config
            .plugins
            .iter()
            .filter(|plugin| plugin.plugin_type == PluginType::Auth)
            .map(|plugin| plugin.name.as_str())
            .collect();
        assert_eq!(allowlisted, HashSet::from(["provider.plugin"]));

        let registry = PluginRegistry::new()
            .register_auth(NamedAuthPlugin("provider.plugin"))
            .expect("provider plugin");
        let result = validate_auth_plugins(&config, Some(&registry));
        assert!(
            result.is_ok(),
            "allowlisted provider plugins should validate"
        );
        assert!(registry
            .get_plugin(PluginType::Auth, "provider.plugin")
            .is_some());

        let plugin = NamedAuthPlugin("contract-check");
        let decision = plugin
            .validate(AuthPluginValidateParams {
                token: "token",
                claims: &json!({"sub":"test"}),
                headers: &http::HeaderMap::new(),
                config: &Value::Null,
            })
            .await;
        assert_eq!(decision, AuthPluginDecision::Accept);
    }

    #[test]
    fn validate_auth_plugins_rejects_provider_plugin_not_allowlisted() {
        let mut config = base_config();
        let provider = crate::config::AuthProviderConfig::plugin("provider", "provider.plugin");
        let auth = config.server.auth_mut_or_insert();
        auth.enabled = Some(true);
        auth.providers = vec![provider];

        let error = validate_auth_plugins(&config, Some(&PluginRegistry::new()))
            .expect_err("provider plugin must be allowlisted");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "auth provider plugin not allowlisted: provider.plugin"
        );
    }
}
