//! Outbound OAuth2 token manager with in-memory cache and refresh coalescing.

use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use oauth2::{AuthType, ClientId, ClientSecret, RefreshToken, Scope, TokenResponse, TokenUrl};
use rmcp::ErrorData as McpError;
use secrecy::{ExposeSecret, SecretString};

use crate::{
    auth::oauth::{
        clock::{Clock, SystemClock},
        http_bridge::{OauthHttpBridgeError, SharedOauthHttpClient},
    },
    config::{UpstreamOauth2AuthConfig, UpstreamOauth2ClientAuthMethod, UpstreamOauth2GrantType},
    http::client::{ReqwestHttpClient, SharedHttpClient},
};

use super::{
    config::{OutboundOauth2MtlsResolvedConfig, OutboundOauth2ResolvedConfig},
    token_cache::{CachedOutboundToken, OutboundTokenCache},
};

#[derive(Clone, Debug)]
pub(crate) struct TokenExchangeResult {
    pub(crate) access_token: SecretString,
    pub(crate) refresh_token: Option<SecretString>,
    pub(crate) expires_in: Option<Duration>,
}

pub(crate) trait OutboundTokenExchanger: Send + Sync {
    fn exchange_client_credentials<'a>(
        &'a self,
        config: &'a OutboundOauth2ResolvedConfig,
    ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>>;

    fn exchange_refresh_token<'a>(
        &'a self,
        config: &'a OutboundOauth2ResolvedConfig,
        refresh_token: &'a SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>>;
}

#[derive(Clone)]
pub(crate) struct Oauth2TokenExchanger {
    http_client: SharedHttpClient,
}

impl fmt::Debug for Oauth2TokenExchanger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Oauth2TokenExchanger")
            .field("http_client", &"<SharedHttpClient>")
            .finish()
    }
}

impl Oauth2TokenExchanger {
    pub(crate) fn new(http_client: SharedHttpClient) -> Self {
        Self { http_client }
    }
}

impl OutboundTokenExchanger for Oauth2TokenExchanger {
    fn exchange_client_credentials<'a>(
        &'a self,
        config: &'a OutboundOauth2ResolvedConfig,
    ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>> {
        Box::pin(async move {
            let token_url = TokenUrl::new(config.token_url.clone()).map_err(|error| {
                McpError::invalid_request(format!("invalid oauth2 token_url: {error}"), None)
            })?;
            let mut client =
                oauth2::basic::BasicClient::new(ClientId::new(config.client_id.clone()))
                    .set_client_secret(ClientSecret::new(
                        config.client_secret.expose_secret().to_owned(),
                    ))
                    .set_token_uri(token_url);
            client = client.set_auth_type(match config.auth_method {
                UpstreamOauth2ClientAuthMethod::Basic => AuthType::BasicAuth,
                UpstreamOauth2ClientAuthMethod::RequestBody => AuthType::RequestBody,
            });

            let mut request = client.exchange_client_credentials();
            for scope in &config.scopes {
                request = request.add_scope(Scope::new(scope.clone()));
            }
            for (key, value) in &config.extra_token_params {
                request = request.add_extra_param(key.clone(), value.clone());
            }

            let http_client = self.oauth_http_client(config)?;
            let response = request
                .request_async(&http_client)
                .await
                .map_err(map_exchange_error)?;
            Ok(to_exchange_result(&response))
        })
    }

    fn exchange_refresh_token<'a>(
        &'a self,
        config: &'a OutboundOauth2ResolvedConfig,
        refresh_token: &'a SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>> {
        Box::pin(async move {
            let token_url = TokenUrl::new(config.token_url.clone()).map_err(|error| {
                McpError::invalid_request(format!("invalid oauth2 token_url: {error}"), None)
            })?;
            let mut client =
                oauth2::basic::BasicClient::new(ClientId::new(config.client_id.clone()))
                    .set_client_secret(ClientSecret::new(
                        config.client_secret.expose_secret().to_owned(),
                    ))
                    .set_token_uri(token_url);
            client = client.set_auth_type(match config.auth_method {
                UpstreamOauth2ClientAuthMethod::Basic => AuthType::BasicAuth,
                UpstreamOauth2ClientAuthMethod::RequestBody => AuthType::RequestBody,
            });

            let refresh_token = RefreshToken::new(refresh_token.expose_secret().to_owned());
            let mut request = client.exchange_refresh_token(&refresh_token);
            for scope in &config.scopes {
                request = request.add_scope(Scope::new(scope.clone()));
            }
            for (key, value) in &config.extra_token_params {
                request = request.add_extra_param(key.clone(), value.clone());
            }

            let http_client = self.oauth_http_client(config)?;
            let response = request
                .request_async(&http_client)
                .await
                .map_err(map_exchange_error)?;
            Ok(to_exchange_result(&response))
        })
    }
}

impl Oauth2TokenExchanger {
    fn oauth_http_client(
        &self,
        config: &OutboundOauth2ResolvedConfig,
    ) -> Result<SharedOauthHttpClient, McpError> {
        let Some(mtls) = config.mtls.as_ref() else {
            return Ok(SharedOauthHttpClient::new(Arc::clone(&self.http_client)));
        };
        let client = build_mtls_reqwest_client(mtls)?;
        Ok(SharedOauthHttpClient::new(Arc::new(
            ReqwestHttpClient::new(client),
        )))
    }
}

fn build_mtls_reqwest_client(
    mtls: &OutboundOauth2MtlsResolvedConfig,
) -> Result<reqwest::Client, McpError> {
    let mut builder = reqwest::Client::builder();
    if let Some(ca_cert) = mtls.ca_cert.as_ref() {
        let ca_cert = reqwest::Certificate::from_pem(ca_cert.expose_secret().as_bytes()).map_err(
            |error| {
                McpError::invalid_request(format!("invalid oauth2 mTLS ca_cert PEM: {error}"), None)
            },
        )?;
        builder = builder.add_root_certificate(ca_cert);
    }

    let identity_pem = format!(
        "{}\n{}",
        mtls.client_cert.expose_secret(),
        mtls.client_key.expose_secret()
    );
    let identity = reqwest::Identity::from_pem(identity_pem.as_bytes()).map_err(|error| {
        McpError::invalid_request(
            format!("invalid oauth2 mTLS client identity PEM: {error}"),
            None,
        )
    })?;
    builder.identity(identity).build().map_err(|error| {
        McpError::internal_error(format!("failed to build oauth2 mTLS client: {error}"), None)
    })
}

#[derive(Clone)]
pub(crate) struct OutboundTokenManager {
    cache: OutboundTokenCache,
    exchanger: Arc<dyn OutboundTokenExchanger>,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for OutboundTokenManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundTokenManager")
            .field("cache", &self.cache)
            .field("exchanger", &"<dyn OutboundTokenExchanger>")
            .field("clock", &"<dyn Clock>")
            .finish()
    }
}

impl OutboundTokenManager {
    pub(crate) fn new(http_client: SharedHttpClient) -> Self {
        Self::with_parts(
            OutboundTokenCache::default(),
            Arc::new(Oauth2TokenExchanger::new(http_client)),
            Arc::new(SystemClock),
        )
    }

    pub(crate) fn with_parts(
        cache: OutboundTokenCache,
        exchanger: Arc<dyn OutboundTokenExchanger>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            cache,
            exchanger,
            clock,
        }
    }

    pub(crate) async fn access_token(
        &self,
        upstream_name: &str,
        auth: &UpstreamOauth2AuthConfig,
        force_refresh: bool,
    ) -> Result<SecretString, McpError> {
        let resolved = OutboundOauth2ResolvedConfig::from_auth(upstream_name, auth)?;
        let now = self.clock.now();
        if !force_refresh {
            if let Some(entry) = self
                .cache
                .get_fresh(&resolved.cache_key, now, resolved.refresh_policy.skew_sec)
                .await
            {
                return Ok(entry.access_token);
            }
        }

        let refresh_lock = self.cache.refresh_lock(&resolved.cache_key).await;
        let _refresh_guard = refresh_lock.lock().await;

        let now = self.clock.now();
        if !force_refresh {
            if let Some(entry) = self
                .cache
                .get_fresh(&resolved.cache_key, now, resolved.refresh_policy.skew_sec)
                .await
            {
                return Ok(entry.access_token);
            }
        }
        let exchange_result = self.exchange_token(&resolved).await?;
        let expires_at = expires_at(self.clock.now(), exchange_result.expires_in)?;

        let existing = self.cache.get(&resolved.cache_key).await;
        let refresh_token = exchange_result
            .refresh_token
            .or_else(|| existing.and_then(|entry| entry.refresh_token))
            .or_else(|| resolved.bootstrap_refresh_token.clone());

        self.cache
            .upsert(
                resolved.cache_key,
                CachedOutboundToken {
                    access_token: exchange_result.access_token.clone(),
                    refresh_token,
                    expires_at,
                },
            )
            .await;

        Ok(exchange_result.access_token)
    }

    async fn exchange_token(
        &self,
        resolved: &OutboundOauth2ResolvedConfig,
    ) -> Result<TokenExchangeResult, McpError> {
        match resolved.grant {
            UpstreamOauth2GrantType::ClientCredentials => {
                self.exchanger.exchange_client_credentials(resolved).await
            }
            UpstreamOauth2GrantType::RefreshToken => {
                let cached_refresh_token = self
                    .cache
                    .get(&resolved.cache_key)
                    .await
                    .and_then(|entry| entry.refresh_token);
                let refresh_token = cached_refresh_token
                    .or_else(|| resolved.bootstrap_refresh_token.clone())
                    .ok_or_else(|| {
                        McpError::invalid_request(
                            "oauth2 refresh_token grant requires a refresh token".to_owned(),
                            None,
                        )
                    })?;
                self.exchanger
                    .exchange_refresh_token(resolved, &refresh_token)
                    .await
            }
        }
    }
}

fn to_exchange_result(
    response: &oauth2::StandardTokenResponse<
        oauth2::EmptyExtraTokenFields,
        oauth2::basic::BasicTokenType,
    >,
) -> TokenExchangeResult {
    TokenExchangeResult {
        access_token: SecretString::new(
            response.access_token().secret().to_owned().into_boxed_str(),
        ),
        refresh_token: response
            .refresh_token()
            .map(|token| SecretString::new(token.secret().to_owned().into_boxed_str())),
        expires_in: response.expires_in(),
    }
}

fn map_exchange_error(
    error: oauth2::RequestTokenError<
        OauthHttpBridgeError,
        oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
    >,
) -> McpError {
    let message = match error {
        oauth2::RequestTokenError::ServerResponse(error_response) => {
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
            format!("oauth2 token endpoint returned an error response: {details}")
        }
        oauth2::RequestTokenError::Request(error) => {
            format!("oauth2 token request failed: {error}")
        }
        oauth2::RequestTokenError::Parse(parse_error, _) => {
            format!("oauth2 token endpoint returned invalid JSON: {parse_error}")
        }
        oauth2::RequestTokenError::Other(message) => {
            format!("oauth2 token exchange failed: {message}")
        }
    };
    McpError::internal_error(message, None)
}

fn expires_at(
    now: DateTime<Utc>,
    expires_in: Option<Duration>,
) -> Result<Option<DateTime<Utc>>, McpError> {
    let Some(expires_in) = expires_in else {
        return Ok(None);
    };
    let expires_in = chrono::Duration::from_std(expires_in).map_err(|error| {
        McpError::internal_error(format!("invalid oauth2 expiry: {error}"), None)
    })?;
    Ok(Some(now + expires_in))
}

#[cfg(test)]
// Inline tests cover private helper branches not reachable via external tests.
mod tests {
    use super::{
        build_mtls_reqwest_client, expires_at, map_exchange_error, Oauth2TokenExchanger,
        OutboundTokenExchanger, OutboundTokenManager, TokenExchangeResult,
    };
    use crate::{
        auth::{
            oauth::clock::SystemClock,
            outbound::{config::OutboundOauth2MtlsResolvedConfig, token_cache::OutboundTokenCache},
        },
        config::{
            SecretValueConfig, SecretValueSource, UpstreamOauth2AuthConfig,
            UpstreamOauth2ClientAuthMethod, UpstreamOauth2GrantType,
        },
        http::client::ReqwestHttpClient,
        mcp::ErrorCode,
        McpError,
    };
    use oauth2::RequestTokenError;
    use secrecy::{ExposeSecret, SecretString};
    use std::{
        collections::{BTreeMap, HashMap},
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    #[derive(Clone, Debug)]
    struct CountingExchanger {
        client_credentials_calls: Arc<AtomicUsize>,
    }

    impl OutboundTokenExchanger for CountingExchanger {
        fn exchange_client_credentials<'a>(
            &'a self,
            _config: &'a crate::auth::outbound::config::OutboundOauth2ResolvedConfig,
        ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>>
        {
            let calls = Arc::clone(&self.client_credentials_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(TokenExchangeResult {
                    access_token: SecretString::new("counted-token".to_owned().into_boxed_str()),
                    refresh_token: None,
                    expires_in: Some(Duration::from_secs(3600)),
                })
            })
        }

        fn exchange_refresh_token<'a>(
            &'a self,
            _config: &'a crate::auth::outbound::config::OutboundOauth2ResolvedConfig,
            _refresh_token: &'a SecretString,
        ) -> Pin<Box<dyn Future<Output = Result<TokenExchangeResult, McpError>> + Send + 'a>>
        {
            Box::pin(async {
                Err(McpError::internal_error(
                    "refresh exchange should not run in this test".to_owned(),
                    None,
                ))
            })
        }
    }

    #[test]
    fn map_exchange_error_other_branch_is_covered() {
        let error: RequestTokenError<
            crate::auth::oauth::http_bridge::OauthHttpBridgeError,
            oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
        > = RequestTokenError::Other("other-branch".to_owned());

        let mapped = map_exchange_error(error);
        assert_eq!(mapped.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mapped.message, "oauth2 token exchange failed: other-branch");
        assert_eq!(mapped.data, None);
    }

    #[test]
    fn map_exchange_error_server_response_branch_is_covered() {
        let error_response = oauth2::StandardErrorResponse::new(
            oauth2::basic::BasicErrorResponseType::InvalidGrant,
            Some("client is not authorized for this audience".to_owned()),
            Some("https://auth.example.com/docs/client-grants".to_owned()),
        );
        let mapped = map_exchange_error(RequestTokenError::ServerResponse(error_response));
        assert_eq!(mapped.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            mapped.message,
            "oauth2 token endpoint returned an error response: invalid_grant: client is not authorized for this audience (https://auth.example.com/docs/client-grants)"
        );
        assert_eq!(mapped.data, None);
    }

    #[test]
    fn map_exchange_error_request_branch_is_covered() {
        let mapped = map_exchange_error(RequestTokenError::Request(
            crate::auth::oauth::http_bridge::OauthHttpBridgeError::new("network down"),
        ));
        assert_eq!(mapped.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mapped.message, "oauth2 token request failed: network down");
        assert_eq!(mapped.data, None);
    }

    #[test]
    fn expires_at_handles_none_and_invalid_duration() {
        let now = chrono::Utc::now();
        let no_expiry = expires_at(now, None).expect("none expiry should be accepted");
        assert!(no_expiry.is_none());

        let error = expires_at(now, Some(Duration::MAX)).expect_err("huge duration should fail");
        assert!(error.message.contains("invalid oauth2 expiry"));
    }

    #[tokio::test]
    async fn manager_caches_access_token_from_exchanger() {
        let calls = Arc::new(AtomicUsize::new(0));
        let exchanger = CountingExchanger {
            client_credentials_calls: Arc::clone(&calls),
        };
        let manager = OutboundTokenManager::with_parts(
            OutboundTokenCache::default(),
            Arc::new(exchanger),
            Arc::new(SystemClock),
        );
        let auth = UpstreamOauth2AuthConfig {
            grant: UpstreamOauth2GrantType::ClientCredentials,
            token_url: "https://auth.example.com/token".to_owned(),
            client_id: "client".to_owned(),
            client_secret: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "secret".to_owned(),
            },
            auth_method: Some(UpstreamOauth2ClientAuthMethod::Basic),
            scopes: vec!["read".to_owned()],
            audience: None,
            extra_token_params: HashMap::new(),
            refresh: None,
            refresh_token: None,
            mtls: None,
        };

        let first = manager
            .access_token("reports", &auth, false)
            .await
            .expect("first access token");
        assert_eq!(first.expose_secret(), "counted-token");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = manager
            .access_token("reports", &auth, false)
            .await
            .expect("cached access token");
        assert_eq!(second.expose_secret(), "counted-token");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call should reuse cached token"
        );
    }

    #[tokio::test]
    async fn exchange_token_refresh_grant_errors_without_cached_or_bootstrap_refresh_token() {
        let manager = OutboundTokenManager::with_parts(
            OutboundTokenCache::default(),
            Arc::new(Oauth2TokenExchanger::new(Arc::new(
                ReqwestHttpClient::default(),
            ))),
            Arc::new(SystemClock),
        );
        let resolved = crate::auth::outbound::config::OutboundOauth2ResolvedConfig {
            cache_key: crate::auth::outbound::config::OutboundOauth2CacheKey {
                upstream_name: "reports".to_owned(),
                grant: UpstreamOauth2GrantType::RefreshToken,
                scopes: Vec::new(),
                audience: None,
                resource: None,
                extra_token_params: Vec::new(),
            },
            grant: UpstreamOauth2GrantType::RefreshToken,
            token_url: "https://auth.example.com/token".to_owned(),
            client_id: "client".to_owned(),
            client_secret: SecretString::new("secret".to_owned().into_boxed_str()),
            auth_method: UpstreamOauth2ClientAuthMethod::Basic,
            scopes: Vec::new(),
            extra_token_params: BTreeMap::new(),
            bootstrap_refresh_token: None,
            mtls: None,
            refresh_policy: crate::auth::outbound::config::OutboundOauth2RefreshPolicy {
                skew_sec: 0,
                retry_on_401_once: true,
            },
        };

        let error = manager
            .exchange_token(&resolved)
            .await
            .expect_err("missing refresh token sources should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "oauth2 refresh_token grant requires a refresh token"
        );
        assert_eq!(error.data, None);
    }

    #[test]
    fn build_mtls_reqwest_client_with_invalid_ca_still_rejects_invalid_identity_pem() {
        let mtls = OutboundOauth2MtlsResolvedConfig {
            ca_cert: Some(SecretString::new(
                "-----BEGIN CERTIFICATE-----\ninvalid"
                    .to_owned()
                    .into_boxed_str(),
            )),
            client_cert: SecretString::new("invalid-cert".to_owned().into_boxed_str()),
            client_key: SecretString::new("invalid-key".to_owned().into_boxed_str()),
        };

        let error =
            build_mtls_reqwest_client(&mtls).expect_err("invalid mTLS PEM should be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("invalid oauth2 mTLS client identity PEM:"));
        assert_eq!(error.data, None);
    }

    #[test]
    fn build_mtls_reqwest_client_rejects_invalid_identity_pem() {
        let mtls = OutboundOauth2MtlsResolvedConfig {
            ca_cert: None,
            client_cert: SecretString::new("invalid-cert".to_owned().into_boxed_str()),
            client_key: SecretString::new("invalid-key".to_owned().into_boxed_str()),
        };

        let error = build_mtls_reqwest_client(&mtls)
            .expect_err("invalid mTLS identity PEM should be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("invalid oauth2 mTLS client identity PEM:"));
        assert_eq!(error.data, None);
    }

    #[tokio::test]
    async fn exchange_client_credentials_maps_parse_errors_with_details() {
        let server = httpmock::MockServer::start_async().await;
        let _mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body("{not-json");
            })
            .await;

        let exchanger = Oauth2TokenExchanger::new(Arc::new(ReqwestHttpClient::default()));
        let config = crate::auth::outbound::config::OutboundOauth2ResolvedConfig::from_auth(
            "reports",
            &UpstreamOauth2AuthConfig {
                grant: UpstreamOauth2GrantType::ClientCredentials,
                token_url: format!("{}/token", server.base_url()),
                client_id: "client".to_owned(),
                client_secret: SecretValueConfig {
                    source: SecretValueSource::Inline,
                    value: "secret".to_owned(),
                },
                auth_method: Some(UpstreamOauth2ClientAuthMethod::Basic),
                scopes: Vec::new(),
                audience: None,
                extra_token_params: HashMap::new(),
                refresh: None,
                refresh_token: None,
                mtls: None,
            },
        )
        .expect("resolved config should build");

        let error = exchanger
            .exchange_client_credentials(&config)
            .await
            .expect_err("invalid JSON should map to parse details");
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert!(
            error
                .message
                .starts_with("oauth2 token endpoint returned invalid JSON:"),
            "unexpected parse error mapping: {}",
            error.message
        );
    }
}
