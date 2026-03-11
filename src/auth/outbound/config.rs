//! Resolved outbound OAuth2 runtime config and deterministic cache key shape.

use std::collections::{BTreeMap, BTreeSet};

use rmcp::ErrorData as McpError;
use secrecy::SecretString;

use crate::config::{
    UpstreamOauth2AuthConfig, UpstreamOauth2ClientAuthMethod, UpstreamOauth2GrantType,
    UpstreamOauth2MtlsConfig,
};

const DEFAULT_REFRESH_SKEW_SEC: u64 = 60;
const DEFAULT_RETRY_ON_401_ONCE: bool = true;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct OutboundOauth2CacheKey {
    pub(crate) upstream_name: String,
    pub(crate) grant: UpstreamOauth2GrantType,
    pub(crate) scopes: Vec<String>,
    pub(crate) audience: Option<String>,
    pub(crate) resource: Option<String>,
    pub(crate) extra_token_params: Vec<(String, String)>,
}

impl OutboundOauth2CacheKey {
    pub(crate) fn from_auth(upstream_name: &str, auth: &UpstreamOauth2AuthConfig) -> Self {
        let scopes = normalize_scopes(&auth.scopes);
        let extra_token_params = normalize_extra_params(auth);
        let resource = extra_token_params
            .iter()
            .find_map(|(key, value)| (key == "resource").then(|| value.clone()));

        Self {
            upstream_name: upstream_name.to_owned(),
            grant: auth.grant,
            scopes,
            audience: auth.audience.clone(),
            resource,
            extra_token_params,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutboundOauth2RefreshPolicy {
    pub(crate) skew_sec: u64,
    pub(crate) retry_on_401_once: bool,
}

impl OutboundOauth2RefreshPolicy {
    pub(crate) fn from_auth(auth: &UpstreamOauth2AuthConfig) -> Self {
        let skew_sec = auth
            .refresh
            .as_ref()
            .and_then(|refresh| refresh.skew_sec)
            .unwrap_or(DEFAULT_REFRESH_SKEW_SEC);
        let retry_on_401_once = auth
            .refresh
            .as_ref()
            .and_then(|refresh| refresh.retry_on_401_once)
            .unwrap_or(DEFAULT_RETRY_ON_401_ONCE);

        Self {
            skew_sec,
            retry_on_401_once,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OutboundOauth2MtlsResolvedConfig {
    pub(crate) ca_cert: Option<SecretString>,
    pub(crate) client_cert: SecretString,
    pub(crate) client_key: SecretString,
}

#[derive(Clone, Debug)]
pub(crate) struct OutboundOauth2ResolvedConfig {
    pub(crate) cache_key: OutboundOauth2CacheKey,
    pub(crate) grant: UpstreamOauth2GrantType,
    pub(crate) token_url: String,
    pub(crate) client_id: String,
    pub(crate) client_secret: SecretString,
    pub(crate) auth_method: UpstreamOauth2ClientAuthMethod,
    pub(crate) scopes: Vec<String>,
    pub(crate) extra_token_params: BTreeMap<String, String>,
    pub(crate) bootstrap_refresh_token: Option<SecretString>,
    pub(crate) mtls: Option<OutboundOauth2MtlsResolvedConfig>,
    pub(crate) refresh_policy: OutboundOauth2RefreshPolicy,
}

impl OutboundOauth2ResolvedConfig {
    pub(crate) fn from_auth(
        upstream_name: &str,
        auth: &UpstreamOauth2AuthConfig,
    ) -> Result<Self, McpError> {
        let client_secret = auth.client_secret.resolve_secret().map_err(|error| {
            McpError::invalid_request(
                format!("failed to resolve oauth2 client_secret: {error}"),
                None,
            )
        })?;

        let bootstrap_refresh_token = auth
            .refresh_token
            .as_ref()
            .map(crate::config::SecretValueConfig::resolve_secret)
            .transpose()
            .map_err(|error| {
                McpError::invalid_request(
                    format!("failed to resolve oauth2 refresh_token: {error}"),
                    None,
                )
            })?;

        if auth.grant == UpstreamOauth2GrantType::RefreshToken && bootstrap_refresh_token.is_none()
        {
            return Err(McpError::invalid_request(
                "oauth2 refresh_token grant requires refresh_token".to_owned(),
                None,
            ));
        }

        let mtls = auth.mtls.as_ref().map(resolve_mtls).transpose()?;

        Ok(Self {
            cache_key: OutboundOauth2CacheKey::from_auth(upstream_name, auth),
            grant: auth.grant,
            token_url: auth.token_url.clone(),
            client_id: auth.client_id.clone(),
            client_secret,
            auth_method: auth
                .auth_method
                .unwrap_or(UpstreamOauth2ClientAuthMethod::Basic),
            scopes: normalize_scopes(&auth.scopes),
            extra_token_params: normalized_token_params_map(auth),
            bootstrap_refresh_token,
            mtls,
            refresh_policy: OutboundOauth2RefreshPolicy::from_auth(auth),
        })
    }
}

fn resolve_mtls(
    mtls: &UpstreamOauth2MtlsConfig,
) -> Result<OutboundOauth2MtlsResolvedConfig, McpError> {
    let ca_cert = mtls
        .ca_cert
        .as_ref()
        .map(crate::config::SecretValueConfig::resolve_secret)
        .transpose()
        .map_err(|error| {
            McpError::invalid_request(
                format!("failed to resolve oauth2 mTLS ca_cert: {error}"),
                None,
            )
        })?;
    let client_cert = mtls.client_cert.resolve_secret().map_err(|error| {
        McpError::invalid_request(
            format!("failed to resolve oauth2 mTLS client_cert: {error}"),
            None,
        )
    })?;
    let client_key = mtls.client_key.resolve_secret().map_err(|error| {
        McpError::invalid_request(
            format!("failed to resolve oauth2 mTLS client_key: {error}"),
            None,
        )
    })?;

    Ok(OutboundOauth2MtlsResolvedConfig {
        ca_cert,
        client_cert,
        client_key,
    })
}

fn normalize_scopes(scopes: &[String]) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    normalized.extend(
        scopes
            .iter()
            .map(|scope| scope.trim())
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned),
    );
    normalized.into_iter().collect()
}

fn normalized_token_params_map(auth: &UpstreamOauth2AuthConfig) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.extend(
        auth.extra_token_params
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    if let Some(audience) = auth.audience.as_ref() {
        params
            .entry("audience".to_owned())
            .or_insert_with(|| audience.clone());
    }
    params
}

fn normalize_extra_params(auth: &UpstreamOauth2AuthConfig) -> Vec<(String, String)> {
    normalized_token_params_map(auth).into_iter().collect()
}

#[cfg(test)]
// Inline tests verify private normalization and secret resolution paths.
mod tests {
    use super::{
        OutboundOauth2CacheKey, OutboundOauth2RefreshPolicy, OutboundOauth2ResolvedConfig,
    };
    use crate::config::{
        SecretValueConfig, SecretValueSource, UpstreamOauth2AuthConfig,
        UpstreamOauth2ClientAuthMethod, UpstreamOauth2GrantType, UpstreamOauth2MtlsConfig,
        UpstreamOauth2RefreshConfig,
    };
    use crate::mcp::ErrorCode;
    use secrecy::ExposeSecret;
    use std::{collections::HashMap, sync::Mutex};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn oauth2_config() -> UpstreamOauth2AuthConfig {
        UpstreamOauth2AuthConfig {
            grant: UpstreamOauth2GrantType::ClientCredentials,
            token_url: "https://auth.example.com/oauth/token".to_owned(),
            client_id: "client-a".to_owned(),
            client_secret: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "secret".to_owned(),
            },
            auth_method: None,
            scopes: vec!["write".to_owned(), "read".to_owned(), "read".to_owned()],
            audience: Some("https://api.example.com".to_owned()),
            extra_token_params: HashMap::from([("resource".to_owned(), "reports".to_owned())]),
            refresh: None,
            refresh_token: None,
            mtls: None,
        }
    }

    #[test]
    fn cache_key_normalizes_scopes_and_params() {
        let auth = oauth2_config();
        let key = OutboundOauth2CacheKey::from_auth("reports", &auth);

        assert_eq!(key.scopes, vec!["read".to_owned(), "write".to_owned()]);
        assert_eq!(
            key.extra_token_params,
            vec![
                ("audience".to_owned(), "https://api.example.com".to_owned()),
                ("resource".to_owned(), "reports".to_owned()),
            ]
        );

        let mut empty_auth = oauth2_config();
        empty_auth.scopes = vec![String::new(), "   ".to_owned()];
        empty_auth.audience = None;
        empty_auth.extra_token_params = HashMap::new();
        let empty_key = OutboundOauth2CacheKey::from_auth("reports", &empty_auth);
        assert!(empty_key.scopes.is_empty());
        assert!(empty_key.extra_token_params.is_empty());

        let mut unsorted = oauth2_config();
        unsorted.scopes = vec![
            "write".to_owned(),
            "read".to_owned(),
            "write".to_owned(),
            " read ".to_owned(),
        ];
        let unsorted_key = OutboundOauth2CacheKey::from_auth("reports", &unsorted);
        assert_eq!(
            unsorted_key.scopes,
            vec!["read".to_owned(), "write".to_owned()]
        );
    }

    #[test]
    fn refresh_policy_uses_defaults() {
        let auth = oauth2_config();
        let policy = OutboundOauth2RefreshPolicy::from_auth(&auth);
        assert_eq!(policy.skew_sec, 60);
        assert!(policy.retry_on_401_once);
    }

    #[test]
    fn resolved_config_requires_refresh_token_for_refresh_grant() {
        let mut auth = oauth2_config();
        auth.grant = UpstreamOauth2GrantType::RefreshToken;
        auth.refresh_token = None;

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing refresh token should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "oauth2 refresh_token grant requires refresh_token"
        );
        assert_eq!(error.data, None);
    }

    #[test]
    fn resolved_config_resolves_inline_secret_and_defaults_auth_method() {
        let auth = oauth2_config();
        let resolved = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect("config should resolve");

        assert_eq!(resolved.client_id, "client-a");
        assert_eq!(resolved.scopes, vec!["read".to_owned(), "write".to_owned()]);
        assert_eq!(resolved.token_url, "https://auth.example.com/oauth/token");
        assert_eq!(resolved.client_secret.expose_secret(), "secret");
        assert!(resolved.mtls.is_none());
    }

    #[test]
    fn resolved_config_uses_custom_auth_method_and_refresh_policy() {
        let mut auth = oauth2_config();
        auth.auth_method = Some(UpstreamOauth2ClientAuthMethod::RequestBody);
        auth.refresh = Some(UpstreamOauth2RefreshConfig {
            skew_sec: Some(5),
            retry_on_401_once: Some(false),
        });

        let resolved = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect("config should resolve");
        assert_eq!(
            resolved.auth_method,
            UpstreamOauth2ClientAuthMethod::RequestBody
        );
        assert_eq!(resolved.refresh_policy.skew_sec, 5);
        assert!(!resolved.refresh_policy.retry_on_401_once);
    }

    #[test]
    fn resolved_config_fails_when_client_secret_cannot_be_resolved() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let mut auth = oauth2_config();
        auth.client_secret = SecretValueConfig {
            source: SecretValueSource::Env,
            value: "MISSING_OAUTH_CLIENT_SECRET_ENV".to_owned(),
        };
        std::env::remove_var("MISSING_OAUTH_CLIENT_SECRET_ENV");

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing client_secret env should error");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("failed to resolve oauth2 client_secret: failed to read env var 'MISSING_OAUTH_CLIENT_SECRET_ENV' for secret value:"));
        assert_eq!(error.data, None);
    }

    #[test]
    fn resolved_config_fails_when_refresh_token_cannot_be_resolved() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let mut auth = oauth2_config();
        auth.grant = UpstreamOauth2GrantType::RefreshToken;
        auth.refresh_token = Some(SecretValueConfig {
            source: SecretValueSource::Env,
            value: "MISSING_OAUTH_REFRESH_TOKEN_ENV".to_owned(),
        });
        std::env::remove_var("MISSING_OAUTH_REFRESH_TOKEN_ENV");

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing refresh token env should error");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("failed to resolve oauth2 refresh_token: failed to read env var 'MISSING_OAUTH_REFRESH_TOKEN_ENV' for secret value:"));
        assert_eq!(error.data, None);
    }

    #[test]
    fn cache_key_does_not_override_existing_audience_extra_param() {
        let mut auth = oauth2_config();
        auth.extra_token_params.insert(
            "audience".to_owned(),
            "https://override.example.com".to_owned(),
        );

        let key = OutboundOauth2CacheKey::from_auth("reports", &auth);
        assert_eq!(
            key.extra_token_params,
            vec![
                (
                    "audience".to_owned(),
                    "https://override.example.com".to_owned()
                ),
                ("resource".to_owned(), "reports".to_owned()),
            ]
        );
    }

    #[test]
    fn resolved_config_resolves_mtls_from_env_sources() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::set_var(
            "TEST_MTLS_CERT",
            "-----BEGIN CERTIFICATE-----\ntest-cert\n-----END CERTIFICATE-----",
        );
        std::env::set_var(
            "TEST_MTLS_KEY",
            "-----BEGIN PRIVATE KEY-----\ntest-key\n-----END PRIVATE KEY-----",
        );
        std::env::set_var(
            "TEST_MTLS_CA",
            "-----BEGIN CERTIFICATE-----\nca-cert\n-----END CERTIFICATE-----",
        );

        let mut auth = oauth2_config();
        auth.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: Some(SecretValueConfig {
                source: SecretValueSource::Env,
                value: "TEST_MTLS_CA".to_owned(),
            }),
            client_cert: SecretValueConfig {
                source: SecretValueSource::Env,
                value: "TEST_MTLS_CERT".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Env,
                value: "TEST_MTLS_KEY".to_owned(),
            },
        });

        let resolved = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect("mTLS env values should resolve");
        let mtls = resolved.mtls.expect("mTLS config should exist");
        assert_eq!(
            mtls.client_cert.expose_secret(),
            "-----BEGIN CERTIFICATE-----\ntest-cert\n-----END CERTIFICATE-----"
        );
        assert_eq!(
            mtls.client_key.expose_secret(),
            "-----BEGIN PRIVATE KEY-----\ntest-key\n-----END PRIVATE KEY-----"
        );
        assert_eq!(
            mtls.ca_cert.expect("ca cert should exist").expose_secret(),
            "-----BEGIN CERTIFICATE-----\nca-cert\n-----END CERTIFICATE-----"
        );

        std::env::remove_var("TEST_MTLS_CERT");
        std::env::remove_var("TEST_MTLS_KEY");
        std::env::remove_var("TEST_MTLS_CA");
    }

    #[test]
    fn resolved_config_resolves_mtls_without_optional_ca() {
        let mut auth = oauth2_config();
        auth.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: None,
            client_cert: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "-----BEGIN CERTIFICATE-----\nclient-cert\n-----END CERTIFICATE-----"
                    .to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "-----BEGIN PRIVATE KEY-----\nclient-key\n-----END PRIVATE KEY-----"
                    .to_owned(),
            },
        });

        let resolved = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect("mTLS without CA should resolve");
        let mtls = resolved.mtls.expect("mTLS config should exist");
        assert!(mtls.ca_cert.is_none());
    }

    #[test]
    fn resolved_config_rejects_unresolvable_mtls_client_cert() {
        let mut auth = oauth2_config();
        auth.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: None,
            client_cert: SecretValueConfig {
                source: SecretValueSource::Env,
                value: "MISSING_MTLS_CERT".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "key".to_owned(),
            },
        });

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing mTLS client cert should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("failed to resolve oauth2 mTLS client_cert: failed to read env var 'MISSING_MTLS_CERT' for secret value:"));
        assert_eq!(error.data, None);
    }

    #[test]
    fn resolved_config_rejects_unresolvable_mtls_client_key() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let mut auth = oauth2_config();
        auth.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: None,
            client_cert: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "cert".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Env,
                value: "MISSING_MTLS_KEY".to_owned(),
            },
        });
        std::env::remove_var("MISSING_MTLS_KEY");

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing mTLS client key should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("failed to resolve oauth2 mTLS client_key: failed to read env var 'MISSING_MTLS_KEY' for secret value:"));
        assert_eq!(error.data, None);
    }

    #[test]
    fn resolved_config_rejects_unresolvable_mtls_ca_cert() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let mut auth = oauth2_config();
        auth.mtls = Some(UpstreamOauth2MtlsConfig {
            ca_cert: Some(SecretValueConfig {
                source: SecretValueSource::Env,
                value: "MISSING_MTLS_CA".to_owned(),
            }),
            client_cert: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "cert".to_owned(),
            },
            client_key: SecretValueConfig {
                source: SecretValueSource::Inline,
                value: "key".to_owned(),
            },
        });
        std::env::remove_var("MISSING_MTLS_CA");

        let error = OutboundOauth2ResolvedConfig::from_auth("reports", &auth)
            .expect_err("missing mTLS ca cert should fail");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error
            .message
            .starts_with("failed to resolve oauth2 mTLS ca_cert: failed to read env var 'MISSING_MTLS_CA' for secret value:"));
        assert_eq!(error.data, None);
    }
}
