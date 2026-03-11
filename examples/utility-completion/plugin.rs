use async_trait::async_trait;
use rust_mcp_core::mcp::{CompleteRequestParams, CompletionInfo, McpError};
use rust_mcp_core::{CompletionPlugin, PluginCallParams};

pub(crate) struct DomainCompletionPlugin;

#[async_trait]
impl CompletionPlugin for DomainCompletionPlugin {
    fn name(&self) -> &'static str {
        "completion.domains"
    }

    async fn complete(
        &self,
        req: &CompleteRequestParams,
        _params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError> {
        let prefix = req.argument.value.as_str();
        let candidates = [
            "example.com",
            "example.org",
            "docs.internal",
            "api.internal",
        ];
        let values = candidates
            .iter()
            .filter(|item| item.starts_with(prefix))
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>();
        CompletionInfo::with_all_values(values).map_err(|err| {
            McpError::internal_error(format!("invalid completion info: {err}"), None)
        })
    }
}
