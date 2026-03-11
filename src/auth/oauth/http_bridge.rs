//! Async bridge from oauth2 HTTP requests to the crate's `HttpClient` abstraction.
//!
//! TODO: remove this bridge once oauth2 provides an adapter for reqwest 0.13.

use std::{error::Error, fmt, future::Future, pin::Pin};

use crate::http::client::{HttpClient, OutboundHttpRequest, SharedHttpClient};

#[derive(Clone, Debug)]
pub(crate) struct OauthHttpBridgeError {
    message: String,
}

impl OauthHttpBridgeError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OauthHttpBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for OauthHttpBridgeError {}

// Per-request options applied by oauth HTTP bridge adapters.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OauthRequestOptions {
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_response_bytes: Option<u64>,
}

#[cfg(test)]
pub(crate) struct OauthHttpClientAdapter<'a> {
    client: &'a dyn HttpClient,
    options: OauthRequestOptions,
}

#[cfg(test)]
impl<'a> OauthHttpClientAdapter<'a> {
    pub(crate) fn new(client: &'a dyn HttpClient) -> Self {
        Self::with_options(client, OauthRequestOptions::default())
    }

    pub(crate) const fn with_options(
        client: &'a dyn HttpClient,
        options: OauthRequestOptions,
    ) -> Self {
        Self { client, options }
    }
}

#[cfg(test)]
impl<'a> oauth2::AsyncHttpClient<'a> for OauthHttpClientAdapter<'a> {
    type Error = OauthHttpBridgeError;
    type Future =
        Pin<Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + Send + 'a>>;

    fn call(&'a self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(async move {
            send_oauth_request_with_options(self.client, request, self.options).await
        })
    }
}

#[derive(Clone)]
pub(crate) struct SharedOauthHttpClient {
    client: SharedHttpClient,
    options: OauthRequestOptions,
}

impl SharedOauthHttpClient {
    #[cfg(feature = "http_tools")]
    pub(crate) fn new(client: SharedHttpClient) -> Self {
        Self::with_options(client, OauthRequestOptions::default())
    }

    pub(crate) const fn with_options(
        client: SharedHttpClient,
        options: OauthRequestOptions,
    ) -> Self {
        Self { client, options }
    }
}

impl fmt::Debug for SharedOauthHttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedOauthHttpClient")
            .field("client", &"<SharedHttpClient>")
            .finish()
    }
}

impl<'a> oauth2::AsyncHttpClient<'a> for SharedOauthHttpClient {
    type Error = OauthHttpBridgeError;
    type Future =
        Pin<Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + Send + 'a>>;

    fn call(&'a self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(async move {
            send_oauth_request_with_options(self.client.as_ref(), request, self.options).await
        })
    }
}

#[cfg(test)]
pub(crate) async fn send_oauth_request(
    client: &dyn HttpClient,
    request: oauth2::HttpRequest,
) -> Result<oauth2::HttpResponse, OauthHttpBridgeError> {
    send_oauth_request_with_options(client, request, OauthRequestOptions::default()).await
}

pub(crate) async fn send_oauth_request_with_options(
    client: &dyn HttpClient,
    request: oauth2::HttpRequest,
    options: OauthRequestOptions,
) -> Result<oauth2::HttpResponse, OauthHttpBridgeError> {
    let (parts, body) = request.into_parts();

    let headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            value
                .to_str()
                .map(|value| (name.to_string(), value.to_owned()))
                .map_err(|error| {
                    OauthHttpBridgeError::new(format!(
                        "oauth2 bridge does not support non-utf8 header value: {error}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let form_body = url::form_urlencoded::parse(&body)
        .filter_map(|(key, value)| {
            let key = key.into_owned();
            let value = value.into_owned();
            if key == "client_id" && value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect::<Vec<_>>();

    let outbound = OutboundHttpRequest {
        method: parts.method.as_str().to_owned(),
        url: parts.uri.to_string(),
        headers,
        timeout_ms: options.timeout_ms,
        max_response_bytes: options.max_response_bytes,
        form_body,
        ..OutboundHttpRequest::default()
    };

    let response = client
        .send(outbound)
        .await
        .map_err(|error| OauthHttpBridgeError::new(error.message))?;
    let (status, response_body) = response.into_parts();

    let mut oauth_response = oauth2::HttpResponse::new(response_body);
    *oauth_response.status_mut() = http::StatusCode::from_u16(status).map_err(|error| {
        OauthHttpBridgeError::new(format!(
            "failed to map outbound status code to oauth2 response: {error}"
        ))
    })?;
    Ok(oauth_response)
}

#[cfg(test)]
// Inline tests verify private request/response translation details.
mod tests {
    use super::{
        send_oauth_request, OauthHttpClientAdapter, OauthRequestOptions, SharedOauthHttpClient,
    };
    use crate::http::client::{HttpClient, OutboundHttpRequest, OutboundHttpResponse};
    use async_trait::async_trait;
    use oauth2::AsyncHttpClient;
    use rmcp::ErrorData as McpError;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Clone, Debug)]
    struct RecordingClient {
        last_request: Arc<Mutex<Option<OutboundHttpRequest>>>,
        status: u16,
    }

    #[derive(Clone, Debug)]
    struct FailingClient;

    #[async_trait]
    impl HttpClient for FailingClient {
        async fn send(
            &self,
            _request: OutboundHttpRequest,
        ) -> Result<OutboundHttpResponse, McpError> {
            Err(McpError::internal_error(
                "simulated outbound error".to_owned(),
                None,
            ))
        }
    }

    #[async_trait]
    impl HttpClient for RecordingClient {
        async fn send(
            &self,
            request: OutboundHttpRequest,
        ) -> Result<OutboundHttpResponse, McpError> {
            *self.last_request.lock().await = Some(request);
            Ok(OutboundHttpResponse::from_parts(
                self.status,
                br#"{"access_token":"abc","token_type":"Bearer","expires_in":3600}"#.to_vec(),
            ))
        }
    }

    #[tokio::test]
    async fn bridge_translates_oauth_request_into_form_outbound_request() {
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(b"grant_type=client_credentials&scope=read".to_vec())
            .expect("request should build");

        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 200,
        };
        let _ = send_oauth_request(&client, request)
            .await
            .expect("bridge should succeed");

        let recorded = client
            .last_request
            .lock()
            .await
            .clone()
            .expect("request should be recorded");
        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://auth.example.com/token");
        assert_eq!(
            recorded.form_body,
            vec![
                ("grant_type".to_owned(), "client_credentials".to_owned()),
                ("scope".to_owned(), "read".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn bridge_adapter_calls_underlying_client() {
        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 200,
        };
        let adapter = OauthHttpClientAdapter::new(&client);
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(b"grant_type=client_credentials&scope=read".to_vec())
            .expect("request should build");
        let response = adapter
            .call(request)
            .await
            .expect("adapter should return response");
        assert_eq!(response.status(), http::StatusCode::OK);

        let recorded = client
            .last_request
            .lock()
            .await
            .clone()
            .expect("request should be recorded");
        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://auth.example.com/token");
        assert_eq!(
            recorded.form_body,
            vec![
                ("grant_type".to_owned(), "client_credentials".to_owned()),
                ("scope".to_owned(), "read".to_owned()),
            ]
        );
        assert_eq!(recorded.timeout_ms, None);
        assert_eq!(recorded.max_response_bytes, None);
        assert!(recorded
            .headers
            .iter()
            .any(|(name, value)| name == "content-type"
                && value == "application/x-www-form-urlencoded"));
    }

    #[tokio::test]
    async fn bridge_adapter_with_options_propagates_request_limits() {
        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 200,
        };
        let adapter = OauthHttpClientAdapter::with_options(
            &client,
            OauthRequestOptions {
                timeout_ms: Some(321),
                max_response_bytes: Some(654),
            },
        );
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(b"grant_type=client_credentials".to_vec())
            .expect("request should build");
        let response = adapter
            .call(request)
            .await
            .expect("adapter should return response");
        assert_eq!(response.status(), http::StatusCode::OK);

        let recorded = client
            .last_request
            .lock()
            .await
            .clone()
            .expect("request should be recorded");
        assert_eq!(recorded.timeout_ms, Some(321));
        assert_eq!(recorded.max_response_bytes, Some(654));
    }

    #[tokio::test]
    async fn shared_bridge_adapter_calls_underlying_client() {
        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 200,
        };
        let shared = SharedOauthHttpClient::with_options(
            Arc::new(client.clone()),
            OauthRequestOptions {
                timeout_ms: Some(777),
                max_response_bytes: Some(888),
            },
        );
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(b"grant_type=client_credentials".to_vec())
            .expect("request should build");
        let response = shared
            .call(request)
            .await
            .expect("shared adapter should return response");
        assert_eq!(response.status(), http::StatusCode::OK);

        let recorded = client
            .last_request
            .lock()
            .await
            .clone()
            .expect("request should be recorded");
        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.url, "https://auth.example.com/token");
        assert_eq!(
            recorded.form_body,
            vec![("grant_type".to_owned(), "client_credentials".to_owned())]
        );
        assert_eq!(recorded.timeout_ms, Some(777));
        assert_eq!(recorded.max_response_bytes, Some(888));
        assert!(recorded
            .headers
            .iter()
            .any(|(name, value)| name == "content-type"
                && value == "application/x-www-form-urlencoded"));
    }

    #[tokio::test]
    async fn bridge_maps_outbound_send_error() {
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .body(b"grant_type=client_credentials".to_vec())
            .expect("request should build");

        let error = send_oauth_request(&FailingClient, request)
            .await
            .expect_err("send error should map");
        assert_eq!(error.to_string(), "simulated outbound error");
    }

    #[tokio::test]
    async fn bridge_rejects_non_utf8_header_values() {
        let mut request = http::Request::new(b"grant_type=client_credentials".to_vec());
        *request.method_mut() = http::Method::POST;
        *request.uri_mut() = "https://auth.example.com/token".parse().expect("uri");
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_bytes(&[0xff]).expect("header value"),
        );

        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 200,
        };
        let error = send_oauth_request(&client, request)
            .await
            .expect_err("non-utf8 header should be rejected");
        assert!(error
            .to_string()
            .starts_with("oauth2 bridge does not support non-utf8 header value:"));
    }

    #[tokio::test]
    async fn bridge_preserves_http_status_code_in_oauth_response() {
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .body(b"grant_type=client_credentials".to_vec())
            .expect("request should build");

        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 401,
        };
        let response = send_oauth_request(&client, request)
            .await
            .expect("bridge should still return response");
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bridge_maps_invalid_outbound_status_code_error() {
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("https://auth.example.com/token")
            .body(b"grant_type=client_credentials".to_vec())
            .expect("request should build");

        let client = RecordingClient {
            last_request: Arc::new(Mutex::new(None)),
            status: 1000,
        };
        let error = send_oauth_request(&client, request)
            .await
            .expect_err("invalid status code should map");
        let message = error.to_string();
        assert!(message.starts_with("failed to map outbound status code to oauth2 response:"));
        assert!(message.contains("invalid status code"));
    }
}
