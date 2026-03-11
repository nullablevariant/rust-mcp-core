//! Outbound HTTP client abstraction for upstream API calls and auth introspection.

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::ErrorData as McpError;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// An outbound HTTP request built from tool config templates or auth introspection.
///
/// Supports JSON body, form body, query parameters, headers, basic auth,
/// and configurable timeout/response size limits.
#[derive(Clone, Debug, Default)]
pub struct OutboundHttpRequest {
    pub method: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub timeout_ms: Option<u64>,
    pub max_response_bytes: Option<u64>,
    pub json_body: Option<Value>,
    pub form_body: Vec<(String, String)>,
    pub basic_auth: Option<(String, Option<String>)>,
}

/// The response from an outbound HTTP request.
///
/// Provides status code access and JSON deserialization of the body.
#[derive(Clone, Debug)]
pub struct OutboundHttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl OutboundHttpResponse {
    /// HTTP status code of the response.
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Returns `true` if the status code is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Deserialize the response body as JSON.
    ///
    /// # Errors
    ///
    /// Returns `McpError` if the body is not valid JSON or cannot be
    /// deserialized into `T`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, McpError> {
        serde_json::from_slice(&self.body)
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    pub(crate) const fn from_parts(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    #[allow(dead_code)] // used by oauth bridge when http_tools is enabled; may be unused in auth-only builds.
    pub(crate) fn into_parts(self) -> (u16, Vec<u8>) {
        (self.status, self.body)
    }
}

/// Trait for sending outbound HTTP requests.
///
/// Implement this trait to provide a custom HTTP client (e.g., for testing or
/// custom TLS configuration). The default implementation uses `reqwest`.
///
/// # Errors
///
/// Returns `McpError` on network failure, timeout, or response size limit exceeded.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Send an HTTP request and return the response.
    async fn send(&self, request: OutboundHttpRequest) -> Result<OutboundHttpResponse, McpError>;
}

/// A thread-safe, shared HTTP client handle.
pub type SharedHttpClient = Arc<dyn HttpClient>;

#[cfg(any(feature = "auth", feature = "http_tools"))]
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_048_576;

#[cfg(any(feature = "auth", feature = "http_tools"))]
fn effective_max_response_bytes(request: &OutboundHttpRequest) -> Result<usize, McpError> {
    match request.max_response_bytes {
        Some(limit) => usize::try_from(limit).map_err(|_| {
            McpError::invalid_request(
                "max_response_bytes is too large for this platform".to_owned(),
                None,
            )
        }),
        None => Ok(DEFAULT_MAX_RESPONSE_BYTES),
    }
}

/// [`HttpClient`] implementation backed by [`reqwest::Client`].
///
/// Available when either the `auth` or `http_tools` feature is enabled.
/// Enforces a configurable max response body size (default 1 MB).
#[cfg(any(feature = "auth", feature = "http_tools"))]
#[derive(Clone, Debug, Default)]
pub struct ReqwestHttpClient {
    inner: reqwest::Client,
}

#[cfg(any(feature = "auth", feature = "http_tools"))]
impl ReqwestHttpClient {
    pub const fn new(inner: reqwest::Client) -> Self {
        Self { inner }
    }
}

#[cfg(any(feature = "auth", feature = "http_tools"))]
#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(&self, request: OutboundHttpRequest) -> Result<OutboundHttpResponse, McpError> {
        let max_response_bytes = effective_max_response_bytes(&request)?;

        if request.json_body.is_some() && !request.form_body.is_empty() {
            return Err(McpError::invalid_request(
                "outbound request cannot use json and form body together".to_owned(),
                None,
            ));
        }

        let method = reqwest::Method::from_bytes(request.method.trim().as_bytes())
            .map_err(|error| McpError::invalid_request(error.to_string(), None))?;
        let mut builder = self.inner.request(method, &request.url);

        if !request.query.is_empty() {
            builder = builder.query(&request.query);
        }
        for (key, value) in request.headers {
            builder = builder.header(key, value);
        }
        if let Some(timeout_ms) = request.timeout_ms {
            builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
        }
        if let Some((username, password)) = request.basic_auth {
            builder = builder.basic_auth(username, password);
        }
        if let Some(body) = request.json_body {
            builder = builder.json(&body);
        } else if !request.form_body.is_empty() {
            builder = builder.form(&request.form_body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let status = response.status().as_u16();
        let mut response = response;
        let mut body = Vec::new();

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?
        {
            let next_len = body.len().checked_add(chunk.len()).ok_or_else(|| {
                McpError::internal_error("response body overflow".to_owned(), None)
            })?;
            if next_len > max_response_bytes {
                return Err(McpError::internal_error(
                    "outbound response body exceeds configured max_response_bytes".to_owned(),
                    None,
                ));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(OutboundHttpResponse::from_parts(status, body))
    }
}

#[cfg(not(any(feature = "auth", feature = "http_tools")))]
#[derive(Clone, Debug, Default)]
pub(crate) struct DisabledHttpClient;

#[cfg(not(any(feature = "auth", feature = "http_tools")))]
#[async_trait]
impl HttpClient for DisabledHttpClient {
    async fn send(&self, _request: OutboundHttpRequest) -> Result<OutboundHttpResponse, McpError> {
        Err(McpError::invalid_request(
            "outbound HTTP client is unavailable (enable auth or http_tools feature)".to_owned(),
            None,
        ))
    }
}

/// Create the default [`SharedHttpClient`].
///
/// Returns a [`ReqwestHttpClient`] when `auth` or `http_tools` is enabled,
/// or a disabled stub that rejects all requests otherwise.
#[cfg(any(feature = "auth", feature = "http_tools"))]
pub fn default_http_client() -> SharedHttpClient {
    Arc::new(ReqwestHttpClient::default())
}

#[cfg(not(any(feature = "auth", feature = "http_tools")))]
pub fn default_http_client() -> SharedHttpClient {
    Arc::new(DisabledHttpClient)
}

#[cfg(test)]
// Inline tests here cover internal request/response helpers and error branches
// on the outbound client implementation that are not part of external API
// behavior contracts.
mod tests {
    use super::OutboundHttpResponse;
    #[cfg(any(feature = "auth", feature = "http_tools"))]
    use super::ReqwestHttpClient;
    #[cfg(any(feature = "auth", feature = "http_tools"))]
    use super::{default_http_client, HttpClient, OutboundHttpRequest};
    use rmcp::model::ErrorCode;
    use serde_json::Value;

    #[test]
    fn outbound_http_response_json_reports_parse_error() {
        let response = OutboundHttpResponse {
            status: 200,
            body: b"not-json".to_vec(),
        };
        let error = response
            .json::<Value>()
            .expect_err("invalid JSON must map to MCP internal error");
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "expected ident at line 1 column 2");
        assert!(error.data.is_none());
    }

    #[test]
    fn outbound_http_response_status_helpers() {
        let ok = OutboundHttpResponse {
            status: 200,
            body: b"{}".to_vec(),
        };
        assert_eq!(ok.status(), 200);
        assert!(ok.is_success());

        let err = OutboundHttpResponse {
            status: 500,
            body: b"{}".to_vec(),
        };
        assert!(!err.is_success());
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    #[tokio::test]
    async fn reqwest_http_client_rejects_json_and_form_body_together() {
        let client = ReqwestHttpClient::default();
        let error = client
            .send(OutboundHttpRequest {
                method: "POST".to_owned(),
                url: "http://example.com".to_owned(),
                json_body: Some(serde_json::json!({"a": 1})),
                form_body: vec![("b".to_owned(), "2".to_owned())],
                ..OutboundHttpRequest::default()
            })
            .await
            .expect_err("json and form body together must be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "outbound request cannot use json and form body together"
        );
        assert!(error.data.is_none());
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    #[tokio::test]
    async fn reqwest_http_client_sends_request_and_parses_json() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/status")
                    .query_param("q", "ok")
                    .header("x-test", "1");
                then.status(200)
                    .json_body(serde_json::json!({"ok": true, "source": "mock"}));
            })
            .await;

        let client = ReqwestHttpClient::new(reqwest::Client::new());
        let response = client
            .send(OutboundHttpRequest {
                method: "GET".to_owned(),
                url: format!("{}/status", server.base_url()),
                query: vec![("q".to_owned(), "ok".to_owned())],
                headers: vec![("x-test".to_owned(), "1".to_owned())],
                timeout_ms: Some(500),
                ..OutboundHttpRequest::default()
            })
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), 200);
        assert!(response.is_success());
        let json: Value = response.json().expect("json parse");
        assert_eq!(json, serde_json::json!({"ok": true, "source": "mock"}));
        mock.assert_calls_async(1).await;
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    #[tokio::test]
    async fn default_http_client_returns_working_client() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/ping");
                then.status(200)
                    .json_body(serde_json::json!({"pong": true, "source": "default"}));
            })
            .await;

        let client = default_http_client();
        let response = client
            .send(OutboundHttpRequest {
                method: "GET".to_owned(),
                url: format!("{}/ping", server.base_url()),
                ..OutboundHttpRequest::default()
            })
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), 200);
        assert!(response.is_success());
        let json: Value = response.json().expect("json parse");
        assert_eq!(json, serde_json::json!({"pong": true, "source": "default"}));
        mock.assert_calls_async(1).await;
    }

    #[cfg(any(feature = "auth", feature = "http_tools"))]
    #[tokio::test]
    async fn reqwest_http_client_rejects_response_over_max_response_bytes() {
        let server = httpmock::MockServer::start_async().await;
        let _mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/too-large");
                then.status(200).body("123456789");
            })
            .await;

        let client = ReqwestHttpClient::new(reqwest::Client::new());
        let result = client
            .send(OutboundHttpRequest {
                method: "GET".to_owned(),
                url: format!("{}/too-large", server.base_url()),
                max_response_bytes: Some(8),
                ..OutboundHttpRequest::default()
            })
            .await;

        let error = result.expect_err("response should exceed limit");
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            error.message,
            "outbound response body exceeds configured max_response_bytes"
        );
        assert!(error.data.is_none());
    }
}
