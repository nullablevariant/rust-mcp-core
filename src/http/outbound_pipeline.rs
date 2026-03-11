//! Shared outbound request policy helpers for HTTP tools and plugin outbound calls.

use std::{future::Future, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::config::{OutboundHttpConfig, OutboundRetryConfig, UpstreamAuth, UpstreamConfig};
use crate::McpError;

// Auth behavior controls for plugin outbound header merging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PluginRequestAuth<'a> {
    pub include_upstream_auth: bool,
    pub oauth2_bearer: Option<&'a str>,
    pub explicit_authorization: Option<&'a str>,
}

impl<'a> PluginRequestAuth<'a> {
    #[must_use]
    pub(crate) const fn inherit(oauth2_bearer: Option<&'a str>) -> Self {
        Self {
            include_upstream_auth: true,
            oauth2_bearer,
            explicit_authorization: None,
        }
    }

    #[must_use]
    pub(crate) const fn none() -> Self {
        Self {
            include_upstream_auth: false,
            oauth2_bearer: None,
            explicit_authorization: None,
        }
    }

    #[must_use]
    pub(crate) const fn explicit(authorization: &'a str) -> Self {
        Self {
            include_upstream_auth: false,
            oauth2_bearer: None,
            explicit_authorization: Some(authorization),
        }
    }
}

// Resolves a plugin outbound URL against a configured upstream base URL.
pub(crate) fn resolve_upstream_url(base_url: &str, url: &str) -> Result<String, McpError> {
    if url.starts_with('/') {
        let base = base_url.trim_end_matches('/');
        Ok(format!("{base}{url}"))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_owned())
    } else {
        Err(McpError::invalid_request(
            "plugin upstream request url must be absolute or begin with '/'".to_owned(),
            None,
        ))
    }
}

// Resolves timeout fallback order: request -> upstream -> outbound defaults.
pub(crate) fn resolve_timeout_ms(
    request_timeout_ms: Option<u64>,
    upstream: &UpstreamConfig,
    outbound_http: Option<&OutboundHttpConfig>,
) -> Option<u64> {
    request_timeout_ms
        .or(upstream.timeout_ms)
        .or_else(|| outbound_http.and_then(|outbound| outbound.timeout_ms))
}

// Resolves max response bytes fallback order: request -> upstream -> outbound defaults.
pub(crate) fn resolve_max_response_bytes(
    request_max_response_bytes: Option<u64>,
    upstream: &UpstreamConfig,
    outbound_http: Option<&OutboundHttpConfig>,
) -> Option<u64> {
    request_max_response_bytes
        .or(upstream.max_response_bytes)
        .or_else(|| outbound_http.and_then(|outbound| outbound.max_response_bytes))
}

// Resolves retry fallback order: tool/request -> upstream -> outbound global.
pub(crate) fn resolve_retry_config<'a>(
    request_retry: Option<&'a OutboundRetryConfig>,
    upstream: &'a UpstreamConfig,
    outbound_http: Option<&'a OutboundHttpConfig>,
) -> Option<&'a OutboundRetryConfig> {
    request_retry
        .or(upstream.retry.as_ref())
        .or_else(|| outbound_http.and_then(|outbound| outbound.retry.as_ref()))
}

// Returns whether retries are allowed for this HTTP method.
//
// Retry v1 is intentionally scoped to idempotent methods only.
pub(crate) fn method_is_retryable(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS" | "DELETE"
    )
}

// Shared retry execution inputs for outbound HTTP operations.
#[derive(Clone, Copy)]
pub(crate) struct RetryExecutionParams<'a, E> {
    pub method: &'a str,
    pub retry: Option<&'a OutboundRetryConfig>,
    pub cancellation: Option<&'a CancellationToken>,
    pub cancelled_error: fn() -> E,
}

// Executes an outbound operation with optional fixed-delay retries.
//
// Retry is applied only when:
// - a retry config exists,
// - `max_attempts > 1`,
// - the HTTP method is retryable.
//
// Cancellation (if provided) is checked before each attempt and during retry
// delay sleeps.
pub(crate) async fn execute_with_retry<T, E, F, Fut, R>(
    params: RetryExecutionParams<'_, E>,
    mut operation: F,
    mut should_retry: R,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    R: FnMut(&Result<T, E>, &OutboundRetryConfig) -> bool,
{
    let Some(retry) = params.retry else {
        return operation().await;
    };
    if retry.max_attempts <= 1 || !method_is_retryable(params.method) {
        return operation().await;
    }

    let delay = Duration::from_millis(retry.delay_ms);
    let mut attempt: u32 = 1;

    loop {
        if params
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err((params.cancelled_error)());
        }

        let result = operation().await;
        if attempt >= retry.max_attempts || !should_retry(&result, retry) {
            return result;
        }

        attempt += 1;
        if let Some(token) = params.cancellation {
            tokio::select! {
                () = token.cancelled() => return Err((params.cancelled_error)()),
                () = tokio::time::sleep(delay) => {}
            }
        } else {
            tokio::time::sleep(delay).await;
        }
    }
}

// Builds merged HTTP-tool header template map with last-wins precedence:
// defaults -> upstream -> static auth -> oauth2 bearer -> tool headers.
#[cfg(feature = "http_tools")]
pub(crate) fn build_templated_headers(
    outbound_http: Option<&OutboundHttpConfig>,
    upstream: &UpstreamConfig,
    oauth2_bearer: Option<&str>,
    tool_headers: Option<&Value>,
) -> Option<Value> {
    let mut headers = build_base_header_map(
        outbound_http,
        upstream,
        PluginRequestAuth::inherit(oauth2_bearer),
    );

    if let Some(Value::Object(map)) = tool_headers {
        for (key, value) in map {
            headers.insert(key.clone(), value.clone());
        }
    }

    if headers.is_empty() {
        None
    } else {
        Some(Value::Object(headers))
    }
}

// Builds merged concrete headers for plugin outbound calls with last-wins precedence:
// defaults -> upstream -> static auth -> oauth2 bearer -> request headers.
pub(crate) fn build_request_headers(
    outbound_http: Option<&OutboundHttpConfig>,
    upstream: &UpstreamConfig,
    auth: PluginRequestAuth<'_>,
    request_headers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut headers = build_base_header_map(outbound_http, upstream, auth);

    for (key, value) in request_headers {
        headers.insert(key.clone(), Value::String(value.clone()));
    }

    headers
        .into_iter()
        .map(|(key, value)| {
            // All entries in this map are inserted as Value::String.
            let value = value
                .as_str()
                .expect("header map should contain only string values")
                .to_owned();
            (key, value)
        })
        .collect()
}

fn build_base_header_map(
    outbound_http: Option<&OutboundHttpConfig>,
    upstream: &UpstreamConfig,
    auth: PluginRequestAuth<'_>,
) -> Map<String, Value> {
    let mut headers = Map::new();

    if let Some(outbound_http) = outbound_http {
        if let Some(user_agent) = outbound_http.user_agent.as_ref() {
            headers.insert("User-Agent".to_owned(), Value::String(user_agent.clone()));
        }
        for (key, value) in &outbound_http.headers {
            headers.insert(key.clone(), Value::String(value.clone()));
        }
    }
    if let Some(user_agent) = upstream.user_agent.as_ref() {
        headers.insert("User-Agent".to_owned(), Value::String(user_agent.clone()));
    }

    for (key, value) in &upstream.headers {
        headers.insert(key.clone(), Value::String(value.clone()));
    }

    if auth.include_upstream_auth {
        if let Some(upstream_auth) = upstream.auth.as_ref() {
            match upstream_auth {
                UpstreamAuth::Bearer { token } => {
                    headers.insert(
                        "Authorization".to_owned(),
                        Value::String(format!("Bearer {token}")),
                    );
                }
                UpstreamAuth::Basic { username, password } => {
                    let encoded = STANDARD.encode(format!("{username}:{password}"));
                    headers.insert(
                        "Authorization".to_owned(),
                        Value::String(format!("Basic {encoded}")),
                    );
                }
                UpstreamAuth::None | UpstreamAuth::Oauth2(_) => {}
            }
        }
    }

    if let Some(token) = auth.oauth2_bearer {
        headers.insert(
            "Authorization".to_owned(),
            Value::String(format!("Bearer {token}")),
        );
    }

    if let Some(authorization) = auth.explicit_authorization {
        headers.insert(
            "Authorization".to_owned(),
            Value::String(authorization.to_owned()),
        );
    }

    headers
}

#[cfg(test)]
// These tests stay inline because outbound_pipeline is crate-private and not
// reachable from tests/ integration modules without widening visibility.
mod tests {
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicU32, Ordering},
        time::Duration,
    };

    use base64::Engine as _;
    #[cfg(feature = "http_tools")]
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::{
        build_request_headers, execute_with_retry, method_is_retryable, resolve_max_response_bytes,
        resolve_timeout_ms, OutboundHttpConfig, OutboundRetryConfig, PluginRequestAuth,
        RetryExecutionParams, UpstreamAuth, UpstreamConfig,
    };

    fn upstream(timeout_ms: Option<u64>, max_response_bytes: Option<u64>) -> UpstreamConfig {
        UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms,
            max_response_bytes,
            retry: None,
            auth: None,
        }
    }

    fn outbound(
        default_timeout_ms: Option<u64>,
        default_max_response_bytes: Option<u64>,
    ) -> OutboundHttpConfig {
        OutboundHttpConfig {
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: default_timeout_ms,
            max_response_bytes: default_max_response_bytes,
            retry: None,
        }
    }

    #[test]
    fn resolve_timeout_prefers_request_then_upstream_then_defaults() {
        let upstream_cfg = upstream(Some(200), None);
        let outbound = outbound(Some(300), None);
        assert_eq!(
            resolve_timeout_ms(Some(100), &upstream_cfg, Some(&outbound)),
            Some(100)
        );
        assert_eq!(
            resolve_timeout_ms(None, &upstream_cfg, Some(&outbound)),
            Some(200)
        );

        let upstream_without_timeout = upstream(None, None);
        assert_eq!(
            resolve_timeout_ms(None, &upstream_without_timeout, Some(&outbound)),
            Some(300)
        );
    }

    #[test]
    fn resolve_timeout_returns_none_when_no_value_resolves() {
        let upstream = upstream(None, None);
        let outbound = outbound(None, None);
        assert_eq!(resolve_timeout_ms(None, &upstream, Some(&outbound)), None);
        assert_eq!(resolve_timeout_ms(None, &upstream, None), None);
    }

    #[test]
    fn resolve_max_response_prefers_request_then_upstream_then_defaults() {
        let upstream_cfg = upstream(None, Some(200));
        let outbound = outbound(None, Some(300));
        assert_eq!(
            resolve_max_response_bytes(Some(100), &upstream_cfg, Some(&outbound)),
            Some(100)
        );
        assert_eq!(
            resolve_max_response_bytes(None, &upstream_cfg, Some(&outbound)),
            Some(200)
        );

        let upstream_without_limit = upstream(None, None);
        assert_eq!(
            resolve_max_response_bytes(None, &upstream_without_limit, Some(&outbound)),
            Some(300)
        );
    }

    #[test]
    fn resolve_max_response_returns_none_when_no_value_resolves() {
        let upstream = upstream(None, None);
        let outbound = outbound(None, None);
        assert_eq!(
            resolve_max_response_bytes(None, &upstream, Some(&outbound)),
            None
        );
        assert_eq!(resolve_max_response_bytes(None, &upstream, None), None);
    }

    #[test]
    fn build_request_headers_applies_precedence() {
        let mut upstream_headers = HashMap::new();
        upstream_headers.insert("X-Upstream".to_owned(), "ok".to_owned());
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: upstream_headers,
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Bearer {
                token: "upstream-token".to_owned(),
            }),
        };
        let outbound_http = OutboundHttpConfig {
            headers: HashMap::from([("X-Default".to_owned(), "yes".to_owned())]),
            user_agent: Some("default-agent".to_owned()),
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
        };

        let merged = build_request_headers(
            Some(&outbound_http),
            &upstream,
            PluginRequestAuth::inherit(Some("oauth-token")),
            &[
                ("Authorization".to_owned(), "Bearer explicit".to_owned()),
                ("X-Request".to_owned(), "request".to_owned()),
            ],
        );
        let headers = merged.into_iter().collect::<HashMap<_, _>>();

        assert_eq!(
            headers.get("User-Agent").map(String::as_str),
            Some("default-agent")
        );
        assert_eq!(headers.get("X-Default").map(String::as_str), Some("yes"));
        assert_eq!(headers.get("X-Upstream").map(String::as_str), Some("ok"));
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer explicit")
        );
        assert_eq!(
            headers.get("X-Request").map(String::as_str),
            Some("request")
        );
    }

    #[test]
    fn build_request_headers_upstream_user_agent_overrides_outbound_default() {
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: Some("upstream-agent".to_owned()),
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: None,
        };
        let outbound_http = OutboundHttpConfig {
            headers: HashMap::new(),
            user_agent: Some("default-agent".to_owned()),
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
        };

        let merged = build_request_headers(
            Some(&outbound_http),
            &upstream,
            PluginRequestAuth::none(),
            &[],
        );
        let headers = merged.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            headers.get("User-Agent").map(String::as_str),
            Some("upstream-agent")
        );
    }

    #[test]
    fn build_request_headers_inherit_applies_basic_auth_without_oauth_override() {
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Basic {
                username: "user".to_owned(),
                password: "pass".to_owned(),
            }),
        };

        let merged = build_request_headers(
            None,
            &upstream,
            PluginRequestAuth::inherit(None),
            &[("X-Request".to_owned(), "request".to_owned())],
        );
        let headers = merged.into_iter().collect::<HashMap<_, _>>();

        let expected_basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("user:pass")
        );
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some(expected_basic.as_str())
        );
        assert_eq!(
            headers.get("X-Request").map(String::as_str),
            Some("request")
        );
    }

    #[test]
    fn build_request_headers_none_excludes_upstream_auth_but_keeps_request_header() {
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Bearer {
                token: "upstream-token".to_owned(),
            }),
        };

        let merged = build_request_headers(
            None,
            &upstream,
            PluginRequestAuth::none(),
            &[("Authorization".to_owned(), "Bearer request".to_owned())],
        );
        let headers = merged.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer request")
        );
    }

    #[test]
    fn build_request_headers_explicit_auth_overrides_upstream_and_oauth() {
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Bearer {
                token: "upstream-token".to_owned(),
            }),
        };

        let merged = build_request_headers(
            None,
            &upstream,
            PluginRequestAuth::explicit("Bearer explicit"),
            &[],
        );
        let headers = merged.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer explicit")
        );
    }

    #[cfg(feature = "http_tools")]
    #[test]
    fn build_templated_headers_applies_tool_override_last() {
        let upstream = UpstreamConfig {
            base_url: "https://example.com".to_owned(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Basic {
                username: "user".to_owned(),
                password: "pass".to_owned(),
            }),
        };
        let outbound_http = OutboundHttpConfig {
            headers: HashMap::new(),
            user_agent: Some("default-agent".to_owned()),
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
        };

        let merged = super::build_templated_headers(
            Some(&outbound_http),
            &upstream,
            Some("oauth-token"),
            Some(&json!({
                "Authorization": "Bearer tool",
                "X-Tool": "yes"
            })),
        )
        .expect("headers");
        let object = merged.as_object().expect("object");
        assert_eq!(object.get("Authorization"), Some(&json!("Bearer tool")));
        assert_eq!(object.get("X-Tool"), Some(&json!("yes")));
        assert_eq!(object.get("User-Agent"), Some(&json!("default-agent")));
    }

    fn retry_config() -> OutboundRetryConfig {
        OutboundRetryConfig {
            max_attempts: 3,
            delay_ms: 1,
            on_network_errors: true,
            on_statuses: vec![503],
        }
    }

    #[test]
    fn method_is_retryable_only_for_allowed_methods() {
        assert!(method_is_retryable("GET"));
        assert!(method_is_retryable("head"));
        assert!(method_is_retryable(" OPTIONS "));
        assert!(method_is_retryable("delete"));
        assert!(!method_is_retryable("POST"));
        assert!(!method_is_retryable("PATCH"));
    }

    #[tokio::test]
    async fn execute_with_retry_retries_until_success() {
        let attempts = AtomicU32::new(0);
        let result = execute_with_retry(
            RetryExecutionParams {
                method: "GET",
                retry: Some(&retry_config()),
                cancellation: None,
                cancelled_error: || "cancelled",
            },
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async {
                    if attempts.load(Ordering::SeqCst) < 2 {
                        Err("retry")
                    } else {
                        Ok("ok")
                    }
                }
            },
            |result: &Result<&str, &str>, _retry| result.is_err(),
        )
        .await;

        assert_eq!(result, Ok("ok"));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn execute_with_retry_skips_non_retryable_method() {
        let attempts = AtomicU32::new(0);
        let result = execute_with_retry(
            RetryExecutionParams {
                method: "POST",
                retry: Some(&retry_config()),
                cancellation: None,
                cancelled_error: || "cancelled",
            },
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("fail") }
            },
            |result: &Result<&str, &str>, _retry| result.is_err(),
        )
        .await;

        assert_eq!(result, Err("fail"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn execute_with_retry_respects_cancellation_during_delay() {
        let token = CancellationToken::new();
        let attempts = AtomicU32::new(0);
        let token_for_task = token.clone();
        let task = tokio::spawn(async move {
            execute_with_retry(
                RetryExecutionParams {
                    method: "GET",
                    retry: Some(&retry_config()),
                    cancellation: Some(&token_for_task),
                    cancelled_error: || "cancelled",
                },
                || {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    async { Err("fail") }
                },
                |result: &Result<&str, &str>, _retry| result.is_err(),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(1)).await;
        token.cancel();
        let result = task.await.expect("task should join");

        assert_eq!(result, Err("cancelled"));
    }

    #[tokio::test]
    async fn execute_with_retry_max_attempts_one_runs_exactly_once() {
        let attempts = AtomicU32::new(0);
        let retry = OutboundRetryConfig {
            max_attempts: 1,
            delay_ms: 1,
            on_network_errors: true,
            on_statuses: vec![503],
        };
        let result = execute_with_retry(
            RetryExecutionParams {
                method: "GET",
                retry: Some(&retry),
                cancellation: None,
                cancelled_error: || "cancelled",
            },
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("fail") }
            },
            |result: &Result<&str, &str>, _retry| result.is_err(),
        )
        .await;

        assert_eq!(result, Err("fail"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn execute_with_retry_returns_terminal_error_after_max_attempts_exhausted() {
        let attempts = AtomicU32::new(0);
        let retry = OutboundRetryConfig {
            max_attempts: 3,
            delay_ms: 1,
            on_network_errors: true,
            on_statuses: vec![503],
        };
        let result = execute_with_retry(
            RetryExecutionParams {
                method: "GET",
                retry: Some(&retry),
                cancellation: None,
                cancelled_error: || "cancelled",
            },
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("still-failing") }
            },
            |result: &Result<&str, &str>, _retry| result.is_err(),
        )
        .await;

        assert_eq!(result, Err("still-failing"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
