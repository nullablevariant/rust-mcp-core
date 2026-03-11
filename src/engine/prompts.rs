//! Prompt helpers for schema validation, duplicate handling, and typed content rendering.

use std::collections::{HashMap, HashSet};

use rmcp::{
    model::{
        AnnotateAble, Annotations, Icon, Meta, Prompt, PromptArgument, PromptMessage,
        PromptMessageContent, PromptMessageRole, RawEmbeddedResource, RawImageContent, RawResource,
        ResourceContents,
    },
    ErrorData as McpError,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tracing::warn;

use crate::config::{PromptMessageRoleConfig, PromptTemplateMessageConfig};

use super::templating::{render_value, RenderContext};

#[derive(Clone)]
pub(super) enum PromptSource {
    Inline {
        messages: Vec<PromptTemplateMessageConfig>,
    },
    Plugin {
        plugin_name: String,
        config: Value,
    },
}

#[derive(Clone)]
pub(super) struct ResolvedPrompt {
    pub(super) prompt: Prompt,
    pub(super) arguments_schema: Value,
    #[cfg_attr(not(feature = "completion"), allow(dead_code))]
    pub(super) completions: Option<HashMap<String, String>>,
    pub(super) source: PromptSource,
}

#[derive(Clone, Default)]
pub(super) struct PromptCatalog {
    entries: Vec<ResolvedPrompt>,
    by_name: HashMap<String, usize>,
}

impl PromptCatalog {
    pub(super) fn new(entries: Vec<ResolvedPrompt>) -> Self {
        let mut by_name = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            by_name.insert(entry.prompt.name.clone(), index);
        }

        Self { entries, by_name }
    }

    pub(super) fn entries(&self) -> &[ResolvedPrompt] {
        &self.entries
    }

    pub(super) fn prompts(&self) -> Vec<Prompt> {
        self.entries
            .iter()
            .map(|entry| entry.prompt.clone())
            .collect::<Vec<_>>()
    }

    pub(super) fn by_name(&self, name: &str) -> Option<&ResolvedPrompt> {
        self.by_name
            .get(name)
            .and_then(|index| self.entries.get(*index))
    }
}

pub(super) fn derive_prompt_arguments(
    schema: &Value,
) -> Result<Option<Vec<PromptArgument>>, McpError> {
    let Value::Object(schema_obj) = schema else {
        return Err(McpError::invalid_request(
            "prompt arguments_schema must be an object".to_owned(),
            None,
        ));
    };

    let properties = schema_obj
        .get("properties")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let required = schema_obj
        .get("required")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut arguments = Vec::new();
    for (name, value) in properties {
        let title = value
            .get("title")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let description = value
            .get("description")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let mut argument = PromptArgument::new(name.clone());
        argument.title = title;
        argument.description = description;
        argument.required = Some(required.contains(name.as_str()));
        arguments.push(argument);
    }

    if arguments.is_empty() {
        Ok(None)
    } else {
        Ok(Some(arguments))
    }
}

pub(super) fn warn_prompt_duplicates(prompts: &[ResolvedPrompt]) {
    let mut seen = HashMap::new();
    for prompt in prompts {
        let source = prompt_source_label(&prompt.source);
        if let Some(previous_source) = seen.insert(prompt.prompt.name.clone(), source.clone()) {
            warn!(
                "duplicate prompt '{}' detected; keeping last provider for prompts/get (previous='{}', winner='{}')",
                prompt.prompt.name,
                previous_source,
                source
            );
        }
    }
}

pub(super) fn prompt_source_label(source: &PromptSource) -> String {
    match source {
        PromptSource::Inline { .. } => "inline".to_owned(),
        PromptSource::Plugin { plugin_name, .. } => format!("plugin:{plugin_name}"),
    }
}

pub(super) fn render_prompt_messages(
    messages: &[PromptTemplateMessageConfig],
    args: &Value,
) -> Result<Vec<PromptMessage>, McpError> {
    let ctx = RenderContext::new(args, None);
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                PromptMessageRoleConfig::User => PromptMessageRole::User,
                PromptMessageRoleConfig::Assistant => PromptMessageRole::Assistant,
            };
            let rendered_content = render_value(&message.content, &ctx)?;
            let content = render_prompt_message_content(rendered_content)?;
            Ok(PromptMessage::new(role, content))
        })
        .collect()
}

pub(super) fn render_prompt_message_content(
    value: Value,
) -> Result<PromptMessageContent, McpError> {
    match value {
        Value::String(text) => Ok(PromptMessageContent::Text { text }),
        Value::Object(mut object) => render_typed_prompt_content(&mut object),
        _ => Err(McpError::invalid_params(
            "prompt message content must be a string or object".to_owned(),
            None,
        )),
    }
}

fn render_typed_prompt_content(
    object: &mut Map<String, Value>,
) -> Result<PromptMessageContent, McpError> {
    let content_type = object
        .remove("type")
        .and_then(|value| value.as_str().map(ToString::to_string))
        .ok_or_else(|| {
            McpError::invalid_params(
                "prompt message content object requires type".to_owned(),
                None,
            )
        })?;
    match content_type.as_str() {
        "text" => render_text_content(object),
        "image" => render_image_content(object),
        "resource" => render_resource_content(object),
        "resource_link" => render_resource_link_content(object),
        _ => Err(McpError::invalid_params(
            format!("unsupported prompt message content type: {content_type}"),
            None,
        )),
    }
}

fn render_text_content(object: &mut Map<String, Value>) -> Result<PromptMessageContent, McpError> {
    let Some(text) = object
        .remove("text")
        .and_then(|value| value.as_str().map(ToString::to_string))
    else {
        return Err(McpError::invalid_params(
            "text content requires text".to_owned(),
            None,
        ));
    };
    Ok(PromptMessageContent::Text { text })
}

fn render_image_content(object: &mut Map<String, Value>) -> Result<PromptMessageContent, McpError> {
    let Some(data) = object
        .remove("data")
        .and_then(|value| value.as_str().map(ToString::to_string))
    else {
        return Err(McpError::invalid_params(
            "image content requires data".to_owned(),
            None,
        ));
    };
    let mime_type = extract_mime_type(object).ok_or_else(|| {
        McpError::invalid_params("image content requires mime_type".to_owned(), None)
    })?;
    let meta = extract_optional::<Meta>(object, "_meta", "image _meta")?;
    let annotations = extract_optional::<Annotations>(object, "annotations", "image annotations")?;
    Ok(PromptMessageContent::Image {
        image: RawImageContent {
            data,
            mime_type,
            meta,
        }
        .optional_annotate(annotations),
    })
}

fn render_resource_content(
    object: &mut Map<String, Value>,
) -> Result<PromptMessageContent, McpError> {
    let Some(uri) = object
        .remove("uri")
        .and_then(|value| value.as_str().map(ToString::to_string))
    else {
        return Err(McpError::invalid_params(
            "resource content requires uri".to_owned(),
            None,
        ));
    };
    let mime_type = extract_mime_type(object);
    let content_meta = extract_optional::<Meta>(object, "content_meta", "resource content_meta")?;
    let embedded_meta = extract_optional::<Meta>(object, "_meta", "resource _meta")?;
    let annotations =
        extract_optional::<Annotations>(object, "annotations", "resource annotations")?;
    let resource = if let Some(text) = object
        .remove("text")
        .and_then(|value| value.as_str().map(ToString::to_string))
    {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            meta: content_meta,
        }
    } else if let Some(blob) = object
        .remove("blob")
        .and_then(|value| value.as_str().map(ToString::to_string))
    {
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            meta: content_meta,
        }
    } else {
        return Err(McpError::invalid_params(
            "resource content requires text or blob".to_owned(),
            None,
        ));
    };
    Ok(PromptMessageContent::Resource {
        resource: RawEmbeddedResource {
            meta: embedded_meta,
            resource,
        }
        .optional_annotate(annotations),
    })
}

fn render_resource_link_content(
    object: &mut Map<String, Value>,
) -> Result<PromptMessageContent, McpError> {
    let Some(uri) = object
        .remove("uri")
        .and_then(|value| value.as_str().map(ToString::to_string))
    else {
        return Err(McpError::invalid_params(
            "resource_link content requires uri".to_owned(),
            None,
        ));
    };
    let mut raw = RawResource::new(
        uri,
        object
            .remove("name")
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "resource".to_owned()),
    );
    raw.title = object
        .remove("title")
        .and_then(|value| value.as_str().map(ToString::to_string));
    raw.description = object
        .remove("description")
        .and_then(|value| value.as_str().map(ToString::to_string));
    raw.mime_type = extract_mime_type(object);
    raw.size = extract_resource_link_size(object)?;
    raw.meta = extract_optional::<Meta>(object, "_meta", "resource_link _meta")?;
    raw.icons = extract_optional::<Vec<Icon>>(object, "icons", "resource_link icons")?;
    let annotations =
        extract_optional::<Annotations>(object, "annotations", "resource_link annotations")?;
    Ok(PromptMessageContent::ResourceLink {
        link: raw.optional_annotate(annotations),
    })
}

fn extract_mime_type(object: &mut Map<String, Value>) -> Option<String> {
    object
        .remove("mime_type")
        .or_else(|| object.remove("mimeType"))
        .and_then(|value| value.as_str().map(ToString::to_string))
}

fn extract_resource_link_size(object: &mut Map<String, Value>) -> Result<Option<u32>, McpError> {
    let Some(value) = object.remove("size") else {
        return Ok(None);
    };
    let Some(size) = value.as_u64() else {
        return Ok(None);
    };
    let converted = u32::try_from(size).map_err(|_| {
        McpError::invalid_params("resource_link size must be <= 4294967295".to_owned(), None)
    })?;
    Ok(Some(converted))
}

fn extract_optional<T>(
    object: &mut Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<Option<T>, McpError>
where
    T: DeserializeOwned,
{
    object
        .remove(key)
        .map(serde_json::from_value::<T>)
        .transpose()
        .map_err(|error| McpError::invalid_params(format!("invalid {label}: {error}"), None))
}
