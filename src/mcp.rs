//! Crate-owned facade for RMCP model and error types.
//!
//! Consumers can import MCP types from this module instead of depending on
//! `rmcp` directly.

pub use rmcp::model::{
    AnnotateAble, CallToolResult, ClientCapabilities, CompleteRequestParams, CompletionInfo,
    Content, ErrorCode, GetPromptResult, Implementation, InitializeRequestParams, LoggingLevel,
    ProgressToken, Prompt, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole,
    ProtocolVersion, RawResource, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    TaskSupport,
};
pub use rmcp::ErrorData as McpError;

#[cfg(feature = "client_features")]
#[cfg_attr(docsrs, doc(cfg(feature = "client_features")))]
pub use rmcp::model::{
    CreateElicitationRequestParams, CreateElicitationResult, CreateMessageRequestParams,
    CreateMessageResult, ElicitationAction, ElicitationSchema, ListRootsResult, RawTextContent,
    Role, SamplingContent, SamplingMessage, SamplingMessageContent,
};
