//! Handler implementations for tools/list and tools/call.
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams},
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};

use super::super::{orchestration::Engine, pagination::paginate_items};
use super::common::global_page_size;
use crate::errors::cancelled_error;

impl Engine {
    pub(super) fn handle_list_tools_request(
        &self,
        request: Option<PaginatedRequestParams>,
        context: &RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let request_cursor = request.and_then(|params| params.cursor);
        if let Some(page_size) = global_page_size(&self.config) {
            let (tools, next_cursor) = paginate_items(&self.tools, request_cursor, page_size)?;
            Ok(ListToolsResult {
                tools,
                next_cursor,
                ..Default::default()
            })
        } else {
            Ok(ListToolsResult {
                tools: self.list_tools(),
                ..Default::default()
            })
        }
    }

    pub(super) async fn handle_call_tool_request(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        self.execute_tool_with_context(&name, args, Some(context))
            .await
    }
}
