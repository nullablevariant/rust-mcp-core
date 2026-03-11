//! ServerHandler trait implementation wiring all MCP request handlers.
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, CancelTaskResult, CompleteRequestParams,
        CompleteResult, CreateTaskResult, GetPromptRequestParams, GetPromptResult,
        GetTaskInfoParams, GetTaskPayloadResult, GetTaskResult, GetTaskResultParams,
        InitializeResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
        ListTasksResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
        ReadResourceResult, ServerInfo, SetLevelRequestParams, SubscribeRequestParams, Tool,
        UnsubscribeRequestParams,
    },
    service::{NotificationContext, RequestContext, RoleServer},
    ErrorData as McpError, ServerHandler,
};

mod common;
#[cfg(feature = "prompts")]
mod prompt_requests;
#[cfg(feature = "resources")]
mod resource_requests;
mod server_info;
#[cfg(feature = "tasks_utility")]
mod task_requests;
mod tool_requests;
#[cfg(any(
    feature = "completion",
    feature = "client_logging",
    feature = "client_features",
))]
mod utility_requests;

use super::orchestration::Engine;

impl ServerHandler for Engine {
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        self.handle_initialize(request, context).await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.handle_get_tool(name)
    }

    fn get_info(&self) -> ServerInfo {
        self.build_server_info()
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        #[cfg(all(
            feature = "completion",
            any(feature = "prompts", feature = "resources")
        ))]
        {
            self.handle_complete_request(request, context).await
        }
        #[cfg(all(
            feature = "completion",
            not(any(feature = "prompts", feature = "resources"))
        ))]
        {
            self.handle_complete_request(request, context)
        }
        #[cfg(not(feature = "completion"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::CompleteRequestMethod,
            >())
        }
    }

    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        #[cfg(feature = "client_logging")]
        {
            self.handle_set_level_request(&request, &context)
        }
        #[cfg(not(feature = "client_logging"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::SetLevelRequestMethod,
            >())
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.handle_list_tools_request(request, &context)
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        #[cfg(feature = "prompts")]
        {
            self.handle_list_prompts_request(request, context).await
        }
        #[cfg(not(feature = "prompts"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::ListPromptsRequestMethod,
            >())
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        #[cfg(feature = "resources")]
        {
            self.handle_list_resources_request(request, context).await
        }
        #[cfg(not(feature = "resources"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::ListResourcesRequestMethod,
            >())
        }
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        #[cfg(feature = "resources")]
        {
            self.handle_list_resource_templates_request(request, &context)
                .await
        }
        #[cfg(not(feature = "resources"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::ListResourceTemplatesRequestMethod,
            >())
        }
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        #[cfg(feature = "resources")]
        {
            self.handle_read_resource_request(request, context).await
        }
        #[cfg(not(feature = "resources"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::ReadResourceRequestMethod,
            >())
        }
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        #[cfg(feature = "resources")]
        {
            self.handle_subscribe_request(request, context).await
        }
        #[cfg(not(feature = "resources"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::SubscribeRequestMethod,
            >())
        }
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        #[cfg(feature = "resources")]
        {
            self.handle_unsubscribe_request(request, context).await
        }
        #[cfg(not(feature = "resources"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::UnsubscribeRequestMethod,
            >())
        }
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        #[cfg(feature = "prompts")]
        {
            self.handle_get_prompt_request(request, context).await
        }
        #[cfg(not(feature = "prompts"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::GetPromptRequestMethod,
            >())
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.handle_call_tool_request(request, context).await
    }

    async fn enqueue_task(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CreateTaskResult, McpError> {
        #[cfg(feature = "tasks_utility")]
        {
            self.handle_enqueue_task_request(request, context).await
        }
        #[cfg(not(feature = "tasks_utility"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >())
        }
    }

    async fn list_tasks(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListTasksResult, McpError> {
        #[cfg(feature = "tasks_utility")]
        {
            self.handle_list_tasks_request(request, context).await
        }
        #[cfg(not(feature = "tasks_utility"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<rmcp::model::ListTasksMethod>())
        }
    }

    async fn get_task_info(
        &self,
        request: GetTaskInfoParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        #[cfg(feature = "tasks_utility")]
        {
            self.handle_get_task_info_request(request, context).await
        }
        #[cfg(not(feature = "tasks_utility"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<rmcp::model::GetTaskInfoMethod>())
        }
    }

    async fn get_task_result(
        &self,
        request: GetTaskResultParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskPayloadResult, McpError> {
        #[cfg(feature = "tasks_utility")]
        {
            self.handle_get_task_result_request(request, context).await
        }
        #[cfg(not(feature = "tasks_utility"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<rmcp::model::GetTaskResultMethod>())
        }
    }

    async fn cancel_task(
        &self,
        request: rmcp::model::CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CancelTaskResult, McpError> {
        #[cfg(feature = "tasks_utility")]
        {
            self.handle_cancel_task_request(request, context).await
        }
        #[cfg(not(feature = "tasks_utility"))]
        {
            let _ = (request, context);
            Err(McpError::method_not_found::<rmcp::model::CancelTaskMethod>())
        }
    }

    async fn on_roots_list_changed(&self, context: NotificationContext<RoleServer>) {
        #[cfg(feature = "client_features")]
        self.handle_roots_list_changed(context);
        #[cfg(not(feature = "client_features"))]
        let _ = context;
    }
}
