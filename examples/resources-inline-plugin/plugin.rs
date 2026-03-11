use async_trait::async_trait;
use rust_mcp_core::mcp::{
    AnnotateAble, McpError, RawResource, ReadResourceResult, ResourceContents,
};
use rust_mcp_core::{PluginCallParams, ResourceEntry, ResourcePlugin};

pub(crate) struct DocsResourcePlugin;

#[async_trait]
impl ResourcePlugin for DocsResourcePlugin {
    fn name(&self) -> &'static str {
        "resource.plugin"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        Ok(vec![ResourceEntry {
            resource: RawResource::new("resource://repo/readme.md", "readme").no_annotation(),
        }])
    }

    async fn read(
        &self,
        uri: &str,
        _params: PluginCallParams,
    ) -> Result<ReadResourceResult, McpError> {
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: uri.to_owned(),
                mime_type: Some("text/markdown".to_owned()),
                text: "# README".to_owned(),
                meta: None,
            },
        ]))
    }

    async fn subscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }

    async fn unsubscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }
}
