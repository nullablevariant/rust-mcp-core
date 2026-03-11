//! Upstream service and outbound HTTP configuration types.
use std::{collections::HashMap, fmt, fs};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamConfig {
    pub base_url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub user_agent: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_response_bytes: Option<u64>,
    #[serde(default)]
    pub retry: Option<OutboundRetryConfig>,
    #[serde(default)]
    pub auth: Option<UpstreamAuth>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UpstreamAuth {
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    #[serde(rename = "oauth2")]
    Oauth2(Box<UpstreamOauth2AuthConfig>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamOauth2AuthConfig {
    pub grant: UpstreamOauth2GrantType,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: SecretValueConfig,
    pub auth_method: Option<UpstreamOauth2ClientAuthMethod>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub audience: Option<String>,
    #[serde(default)]
    pub extra_token_params: HashMap<String, String>,
    pub refresh: Option<UpstreamOauth2RefreshConfig>,
    pub refresh_token: Option<SecretValueConfig>,
    pub mtls: Option<UpstreamOauth2MtlsConfig>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamOauth2GrantType {
    ClientCredentials,
    RefreshToken,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamOauth2ClientAuthMethod {
    Basic,
    RequestBody,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamOauth2RefreshConfig {
    pub skew_sec: Option<u64>,
    pub retry_on_401_once: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamOauth2MtlsConfig {
    pub ca_cert: Option<SecretValueConfig>,
    pub client_cert: SecretValueConfig,
    pub client_key: SecretValueConfig,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SecretValueConfig {
    pub source: SecretValueSource,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SecretValueSource {
    Inline,
    Env,
    Path,
}

impl SecretValueConfig {
    pub fn resolve_secret(&self) -> Result<SecretString, String> {
        match self.source {
            SecretValueSource::Inline => Ok(SecretString::new(self.value.clone().into_boxed_str())),
            SecretValueSource::Env => std::env::var(&self.value)
                .map(|value| SecretString::new(value.into_boxed_str()))
                .map_err(|error| {
                    format!(
                        "failed to read env var '{}' for secret value: {error}",
                        self.value
                    )
                }),
            SecretValueSource::Path => fs::read_to_string(&self.value)
                .map(|value| SecretString::new(value.trim_end().to_owned().into_boxed_str()))
                .map_err(|error| format!("failed to read secret file '{}': {error}", self.value)),
        }
    }
}

impl fmt::Debug for SecretValueConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value: &str = if self.source == SecretValueSource::Inline {
            "[REDACTED]"
        } else {
            self.value.as_str()
        };
        f.debug_struct("SecretValueConfig")
            .field("source", &self.source)
            .field("value", &value)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundRetryConfig {
    pub max_attempts: u32,
    pub delay_ms: u64,
    #[serde(default)]
    pub on_network_errors: bool,
    #[serde(default)]
    pub on_statuses: Vec<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundHttpConfig {
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub user_agent: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_response_bytes: Option<u64>,
    #[serde(default)]
    pub retry: Option<OutboundRetryConfig>,
}

impl OutboundRetryConfig {
    #[must_use]
    pub const fn enabled(&self) -> bool {
        true
    }
}

impl Default for OutboundRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_max_attempts(),
            delay_ms: default_retry_delay_ms(),
            on_network_errors: default_on_network_errors(),
            on_statuses: default_on_statuses(),
        }
    }
}

const fn default_retry_max_attempts() -> u32 {
    3
}

const fn default_retry_delay_ms() -> u64 {
    200
}

const fn default_on_network_errors() -> bool {
    true
}

fn default_on_statuses() -> Vec<u16> {
    vec![429, 502, 503, 504]
}
