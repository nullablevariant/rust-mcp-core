//! Tool response building: output schema validation, structured content, and content rendering.

use rmcp::{
    model::{CallToolResult, Content},
    ErrorData as McpError,
};
use serde_json::Value;

#[cfg(feature = "http_tools")]
use rmcp::model::RawTextContent;
#[cfg(feature = "http_tools")]
use rmcp::model::{AnnotateAble, RawResource, ResourceContents};
#[cfg(feature = "http_tools")]
use rmcp::model::{RawEmbeddedResource, RawImageContent};
#[cfg(feature = "http_tools")]
use serde_json::Map;

#[cfg(feature = "http_tools")]
use super::templating::{render_value, RenderContext};
#[cfg(feature = "http_tools")]
use super::value_helpers::{
    remove_optional_annotations, remove_optional_icons, remove_optional_meta,
    remove_optional_string, remove_required_string, remove_required_text,
};

pub fn build_structured_result(
    structured: Value,
    output_schema: Option<&Value>,
    fallback: Option<&str>,
) -> Result<CallToolResult, McpError> {
    build_structured_result_with_cache(
        structured,
        output_schema,
        fallback,
        &crate::engine::SchemaValidatorCache::default(),
    )
}

pub(crate) fn build_structured_result_with_cache(
    structured: Value,
    output_schema: Option<&Value>,
    fallback: Option<&str>,
    schema_validator_cache: &crate::engine::SchemaValidatorCache,
) -> Result<CallToolResult, McpError> {
    if let Some(schema) = output_schema {
        validate_structured_output_with_cache(&structured, schema, schema_validator_cache)?;
    }

    let content_text = match fallback {
        Some("text") => structured
            .as_str()
            .map_or_else(|| structured.to_string(), str::to_owned),
        _ => structured.to_string(),
    };

    let mut result = CallToolResult::default();
    result.content = vec![Content::text(content_text)];
    result.structured_content = Some(structured);
    result.is_error = Some(false);
    Ok(result)
}

pub fn validate_structured_output(
    structured: &Value,
    output_schema: &Value,
) -> Result<(), McpError> {
    validate_structured_output_with_cache(
        structured,
        output_schema,
        &crate::engine::SchemaValidatorCache::default(),
    )
}

pub(crate) fn validate_structured_output_with_cache(
    structured: &Value,
    output_schema: &Value,
    schema_validator_cache: &crate::engine::SchemaValidatorCache,
) -> Result<(), McpError> {
    let compiled = schema_validator_cache.get_or_compile(output_schema, |error| {
        McpError::invalid_request(error, None)
    })?;
    let messages = compiled
        .iter_errors(structured)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !messages.is_empty() {
        return Err(McpError::invalid_request(
            format!("output schema validation failed: {}", messages.join("; ")),
            None,
        ));
    }
    Ok(())
}

pub fn build_content_result(content: Vec<Content>) -> CallToolResult {
    CallToolResult::success(content)
}

#[cfg(feature = "http_tools")]
pub(super) fn build_content_blocks(
    content: Option<&Vec<Value>>,
    ctx: &RenderContext<'_>,
) -> Result<Vec<Content>, McpError> {
    let Some(content) = content else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in content {
        let rendered = render_value(item, ctx)?;
        out.push(parse_tool_content_block(rendered)?);
    }

    Ok(out)
}

// Dispatches a rendered JSON object to the appropriate MCP content type
// (text, image, audio, resource_link, resource) based on the "type" field.
// Each branch extracts required/optional fields and maps them to rmcp model types.
#[cfg(feature = "http_tools")]
fn parse_tool_content_block(value: Value) -> Result<Content, McpError> {
    let Value::Object(mut object) = value else {
        return Err(McpError::invalid_params(
            "tool response content item must be an object".to_owned(),
            None,
        ));
    };
    let content_type = remove_required_string(
        &mut object,
        &["type"],
        "tool response content item requires type",
    )?;
    match content_type.as_str() {
        "text" => {
            let text = remove_required_text(&mut object, &["text"], "text content requires text")?;
            let meta = remove_optional_meta(&mut object, "_meta", "invalid text _meta")?;
            let annotations = remove_optional_annotations(
                &mut object,
                "annotations",
                "invalid text annotations",
            )?;
            Ok(rmcp::model::RawContent::Text(RawTextContent { text, meta })
                .optional_annotate(annotations))
        }
        "image" => {
            let data =
                remove_required_string(&mut object, &["data"], "image content requires data")?;
            let mime_type = remove_required_string(
                &mut object,
                &["mime_type", "mimeType"],
                "image content requires mime_type",
            )?;
            let meta = remove_optional_meta(&mut object, "_meta", "invalid image _meta")?;
            let annotations = remove_optional_annotations(
                &mut object,
                "annotations",
                "invalid image annotations",
            )?;
            Ok(rmcp::model::RawContent::Image(RawImageContent {
                data,
                mime_type,
                meta,
            })
            .optional_annotate(annotations))
        }
        "audio" => {
            let data =
                remove_required_string(&mut object, &["data"], "audio content requires data")?;
            let mime_type = remove_required_string(
                &mut object,
                &["mime_type", "mimeType"],
                "audio content requires mime_type",
            )?;
            let annotations = remove_optional_annotations(
                &mut object,
                "annotations",
                "invalid audio annotations",
            )?;
            Ok(
                rmcp::model::RawContent::Audio(rmcp::model::RawAudioContent { data, mime_type })
                    .optional_annotate(annotations),
            )
        }
        "resource_link" => parse_tool_resource_link_content(object),
        "resource" => parse_tool_resource_content(object),
        _ => Err(McpError::invalid_params(
            format!("unsupported tool response content type: {content_type}"),
            None,
        )),
    }
}

// Parses `type: "resource_link"` content into an annotated RawResource.
// Extracts uri (required), optional name/title/description/mime_type/size/meta/icons,
// and top-level annotations.
#[cfg(feature = "http_tools")]
fn parse_tool_resource_link_content(mut object: Map<String, Value>) -> Result<Content, McpError> {
    let uri = remove_required_string(&mut object, &["uri"], "resource_link content requires uri")?;
    let mut raw = RawResource::new(
        uri,
        remove_optional_string(&mut object, &["name"]).unwrap_or_else(|| "resource".to_owned()),
    );
    raw.title = remove_optional_string(&mut object, &["title"]);
    raw.description = remove_optional_string(&mut object, &["description"]);
    raw.mime_type = remove_optional_string(&mut object, &["mime_type", "mimeType"]);
    raw.size = object
        .remove("size")
        .and_then(|value| value.as_u64())
        .map(|size| {
            u32::try_from(size).map_err(|_| {
                McpError::invalid_params(
                    "resource_link size must be <= 4294967295".to_owned(),
                    None,
                )
            })
        })
        .transpose()?;
    raw.meta = remove_optional_meta(&mut object, "_meta", "invalid resource_link _meta")?;
    raw.icons = remove_optional_icons(&mut object, "icons", "invalid resource_link icons")?;
    let annotations = remove_optional_annotations(
        &mut object,
        "annotations",
        "invalid resource_link annotations",
    )?;
    Ok(rmcp::model::RawContent::ResourceLink(raw).optional_annotate(annotations))
}

// Parses `type: "resource"` content, which wraps a text or blob resource.
// Annotations can appear at top level or nested inside the resource object,
// but not both — that's an error to avoid ambiguity.
#[cfg(feature = "http_tools")]
fn parse_tool_resource_content(mut object: Map<String, Value>) -> Result<Content, McpError> {
    let top_level_annotations =
        remove_optional_annotations(&mut object, "annotations", "invalid resource annotations")?;
    let embedded_meta = remove_optional_meta(&mut object, "_meta", "invalid resource _meta")?;
    let resource_value = object.remove("resource").ok_or_else(|| {
        McpError::invalid_params("resource content requires resource".to_owned(), None)
    })?;
    let Value::Object(mut resource_object) = resource_value else {
        return Err(McpError::invalid_params(
            "resource content requires resource object".to_owned(),
            None,
        ));
    };
    let uri = remove_required_string(
        &mut resource_object,
        &["uri"],
        "resource content requires uri",
    )?;
    let mime_type = remove_optional_string(&mut resource_object, &["mime_type", "mimeType"]);
    let content_meta = remove_optional_meta(
        &mut resource_object,
        "_meta",
        "invalid resource content _meta",
    )?;
    let nested_annotations = remove_optional_annotations(
        &mut resource_object,
        "annotations",
        "invalid resource annotations",
    )?;
    let text = remove_optional_string(&mut resource_object, &["text"]);
    let blob = remove_optional_string(&mut resource_object, &["blob"]);
    let resource = match (text, blob) {
        (Some(text), None) => ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            meta: content_meta,
        },
        (None, Some(blob)) => ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            meta: content_meta,
        },
        (None, None) => {
            return Err(McpError::invalid_params(
                "resource content requires text or blob".to_owned(),
                None,
            ))
        }
        (Some(_), Some(_)) => {
            return Err(McpError::invalid_params(
                "resource content cannot include both text and blob".to_owned(),
                None,
            ))
        }
    };
    let annotations = match (top_level_annotations, nested_annotations) {
        (Some(_), Some(_)) => {
            return Err(McpError::invalid_params(
                "resource content annotations must be set either at top level or in resource"
                    .to_owned(),
                None,
            ))
        }
        (Some(annotations), None) | (None, Some(annotations)) => Some(annotations),
        (None, None) => None,
    };
    Ok(rmcp::model::RawContent::Resource(RawEmbeddedResource {
        meta: embedded_meta,
        resource,
    })
    .optional_annotate(annotations))
}
