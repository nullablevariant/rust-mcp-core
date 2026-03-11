//! Handler for server info and capability advertisement during initialize.
use rmcp::{
    model::{Implementation, InitializeResult, ServerCapabilities, ServerInfo, Tool},
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};

#[cfg(feature = "tasks_utility")]
use rmcp::model::{
    TaskRequestsCapability, TasksCapability as ModelTasksCapability, ToolsTaskCapability,
};

#[cfg(feature = "tasks_utility")]
use crate::config::TaskSupport;

use super::super::{orchestration::Engine, tool_builders::map_icons};

impl Engine {
    pub(super) async fn handle_initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        self.observe_request_peer(&context).await;
        Ok(self.build_server_info())
    }

    pub(super) fn handle_get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name.as_ref() == name).cloned()
    }

    // Assembles initialize ServerInfo from capabilities + implementation data,
    // then applies optional instructions from server.info.instructions.
    pub(super) fn build_server_info(&self) -> ServerInfo {
        let capabilities = self.build_server_capabilities();
        let server_info_impl = self.build_server_implementation();
        let mut server_info =
            InitializeResult::new(capabilities).with_server_info(server_info_impl);
        server_info.instructions = self
            .config
            .server
            .info
            .as_ref()
            .and_then(|info| info.instructions.clone());
        server_info
    }

    // Builds initialize capability advertisement from effective config state.
    // Capabilities behind feature flags are conditionally included; tools is
    // always advertised with options derived from tools config.
    fn build_server_capabilities(&self) -> ServerCapabilities {
        let prompts_capability = Self::build_prompts_capability(self);
        let resources_capability = Self::build_resources_capability(self);
        let logging_capability = Self::build_logging_capability(self);
        let completion_capability = Self::build_completion_capability(self);
        let tools_capability = self.build_tools_capability();
        let tasks_capability = Self::build_tasks_capability(self);
        let mut capabilities = ServerCapabilities::default();
        capabilities.tools = Some(tools_capability);
        capabilities.logging = logging_capability;
        capabilities.completions = completion_capability;
        capabilities.prompts = prompts_capability;
        capabilities.resources = resources_capability;
        capabilities.tasks = tasks_capability;
        capabilities
    }

    #[cfg(feature = "prompts")]
    fn build_prompts_capability(&self) -> Option<rmcp::model::PromptsCapability> {
        if !self.config.prompts_active() {
            return None;
        }
        let prompts = self.config.prompts.as_ref().expect("active prompts config");
        let mut capability = rmcp::model::PromptsCapability::default();
        if prompts.notify_list_changed {
            capability.list_changed = Some(true);
        }
        Some(capability)
    }

    #[cfg(not(feature = "prompts"))]
    const fn build_prompts_capability(_self: &Self) -> Option<rmcp::model::PromptsCapability> {
        None
    }

    #[cfg(feature = "resources")]
    fn build_resources_capability(&self) -> Option<rmcp::model::ResourcesCapability> {
        if !self.config.resources_active() {
            return None;
        }
        let resources = self
            .config
            .resources
            .as_ref()
            .expect("active resources config");
        let mut capability = rmcp::model::ResourcesCapability::default();
        if resources.notify_list_changed {
            capability.list_changed = Some(true);
        }
        if resources.clients_can_subscribe {
            capability.subscribe = Some(true);
        }
        Some(capability)
    }

    #[cfg(not(feature = "resources"))]
    const fn build_resources_capability(_self: &Self) -> Option<rmcp::model::ResourcesCapability> {
        None
    }

    #[cfg(feature = "client_logging")]
    fn build_logging_capability(&self) -> Option<rmcp::model::JsonObject> {
        self.client_logging
            .enabled()
            .then_some(rmcp::model::JsonObject::default())
    }

    #[cfg(not(feature = "client_logging"))]
    const fn build_logging_capability(_self: &Self) -> Option<rmcp::model::JsonObject> {
        None
    }

    #[cfg(feature = "completion")]
    fn build_completion_capability(&self) -> Option<rmcp::model::JsonObject> {
        self.config
            .completion_active()
            .then_some(rmcp::model::JsonObject::default())
    }

    #[cfg(not(feature = "completion"))]
    const fn build_completion_capability(_self: &Self) -> Option<rmcp::model::JsonObject> {
        None
    }

    fn build_tools_capability(&self) -> rmcp::model::ToolsCapability {
        let mut capability = rmcp::model::ToolsCapability::default();
        if self.config.tools_notify_list_changed() {
            capability.list_changed = Some(true);
        }
        capability
    }

    // Builds the tasks capability section when the tasks_utility feature is
    // enabled and tasks config is active.
    #[cfg(feature = "tasks_utility")]
    fn build_tasks_capability(&self) -> Option<rmcp::model::TasksCapability> {
        if self.config.tasks_active() {
            let any_task_tool = self
                .tool_map
                .values()
                .any(|tool| tool.execute.task_support() != TaskSupport::Forbidden);
            let tasks = self.config.tasks.as_ref().expect("active tasks config");
            let requests = any_task_tool.then_some(TaskRequestsCapability {
                tools: Some(ToolsTaskCapability {
                    call: Some(rmcp::model::JsonObject::new()),
                }),
                ..Default::default()
            });
            return Some(ModelTasksCapability {
                requests,
                list: tasks
                    .capabilities
                    .list
                    .then_some(rmcp::model::JsonObject::new()),
                cancel: tasks
                    .capabilities
                    .cancel
                    .then_some(rmcp::model::JsonObject::new()),
            });
        }
        None
    }

    #[cfg(not(feature = "tasks_utility"))]
    const fn build_tasks_capability(_self: &Self) -> Option<rmcp::model::TasksCapability> {
        None
    }

    // Builds the implementation payload for initialize, using server.info
    // values when set and build metadata defaults for unset fields.
    fn build_server_implementation(&self) -> Implementation {
        let build_default = Implementation::from_build_env();
        match &self.config.server.info {
            Some(info) => {
                let mut implementation = Implementation::new(
                    info.name
                        .clone()
                        .unwrap_or_else(|| build_default.name.clone()),
                    info.version
                        .clone()
                        .unwrap_or_else(|| build_default.version.clone()),
                );
                implementation.title.clone_from(&info.title);
                implementation.description.clone_from(&info.description);
                implementation.website_url.clone_from(&info.website_url);
                implementation.icons = map_icons(info.icons.as_deref());
                implementation
            }
            None => build_default,
        }
    }
}
