//! Completion dispatch from engine to inline sources or completion plugins.
use rmcp::{
    model::{CompleteRequestParams, CompletionInfo},
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
#[cfg(any(
    all(feature = "completion", feature = "prompts"),
    all(feature = "completion", feature = "resources")
))]
use serde_json::json;

use crate::{
    config::CompletionProviderConfig, engine::value_helpers::merge_plugin_config,
    plugins::PluginCallParams,
};

#[cfg(any(feature = "prompts", feature = "resources"))]
use super::super::completion::{completion_info_from_values, validate_completion_argument_exists};
use super::{
    super::completion::{complete_inline_values, validate_completion_info},
    helpers::lookup_completion_plugin,
    Engine,
};

impl Engine {
    #[cfg(any(feature = "prompts", feature = "resources"))]
    pub(crate) async fn complete_request(
        &self,
        request: &CompleteRequestParams,
        request_context: Option<RequestContext<RoleServer>>,
    ) -> Result<CompletionInfo, McpError> {
        match &request.r#ref {
            rmcp::model::Reference::Prompt(prompt) => {
                #[cfg(feature = "prompts")]
                {
                    self.complete_prompt_argument(request, &prompt.name, request_context)
                        .await
                }
                #[cfg(not(feature = "prompts"))]
                {
                    let _ = (request, request_context);
                    let _ = prompt;
                    Err(McpError::invalid_params(
                        "prompts feature is not enabled".to_owned(),
                        None,
                    ))
                }
            }
            rmcp::model::Reference::Resource(resource) => {
                #[cfg(feature = "resources")]
                {
                    self.complete_resource_template_argument(
                        request,
                        &resource.uri,
                        request_context,
                    )
                    .await
                }
                #[cfg(not(feature = "resources"))]
                {
                    let _ = (request, request_context);
                    let _ = resource;
                    Err(McpError::invalid_params(
                        "resources feature is not enabled".to_owned(),
                        None,
                    ))
                }
            }
        }
    }

    #[cfg(not(any(feature = "prompts", feature = "resources")))]
    pub(crate) fn complete_request(
        request: CompleteRequestParams,
        request_context: Option<RequestContext<RoleServer>>,
    ) -> Result<CompletionInfo, McpError> {
        let _request_context = request_context;
        match request.r#ref {
            rmcp::model::Reference::Prompt(_prompt) => Err(McpError::invalid_params(
                "prompts feature is not enabled".to_owned(),
                None,
            )),
            rmcp::model::Reference::Resource(_resource) => Err(McpError::invalid_params(
                "resources feature is not enabled".to_owned(),
                None,
            )),
        }
    }

    #[cfg(all(feature = "completion", feature = "prompts"))]
    async fn complete_prompt_argument(
        &self,
        request: &CompleteRequestParams,
        prompt_name: &str,
        request_context: Option<RequestContext<RoleServer>>,
    ) -> Result<CompletionInfo, McpError> {
        let catalog = self.prompt_catalog().await?;
        let Some(selected) = catalog.by_name(prompt_name).cloned() else {
            let available_prompts = catalog
                .entries()
                .iter()
                .map(|entry| entry.prompt.name.clone())
                .collect::<Vec<_>>();
            return Err(McpError::invalid_params(
                format!("prompt '{prompt_name}' not found"),
                Some(json!({ "available_prompts": available_prompts })),
            ));
        };

        validate_completion_argument_exists(
            &selected.arguments_schema,
            &request.argument.name,
            &format!("prompt '{prompt_name}'"),
        )?;
        let Some(source_name) = selected
            .completions
            .as_ref()
            .and_then(|map| map.get(&request.argument.name))
        else {
            return completion_info_from_values(Vec::new());
        };

        self.complete_from_source(source_name, request, request_context)
            .await
    }

    #[cfg(all(feature = "completion", feature = "resources"))]
    async fn complete_resource_template_argument(
        &self,
        request: &CompleteRequestParams,
        template_uri: &str,
        request_context: Option<RequestContext<RoleServer>>,
    ) -> Result<CompletionInfo, McpError> {
        let catalog = self.resource_catalog().await?;
        let Some(selected) = catalog.template_by_uri_template(template_uri).cloned() else {
            let available_templates = catalog
                .templates()
                .iter()
                .map(|entry| entry.template.uri_template.clone())
                .collect::<Vec<_>>();
            return Err(McpError::invalid_params(
                format!("resource template '{template_uri}' not found"),
                Some(json!({ "available_templates": available_templates })),
            ));
        };

        validate_completion_argument_exists(
            &selected.arguments_schema,
            &request.argument.name,
            &format!("resource template '{template_uri}'"),
        )?;
        let Some(source_name) = selected
            .completions
            .as_ref()
            .and_then(|map| map.get(&request.argument.name))
        else {
            return completion_info_from_values(Vec::new());
        };

        self.complete_from_source(source_name, request, request_context)
            .await
    }

    #[cfg(feature = "completion")]
    #[cfg_attr(not(any(feature = "prompts", feature = "resources")), allow(dead_code))]
    async fn complete_from_source(
        &self,
        source_name: &str,
        request: &CompleteRequestParams,
        request_context: Option<RequestContext<RoleServer>>,
    ) -> Result<CompletionInfo, McpError> {
        let source = self
            .completion_sources
            .get(source_name)
            .ok_or_else(|| {
                McpError::invalid_request(
                    format!("completion provider '{source_name}' is not defined"),
                    None,
                )
            })?
            .clone();

        match source {
            CompletionProviderConfig::Inline { values, .. } => {
                complete_inline_values(values.as_slice(), request.argument.value.as_str())
            }
            CompletionProviderConfig::Plugin {
                plugin,
                config: source_config,
                ..
            } => {
                let completion_plugin = lookup_completion_plugin(&self.plugins, &plugin)?;
                let merged_config = merge_plugin_config(
                    &self.config,
                    crate::plugins::PluginType::Completion,
                    &plugin,
                    source_config.as_ref(),
                );
                completion_plugin
                    .complete(
                        request,
                        PluginCallParams {
                            config: merged_config,
                            ctx: self.build_plugin_context(request_context),
                        },
                    )
                    .await
                    .and_then(validate_completion_info)
            }
        }
    }
}
