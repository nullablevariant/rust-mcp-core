//! Resource helpers for schema checks, URI/template handling, and duplicate warnings.

use std::collections::HashMap;

use rmcp::{
    model::{Annotations, Role},
    ErrorData as McpError,
};
use serde_json::Value;
use tracing::warn;

use crate::config::{ResourceAudienceConfig, ResourceTemplateConfig};

#[derive(Clone)]
pub(super) enum ResourceSource {
    Inline {
        content: Option<crate::config::ResourceContentConfig>,
    },
    Plugin {
        plugin_name: String,
        config: Value,
    },
}

#[derive(Clone)]
pub(super) struct ResolvedResource {
    pub(super) resource: rmcp::model::Resource,
    pub(super) source: ResourceSource,
}

#[derive(Clone)]
pub(super) struct ResolvedResourceTemplate {
    pub(super) template: rmcp::model::ResourceTemplate,
    pub(super) arguments_schema: Value,
    #[cfg_attr(not(feature = "completion"), allow(dead_code))]
    pub(super) completions: Option<HashMap<String, String>>,
    pub(super) source: ResourceSource,
}

#[derive(Clone, Default)]
pub(super) struct ResourceCatalog {
    resources: Vec<ResolvedResource>,
    resources_by_uri: HashMap<String, usize>,
    templates: Vec<ResolvedResourceTemplate>,
    #[cfg(feature = "completion")]
    templates_by_uri_template: HashMap<String, usize>,
}

impl ResourceCatalog {
    pub(super) fn new(
        resources: Vec<ResolvedResource>,
        templates: Vec<ResolvedResourceTemplate>,
    ) -> Self {
        let mut resources_by_uri = HashMap::new();
        for (index, entry) in resources.iter().enumerate() {
            resources_by_uri.insert(entry.resource.uri.clone(), index);
        }

        #[cfg(feature = "completion")]
        let mut templates_by_uri_template = HashMap::new();
        #[cfg(feature = "completion")]
        for (index, entry) in templates.iter().enumerate() {
            templates_by_uri_template.insert(entry.template.uri_template.clone(), index);
        }

        Self {
            resources,
            resources_by_uri,
            templates,
            #[cfg(feature = "completion")]
            templates_by_uri_template,
        }
    }

    pub(super) fn templates(&self) -> &[ResolvedResourceTemplate] {
        &self.templates
    }

    pub(super) fn resources_payload(&self) -> Vec<rmcp::model::Resource> {
        self.resources
            .iter()
            .map(|entry| entry.resource.clone())
            .collect::<Vec<_>>()
    }

    pub(super) fn templates_payload(&self) -> Vec<rmcp::model::ResourceTemplate> {
        self.templates
            .iter()
            .map(|entry| entry.template.clone())
            .collect::<Vec<_>>()
    }

    pub(super) fn resource_by_uri(&self, uri: &str) -> Option<&ResolvedResource> {
        self.resources_by_uri
            .get(uri)
            .and_then(|index| self.resources.get(*index))
    }

    #[cfg(feature = "completion")]
    pub(super) fn template_by_uri_template(
        &self,
        uri_template: &str,
    ) -> Option<&ResolvedResourceTemplate> {
        self.templates_by_uri_template
            .get(uri_template)
            .and_then(|index| self.templates.get(*index))
    }
}

pub(super) fn warn_resource_duplicates(resources: &[ResolvedResource]) {
    let mut seen = HashMap::new();
    for resource in resources {
        let source = resource_source_label(&resource.source);
        if let Some(previous_source) = seen.insert(resource.resource.uri.clone(), source.clone()) {
            warn!(
                "duplicate resource '{}' detected; keeping last provider for resources/read (previous='{}', winner='{}')",
                resource.resource.uri,
                previous_source,
                source
            );
        }
    }
}

pub(super) fn warn_resource_template_duplicates(templates: &[ResolvedResourceTemplate]) {
    let mut seen = HashMap::new();
    for template in templates {
        let source = resource_source_label(&template.source);
        if let Some(previous_source) =
            seen.insert(template.template.uri_template.clone(), source.clone())
        {
            warn!(
                "duplicate resource template '{}' detected; keeping last provider for resources/read (previous='{}', winner='{}')",
                template.template.uri_template,
                previous_source,
                source
            );
        }
    }
}

pub(super) fn resource_source_label(source: &ResourceSource) -> String {
    match source {
        ResourceSource::Inline { .. } => "inline".to_owned(),
        ResourceSource::Plugin { plugin_name, .. } => format!("plugin:{plugin_name}"),
    }
}

pub(super) fn map_resource_annotations(
    annotations: Option<&crate::config::ResourceAnnotationsConfig>,
) -> Result<Option<Annotations>, McpError> {
    let Some(annotations) = annotations else {
        return Ok(None);
    };
    if let Some(priority) = annotations.priority {
        if !(0.0..=1.0).contains(&priority) {
            return Err(McpError::invalid_request(
                "resource annotations.priority must be between 0.0 and 1.0".to_owned(),
                None,
            ));
        }
    }
    let audience = annotations.audience.as_ref().map(|audience| {
        audience
            .iter()
            .map(|value| match value {
                ResourceAudienceConfig::User => Role::User,
                ResourceAudienceConfig::Assistant => Role::Assistant,
            })
            .collect::<Vec<_>>()
    });
    let last_modified = annotations
        .last_modified
        .as_ref()
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|ts| ts.with_timezone(&chrono::Utc))
                .map_err(|error| {
                    McpError::invalid_request(
                        format!("invalid resource annotations.last_modified: {error}"),
                        None,
                    )
                })
        })
        .transpose()?;
    let mut mapped = Annotations::default();
    mapped.audience = audience;
    mapped.priority = annotations.priority;
    mapped.last_modified = last_modified;
    Ok(Some(mapped))
}

pub(super) fn validate_resource_uri(uri: &str) -> Result<(), McpError> {
    url::Url::parse(uri).map_err(|error| {
        McpError::invalid_request(format!("invalid resource uri '{uri}': {error}"), None)
    })?;
    Ok(())
}

pub(super) fn validate_resource_template_uri(uri_template: &str) -> Result<(), McpError> {
    let mut normalized = String::with_capacity(uri_template.len());
    let mut chars = uri_template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut name = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                name.push(next);
            }
            if name.is_empty() || !closed {
                return Err(McpError::invalid_request(
                    format!("invalid resource uri_template '{uri_template}': invalid placeholder"),
                    None,
                ));
            }
            normalized.push('x');
            continue;
        }
        normalized.push(ch);
    }

    if normalized.contains('{') || normalized.contains('}') {
        return Err(McpError::invalid_request(
            format!("invalid resource uri_template '{uri_template}': unbalanced braces"),
            None,
        ));
    }

    url::Url::parse(&normalized).map_err(|error| {
        McpError::invalid_request(
            format!("invalid resource uri_template '{uri_template}': {error}"),
            None,
        )
    })?;
    Ok(())
}

pub(super) fn validate_resource_template_schema(
    template: &ResourceTemplateConfig,
    schema_validator_cache: &crate::engine::SchemaValidatorCache,
) -> Result<(), McpError> {
    let _ = schema_validator_cache.get_or_compile(&template.arguments_schema, |error| {
        McpError::invalid_request(
            format!(
                "invalid arguments_schema for resource template '{}': {}",
                template.uri_template, error
            ),
            None,
        )
    })?;
    crate::engine::schema_argument_validation::validate_completion_keys(
        template.completions.as_ref(),
        &template.arguments_schema,
        "resource template",
        &template.uri_template,
    )?;
    Ok(())
}

pub(super) fn match_uri_template(
    uri_template: &str,
    uri: &str,
) -> Option<serde_json::Map<String, Value>> {
    #[derive(Debug)]
    enum Token {
        Literal(String),
        Var(String),
    }

    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut chars = uri_template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            if !literal.is_empty() {
                tokens.push(Token::Literal(std::mem::take(&mut literal)));
            }
            let mut var = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                var.push(next);
            }
            if var.is_empty() || !closed {
                return None;
            }
            tokens.push(Token::Var(var));
            continue;
        }
        literal.push(ch);
    }
    if !literal.is_empty() {
        tokens.push(Token::Literal(literal));
    }

    let mut position = 0usize;
    let mut args = serde_json::Map::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Literal(segment) => {
                if !uri[position..].starts_with(segment) {
                    return None;
                }
                position += segment.len();
            }
            Token::Var(name) => {
                let next_literal = tokens[index + 1..].iter().find_map(|token| match token {
                    Token::Literal(value) if !value.is_empty() => Some(value.as_str()),
                    _ => None,
                });
                let value = if let Some(next_literal) = next_literal {
                    let remaining = &uri[position..];
                    let offset = remaining.find(next_literal)?;
                    let extracted = &remaining[..offset];
                    position += offset;
                    extracted
                } else {
                    let extracted = &uri[position..];
                    position = uri.len();
                    extracted
                };
                args.insert(name.clone(), Value::String(value.to_owned()));
            }
        }
    }
    if position != uri.len() {
        return None;
    }
    Some(args)
}
