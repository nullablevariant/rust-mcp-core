//! Error helpers for MCP request cancellation.

use rmcp::{
    model::{CallToolResult, Content, ErrorCode},
    ErrorData as McpError,
};

// JSON-RPC error code used for cancelled requests (`-32000`).
#[doc(hidden)]
pub const CANCELLED_ERROR_CODE: ErrorCode = ErrorCode(-32000);

// Human-readable message for cancelled requests.
#[doc(hidden)]
pub const CANCELLED_ERROR_MESSAGE: &str = "request cancelled";

// Build an [`McpError`] representing a cancelled request.
//
// Uses [`CANCELLED_ERROR_CODE`] and [`CANCELLED_ERROR_MESSAGE`].
#[doc(hidden)]
pub fn cancelled_error() -> McpError {
    McpError::new(CANCELLED_ERROR_CODE, CANCELLED_ERROR_MESSAGE, None)
}

// Build a [`CallToolResult`] with `is_error=true` for a cancelled tool call.
//
// Returns a text content entry with the cancellation message.
#[doc(hidden)]
pub fn cancelled_tool_result() -> CallToolResult {
    CallToolResult::error(vec![Content::text(CANCELLED_ERROR_MESSAGE)])
}

#[doc(hidden)]
pub fn tool_execution_error_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.into())])
}
