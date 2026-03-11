//! Config type definitions, organized by domain.
mod auth;
mod client_compat;
mod common;
mod completion;
mod hardening;
mod plugins;
mod prompts;
mod resources;
mod root;
mod tools;
mod transport;
mod upstream;
mod utilities;

pub use auth::{
    AuthBearerProviderConfig, AuthConfig, AuthIntrospectionProviderConfig, AuthJwksProviderConfig,
    AuthOauthConfig, AuthPluginProviderConfig, AuthProviderConfig, IntrospectionClientAuthMethod,
};
pub use client_compat::{ClientCompatConfig, InputSchemaCompatConfig, TopLevelCombinatorsPolicy};
pub use common::{IconConfig, PaginationConfig};
pub use completion::{CompletionConfig, CompletionProviderConfig};
pub use hardening::{ErrorExposureConfig, ResponseLimitsConfig};
pub use plugins::{HttpRouterTargetType, PluginConfig, PluginTargetConfig};
pub use prompts::{
    PromptItemConfig, PromptMessageRoleConfig, PromptProviderConfig, PromptTemplateConfig,
    PromptTemplateMessageConfig, PromptsConfig,
};
pub use resources::{
    ResourceAnnotationsConfig, ResourceAudienceConfig, ResourceContentConfig, ResourceItemConfig,
    ResourceProviderConfig, ResourceTemplateConfig, ResourcesConfig,
};
pub use root::{McpConfig, ServerInfoConfig, ServerLoggingConfig, ServerSection};
pub use tools::{
    ContentFallback, ExecuteConfig, ExecuteHttpConfig, ExecutePluginConfig, ExecuteType,
    ResponseConfig, ResponseContentConfig, ResponseStructuredConfig, ToolConfig, ToolsConfig,
};
pub use transport::{
    ProtocolVersionNegotiationConfig, ProtocolVersionNegotiationMode,
    StreamableHttpHardeningConfig, StreamableHttpPerIpRateBucketConfig,
    StreamableHttpRateBucketConfig, StreamableHttpRateLimitConfig,
    StreamableHttpRateLimitKeySource, StreamableHttpSessionHardeningConfig,
    StreamableHttpSessionMode, StreamableHttpTransportConfig, TransportConfig, TransportMode,
};
pub use upstream::{
    OutboundHttpConfig, OutboundRetryConfig, SecretValueConfig, SecretValueSource, UpstreamAuth,
    UpstreamConfig, UpstreamOauth2AuthConfig, UpstreamOauth2ClientAuthMethod,
    UpstreamOauth2GrantType, UpstreamOauth2MtlsConfig, UpstreamOauth2RefreshConfig,
};
pub use utilities::{
    ClientElicitationConfig, ClientFeaturesConfig, ClientLoggingConfig, ClientRootsConfig,
    ClientSamplingConfig, ElicitationMode, ProgressConfig, TaskCapabilities, TasksConfig,
};

pub use tools::TaskSupport;
