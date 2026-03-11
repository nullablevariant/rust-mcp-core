//! Streamable HTTP hardening primitives: session wrapper and session-create throttling.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::Router;
use futures_core::Stream;
use http::Request as HttpRequest;
use rmcp::model::ClientJsonRpcMessage;
use rmcp::model::ServerJsonRpcMessage;
use rmcp::transport::streamable_http_server::session::local::{
    LocalSessionManager, LocalSessionManagerError, SessionConfig,
};
use rmcp::transport::streamable_http_server::session::{
    ServerSseMessage, SessionId, SessionManager,
};

use crate::config::{
    StreamableHttpHardeningConfig, StreamableHttpPerIpRateBucketConfig,
    StreamableHttpRateBucketConfig, StreamableHttpRateLimitKeySource,
    StreamableHttpSessionHardeningConfig, StreamableHttpTransportConfig,
};
use crate::McpError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::{GlobalKeyExtractor, KeyExtractor};
use tower_governor::{GovernorError, GovernorLayer};

use super::MCP_SESSION_ID_HEADER;

const NANOS_PER_SEC: u128 = 1_000_000_000;

#[derive(Debug)]
pub(crate) enum HardeningSessionManagerError {
    Local(LocalSessionManagerError),
    SessionLimitExceeded { limit: u64 },
}

impl std::fmt::Display for HardeningSessionManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(error) => write!(f, "{error}"),
            Self::SessionLimitExceeded { limit } => {
                write!(f, "session limit exceeded: max_sessions={limit}")
            }
        }
    }
}

impl std::error::Error for HardeningSessionManagerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Local(error) => Some(error),
            Self::SessionLimitExceeded { .. } => None,
        }
    }
}

impl From<LocalSessionManagerError> for HardeningSessionManagerError {
    fn from(value: LocalSessionManagerError) -> Self {
        Self::Local(value)
    }
}

#[derive(Debug)]
pub(crate) struct HardeningSessionManager {
    inner: LocalSessionManager,
    created_at: tokio::sync::RwLock<HashMap<SessionId, Instant>>,
    max_sessions: Option<u64>,
    max_lifetime: Option<Duration>,
}

impl HardeningSessionManager {
    #[must_use]
    pub(crate) fn new(
        session_config: SessionConfig,
        session_hardening: Option<&StreamableHttpSessionHardeningConfig>,
    ) -> Self {
        let max_sessions = session_hardening.and_then(|config| config.max_sessions);
        let max_lifetime = session_hardening
            .and_then(|config| config.max_lifetime_secs)
            .map(Duration::from_secs);
        Self {
            inner: LocalSessionManager {
                sessions: tokio::sync::RwLock::new(HashMap::new()),
                session_config,
            },
            created_at: tokio::sync::RwLock::new(HashMap::new()),
            max_sessions,
            max_lifetime,
        }
    }

    async fn enforce_session_limit(&self) -> Result<(), HardeningSessionManagerError> {
        self.prune_expired_sessions().await?;
        if let Some(limit) = self.max_sessions {
            let count = self.inner.sessions.read().await.len();
            let count_u64 = u64::try_from(count).unwrap_or(u64::MAX);
            if count_u64 >= limit {
                return Err(HardeningSessionManagerError::SessionLimitExceeded { limit });
            }
        }
        Ok(())
    }

    async fn prune_expired_sessions(&self) -> Result<(), HardeningSessionManagerError> {
        let Some(max_lifetime) = self.max_lifetime else {
            return Ok(());
        };

        let now = Instant::now();
        let created_at = self.created_at.read().await;
        let expired_ids = created_at
            .iter()
            .filter_map(|(id, created)| {
                if now.duration_since(*created) >= max_lifetime {
                    Some(Arc::clone(id))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        drop(created_at);

        for id in expired_ids {
            self.inner.close_session(&id).await?;
            self.created_at.write().await.remove(&id);
        }

        Ok(())
    }

    async fn expire_if_needed(&self, id: &SessionId) -> Result<bool, HardeningSessionManagerError> {
        let Some(max_lifetime) = self.max_lifetime else {
            return Ok(false);
        };

        let created_at = self.created_at.read().await.get(id).copied();
        let Some(created_at) = created_at else {
            return Ok(false);
        };

        if created_at.elapsed() < max_lifetime {
            return Ok(false);
        }

        self.inner.close_session(id).await?;
        self.created_at.write().await.remove(id);
        Ok(true)
    }
}

impl SessionManager for HardeningSessionManager {
    type Error = HardeningSessionManagerError;
    type Transport = <LocalSessionManager as SessionManager>::Transport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        self.enforce_session_limit().await?;
        let (id, transport) = self.inner.create_session().await?;
        self.created_at
            .write()
            .await
            .insert(Arc::clone(&id), Instant::now());
        Ok((id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if self.expire_if_needed(id).await? {
            return Err(LocalSessionManagerError::SessionNotFound(Arc::clone(id)).into());
        }
        self.inner
            .initialize_session(id, message)
            .await
            .map_err(Into::into)
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        if self.expire_if_needed(id).await? {
            return Ok(false);
        }
        self.inner.has_session(id).await.map_err(Into::into)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        self.inner.close_session(id).await?;
        self.created_at.write().await.remove(id);
        Ok(())
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        if self.expire_if_needed(id).await? {
            return Err(LocalSessionManagerError::SessionNotFound(Arc::clone(id)).into());
        }
        self.inner
            .create_stream(id, message)
            .await
            .map_err(Into::into)
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        if self.expire_if_needed(id).await? {
            return Err(LocalSessionManagerError::SessionNotFound(Arc::clone(id)).into());
        }
        self.inner
            .accept_message(id, message)
            .await
            .map_err(Into::into)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        if self.expire_if_needed(id).await? {
            return Err(LocalSessionManagerError::SessionNotFound(Arc::clone(id)).into());
        }
        self.inner
            .create_standalone_stream(id)
            .await
            .map_err(Into::into)
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        if self.expire_if_needed(id).await? {
            return Err(LocalSessionManagerError::SessionNotFound(Arc::clone(id)).into());
        }
        self.inner
            .resume(id, last_event_id)
            .await
            .map_err(Into::into)
    }
}

#[must_use]
pub(crate) fn build_streamable_http_session_manager(
    transport: &StreamableHttpTransportConfig,
) -> Arc<HardeningSessionManager> {
    let mut session_config = SessionConfig::default();
    if let Some(idle_ttl_secs) = transport
        .hardening
        .as_ref()
        .and_then(|hardening| hardening.session.as_ref())
        .and_then(|session| session.idle_ttl_secs)
    {
        session_config.keep_alive = Some(Duration::from_secs(idle_ttl_secs));
    }

    Arc::new(HardeningSessionManager::new(
        session_config,
        transport
            .hardening
            .as_ref()
            .and_then(|hardening| hardening.session.as_ref()),
    ))
}

#[derive(Clone, Copy)]
struct RateBucketSpec {
    capacity: u64,
    refill_per_sec: u64,
}

#[derive(Clone, Copy)]
struct TokenBucket {
    tokens: u64,
    last_refill: Instant,
    rem_nanos: u128,
}

impl TokenBucket {
    fn new(spec: RateBucketSpec) -> Self {
        Self {
            tokens: spec.capacity,
            last_refill: Instant::now(),
            rem_nanos: 0,
        }
    }

    fn take(&mut self, spec: RateBucketSpec) -> bool {
        let now = Instant::now();
        let elapsed_nanos = now.duration_since(self.last_refill).as_nanos();
        self.last_refill = now;
        let refill_nanos = elapsed_nanos
            .saturating_mul(u128::from(spec.refill_per_sec))
            .saturating_add(self.rem_nanos);
        let produced = refill_nanos / NANOS_PER_SEC;
        self.rem_nanos = refill_nanos % NANOS_PER_SEC;
        let produced_u64 = u64::try_from(produced).unwrap_or(u64::MAX);
        self.tokens = self.tokens.saturating_add(produced_u64).min(spec.capacity);

        if self.tokens >= 1 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionCreationRateLimiter {
    global: Option<Arc<tokio::sync::Mutex<TokenBucket>>>,
    global_spec: Option<RateBucketSpec>,
    per_ip: Option<Arc<tokio::sync::Mutex<HashMap<String, TokenBucket>>>>,
    per_ip_spec: Option<RateBucketSpec>,
    per_ip_key_source: StreamableHttpRateLimitKeySource,
}

impl SessionCreationRateLimiter {
    pub(crate) fn extract_key(&self, request: &HttpRequest<Body>) -> Option<String> {
        self.per_ip_spec
            .map(|_| extract_rate_limit_key(request, self.per_ip_key_source))
    }

    pub(crate) async fn allow_with_key(&self, per_ip_key: Option<String>) -> bool {
        if let (Some(spec), Some(bucket)) = (self.global_spec, self.global.as_ref()) {
            let mut bucket = bucket.lock().await;
            if !bucket.take(spec) {
                return false;
            }
        }

        if let (Some(spec), Some(per_ip)) = (self.per_ip_spec, self.per_ip.as_ref()) {
            let key = per_ip_key.unwrap_or_else(|| "unknown-peer".to_owned());
            let mut buckets = per_ip.lock().await;
            let bucket = buckets.entry(key).or_insert_with(|| TokenBucket::new(spec));
            if !bucket.take(spec) {
                return false;
            }
        }

        true
    }
}

#[must_use]
pub(crate) fn build_session_creation_rate_limiter(
    hardening: &StreamableHttpHardeningConfig,
) -> Option<Arc<SessionCreationRateLimiter>> {
    let creation_rate = hardening
        .session
        .as_ref()
        .and_then(|session| session.creation_rate.as_ref())
        .filter(|rate| rate.is_active())?;

    if creation_rate.global.is_none() && creation_rate.per_ip.is_none() {
        return None;
    }

    let (global, global_spec) = if let Some(global) = creation_rate.global {
        let spec = to_spec(global);
        (
            Some(Arc::new(tokio::sync::Mutex::new(TokenBucket::new(spec)))),
            Some(spec),
        )
    } else {
        (None, None)
    };

    let (per_ip, per_ip_spec, per_ip_key_source) = if let Some(per_ip) = creation_rate.per_ip {
        (
            Some(Arc::new(tokio::sync::Mutex::new(HashMap::new()))),
            Some(to_spec_per_ip(per_ip)),
            per_ip.key_source,
        )
    } else {
        (None, None, StreamableHttpRateLimitKeySource::PeerAddr)
    };

    Some(Arc::new(SessionCreationRateLimiter {
        global,
        global_spec,
        per_ip,
        per_ip_spec,
        per_ip_key_source,
    }))
}

pub(crate) fn is_session_creation_request(request: &HttpRequest<Body>) -> bool {
    request.method() == axum::http::Method::POST
        && !request.headers().contains_key(MCP_SESSION_ID_HEADER)
}

fn extract_rate_limit_key<T>(
    request: &HttpRequest<T>,
    key_source: StreamableHttpRateLimitKeySource,
) -> String {
    match key_source {
        StreamableHttpRateLimitKeySource::PeerAddr => request_peer_key(request),
        StreamableHttpRateLimitKeySource::XForwardedFor => request
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map_or_else(|| request_peer_key(request), str::to_owned),
    }
}

fn request_peer_key<T>(request: &HttpRequest<T>) -> String {
    request.extensions().get::<SocketAddr>().map_or_else(
        || "unknown-peer".to_owned(),
        |socket| socket.ip().to_string(),
    )
}

const fn to_spec(config: StreamableHttpRateBucketConfig) -> RateBucketSpec {
    RateBucketSpec {
        capacity: config.capacity,
        refill_per_sec: config.refill_per_sec,
    }
}

const fn to_spec_per_ip(config: StreamableHttpPerIpRateBucketConfig) -> RateBucketSpec {
    RateBucketSpec {
        capacity: config.capacity,
        refill_per_sec: config.refill_per_sec,
    }
}

#[derive(Clone, Copy)]
struct PerIpRateLimitKeyExtractor {
    key_source: StreamableHttpRateLimitKeySource,
}

impl KeyExtractor for PerIpRateLimitKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &HttpRequest<T>) -> Result<Self::Key, GovernorError> {
        Ok(extract_rate_limit_key(req, self.key_source))
    }
}

fn rate_bucket_period(refill_per_sec: u64, config_path: &str) -> Result<Duration, McpError> {
    if refill_per_sec == 0 {
        return Err(McpError::invalid_request(
            format!("{config_path}.refill_per_sec must be greater than 0"),
            None,
        ));
    }
    let nanos = (NANOS_PER_SEC / u128::from(refill_per_sec)).max(1);
    Ok(Duration::from_nanos(u64::try_from(nanos).unwrap_or(1)))
}

fn rate_bucket_burst_size(capacity: u64, config_path: &str) -> Result<u32, McpError> {
    if capacity == 0 {
        return Err(McpError::invalid_request(
            format!("{config_path}.capacity must be greater than 0"),
            None,
        ));
    }
    u32::try_from(capacity).map_err(|_| {
        McpError::invalid_request(
            format!(
                "{config_path}.capacity exceeds maximum supported value {}",
                u32::MAX
            ),
            None,
        )
    })
}

fn build_global_rate_limit_layer(
    router: Router,
    bucket: StreamableHttpRateBucketConfig,
) -> Result<Router, McpError> {
    let config_path = "transport.streamable_http.hardening.rate_limit.global";
    let mut builder = GovernorConfigBuilder::default().key_extractor(GlobalKeyExtractor);
    builder
        .period(rate_bucket_period(bucket.refill_per_sec, config_path)?)
        .burst_size(rate_bucket_burst_size(bucket.capacity, config_path)?);
    let config = builder.finish().ok_or_else(|| {
        McpError::invalid_request(
            format!("{config_path} must define positive rate values"),
            None,
        )
    })?;
    Ok(router.layer(GovernorLayer::new(config)))
}

fn build_per_ip_rate_limit_layer(
    router: Router,
    bucket: StreamableHttpPerIpRateBucketConfig,
) -> Result<Router, McpError> {
    let config_path = "transport.streamable_http.hardening.rate_limit.per_ip";
    let mut builder = GovernorConfigBuilder::default().key_extractor(PerIpRateLimitKeyExtractor {
        key_source: bucket.key_source,
    });
    builder
        .period(rate_bucket_period(bucket.refill_per_sec, config_path)?)
        .burst_size(rate_bucket_burst_size(bucket.capacity, config_path)?);
    let config = builder.finish().ok_or_else(|| {
        McpError::invalid_request(
            format!("{config_path} must define positive rate values"),
            None,
        )
    })?;
    Ok(router.layer(GovernorLayer::new(config)))
}

pub(crate) fn apply_inbound_rate_limit_layer(
    mut router: Router,
    hardening: &StreamableHttpHardeningConfig,
) -> Result<Router, McpError> {
    let Some(rate_limit) = hardening
        .rate_limit
        .as_ref()
        .filter(|rate_limit| rate_limit.is_active())
    else {
        return Ok(router);
    };

    if !rate_limit.has_any_bucket() {
        return Err(McpError::invalid_request(
            "transport.streamable_http.hardening.rate_limit requires global or per_ip when enabled"
                .to_owned(),
            None,
        ));
    }

    if let Some(global_bucket) = rate_limit.global {
        router = build_global_rate_limit_layer(router, global_bucket)?;
    }
    if let Some(per_ip_bucket) = rate_limit.per_ip {
        router = build_per_ip_rate_limit_layer(router, per_ip_bucket)?;
    }

    Ok(router)
}
