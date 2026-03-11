use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use rust_mcp_core::mcp::{
    AnnotateAble, McpError, RawResource, ReadResourceResult, ResourceContents,
};
use rust_mcp_core::{PluginCallParams, ResourceEntry, ResourcePlugin};
use tokio::sync::Mutex;

pub(crate) struct SubscribableResourcePlugin {
    subscriptions: Arc<Mutex<HashSet<String>>>,
}

impl SubscribableResourcePlugin {
    pub(crate) fn new() -> Self {
        Self {
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[async_trait]
impl ResourcePlugin for SubscribableResourcePlugin {
    fn name(&self) -> &'static str {
        "resource.subscribable"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        Ok(vec![ResourceEntry {
            resource: RawResource::new("resource://events/feed", "events-feed").no_annotation(),
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
                mime_type: Some("application/json".to_owned()),
                text: "{\"status\":\"ok\"}".to_owned(),
                meta: None,
            },
        ]))
    }

    async fn subscribe(&self, uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        self.subscriptions.lock().await.insert(uri.to_owned());
        Ok(())
    }

    async fn unsubscribe(&self, uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        self.subscriptions.lock().await.remove(uri);
        Ok(())
    }
}
