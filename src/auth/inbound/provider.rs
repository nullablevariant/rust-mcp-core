//! Auth provider configuration and JWKS/introspection endpoint resolution.
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use crate::config::AuthProviderConfig;
use crate::http::client::{HttpClient, OutboundHttpRequest};
use crate::log_safety::truncate_string_for_log;

pub(super) const JWKS_TTL_MINUTES: i64 = 10;
pub(super) const DISCOVERY_TTL_MINUTES: i64 = 10;
const DEFAULT_SERVER_LOG_PAYLOAD_MAX_BYTES: u64 = 4096;

// Options controlling OIDC discovery URL generation for an auth provider.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiscoveryOptions {
    pub(crate) enable_oidc: bool,
    pub(crate) allow_fallback: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            enable_oidc: true,
            allow_fallback: true,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct TimedCacheEntry<T> {
    fetched_at: DateTime<Utc>,
    value: T,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct DiscoveryDocument {
    pub(super) issuer: Option<String>,
    pub(super) jwks_uri: Option<String>,
}

pub(super) type JwksCacheEntry = TimedCacheEntry<Arc<JwkSet>>;
pub(super) type DiscoveryCacheEntry = TimedCacheEntry<Arc<DiscoveryDocument>>;

#[derive(Clone, Debug)]
pub(crate) struct AuthProvider {
    pub(crate) config: AuthProviderConfig,
    pub(super) outbound_timeout_ms: Option<u64>,
    pub(super) outbound_max_response_bytes: Option<u64>,
    pub(super) discovery: Arc<RwLock<Option<DiscoveryCacheEntry>>>,
    pub(super) jwks_cache: Arc<RwLock<Option<JwksCacheEntry>>>,
    discovery_refresh_lock: Arc<Mutex<()>>,
    jwks_refresh_lock: Arc<Mutex<()>>,
    pub(super) discovery_options: DiscoveryOptions,
    server_log_payload_max_bytes: u64,
}

impl AuthProvider {
    pub(crate) fn new(
        config: AuthProviderConfig,
        outbound_timeout_ms: Option<u64>,
        outbound_max_response_bytes: Option<u64>,
    ) -> Self {
        let discovery_options = match &config {
            AuthProviderConfig::Jwks(cfg) => DiscoveryOptions {
                enable_oidc: cfg.enable_oidc_discovery,
                allow_fallback: cfg.allow_well_known_fallback,
            },
            AuthProviderConfig::Bearer(_)
            | AuthProviderConfig::Introspection(_)
            | AuthProviderConfig::Plugin(_) => DiscoveryOptions::default(),
        };
        Self {
            config,
            outbound_timeout_ms,
            outbound_max_response_bytes,
            discovery: Arc::new(RwLock::new(None)),
            jwks_cache: Arc::new(RwLock::new(None)),
            discovery_refresh_lock: Arc::new(Mutex::new(())),
            jwks_refresh_lock: Arc::new(Mutex::new(())),
            discovery_options,
            server_log_payload_max_bytes: DEFAULT_SERVER_LOG_PAYLOAD_MAX_BYTES,
        }
    }

    pub(crate) const fn with_server_log_payload_max_bytes(mut self, max_bytes: u64) -> Self {
        self.server_log_payload_max_bytes = max_bytes;
        self
    }

    fn truncate_log_message(&self, message: &str) -> crate::log_safety::TruncatedField {
        truncate_string_for_log(message, self.server_log_payload_max_bytes)
    }

    fn log_discovery_request_error(
        &self,
        provider_name: &str,
        discovery_url: &str,
        error: &rmcp::ErrorData,
    ) {
        let safe_message = self.truncate_log_message(&error.message);
        debug!(
            provider = %provider_name,
            discovery_url = %discovery_url,
            error_message = %safe_message.value,
            error_message_bytes = safe_message.original_bytes,
            error_message_truncated = safe_message.truncated,
            "auth discovery candidate request failed"
        );
    }

    fn log_discovery_non_success(provider_name: &str, discovery_url: &str, status: u16) {
        debug!(
            provider = %provider_name,
            discovery_url = %discovery_url,
            status,
            "auth discovery candidate returned non-success status"
        );
    }

    fn log_discovery_parse_error(
        &self,
        provider_name: &str,
        discovery_url: &str,
        error: &rmcp::ErrorData,
    ) {
        let safe_message = self.truncate_log_message(&error.message);
        debug!(
            provider = %provider_name,
            discovery_url = %discovery_url,
            error_message = %safe_message.value,
            error_message_bytes = safe_message.original_bytes,
            error_message_truncated = safe_message.truncated,
            "auth discovery candidate returned invalid JSON"
        );
    }

    async fn fetch_discovery_candidate(
        &self,
        client: &dyn HttpClient,
        provider_name: &str,
        discovery_url: &str,
    ) -> Option<Arc<DiscoveryDocument>> {
        let response = match client
            .send(OutboundHttpRequest {
                method: "GET".to_owned(),
                url: discovery_url.to_owned(),
                timeout_ms: self.outbound_timeout_ms,
                max_response_bytes: self.outbound_max_response_bytes,
                ..OutboundHttpRequest::default()
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                self.log_discovery_request_error(provider_name, discovery_url, &error);
                return None;
            }
        };

        if !response.is_success() {
            Self::log_discovery_non_success(provider_name, discovery_url, response.status());
            return None;
        }

        match response.json::<DiscoveryDocument>() {
            Ok(doc) => Some(Arc::new(doc)),
            Err(error) => {
                self.log_discovery_parse_error(provider_name, discovery_url, &error);
                None
            }
        }
    }

    fn log_jwks_request_error(&self, provider_name: &str, jwks_url: &str, error: &rmcp::ErrorData) {
        let safe_message = self.truncate_log_message(&error.message);
        warn!(
            provider = %provider_name,
            jwks_url = %jwks_url,
            error_message = %safe_message.value,
            error_message_bytes = safe_message.original_bytes,
            error_message_truncated = safe_message.truncated,
            "auth jwks request failed"
        );
    }

    fn log_jwks_non_success(provider_name: &str, jwks_url: &str, status: u16) {
        warn!(
            provider = %provider_name,
            jwks_url = %jwks_url,
            status,
            "auth jwks endpoint returned non-success status"
        );
    }

    fn log_jwks_parse_error(&self, provider_name: &str, jwks_url: &str, error: &rmcp::ErrorData) {
        let safe_message = self.truncate_log_message(&error.message);
        warn!(
            provider = %provider_name,
            jwks_url = %jwks_url,
            error_message = %safe_message.value,
            error_message_bytes = safe_message.original_bytes,
            error_message_truncated = safe_message.truncated,
            "auth jwks endpoint returned invalid JSON"
        );
    }

    pub(crate) async fn issuer(&self, client: &dyn HttpClient) -> Option<String> {
        if let Some(issuer) = self.config.issuer() {
            return Some(issuer.to_owned());
        }
        let discovery = self.discovery(client).await?;
        discovery.issuer.clone()
    }

    pub(crate) async fn jwks_url(&self, client: &dyn HttpClient) -> Option<String> {
        if let Some(jwks) = self.config.jwks_url() {
            return Some(jwks.to_owned());
        }
        let discovery = self.discovery(client).await?;
        discovery.jwks_uri.clone()
    }

    pub(super) async fn discovery(
        &self,
        client: &dyn HttpClient,
    ) -> Option<Arc<DiscoveryDocument>> {
        cached_fetch(
            &self.discovery,
            &self.discovery_refresh_lock,
            ChronoDuration::minutes(DISCOVERY_TTL_MINUTES),
            || self.fetch_discovery(client),
        )
        .await
    }

    // Builds the list of discovery URLs to try: explicit discovery_url if
    // configured, otherwise derives .well-known paths from the issuer URL
    // per RFC 8414 (OAuth) and OpenID Connect Discovery.
    pub(super) fn discovery_candidates(&self) -> Vec<String> {
        if let Some(url) = self.config.discovery_url() {
            return vec![url.to_owned()];
        }

        let Some(issuer) = self.config.issuer() else {
            return Vec::new();
        };
        let mut candidates = build_discovery_candidates_from_issuer(issuer, self.discovery_options);
        candidates.dedup();
        candidates
    }

    pub(crate) async fn jwks(&self, client: &dyn HttpClient) -> Option<Arc<JwkSet>> {
        cached_fetch(
            &self.jwks_cache,
            &self.jwks_refresh_lock,
            ChronoDuration::minutes(JWKS_TTL_MINUTES),
            || self.fetch_jwks(client),
        )
        .await
    }

    // Tries each discovery candidate URL in order, returning the first
    // successful response. This handles providers that support different
    // .well-known paths (OAuth AS vs OIDC).
    async fn fetch_discovery(&self, client: &dyn HttpClient) -> Option<Arc<DiscoveryDocument>> {
        let provider_name = self.config.name();
        let candidates = self.discovery_candidates();
        if candidates.is_empty() {
            debug!(
                provider = %provider_name,
                "auth discovery skipped: no discovery candidates available"
            );
            return None;
        }

        for url in &candidates {
            if let Some(doc) = self
                .fetch_discovery_candidate(client, provider_name, url)
                .await
            {
                return Some(doc);
            }
        }
        warn!(
            provider = %provider_name,
            candidates_attempted = candidates.len(),
            "auth discovery failed for all candidates"
        );
        None
    }

    async fn fetch_jwks(&self, client: &dyn HttpClient) -> Option<Arc<JwkSet>> {
        let provider_name = self.config.name();
        let Some(url) = self.jwks_url(client).await else {
            warn!(
                provider = %provider_name,
                "auth jwks fetch failed: no jwks_url resolved from provider or discovery"
            );
            return None;
        };
        let response = match client
            .send(OutboundHttpRequest {
                method: "GET".to_owned(),
                url: url.clone(),
                timeout_ms: self.outbound_timeout_ms,
                max_response_bytes: self.outbound_max_response_bytes,
                ..OutboundHttpRequest::default()
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                self.log_jwks_request_error(provider_name, &url, &error);
                return None;
            }
        };
        if !response.is_success() {
            Self::log_jwks_non_success(provider_name, &url, response.status());
            return None;
        }
        match response.json::<JwkSet>() {
            Ok(jwks) => Some(Arc::new(jwks)),
            Err(error) => {
                self.log_jwks_parse_error(provider_name, &url, &error);
                None
            }
        }
    }
}

// Double-checked locking cache: reads the cache first, then acquires the
// refresh lock and re-checks to prevent thundering herd when multiple
// requests hit a stale cache simultaneously.
async fn cached_fetch<T, F, Fut>(
    cache: &RwLock<Option<CacheEntry<Arc<T>>>>,
    refresh_lock: &Mutex<()>,
    ttl: ChronoDuration,
    fetch: F,
) -> Option<Arc<T>>
where
    T: Send + Sync,
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = Option<Arc<T>>> + Send,
{
    let now = Utc::now();
    {
        let guard = cache.read().await;
        if let Some(entry) = guard.as_ref() {
            if now - entry.fetched_at < ttl {
                return Some(Arc::clone(&entry.value));
            }
        }
    }

    let _guard = refresh_lock.lock().await;
    let now = Utc::now();
    {
        let guard = cache.read().await;
        if let Some(entry) = guard.as_ref() {
            if now - entry.fetched_at < ttl {
                return Some(Arc::clone(&entry.value));
            }
        }
    }

    let fetched = fetch().await?;
    let mut guard = cache.write().await;
    *guard = Some(CacheEntry {
        fetched_at: Utc::now(),
        value: Arc::clone(&fetched),
    });
    Some(fetched)
}

type CacheEntry<T> = TimedCacheEntry<T>;

// Generates .well-known discovery URLs from an issuer URL per RFC 8414
// and OpenID Connect Discovery. For root issuers (no path), tries OAuth AS
// then OIDC. For path-based issuers (e.g. /tenant1), also tries the OIDC
// path-suffix variant.
pub(super) fn build_discovery_candidates_from_issuer(
    issuer: &str,
    options: DiscoveryOptions,
) -> Vec<String> {
    let Ok(mut parsed) = url::Url::parse(issuer) else {
        return Vec::new();
    };
    let path = parsed.path().trim_matches('/').to_owned();
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);
    let origin = parsed.as_str().trim_end_matches('/').to_owned();

    let mut out = Vec::new();
    if path.is_empty() {
        out.push(format!("{origin}/.well-known/oauth-authorization-server"));
        if options.enable_oidc && options.allow_fallback {
            out.push(format!("{origin}/.well-known/openid-configuration"));
        }
    } else {
        out.push(format!(
            "{origin}/.well-known/oauth-authorization-server/{path}"
        ));
        if options.enable_oidc && options.allow_fallback {
            out.push(format!("{origin}/.well-known/openid-configuration/{path}"));
            out.push(format!("{origin}/{path}/.well-known/openid-configuration"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{
        build_discovery_candidates_from_issuer, AuthProvider, DiscoveryCacheEntry,
        DiscoveryDocument, DiscoveryOptions, JwksCacheEntry, DISCOVERY_TTL_MINUTES,
        JWKS_TTL_MINUTES,
    };
    use crate::http::client::ReqwestHttpClient;
    use crate::http::client::{HttpClient, OutboundHttpRequest, OutboundHttpResponse};
    use crate::inline_test_fixtures::{base_provider, capture_logs_async};
    use async_trait::async_trait;
    use chrono::{Duration as ChronoDuration, Utc};
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tracing::Level;

    #[derive(Clone, Debug)]
    struct FailingClient {
        message: String,
    }

    #[async_trait]
    impl HttpClient for FailingClient {
        async fn send(
            &self,
            _request: OutboundHttpRequest,
        ) -> Result<OutboundHttpResponse, rmcp::ErrorData> {
            Err(rmcp::ErrorData::internal_error(self.message.clone(), None))
        }
    }

    #[test]
    fn build_discovery_candidates_from_issuer_cases() {
        let opts_all = DiscoveryOptions {
            enable_oidc: true,
            allow_fallback: true,
        };
        let opts_no_fallback = DiscoveryOptions {
            enable_oidc: true,
            allow_fallback: false,
        };
        for invalid_issuer in ["not-a-url", "http://", "/relative/path"] {
            assert!(
                build_discovery_candidates_from_issuer(invalid_issuer, opts_all).is_empty(),
                "invalid issuer should produce no discovery candidates: {invalid_issuer}"
            );
        }

        let root = build_discovery_candidates_from_issuer("https://auth.example.com", opts_all);
        assert_eq!(
            root,
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server".to_owned(),
                "https://auth.example.com/.well-known/openid-configuration".to_owned(),
            ]
        );

        let path =
            build_discovery_candidates_from_issuer("https://auth.example.com/tenant1", opts_all);
        assert_eq!(
            path,
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1"
                    .to_owned(),
                "https://auth.example.com/.well-known/openid-configuration/tenant1".to_owned(),
                "https://auth.example.com/tenant1/.well-known/openid-configuration".to_owned(),
            ]
        );

        let no_fallback = build_discovery_candidates_from_issuer(
            "https://auth.example.com/tenant1",
            opts_no_fallback,
        );
        assert_eq!(
            no_fallback,
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1"
                    .to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn provider_jwks_uses_cached_entry_when_fresh() {
        let server = MockServer::start_async().await;
        let jwks_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/jwks");
                then.status(200).json_body(json!({
                    "keys": [{ "kty": "oct", "k": "QUJDREVGR0g", "kid": "network-kid" }]
                }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some(server.url("/jwks"));
        let provider = AuthProvider::new(config, None, None);
        let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(json!({
            "keys": [{ "kty": "oct", "k": "QUJDREVGR0g", "kid": "cached-kid" }]
        }))
        .expect("cached jwks");
        let now = Utc::now();
        let cached = Arc::new(jwks.clone());
        *provider.jwks_cache.write().await = Some(JwksCacheEntry {
            fetched_at: now,
            value: Arc::clone(&cached),
        });

        let client = ReqwestHttpClient::default();
        let result = provider.jwks(&client).await.expect("cached jwks");
        assert!(Arc::ptr_eq(&result, &cached));
        assert_eq!(
            serde_json::to_value(result.as_ref()).expect("jwks json"),
            serde_json::to_value(&jwks).expect("expected jwks json")
        );
        jwks_mock.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn provider_jwks_falls_through_when_cache_is_stale() {
        let server = MockServer::start_async().await;
        let jwks_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/jwks");
                then.status(200).json_body(json!({
                    "keys": [{ "kty": "oct", "k": "SElKS0xNTk8", "kid": "fresh-kid" }]
                }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some(server.url("/jwks"));
        let provider = AuthProvider::new(config, None, None);
        let stale = Utc::now() - ChronoDuration::minutes(JWKS_TTL_MINUTES + 1);
        let stale_jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(json!({
            "keys": [{ "kty": "oct", "k": "UVJTVFVWV1g", "kid": "stale-kid" }]
        }))
        .expect("stale jwks");
        let stale_cached = Arc::new(stale_jwks);
        *provider.jwks_cache.write().await = Some(JwksCacheEntry {
            fetched_at: stale,
            value: Arc::clone(&stale_cached),
        });

        let client = ReqwestHttpClient::default();
        let result = provider.jwks(&client).await.expect("refreshed jwks");
        assert!(!Arc::ptr_eq(&result, &stale_cached));
        assert_eq!(
            serde_json::to_value(result.as_ref()).expect("jwks json"),
            json!({ "keys": [{ "kty": "oct", "k": "SElKS0xNTk8", "kid": "fresh-kid" }] })
        );
        jwks_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_discovery_uses_cached_entry_when_fresh() {
        let server = MockServer::start_async().await;
        let discovery_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/discovery");
                then.status(200).json_body(json!({
                    "issuer": "https://network.example.com",
                    "jwks_uri": "https://network.example.com/jwks"
                }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .discovery_url = Some(server.url("/discovery"));
        let provider = AuthProvider::new(config, None, None);
        let cached = DiscoveryDocument {
            issuer: Some("https://issuer.example.com".to_owned()),
            jwks_uri: Some("https://issuer.example.com/jwks".to_owned()),
        };
        let cached = Arc::new(cached);
        *provider.discovery.write().await = Some(DiscoveryCacheEntry {
            fetched_at: Utc::now(),
            value: Arc::clone(&cached),
        });

        let client = ReqwestHttpClient::default();
        let result = provider.discovery(&client).await.expect("cached discovery");
        assert!(Arc::ptr_eq(&result, &cached));
        assert_eq!(result.issuer.as_deref(), cached.issuer.as_deref());
        assert_eq!(result.jwks_uri.as_deref(), cached.jwks_uri.as_deref());
        discovery_mock.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn provider_discovery_refreshes_when_cache_is_stale() {
        let server = MockServer::start_async().await;
        let discovery_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/discovery");
                then.status(200).json_body(json!({
                    "issuer": "https://fresh.example.com",
                    "jwks_uri": "https://fresh.example.com/jwks"
                }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .discovery_url = Some(server.url("/discovery"));
        let provider = AuthProvider::new(config, None, None);
        let stale = Utc::now() - ChronoDuration::minutes(DISCOVERY_TTL_MINUTES + 1);
        let stale_doc = Arc::new(DiscoveryDocument {
            issuer: Some("https://stale.example.com".to_owned()),
            jwks_uri: Some("https://stale.example.com/jwks".to_owned()),
        });
        *provider.discovery.write().await = Some(DiscoveryCacheEntry {
            fetched_at: stale,
            value: Arc::clone(&stale_doc),
        });

        let client = ReqwestHttpClient::default();
        let result = provider
            .discovery(&client)
            .await
            .expect("refreshed discovery");
        assert!(!Arc::ptr_eq(&result, &stale_doc));
        assert_eq!(result.issuer.as_deref(), Some("https://fresh.example.com"));
        assert_eq!(
            result.jwks_uri.as_deref(),
            Some("https://fresh.example.com/jwks")
        );
        discovery_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_jwks_coalesces_concurrent_refresh_fetches() {
        let server = MockServer::start_async().await;
        let jwks_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/jwks");
                then.status(200)
                    .delay(Duration::from_millis(50))
                    .json_body(json!({ "keys": [] }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some(server.url("/jwks"));
        let provider = Arc::new(AuthProvider::new(config, None, None));

        let stale = Utc::now() - ChronoDuration::minutes(JWKS_TTL_MINUTES + 1);
        *provider.jwks_cache.write().await = Some(JwksCacheEntry {
            fetched_at: stale,
            value: Arc::new(serde_json::from_str(r#"{"keys":[]}"#).expect("jwks")),
        });

        let client = ReqwestHttpClient::default();
        let expected = json!({ "keys": [] });
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let provider = Arc::clone(&provider);
            let client = client.clone();
            tasks.push(tokio::spawn(async move { provider.jwks(&client).await }));
        }
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("join").expect("jwks"));
        }

        let first = Arc::clone(results.first().expect("first result"));
        for result in results {
            assert!(Arc::ptr_eq(&first, &result));
            assert_eq!(
                serde_json::to_value(result.as_ref()).expect("jwks json"),
                expected
            );
        }

        jwks_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_discovery_coalesces_concurrent_refresh_fetches() {
        let server = MockServer::start_async().await;
        let discovery_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/discovery");
                then.status(200)
                    .delay(Duration::from_millis(50))
                    .json_body(json!({
                        "issuer": "https://issuer.example.com",
                        "jwks_uri": "https://issuer.example.com/jwks"
                    }));
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .discovery_url = Some(server.url("/discovery"));
        let provider = Arc::new(AuthProvider::new(config, None, None));

        let stale = Utc::now() - ChronoDuration::minutes(DISCOVERY_TTL_MINUTES + 1);
        *provider.discovery.write().await = Some(DiscoveryCacheEntry {
            fetched_at: stale,
            value: Arc::new(DiscoveryDocument {
                issuer: Some("https://stale.example.com".to_owned()),
                jwks_uri: Some("https://stale.example.com/jwks".to_owned()),
            }),
        });

        let client = ReqwestHttpClient::default();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let provider = Arc::clone(&provider);
            let client = client.clone();
            tasks.push(tokio::spawn(
                async move { provider.discovery(&client).await },
            ));
        }
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("join").expect("document"));
        }

        let first = Arc::clone(results.first().expect("first result"));
        for result in results {
            assert!(Arc::ptr_eq(&first, &result));
            assert_eq!(result.issuer.as_deref(), Some("https://issuer.example.com"));
            assert_eq!(
                result.jwks_uri.as_deref(),
                Some("https://issuer.example.com/jwks")
            );
        }

        discovery_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_discovery_logs_debug_candidate_failure_and_terminal_warn() {
        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .discovery_url =
            Some("https://auth.example.com/.well-known/openid-configuration".to_owned());
        let provider = AuthProvider::new(config, None, None).with_server_log_payload_max_bytes(8);
        let client = FailingClient {
            message: "0123456789".to_owned(),
        };

        let logs = capture_logs_async(Level::DEBUG, || async {
            let result = provider.discovery(&client).await;
            assert!(result.is_none());
        })
        .await;

        assert!(
            logs.contains("auth discovery candidate request failed"),
            "missing candidate debug log: {logs}"
        );
        assert!(
            logs.contains("error_message=01234567..."),
            "missing capped error message: {logs}"
        );
        assert!(
            logs.contains("error_message_bytes=10"),
            "missing original message byte count: {logs}"
        );
        assert!(
            logs.contains("error_message_truncated=true"),
            "missing truncation marker: {logs}"
        );
        assert!(
            logs.contains("auth discovery failed for all candidates"),
            "missing terminal discovery warn log: {logs}"
        );
    }

    #[tokio::test]
    async fn provider_jwks_logs_warn_for_non_success_status() {
        let server = MockServer::start_async().await;
        let jwks_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/jwks");
                then.status(503).body("temporary outage");
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some(server.url("/jwks"));
        let provider = AuthProvider::new(config, None, None);
        let client = ReqwestHttpClient::default();

        let logs = capture_logs_async(Level::DEBUG, || async {
            let result = provider.jwks(&client).await;
            assert!(result.is_none());
        })
        .await;

        assert!(
            logs.contains("auth jwks endpoint returned non-success status"),
            "missing jwks warning log: {logs}"
        );
        assert!(
            logs.contains("status=503"),
            "missing status code log field: {logs}"
        );
        jwks_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_discovery_logs_debug_for_invalid_json_response() {
        let server = MockServer::start_async().await;
        let discovery_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/discovery");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{not-json");
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .discovery_url = Some(server.url("/discovery"));
        let provider = AuthProvider::new(config, None, None);
        let client = ReqwestHttpClient::default();

        let logs = capture_logs_async(Level::DEBUG, || async {
            let result = provider.discovery(&client).await;
            assert!(result.is_none());
        })
        .await;

        assert!(
            logs.contains("auth discovery candidate returned invalid JSON"),
            "missing discovery parse debug log: {logs}"
        );
        assert!(
            logs.contains("auth discovery failed for all candidates"),
            "missing terminal discovery warning: {logs}"
        );
        discovery_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_jwks_logs_warn_for_invalid_json_response() {
        let server = MockServer::start_async().await;
        let jwks_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/jwks");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{not-json");
            })
            .await;

        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some(server.url("/jwks"));
        let provider = AuthProvider::new(config, None, None);
        let client = ReqwestHttpClient::default();

        let logs = capture_logs_async(Level::DEBUG, || async {
            let result = provider.jwks(&client).await;
            assert!(result.is_none());
        })
        .await;

        assert!(
            logs.contains("auth jwks endpoint returned invalid JSON"),
            "missing jwks parse warning log: {logs}"
        );
        jwks_mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn provider_jwks_logs_warn_for_request_failure_with_truncation_fields() {
        let mut config = base_provider();
        config
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .jwks_url = Some("https://auth.example.com/jwks".to_owned());
        let provider = AuthProvider::new(config, None, None).with_server_log_payload_max_bytes(8);
        let client = FailingClient {
            message: "0123456789".to_owned(),
        };

        let logs = capture_logs_async(Level::DEBUG, || async {
            let result = provider.jwks(&client).await;
            assert!(result.is_none());
        })
        .await;

        assert!(
            logs.contains("auth jwks request failed"),
            "missing jwks request warning log: {logs}"
        );
        assert!(
            logs.contains("error_message=01234567..."),
            "missing capped error payload: {logs}"
        );
        assert!(
            logs.contains("error_message_bytes=10"),
            "missing original byte count field: {logs}"
        );
        assert!(
            logs.contains("error_message_truncated=true"),
            "missing truncation marker: {logs}"
        );
    }
}
