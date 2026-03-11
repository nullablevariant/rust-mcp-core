//! Tool definition builders: converts ToolConfig into rmcp Tool models.
use std::sync::Arc;

use rmcp::{
    model::{Icon, Meta, Tool, ToolAnnotations},
    ErrorData as McpError,
};
#[cfg(feature = "http_tools")]
use serde_json::Value;

#[cfg(feature = "tasks_utility")]
use rmcp::model::ToolExecution;

#[cfg(feature = "tasks_utility")]
use crate::config::TaskSupport;
use crate::config::ToolConfig;

use super::value_helpers::value_to_object;

pub(super) fn build_tool_attr(tool_cfg: &ToolConfig) -> Result<Tool, McpError> {
    let input_schema = value_to_object(&tool_cfg.input_schema, "inputSchema")?;
    let output_schema = if let Some(schema) = tool_cfg.output_schema.as_ref() {
        Some(value_to_object(schema, "outputSchema")?)
    } else {
        None
    };

    let annotations = if let Some(value) = tool_cfg.annotations.as_ref() {
        Some(
            serde_json::from_value::<ToolAnnotations>(value.clone())
                .map_err(|e| McpError::invalid_request(e.to_string(), None))?,
        )
    } else {
        None
    };

    let icons = map_icons(tool_cfg.icons.as_deref());

    let meta = if let Some(meta_value) = tool_cfg.meta.as_ref() {
        Some(Meta(value_to_object(meta_value, "_meta")?))
    } else {
        None
    };
    let mut tool = Tool::new(
        tool_cfg.name.clone(),
        tool_cfg.description.clone(),
        input_schema,
    );
    tool.title.clone_from(&tool_cfg.title);
    tool.output_schema = output_schema.map(Arc::new);
    tool.annotations = annotations;
    tool.icons = icons;
    tool.meta = meta;
    #[cfg(feature = "tasks_utility")]
    if tool_cfg.execute.task_support() != TaskSupport::Forbidden {
        tool.execution =
            Some(ToolExecution::new().with_task_support(tool_cfg.execute.task_support()));
    }
    Ok(tool)
}

#[cfg(feature = "http_tools")]
pub(super) fn build_headers(
    outbound_http: Option<&crate::config::OutboundHttpConfig>,
    upstream: &crate::config::UpstreamConfig,
    oauth2_bearer: Option<&str>,
    tool_headers: Option<&Value>,
) -> Option<Value> {
    crate::http::outbound_pipeline::build_templated_headers(
        outbound_http,
        upstream,
        oauth2_bearer,
        tool_headers,
    )
}

pub(super) fn map_icons(icons: Option<&[crate::config::IconConfig]>) -> Option<Vec<Icon>> {
    icons.map(|icons| {
        icons
            .iter()
            .map(|icon| {
                let mut mapped = Icon::new(icon.src.clone());
                mapped.mime_type.clone_from(&icon.mime_type);
                mapped.sizes.clone_from(&icon.sizes);
                mapped
            })
            .collect::<Vec<_>>()
    })
}
