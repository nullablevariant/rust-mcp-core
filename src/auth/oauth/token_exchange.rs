//! Shared OAuth token and introspection exchange abstractions.

use async_trait::async_trait;
use oauth2::{
    basic::{
        BasicErrorResponse, BasicErrorResponseType, BasicRevocationErrorResponse,
        BasicTokenResponse, BasicTokenType,
    },
    AccessToken, AuthType, ClientId, ClientSecret, IntrospectionUrl, RequestTokenError,
    StandardErrorResponse, StandardRevocableToken, StandardTokenIntrospectionResponse,
};
use rmcp::ErrorData as McpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::oauth::http_bridge::{OauthHttpBridgeError, OauthRequestOptions, SharedOauthHttpClient},
    config::IntrospectionClientAuthMethod,
    http::client::SharedHttpClient,
};

// Parameters for an RFC 7662 token introspection exchange.
pub(crate) struct IntrospectionExchangeParams<'a> {
    pub(crate) token: &'a str,
    pub(crate) url: &'a str,
    pub(crate) client_auth_method: IntrospectionClientAuthMethod,
    pub(crate) client_id: Option<&'a str>,
    pub(crate) client_secret: Option<&'a str>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_response_bytes: Option<u64>,
}

// Shared oauth abstraction for token/introspection HTTP exchanges.
#[async_trait]
pub(crate) trait TokenExchanger: Send + Sync {
    async fn introspect(
        &self,
        client: SharedHttpClient,
        params: IntrospectionExchangeParams<'_>,
    ) -> Result<Value, McpError>;
}

// Default `TokenExchanger` implementation backed by `HttpClient`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HttpTokenExchanger;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct JsonIntrospectionExtraFields {
    #[serde(flatten)]
    values: serde_json::Map<String, Value>,
}

impl oauth2::ExtraTokenFields for JsonIntrospectionExtraFields {}

type JsonIntrospectionResponse =
    StandardTokenIntrospectionResponse<JsonIntrospectionExtraFields, BasicTokenType>;
type JsonIntrospectionClient = oauth2::Client<
    BasicErrorResponse,
    BasicTokenResponse,
    JsonIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
>;
type IntrospectionError =
    RequestTokenError<OauthHttpBridgeError, StandardErrorResponse<BasicErrorResponseType>>;

const fn introspection_auth_settings<'a>(
    client_auth_method: IntrospectionClientAuthMethod,
    client_id: Option<&'a str>,
    client_secret: Option<&'a str>,
) -> (AuthType, Option<&'a str>, Option<&'a str>) {
    match client_auth_method {
        IntrospectionClientAuthMethod::Basic => {
            if let (Some(client_id), Some(client_secret)) = (client_id, client_secret) {
                (AuthType::BasicAuth, Some(client_id), Some(client_secret))
            } else {
                // Preserve prior behavior: incomplete basic credentials fall back to token-only.
                (AuthType::RequestBody, None, None)
            }
        }
        IntrospectionClientAuthMethod::Post => (AuthType::RequestBody, client_id, client_secret),
        IntrospectionClientAuthMethod::None => {
            // Preserve prior behavior: no client auth parameters for method=none.
            (AuthType::RequestBody, None, None)
        }
    }
}

fn map_introspection_error(error: IntrospectionError) -> McpError {
    let message = match error {
        RequestTokenError::ServerResponse(error_response) => {
            let mut details = error_response.error().to_string();
            if let Some(description) = error_response.error_description() {
                details.push_str(": ");
                details.push_str(description);
            }
            if let Some(uri) = error_response.error_uri() {
                details.push_str(" (");
                details.push_str(uri);
                details.push(')');
            }
            format!("token introspection endpoint returned an error response: {details}")
        }
        RequestTokenError::Request(error) => {
            format!("token introspection request failed: {error}")
        }
        RequestTokenError::Parse(parse_error, _) => {
            format!("token introspection endpoint returned invalid JSON: {parse_error}")
        }
        RequestTokenError::Other(message) => {
            format!("token introspection exchange failed: {message}")
        }
    };
    McpError::invalid_request(message, None)
}

#[async_trait]
impl TokenExchanger for HttpTokenExchanger {
    async fn introspect(
        &self,
        client: SharedHttpClient,
        params: IntrospectionExchangeParams<'_>,
    ) -> Result<Value, McpError> {
        let introspection_url = IntrospectionUrl::new(params.url.to_owned())
            .map_err(|error| McpError::invalid_request(error.to_string(), None))?;

        let (auth_type, client_id, client_secret) = introspection_auth_settings(
            params.client_auth_method,
            params.client_id,
            params.client_secret,
        );

        let mut oauth_client =
            JsonIntrospectionClient::new(ClientId::new(client_id.unwrap_or_default().to_owned()))
                .set_introspection_url(introspection_url)
                .set_auth_type(auth_type);
        if let Some(client_secret) = client_secret {
            oauth_client =
                oauth_client.set_client_secret(ClientSecret::new(client_secret.to_owned()));
        }

        let oauth_http = SharedOauthHttpClient::with_options(
            client,
            OauthRequestOptions {
                timeout_ms: params.timeout_ms,
                max_response_bytes: params.max_response_bytes,
            },
        );

        let payload: JsonIntrospectionResponse = oauth_client
            .introspect(&AccessToken::new(params.token.to_owned()))
            .request_async(&oauth_http)
            .await
            .map_err(map_introspection_error)?;

        serde_json::to_value(payload)
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }
}

#[cfg(test)]
// Inline tests here validate private auth-setting behavior and crate-private
// exchanger logic that is unreachable from external integration tests.
mod tests {
    use super::{
        introspection_auth_settings, map_introspection_error, HttpTokenExchanger,
        IntrospectionExchangeParams, TokenExchanger,
    };
    use crate::config::IntrospectionClientAuthMethod;
    use crate::http::client::{HttpClient, OutboundHttpRequest, OutboundHttpResponse};
    use crate::mcp::ErrorCode;
    use async_trait::async_trait;
    use oauth2::RequestTokenError;
    use rmcp::ErrorData as McpError;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Clone, Debug)]
    struct RecordingClient {
        last_request: Arc<Mutex<Option<OutboundHttpRequest>>>,
    }

    #[async_trait]
    impl HttpClient for RecordingClient {
        async fn send(
            &self,
            request: OutboundHttpRequest,
        ) -> Result<OutboundHttpResponse, McpError> {
            let mut guard = self.last_request.lock().await;
            *guard = Some(request);
            Ok(OutboundHttpResponse::from_parts(
                200,
                br#"{"active":true,"scope":"mcp.read"}"#.to_vec(),
            ))
        }
    }

    #[test]
    fn introspection_auth_settings_preserves_basic_and_none_semantics() {
        let (auth_type, client_id, client_secret) = introspection_auth_settings(
            IntrospectionClientAuthMethod::Basic,
            Some("client"),
            Some("secret"),
        );
        assert!(matches!(auth_type, oauth2::AuthType::BasicAuth));
        assert_eq!(client_id, Some("client"));
        assert_eq!(client_secret, Some("secret"));

        let (auth_type, client_id, client_secret) =
            introspection_auth_settings(IntrospectionClientAuthMethod::Basic, Some("client"), None);
        assert!(matches!(auth_type, oauth2::AuthType::RequestBody));
        assert_eq!(client_id, None);
        assert_eq!(client_secret, None);

        let (auth_type, client_id, client_secret) = introspection_auth_settings(
            IntrospectionClientAuthMethod::None,
            Some("client"),
            Some("secret"),
        );
        assert!(matches!(auth_type, oauth2::AuthType::RequestBody));
        assert_eq!(client_id, None);
        assert_eq!(client_secret, None);

        let (auth_type, client_id, client_secret) = introspection_auth_settings(
            IntrospectionClientAuthMethod::Post,
            Some("post-client"),
            Some("post-secret"),
        );
        assert!(matches!(auth_type, oauth2::AuthType::RequestBody));
        assert_eq!(client_id, Some("post-client"));
        assert_eq!(client_secret, Some("post-secret"));
    }

    #[tokio::test]
    async fn introspect_applies_timeout_and_omits_client_fields_for_none_auth() {
        let last_request = Arc::new(Mutex::new(None));
        let client = RecordingClient {
            last_request: Arc::clone(&last_request),
        };
        let exchanger = HttpTokenExchanger;

        let payload = exchanger
            .introspect(
                Arc::new(client),
                IntrospectionExchangeParams {
                    token: "opaque-token",
                    url: "https://auth.example.com/introspect",
                    client_auth_method: IntrospectionClientAuthMethod::None,
                    client_id: Some("client-id"),
                    client_secret: Some("client-secret"),
                    timeout_ms: Some(750),
                    max_response_bytes: Some(4096),
                },
            )
            .await
            .expect("introspection should succeed");

        assert_eq!(payload, json!({"active": true, "scope": "mcp.read"}));

        let request = last_request
            .lock()
            .await
            .clone()
            .expect("request should be recorded");
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://auth.example.com/introspect");
        assert_eq!(request.timeout_ms, Some(750));
        assert_eq!(request.max_response_bytes, Some(4096));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "content-type"
                && value.starts_with("application/x-www-form-urlencoded")));
        assert_eq!(
            request.form_body,
            vec![("token".to_owned(), "opaque-token".to_owned())]
        );
    }

    #[tokio::test]
    async fn introspect_maps_non_success_to_stable_error() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/introspect");
                then.status(401).json_body(json!({
                    "error":"invalid_request",
                    "error_description":"missing required scope"
                }));
            })
            .await;

        let exchanger = HttpTokenExchanger;
        let client = Arc::new(crate::http::client::ReqwestHttpClient::default());
        let error = exchanger
            .introspect(
                client,
                IntrospectionExchangeParams {
                    token: "opaque-token",
                    url: &format!("{}/introspect", server.base_url()),
                    client_auth_method: IntrospectionClientAuthMethod::None,
                    client_id: None,
                    client_secret: None,
                    timeout_ms: None,
                    max_response_bytes: None,
                },
            )
            .await
            .expect_err("non-success response should map to stable introspection error");

        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "token introspection endpoint returned an error response: invalid_request: missing required scope"
        );
        assert_eq!(error.data, None);
        mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn introspect_maps_parse_failures_with_details() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/introspect");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{not-json");
            })
            .await;

        let exchanger = HttpTokenExchanger;
        let client = Arc::new(crate::http::client::ReqwestHttpClient::default());
        let error = exchanger
            .introspect(
                client,
                IntrospectionExchangeParams {
                    token: "opaque-token",
                    url: &format!("{}/introspect", server.base_url()),
                    client_auth_method: IntrospectionClientAuthMethod::None,
                    client_id: None,
                    client_secret: None,
                    timeout_ms: None,
                    max_response_bytes: None,
                },
            )
            .await
            .expect_err("parse failure should return detailed invalid JSON message");

        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(
            error
                .message
                .starts_with("token introspection endpoint returned invalid JSON:"),
            "unexpected parse mapping: {}",
            error.message
        );
        assert_eq!(error.data, None);
        mock.assert_calls_async(1).await;
    }

    #[test]
    fn map_introspection_error_request_branch_is_covered() {
        let mapped = map_introspection_error(RequestTokenError::Request(
            crate::auth::oauth::http_bridge::OauthHttpBridgeError::new("dial tcp timeout"),
        ));
        assert_eq!(mapped.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            mapped.message,
            "token introspection request failed: dial tcp timeout"
        );
        assert_eq!(mapped.data, None);
    }
}
