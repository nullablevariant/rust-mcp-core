//! Auth configuration types for inbound request authentication.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

const fn default_scope_in_challenges() -> bool {
    true
}

const fn default_enable_oidc_discovery() -> bool {
    true
}

const fn default_allow_well_known_fallback() -> bool {
    true
}

const fn default_allow_missing_iss() -> bool {
    false
}

const fn default_introspection_client_auth_method() -> IntrospectionClientAuthMethod {
    IntrospectionClientAuthMethod::Basic
}

static EMPTY_REQUIRED_CLAIMS: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);

/// Server authentication configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Optional explicit auth toggle. `Some(false)` disables auth.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Ordered list of auth providers evaluated by auth runtime.
    #[serde(default)]
    pub providers: Vec<AuthProviderConfig>,
    /// Optional OAuth protected-resource metadata/challenge settings.
    #[serde(default)]
    pub oauth: Option<AuthOauthConfig>,
}

impl AuthConfig {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled != Some(false)
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_enabled() && !self.providers.is_empty()
    }

    #[must_use]
    pub fn oauth_resource(&self) -> Option<&str> {
        self.oauth.as_ref().map(|oauth| oauth.resource.as_str())
    }

    #[must_use]
    pub fn oauth_public_url(&self) -> Option<&str> {
        self.oauth
            .as_ref()
            .and_then(|oauth| oauth.public_url.as_deref())
    }

    #[must_use]
    pub fn oauth_client_metadata_document_url(&self) -> Option<&str> {
        self.oauth
            .as_ref()
            .and_then(|oauth| oauth.client_metadata_document_url.as_deref())
    }

    #[must_use]
    pub fn scope_challenges_enabled(&self) -> bool {
        self.oauth
            .as_ref()
            .map_or(default_scope_in_challenges(), |oauth| {
                oauth.scope_in_challenges
            })
    }
}

/// OAuth metadata/challenge configuration for this server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthOauthConfig {
    #[serde(default)]
    pub public_url: Option<String>,
    pub resource: String,
    #[serde(default)]
    pub client_metadata_document_url: Option<String>,
    #[serde(default = "default_scope_in_challenges")]
    pub scope_in_challenges: bool,
}

/// Tagged auth provider variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AuthProviderConfig {
    Bearer(AuthBearerProviderConfig),
    Jwks(AuthJwksProviderConfig),
    Introspection(AuthIntrospectionProviderConfig),
    Plugin(AuthPluginProviderConfig),
}

impl AuthProviderConfig {
    #[must_use]
    pub fn jwks(name: impl Into<String>) -> Self {
        Self::Jwks(AuthJwksProviderConfig {
            name: name.into(),
            issuer: None,
            discovery_url: None,
            jwks_url: None,
            audiences: Vec::new(),
            required_scopes: Vec::new(),
            required_claims: HashMap::new(),
            algorithms: Vec::new(),
            clock_skew_sec: None,
            enable_oidc_discovery: default_enable_oidc_discovery(),
            allow_well_known_fallback: default_allow_well_known_fallback(),
        })
    }

    #[must_use]
    pub fn introspection(name: impl Into<String>, introspection_url: impl Into<String>) -> Self {
        Self::Introspection(AuthIntrospectionProviderConfig {
            name: name.into(),
            issuer: None,
            allow_missing_iss: default_allow_missing_iss(),
            introspection_url: introspection_url.into(),
            client_id: None,
            client_secret: None,
            auth_method: default_introspection_client_auth_method(),
            audiences: Vec::new(),
            required_scopes: Vec::new(),
            required_claims: HashMap::new(),
        })
    }

    #[must_use]
    pub fn plugin(name: impl Into<String>, plugin: impl Into<String>) -> Self {
        Self::Plugin(AuthPluginProviderConfig {
            name: name.into(),
            plugin: plugin.into(),
            allow_missing_iss: default_allow_missing_iss(),
            required_scopes: Vec::new(),
        })
    }

    #[must_use]
    pub fn bearer(name: impl Into<String>, token: impl Into<String>) -> Self {
        Self::Bearer(AuthBearerProviderConfig {
            name: name.into(),
            token: token.into(),
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Bearer(cfg) => &cfg.name,
            Self::Jwks(cfg) => &cfg.name,
            Self::Introspection(cfg) => &cfg.name,
            Self::Plugin(cfg) => &cfg.name,
        }
    }

    #[must_use]
    pub const fn allow_missing_iss(&self) -> bool {
        match self {
            Self::Bearer(_) | Self::Jwks(_) => false,
            Self::Introspection(cfg) => cfg.allow_missing_iss,
            Self::Plugin(cfg) => cfg.allow_missing_iss,
        }
    }

    #[must_use]
    pub const fn oauth_capable(&self) -> bool {
        matches!(self, Self::Jwks(_) | Self::Introspection(_))
    }

    #[must_use]
    pub fn issuer(&self) -> Option<&str> {
        match self {
            Self::Jwks(cfg) => cfg.issuer.as_deref(),
            Self::Introspection(cfg) => cfg.issuer.as_deref(),
            Self::Bearer(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn discovery_url(&self) -> Option<&str> {
        match self {
            Self::Jwks(cfg) => cfg.discovery_url.as_deref(),
            Self::Bearer(_) | Self::Introspection(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn jwks_url(&self) -> Option<&str> {
        match self {
            Self::Jwks(cfg) => cfg.jwks_url.as_deref(),
            Self::Bearer(_) | Self::Introspection(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn audiences(&self) -> &[String] {
        match self {
            Self::Jwks(cfg) => &cfg.audiences,
            Self::Introspection(cfg) => &cfg.audiences,
            Self::Bearer(_) | Self::Plugin(_) => &[],
        }
    }

    #[must_use]
    pub fn required_scopes(&self) -> &[String] {
        match self {
            Self::Jwks(cfg) => &cfg.required_scopes,
            Self::Introspection(cfg) => &cfg.required_scopes,
            Self::Plugin(cfg) => &cfg.required_scopes,
            Self::Bearer(_) => &[],
        }
    }

    #[must_use]
    pub fn required_claims(&self) -> &HashMap<String, String> {
        match self {
            Self::Jwks(cfg) => &cfg.required_claims,
            Self::Introspection(cfg) => &cfg.required_claims,
            Self::Bearer(_) | Self::Plugin(_) => &EMPTY_REQUIRED_CLAIMS,
        }
    }

    #[must_use]
    pub fn algorithms(&self) -> &[String] {
        match self {
            Self::Jwks(cfg) => &cfg.algorithms,
            Self::Bearer(_) | Self::Introspection(_) | Self::Plugin(_) => &[],
        }
    }

    #[must_use]
    pub const fn clock_skew_sec(&self) -> Option<u64> {
        match self {
            Self::Jwks(cfg) => cfg.clock_skew_sec,
            Self::Bearer(_) | Self::Introspection(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn introspection_url(&self) -> Option<&str> {
        match self {
            Self::Introspection(cfg) => Some(&cfg.introspection_url),
            Self::Bearer(_) | Self::Jwks(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn introspection_client_id(&self) -> Option<&str> {
        match self {
            Self::Introspection(cfg) => cfg.client_id.as_deref(),
            Self::Bearer(_) | Self::Jwks(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub fn introspection_client_secret(&self) -> Option<&str> {
        match self {
            Self::Introspection(cfg) => cfg.client_secret.as_deref(),
            Self::Bearer(_) | Self::Jwks(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub const fn introspection_client_auth_method(&self) -> IntrospectionClientAuthMethod {
        match self {
            Self::Introspection(cfg) => cfg.auth_method,
            Self::Bearer(_) | Self::Jwks(_) | Self::Plugin(_) => {
                IntrospectionClientAuthMethod::Basic
            }
        }
    }

    #[must_use]
    pub fn plugin_name(&self) -> Option<&str> {
        match self {
            Self::Plugin(cfg) => Some(&cfg.plugin),
            Self::Bearer(_) | Self::Jwks(_) | Self::Introspection(_) => None,
        }
    }

    #[must_use]
    pub fn bearer_token(&self) -> Option<&str> {
        match self {
            Self::Bearer(cfg) => Some(&cfg.token),
            Self::Jwks(_) | Self::Introspection(_) | Self::Plugin(_) => None,
        }
    }

    #[must_use]
    pub const fn is_jwks(&self) -> bool {
        matches!(self, Self::Jwks(_))
    }

    #[must_use]
    pub const fn is_introspection(&self) -> bool {
        matches!(self, Self::Introspection(_))
    }

    #[must_use]
    pub const fn is_plugin(&self) -> bool {
        matches!(self, Self::Plugin(_))
    }

    #[must_use]
    pub const fn is_bearer(&self) -> bool {
        matches!(self, Self::Bearer(_))
    }

    #[must_use]
    pub const fn as_jwks_mut(&mut self) -> Option<&mut AuthJwksProviderConfig> {
        match self {
            Self::Jwks(cfg) => Some(cfg),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_introspection_mut(&mut self) -> Option<&mut AuthIntrospectionProviderConfig> {
        match self {
            Self::Introspection(cfg) => Some(cfg),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_plugin_mut(&mut self) -> Option<&mut AuthPluginProviderConfig> {
        match self {
            Self::Plugin(cfg) => Some(cfg),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bearer_mut(&mut self) -> Option<&mut AuthBearerProviderConfig> {
        match self {
            Self::Bearer(cfg) => Some(cfg),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthBearerProviderConfig {
    pub name: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthJwksProviderConfig {
    pub name: String,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub discovery_url: Option<String>,
    #[serde(default)]
    pub jwks_url: Option<String>,
    #[serde(default)]
    pub audiences: Vec<String>,
    #[serde(default)]
    pub required_scopes: Vec<String>,
    #[serde(default)]
    pub required_claims: HashMap<String, String>,
    #[serde(default)]
    pub algorithms: Vec<String>,
    #[serde(default)]
    pub clock_skew_sec: Option<u64>,
    #[serde(default = "default_enable_oidc_discovery")]
    pub enable_oidc_discovery: bool,
    #[serde(default = "default_allow_well_known_fallback")]
    pub allow_well_known_fallback: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthIntrospectionProviderConfig {
    pub name: String,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default = "default_allow_missing_iss")]
    pub allow_missing_iss: bool,
    pub introspection_url: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default = "default_introspection_client_auth_method")]
    pub auth_method: IntrospectionClientAuthMethod,
    #[serde(default)]
    pub audiences: Vec<String>,
    #[serde(default)]
    pub required_scopes: Vec<String>,
    #[serde(default)]
    pub required_claims: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthPluginProviderConfig {
    pub name: String,
    pub plugin: String,
    #[serde(default = "default_allow_missing_iss")]
    pub allow_missing_iss: bool,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IntrospectionClientAuthMethod {
    Basic,
    Post,
    None,
}
