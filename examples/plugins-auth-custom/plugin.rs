use async_trait::async_trait;
use rust_mcp_core::{AuthPlugin, AuthPluginDecision, AuthPluginValidateParams};

pub(crate) struct CustomValidatorPlugin;
pub(crate) struct ProviderValidatorPlugin;

#[async_trait]
impl AuthPlugin for CustomValidatorPlugin {
    fn name(&self) -> &'static str {
        "auth.custom_validator"
    }

    async fn validate(&self, params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        let token = params.token;
        if token.is_empty() {
            AuthPluginDecision::Reject
        } else {
            AuthPluginDecision::Accept
        }
    }
}

#[async_trait]
impl AuthPlugin for ProviderValidatorPlugin {
    fn name(&self) -> &'static str {
        "auth.provider_validator"
    }

    async fn validate(&self, params: AuthPluginValidateParams<'_>) -> AuthPluginDecision {
        let token = params.token;
        if token.is_empty() {
            AuthPluginDecision::Reject
        } else {
            AuthPluginDecision::Accept
        }
    }
}
