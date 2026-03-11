//! Transport configuration types for stdio and streamable HTTP modes.
use serde::{Deserialize, Serialize};

const fn default_transport_mode() -> TransportMode {
    TransportMode::StreamableHttp
}

const fn default_streamable_http_enable_get_stream() -> bool {
    true
}

const fn default_streamable_http_enable_sse_resumption() -> bool {
    false
}

const fn default_streamable_http_session_mode() -> StreamableHttpSessionMode {
    StreamableHttpSessionMode::Optional
}

const fn default_streamable_http_allow_delete_session() -> bool {
    false
}

const fn default_protocol_version_negotiation_mode() -> ProtocolVersionNegotiationMode {
    ProtocolVersionNegotiationMode::Strict
}

const fn default_streamable_http_max_request_bytes() -> u64 {
    1_048_576
}

const fn default_streamable_http_catch_panics() -> bool {
    true
}

const fn default_streamable_http_sanitize_sensitive_headers() -> bool {
    true
}

const fn default_rate_limit_key_source() -> StreamableHttpRateLimitKeySource {
    StreamableHttpRateLimitKeySource::PeerAddr
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default = "default_transport_mode")]
    pub mode: TransportMode,
    #[serde(default)]
    pub streamable_http: StreamableHttpTransportConfig,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            mode: default_transport_mode(),
            streamable_http: StreamableHttpTransportConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamableHttpTransportConfig {
    #[serde(default = "default_streamable_http_enable_get_stream")]
    pub enable_get_stream: bool,
    #[serde(default = "default_streamable_http_enable_sse_resumption")]
    pub enable_sse_resumption: bool,
    #[serde(default = "default_streamable_http_session_mode")]
    pub session_mode: StreamableHttpSessionMode,
    #[serde(default = "default_streamable_http_allow_delete_session")]
    pub allow_delete_session: bool,
    #[serde(default)]
    pub sse_keep_alive_ms: Option<u64>,
    #[serde(default)]
    pub sse_retry_ms: Option<u64>,
    #[serde(default)]
    pub protocol_version_negotiation: ProtocolVersionNegotiationConfig,
    #[serde(default)]
    pub hardening: Option<StreamableHttpHardeningConfig>,
}

impl Default for StreamableHttpTransportConfig {
    fn default() -> Self {
        Self {
            enable_get_stream: default_streamable_http_enable_get_stream(),
            enable_sse_resumption: default_streamable_http_enable_sse_resumption(),
            session_mode: default_streamable_http_session_mode(),
            allow_delete_session: default_streamable_http_allow_delete_session(),
            sse_keep_alive_ms: None,
            sse_retry_ms: None,
            protocol_version_negotiation: ProtocolVersionNegotiationConfig::default(),
            hardening: None,
        }
    }
}

impl StreamableHttpTransportConfig {
    #[must_use]
    pub fn effective_hardening(&self) -> StreamableHttpHardeningConfig {
        self.hardening.clone().unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamableHttpHardeningConfig {
    #[serde(default = "default_streamable_http_max_request_bytes")]
    pub max_request_bytes: u64,
    #[serde(default = "default_streamable_http_catch_panics")]
    pub catch_panics: bool,
    #[serde(default = "default_streamable_http_sanitize_sensitive_headers")]
    pub sanitize_sensitive_headers: bool,
    #[serde(default)]
    pub session: Option<StreamableHttpSessionHardeningConfig>,
    #[serde(default)]
    pub rate_limit: Option<StreamableHttpRateLimitConfig>,
}

impl Default for StreamableHttpHardeningConfig {
    fn default() -> Self {
        Self {
            max_request_bytes: default_streamable_http_max_request_bytes(),
            catch_panics: default_streamable_http_catch_panics(),
            sanitize_sensitive_headers: default_streamable_http_sanitize_sensitive_headers(),
            session: None,
            rate_limit: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamableHttpSessionHardeningConfig {
    #[serde(default)]
    pub max_sessions: Option<u64>,
    #[serde(default)]
    pub idle_ttl_secs: Option<u64>,
    #[serde(default)]
    pub max_lifetime_secs: Option<u64>,
    #[serde(default)]
    pub creation_rate: Option<StreamableHttpRateLimitConfig>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamableHttpRateLimitConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub global: Option<StreamableHttpRateBucketConfig>,
    #[serde(default)]
    pub per_ip: Option<StreamableHttpPerIpRateBucketConfig>,
}

impl StreamableHttpRateLimitConfig {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled != Some(false)
    }

    #[must_use]
    pub const fn has_any_bucket(&self) -> bool {
        self.global.is_some() || self.per_ip.is_some()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StreamableHttpRateBucketConfig {
    pub capacity: u64,
    pub refill_per_sec: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StreamableHttpPerIpRateBucketConfig {
    pub capacity: u64,
    pub refill_per_sec: u64,
    #[serde(default = "default_rate_limit_key_source")]
    pub key_source: StreamableHttpRateLimitKeySource,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamableHttpRateLimitKeySource {
    PeerAddr,
    XForwardedFor,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolVersionNegotiationConfig {
    #[serde(default = "default_protocol_version_negotiation_mode")]
    pub mode: ProtocolVersionNegotiationMode,
}

impl Default for ProtocolVersionNegotiationConfig {
    fn default() -> Self {
        Self {
            mode: default_protocol_version_negotiation_mode(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolVersionNegotiationMode {
    Strict,
    Negotiate,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamableHttpSessionMode {
    None,
    Optional,
    Required,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    StreamableHttp,
    Stdio,
}
