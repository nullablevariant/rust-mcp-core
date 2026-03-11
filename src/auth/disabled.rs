//! Stub auth module used when the auth feature is disabled.
use std::{collections::HashMap, sync::Arc};

use rmcp::ErrorData as McpError;
use serde_json::Value;

use crate::{
    config::{AuthProviderConfig, McpConfig},
    plugins::PluginRegistry,
};

#[doc(hidden)]
pub mod oauth {
    #[path = "clock.rs"]
    pub mod clock;
    #[cfg(feature = "http_tools")]
    #[path = "http_bridge.rs"]
    pub mod http_bridge;
}

#[cfg(feature = "http_tools")]
pub(crate) mod outbound {
    #[path = "config.rs"]
    pub(crate) mod config;
    #[path = "token_cache.rs"]
    pub(crate) mod token_cache;
    #[path = "token_manager.rs"]
    pub(crate) mod token_manager;
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct AuthState;

impl AuthState {
    pub const fn auth_enabled(&self) -> bool {
        false
    }

    pub const fn oauth_enabled(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum AuthActivation {
    None,
    BearerOnly,
    OauthOnly,
    BearerAndOauth,
}

// Keep in sync with src/auth/inbound/factory.rs::AuthStateParams
#[derive(Debug)]
#[doc(hidden)]
pub struct AuthStateParams {
    pub activation: AuthActivation,
    pub bearer_token: Option<String>,
    pub resource: Option<String>,
    pub resource_metadata_url: Option<String>,
    pub providers: Vec<AuthProviderConfig>,
    pub plugins: Option<Arc<PluginRegistry>>,
    pub auth_plugin_configs: HashMap<String, Value>,
    pub endpoint_path: String,
    pub scope_challenges_enabled: bool,
    pub oauth_client_metadata_document_url: Option<String>,
    pub outbound_timeout_ms: Option<u64>,
    pub outbound_max_response_bytes: Option<u64>,
    pub server_log_payload_max_bytes: u64,
}

impl Default for AuthStateParams {
    fn default() -> Self {
        Self {
            activation: AuthActivation::None,
            bearer_token: None,
            resource: None,
            resource_metadata_url: None,
            providers: Vec::new(),
            plugins: None,
            auth_plugin_configs: HashMap::new(),
            endpoint_path: "/mcp".to_owned(),
            scope_challenges_enabled: true,
            oauth_client_metadata_document_url: None,
            outbound_timeout_ms: None,
            outbound_max_response_bytes: None,
            server_log_payload_max_bytes: 4096,
        }
    }
}

#[doc(hidden)]
pub fn build_auth_state(params: AuthStateParams) -> Arc<AuthState> {
    let AuthStateParams {
        activation,
        bearer_token: _bearer_token,
        resource: _resource,
        resource_metadata_url: _resource_metadata_url,
        providers: _providers,
        plugins: _plugins,
        auth_plugin_configs: _auth_plugin_configs,
        endpoint_path: _endpoint_path,
        scope_challenges_enabled: _scope_challenges_enabled,
        oauth_client_metadata_document_url: _oauth_client_metadata_document_url,
        outbound_timeout_ms: _outbound_timeout_ms,
        outbound_max_response_bytes: _outbound_max_response_bytes,
        server_log_payload_max_bytes: _server_log_payload_max_bytes,
    } = params;
    if activation != AuthActivation::None {
        tracing::warn!(
            "auth feature disabled; build_auth_state coerces enabled auth state to disabled"
        );
    }
    Arc::new(AuthState)
}

#[doc(hidden)]
pub fn build_auth_state_from_config(config: &McpConfig) -> Result<Arc<AuthState>, McpError> {
    build_auth_state_with_plugins(config, None)
}

#[doc(hidden)]
pub fn build_auth_state_with_plugins(
    config: &McpConfig,
    _plugins: Option<Arc<PluginRegistry>>,
) -> Result<Arc<AuthState>, McpError> {
    if config.server.auth_active() {
        return Err(McpError::invalid_request(
            "auth feature disabled but server.auth is active".to_owned(),
            None,
        ));
    }
    Ok(Arc::new(AuthState))
}

#[doc(hidden)]
pub use crate::utils::normalize_endpoint_path;

#[cfg(test)]
mod tests {
    use super::{
        build_auth_state, build_auth_state_from_config, build_auth_state_with_plugins,
        AuthActivation, AuthState, AuthStateParams,
    };
    use crate::config::McpConfig;
    use rmcp::model::ErrorCode;

    fn minimal_config() -> McpConfig {
        serde_yaml::from_str(
            r"
version: 1
server:
  transport:
    mode: stdio
tools:
  items:
    - name: noop
      description: No-op tool
      input_schema:
        type: object
      execute:
        type: http
        upstream: api
        method: GET
        path: /
",
        )
        .expect("config should parse")
    }

    #[test]
    fn auth_state_oauth_enabled_always_false() {
        let state = AuthState;
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_returns_state_for_disabled_activation() {
        let state = build_auth_state(AuthStateParams::default());
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_coerces_bearer_activation() {
        let state = build_auth_state(AuthStateParams {
            activation: AuthActivation::BearerOnly,
            bearer_token: Some("token".to_owned()),
            ..Default::default()
        });
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_coerces_oauth_activation() {
        let state = build_auth_state(AuthStateParams {
            activation: AuthActivation::OauthOnly,
            ..Default::default()
        });
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_coerces_combined_activation() {
        let state = build_auth_state(AuthStateParams {
            activation: AuthActivation::BearerAndOauth,
            ..Default::default()
        });
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_from_config_accepts_disabled_auth() {
        let config = minimal_config();
        let state = build_auth_state_from_config(&config).expect("auth disabled should succeed");
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_returns_state_for_disabled_auth_via_config() {
        let config = minimal_config();
        let state =
            build_auth_state_with_plugins(&config, None).expect("auth disabled should succeed");
        assert!(!state.oauth_enabled());
    }

    #[test]
    fn build_auth_state_with_plugins_rejects_active_auth_config() {
        let mut config = minimal_config();
        let auth = config.server.auth_mut_or_insert();
        auth.providers = vec![crate::config::AuthProviderConfig::bearer("static", "token")];
        auth.enabled = Some(true);
        let error = build_auth_state_with_plugins(&config, None)
            .expect_err("active auth config should fail when auth feature is disabled");
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "auth feature disabled but server.auth is active"
        );
        assert!(error.data.is_none());
    }
}
