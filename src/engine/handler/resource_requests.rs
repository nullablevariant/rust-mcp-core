//! Handler implementations for resources/list, resources/read, and subscribe/unsubscribe.
use rmcp::model::ResourceContents;
use rmcp::{
    model::{
        ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, SubscribeRequestParams,
        UnsubscribeRequestParams,
    },
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde_json::json;

use super::super::orchestration::Engine;
use super::super::orchestration::ResourcePluginCallParams;
use super::super::resources::ResourceSource;
use crate::errors::cancelled_error;

enum ResourceMatch {
    InlineResource {
        mime_type: Option<String>,
        content: Option<crate::config::ResourceContentConfig>,
    },
    InlineTemplate,
    Plugin {
        plugin_name: String,
        config: serde_json::Value,
    },
}

impl Engine {
    pub(super) async fn handle_list_resources_request(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let request_cursor = request.and_then(|params| params.cursor);
        let _ = context;
        let resources = self.resource_catalog().await?.resources_payload();
        if let Some(page_size) = self.resource_page_size() {
            let (resources, next_cursor) =
                crate::engine::pagination::paginate_items(&resources, request_cursor, page_size)?;
            Ok(ListResourcesResult {
                resources,
                next_cursor,
                ..Default::default()
            })
        } else {
            Ok(ListResourcesResult {
                resources,
                ..Default::default()
            })
        }
    }

    pub(super) async fn handle_list_resource_templates_request(
        &self,
        request: Option<PaginatedRequestParams>,
        context: &RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let request_cursor = request.and_then(|params| params.cursor);
        let _ = context;
        let templates = self.resource_catalog().await?.templates_payload();
        if let Some(page_size) = self.resource_page_size() {
            let (resource_templates, next_cursor) =
                crate::engine::pagination::paginate_items(&templates, request_cursor, page_size)?;
            Ok(ListResourceTemplatesResult {
                resource_templates,
                next_cursor,
                ..Default::default()
            })
        } else {
            Ok(ListResourceTemplatesResult {
                resource_templates: templates,
                ..Default::default()
            })
        }
    }

    pub(super) async fn handle_read_resource_request(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        let uri = request.uri;
        match self.resolve_resource_match(&uri).await? {
            ResourceMatch::InlineResource { mime_type, content } => {
                Self::read_inline_resource(uri, mime_type, content)
            }
            ResourceMatch::InlineTemplate => Err(resource_not_found(&uri)),
            ResourceMatch::Plugin {
                plugin_name,
                config,
            } => {
                self.call_resource_plugin_read(ResourcePluginCallParams {
                    plugin_name: &plugin_name,
                    config: &config,
                    uri: &uri,
                    request: Some(context),
                })
                .await
            }
        }
    }

    pub(super) async fn handle_subscribe_request(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.resource_subscribe_enabled() {
            return Err(McpError::method_not_found::<
                rmcp::model::SubscribeRequestMethod,
            >());
        }
        let uri = request.uri;
        self.apply_resource_subscription(uri, context, SubscriptionAction::Subscribe)
            .await
    }

    pub(super) async fn handle_unsubscribe_request(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.resource_subscribe_enabled() {
            return Err(McpError::method_not_found::<
                rmcp::model::UnsubscribeRequestMethod,
            >());
        }
        let uri = request.uri;
        self.apply_resource_subscription(uri, context, SubscriptionAction::Unsubscribe)
            .await
    }

    #[cfg(feature = "resources")]
    async fn apply_resource_subscription(
        &self,
        uri: String,
        context: RequestContext<RoleServer>,
        action: SubscriptionAction,
    ) -> Result<(), McpError> {
        match self.resolve_resource_match(&uri).await? {
            ResourceMatch::InlineResource { .. } | ResourceMatch::InlineTemplate => Ok(()),
            ResourceMatch::Plugin {
                plugin_name,
                config,
            } => match action {
                SubscriptionAction::Subscribe => {
                    self.call_resource_plugin_subscribe(ResourcePluginCallParams {
                        plugin_name: &plugin_name,
                        config: &config,
                        uri: &uri,
                        request: Some(context.clone()),
                    })
                    .await
                }
                SubscriptionAction::Unsubscribe => {
                    self.call_resource_plugin_unsubscribe(ResourcePluginCallParams {
                        plugin_name: &plugin_name,
                        config: &config,
                        uri: &uri,
                        request: Some(context.clone()),
                    })
                    .await
                }
            },
        }
    }

    #[cfg(feature = "resources")]
    async fn resolve_resource_match(&self, uri: &str) -> Result<ResourceMatch, McpError> {
        let catalog = self.resource_catalog().await?;
        if let Some(selected) = catalog.resource_by_uri(uri).cloned() {
            return Ok(match selected.source {
                ResourceSource::Inline { content } => ResourceMatch::InlineResource {
                    mime_type: selected.resource.mime_type.clone(),
                    content,
                },
                ResourceSource::Plugin {
                    plugin_name,
                    config,
                } => ResourceMatch::Plugin {
                    plugin_name,
                    config,
                },
            });
        }

        for template in catalog.templates().iter().rev().cloned() {
            let Some(args) =
                crate::engine::resources::match_uri_template(&template.template.uri_template, uri)
            else {
                continue;
            };
            crate::engine::schema_argument_validation::validate_schema_args(
                &crate::engine::schema_argument_validation::ValidateSchemaArgsParams {
                    schema: &template.arguments_schema,
                    args: &args,
                    entity_label: "resource template",
                    entity_name: &template.template.uri_template,
                    schema_validator_cache: &self.schema_validator_cache,
                },
            )?;
            return Ok(match template.source {
                ResourceSource::Inline { .. } => ResourceMatch::InlineTemplate,
                ResourceSource::Plugin {
                    plugin_name,
                    config,
                } => ResourceMatch::Plugin {
                    plugin_name,
                    config,
                },
            });
        }

        Err(resource_not_found(uri))
    }

    #[cfg(feature = "resources")]
    fn resource_subscribe_enabled(&self) -> bool {
        self.config
            .resources
            .as_ref()
            .is_some_and(|resources| resources.clients_can_subscribe)
    }

    #[cfg(feature = "resources")]
    fn read_inline_resource(
        uri: String,
        mime_type: Option<String>,
        content: Option<crate::config::ResourceContentConfig>,
    ) -> Result<ReadResourceResult, McpError> {
        let Some(content) = content else {
            return Err(resource_not_found(&uri));
        };

        if let Some(text) = content.text {
            Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type,
                    text,
                    meta: None,
                },
            ]))
        } else if let Some(blob) = content.blob {
            Ok(ReadResourceResult::new(vec![
                ResourceContents::BlobResourceContents {
                    uri,
                    mime_type,
                    blob,
                    meta: None,
                },
            ]))
        } else {
            Err(resource_not_found(&uri))
        }
    }
}

#[cfg(feature = "resources")]
#[derive(Clone, Copy)]
enum SubscriptionAction {
    Subscribe,
    Unsubscribe,
}

#[cfg(feature = "resources")]
fn resource_not_found(uri: &str) -> McpError {
    McpError::resource_not_found("Resource not found", Some(json!({ "uri": uri })))
}
