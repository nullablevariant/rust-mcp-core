//! Resource dispatch from engine to inline providers or resource plugins.
use std::sync::Arc;

use rmcp::{
    model::{RawResource, RawResourceTemplate, ReadResourceResult, Resource, ResourceTemplate},
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde_json::Value;

use crate::{
    config::ResourceProviderConfig,
    engine::{
        resources::{
            validate_resource_template_schema, validate_resource_template_uri,
            validate_resource_uri, ResolvedResource, ResolvedResourceTemplate, ResourceCatalog,
            ResourceSource,
        },
        tool_builders::map_icons,
        value_helpers::merge_plugin_config,
    },
    plugins::{PluginCallParams, PluginContext, PluginType, ResourcePlugin},
};

use super::{helpers, Engine};

// Parameters for calling a resource plugin operation (read, subscribe, or unsubscribe).
pub(crate) struct ResourcePluginCallParams<'a> {
    pub(crate) plugin_name: &'a str,
    pub(crate) config: &'a Value,
    pub(crate) uri: &'a str,
    pub(crate) request: Option<RequestContext<RoleServer>>,
}

impl Engine {
    pub(crate) fn resource_page_size(&self) -> Option<usize> {
        helpers::page_size(
            self.config
                .resources
                .as_ref()
                .and_then(|resources| resources.pagination.as_ref()),
            self.config.as_ref(),
        )
    }

    async fn build_resources(&self) -> Result<Vec<ResolvedResource>, McpError> {
        if !self.config.resources_active() {
            return Ok(Vec::new());
        }
        let resources_config = self
            .config
            .resources
            .as_ref()
            .expect("active resources config");

        let mut resources = Vec::new();
        for provider in &resources_config.providers {
            match provider {
                ResourceProviderConfig::Inline { items, .. } => {
                    let Some(items) = items.as_ref() else {
                        continue;
                    };
                    for item in items {
                        validate_resource_uri(&item.uri)?;
                        let mut raw = RawResource::new(item.uri.clone(), item.name.clone());
                        raw.title.clone_from(&item.title);
                        raw.description.clone_from(&item.description);
                        raw.mime_type.clone_from(&item.mime_type);
                        raw.size = item.size;
                        raw.icons = map_icons(item.icons.as_deref());
                        let annotations = crate::engine::resources::map_resource_annotations(
                            item.annotations.as_ref(),
                        )?;
                        resources.push(ResolvedResource {
                            resource: rmcp::model::AnnotateAble::optional_annotate(
                                raw,
                                annotations,
                            ),
                            source: ResourceSource::Inline {
                                content: item.content.clone(),
                            },
                        });
                    }
                }
                ResourceProviderConfig::Plugin {
                    plugin,
                    config: provider_config,
                    ..
                } => {
                    let resource_plugin = helpers::lookup_resource_plugin(&self.plugins, plugin)?;
                    let merged_config = merge_plugin_config(
                        &self.config,
                        PluginType::Resource,
                        plugin,
                        provider_config.as_ref(),
                    );
                    let entries = resource_plugin
                        .list(PluginCallParams {
                            config: merged_config.clone(),
                            ctx: self.build_plugin_context(None),
                        })
                        .await?;
                    for entry in entries {
                        validate_resource_uri(&entry.resource.uri)?;
                        resources.push(ResolvedResource {
                            resource: entry.resource,
                            source: ResourceSource::Plugin {
                                plugin_name: plugin.clone(),
                                config: merged_config.clone(),
                            },
                        });
                    }
                }
            }
        }
        Ok(resources)
    }

    fn build_resource_templates(&self) -> Result<Vec<ResolvedResourceTemplate>, McpError> {
        if !self.config.resources_active() {
            return Ok(Vec::new());
        }
        let resources_config = self
            .config
            .resources
            .as_ref()
            .expect("active resources config");

        let mut templates = Vec::new();
        for provider in &resources_config.providers {
            let (source, provider_templates) = match provider {
                ResourceProviderConfig::Inline { templates, .. } => {
                    (ResourceSource::Inline { content: None }, templates.as_ref())
                }
                ResourceProviderConfig::Plugin {
                    plugin,
                    config: provider_config,
                    templates,
                } => (
                    ResourceSource::Plugin {
                        plugin_name: plugin.clone(),
                        config: merge_plugin_config(
                            &self.config,
                            PluginType::Resource,
                            plugin,
                            provider_config.as_ref(),
                        ),
                    },
                    templates.as_ref(),
                ),
            };
            let Some(provider_templates) = provider_templates else {
                continue;
            };

            for template in provider_templates {
                validate_resource_template_uri(&template.uri_template)?;
                validate_resource_template_schema(template, &self.schema_validator_cache)?;
                let mut raw = RawResourceTemplate {
                    uri_template: template.uri_template.clone(),
                    name: template.name.clone(),
                    title: template.title.clone(),
                    description: template.description.clone(),
                    mime_type: template.mime_type.clone(),
                    icons: map_icons(template.icons.as_deref()),
                };
                if raw.name.is_empty() {
                    "resource-template".clone_into(&mut raw.name);
                }
                let annotations = crate::engine::resources::map_resource_annotations(
                    template.annotations.as_ref(),
                )?;
                #[cfg(feature = "completion")]
                if self.config.completion_active() {
                    crate::engine::completion::validate_completion_source_mapping(
                        template.completions.as_ref(),
                        &self.completion_sources,
                        &format!("resource template '{}'", template.uri_template),
                    )?;
                }
                templates.push(ResolvedResourceTemplate {
                    template: rmcp::model::AnnotateAble::optional_annotate(raw, annotations),
                    arguments_schema: template.arguments_schema.clone(),
                    completions: template.completions.clone(),
                    source: source.clone(),
                });
            }
        }
        Ok(templates)
    }

    async fn build_resource_catalog(&self) -> Result<ResourceCatalog, McpError> {
        let resources = self.build_resources().await?;
        let templates = self.build_resource_templates()?;
        crate::engine::resources::warn_resource_duplicates(&resources);
        crate::engine::resources::warn_resource_template_duplicates(&templates);
        Ok(ResourceCatalog::new(resources, templates))
    }

    pub(in crate::engine) async fn resource_catalog(
        &self,
    ) -> Result<Arc<ResourceCatalog>, McpError> {
        if let Some(existing) = self.resource_catalog.read().await.as_ref() {
            return Ok(Arc::clone(existing));
        }

        let refreshed = self.refresh_resource_catalog().await?;
        Ok(refreshed)
    }

    pub(in crate::engine) async fn refresh_resource_catalog(
        &self,
    ) -> Result<Arc<ResourceCatalog>, McpError> {
        let catalog = Arc::new(self.build_resource_catalog().await?);
        let mut guard = self.resource_catalog.write().await;
        *guard = Some(Arc::clone(&catalog));
        Ok(catalog)
    }

    pub(crate) async fn call_resource_plugin_read(
        &self,
        params: ResourcePluginCallParams<'_>,
    ) -> Result<ReadResourceResult, McpError> {
        let (plugin, ctx) = self.resource_plugin_call_parts(params.plugin_name, params.request)?;
        plugin
            .read(
                params.uri,
                PluginCallParams {
                    config: params.config.clone(),
                    ctx,
                },
            )
            .await
    }

    pub(crate) async fn call_resource_plugin_subscribe(
        &self,
        params: ResourcePluginCallParams<'_>,
    ) -> Result<(), McpError> {
        let (plugin, ctx) = self.resource_plugin_call_parts(params.plugin_name, params.request)?;
        plugin
            .subscribe(
                params.uri,
                PluginCallParams {
                    config: params.config.clone(),
                    ctx,
                },
            )
            .await
    }

    pub(crate) async fn call_resource_plugin_unsubscribe(
        &self,
        params: ResourcePluginCallParams<'_>,
    ) -> Result<(), McpError> {
        let (plugin, ctx) = self.resource_plugin_call_parts(params.plugin_name, params.request)?;
        plugin
            .unsubscribe(
                params.uri,
                PluginCallParams {
                    config: params.config.clone(),
                    ctx,
                },
            )
            .await
    }

    fn resource_plugin_call_parts(
        &self,
        plugin_name: &str,
        request: Option<RequestContext<RoleServer>>,
    ) -> Result<(std::sync::Arc<dyn ResourcePlugin>, PluginContext), McpError> {
        let plugin = helpers::lookup_resource_plugin(&self.plugins, plugin_name)?;
        let ctx = self.build_plugin_context(request);
        Ok((plugin, ctx))
    }

    pub async fn list_resources_for_refresh(&self) -> Result<Vec<Resource>, McpError> {
        Ok(self.refresh_resource_catalog().await?.resources_payload())
    }

    pub async fn list_resource_templates_for_refresh(
        &self,
    ) -> Result<Vec<ResourceTemplate>, McpError> {
        Ok(self.resource_catalog().await?.templates_payload())
    }
}
