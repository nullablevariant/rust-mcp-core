//! Plugin traits defining the extension points for tool, auth, prompt, resource, and completion plugins.

use std::collections::HashMap;

use async_trait::async_trait;
use http::HeaderMap;
use serde_json::Value;

use super::PluginContext;
use crate::{
    mcp::{
        CallToolResult, CompleteRequestParams, CompletionInfo, GetPromptResult, Prompt,
        ReadResourceResult, Resource, ResourceTemplate,
    },
    McpError,
};

/// Bundled parameters for plugin trait method calls.
///
/// Groups the plugin-specific configuration and the request context together
/// so trait methods stay within the 4-parameter limit while remaining
/// extensible (new context fields can be added to [`PluginContext`] without
/// changing trait signatures).
#[derive(Debug)]
pub struct PluginCallParams {
    /// Plugin-specific config from `plugins[].config`, merged with global defaults.
    pub config: Value,
    /// Request context providing cancellation, progress, logging, and HTTP helpers.
    pub ctx: PluginContext,
}

/// Bundled parameters for [`AuthPlugin::validate`].
///
/// Groups the token, claims, headers, and plugin config into a single struct
/// so the validate method stays within the 4-parameter limit.
#[derive(Debug)]
pub struct AuthPluginValidateParams<'a> {
    /// The raw bearer token string from the request.
    pub token: &'a str,
    /// Decoded JWT claims or introspection response as JSON.
    pub claims: &'a Value,
    /// The full HTTP request headers.
    pub headers: &'a HeaderMap,
    /// Plugin-specific config from `plugins[].config`.
    pub config: &'a Value,
}

/// Decision returned by [`AuthPlugin::validate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthPluginDecision {
    /// Token is accepted and authentication succeeds.
    Accept,
    /// Token is rejected and authentication fails immediately.
    Reject,
    /// Plugin does not make a decision; continue provider evaluation.
    Abstain,
}

/// Custom tool execution plugin.
///
/// Implement this trait to provide tool behavior that goes beyond config-driven
/// HTTP calls. The plugin must be both registered in [`PluginRegistry`] and
/// declared in the config `plugins[]` array.
///
/// # Arguments
///
/// `call` receives:
/// - `args` — the tool arguments from the client request
/// - `params` — bundled plugin config and [`PluginContext`] with cancellation,
///   progress, logging, and HTTP helpers
///
/// # Errors
///
/// Returns `McpError` on execution failure. The engine converts this to an
/// error tool result for the client.
///
/// # Examples
///
/// ```rust
/// use rust_mcp_core::{
///     mcp::{CallToolResult, Content, McpError},
///     PluginCallParams, ToolPlugin,
/// };
///
/// struct EchoPlugin;
///
/// #[async_trait::async_trait]
/// impl ToolPlugin for EchoPlugin {
///     fn name(&self) -> &str { "echo" }
///
///     async fn call(
///         &self,
///         args: serde_json::Value,
///         params: PluginCallParams,
///     ) -> Result<CallToolResult, McpError> {
///         Ok(CallToolResult::success(vec![
///             Content::text(args.to_string()),
///         ]))
///     }
/// }
///
/// # let plugin = EchoPlugin;
/// # assert_eq!(plugin.name(), "echo");
/// ```
///
/// [`PluginRegistry`]: crate::PluginRegistry
/// [`PluginContext`]: crate::PluginContext
#[async_trait]
pub trait ToolPlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// Execute the tool with the given arguments and plugin context.
    async fn call(&self, args: Value, params: PluginCallParams)
        -> Result<CallToolResult, McpError>;
}

/// Custom auth validation plugin.
///
/// Used by `type: plugin` auth providers.
///
/// # Arguments
///
/// `validate` receives an [`AuthPluginValidateParams`] bundling the token,
/// claims, headers, and plugin config.
#[async_trait]
pub trait AuthPlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// Validate the token and return an auth decision.
    async fn validate(&self, params: AuthPluginValidateParams<'_>) -> AuthPluginDecision;
}

/// Custom completion provider plugin.
///
/// Provides autocompletion values for prompt or resource template arguments.
/// Referenced from `completion.providers[].plugin` in config.
///
/// # Errors
///
/// Returns `McpError` if the completion provider cannot be queried.
#[async_trait]
pub trait CompletionPlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// Return completion candidates for the given request.
    async fn complete(
        &self,
        req: &CompleteRequestParams,
        params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError>;
}

/// A prompt definition returned by a [`PromptPlugin`].
#[derive(Clone, Debug)]
pub struct PromptEntry {
    /// The MCP prompt model with name, description, and argument definitions.
    pub prompt: Prompt,
    /// JSON Schema for validating prompt arguments.
    pub arguments_schema: Value,
    /// Optional completion provider mappings (argument name to provider name).
    pub completions: Option<HashMap<String, String>>,
}

/// Custom prompt provider plugin.
///
/// Supplies prompt definitions and renders prompt messages from arguments.
/// Referenced from `prompts.providers[].plugin` in config.
///
/// # Errors
///
/// Returns `McpError` if prompts cannot be listed or rendered.
#[async_trait]
pub trait PromptPlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// List all prompts this plugin provides.
    async fn list(&self, params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError>;

    /// Render a prompt by name with the given arguments.
    async fn get(
        &self,
        name: &str,
        args: Value,
        params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError>;
}

/// A resource definition returned by a [`ResourcePlugin`].
#[derive(Clone, Debug)]
pub struct ResourceEntry {
    /// The MCP resource model with URI, name, and MIME type.
    pub resource: Resource,
}

/// A resource template definition returned by a [`ResourcePlugin`].
#[derive(Clone, Debug)]
pub struct ResourceTemplateEntry {
    /// MCP resource template with URI template and metadata.
    pub template: ResourceTemplate,
    /// JSON Schema for validating template arguments.
    pub arguments_schema: Value,
    /// Optional completion provider mappings (argument name to provider name).
    pub completions: Option<HashMap<String, String>>,
}

/// Custom resource provider plugin.
///
/// Supplies resource definitions, reads resource content, and handles
/// subscribe/unsubscribe for resource change notifications.
/// Referenced from `resources.providers[].plugin` in config.
///
/// # Errors
///
/// Returns `McpError` if resources cannot be listed, read, or subscribed to.
#[async_trait]
pub trait ResourcePlugin: Send + Sync {
    /// Unique plugin name. Must match the `plugins[].name` entry in config.
    fn name(&self) -> &str;

    /// List all resources this plugin provides.
    async fn list(&self, params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError>;

    /// Read the content of a resource by URI.
    async fn read(
        &self,
        uri: &str,
        params: PluginCallParams,
    ) -> Result<ReadResourceResult, McpError>;

    /// Subscribe to change notifications for a resource URI.
    async fn subscribe(&self, uri: &str, params: PluginCallParams) -> Result<(), McpError>;

    /// Unsubscribe from change notifications for a resource URI.
    async fn unsubscribe(&self, uri: &str, params: PluginCallParams) -> Result<(), McpError>;
}

/// Identifies which list capability a refresh applies to.
///
/// Used with [`PluginContext::request_list_refresh`] to trigger a
/// `notifications/list_changed` notification for the specified feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListFeature {
    /// Tools list changed.
    Tools,
    /// Prompts list changed.
    Prompts,
    /// Resources list changed.
    Resources,
}

#[async_trait]
#[doc(hidden)]
pub trait ListRefreshHandle: Send + Sync {
    async fn refresh_list(&self, feature: ListFeature) -> Result<bool, McpError>;
}
