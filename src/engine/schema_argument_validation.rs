//! Shared schema-driven argument validation helpers for prompts and resources.

use std::collections::{HashMap, HashSet};

use rmcp::ErrorData as McpError;
use serde_json::{Map, Value};

pub(super) fn validate_completion_keys(
    completions: Option<&HashMap<String, String>>,
    schema: &Value,
    entity_label: &str,
    entity_name: &str,
) -> Result<(), McpError> {
    let Some(completions) = completions else {
        return Ok(());
    };

    let schema_keys = schema
        .get("properties")
        .and_then(|value| value.as_object())
        .map(|properties| {
            properties
                .keys()
                .map(std::string::String::as_str)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    for key in completions.keys().map(std::string::String::as_str) {
        if !schema_keys.contains(key) {
            return Err(McpError::invalid_request(
                format!(
                    "{entity_label} '{entity_name}' completions key '{key}' is not defined in arguments_schema.properties"
                ),
                None,
            ));
        }
    }

    Ok(())
}

pub(super) struct ValidateSchemaArgsParams<'a> {
    pub(super) schema: &'a Value,
    pub(super) args: &'a Map<String, Value>,
    pub(super) entity_label: &'a str,
    pub(super) entity_name: &'a str,
    pub(super) schema_validator_cache: &'a crate::engine::SchemaValidatorCache,
}

pub(super) fn validate_schema_args(params: &ValidateSchemaArgsParams<'_>) -> Result<(), McpError> {
    let ValidateSchemaArgsParams {
        schema,
        args,
        entity_label,
        entity_name,
        schema_validator_cache,
    } = *params;

    let compiled = schema_validator_cache.get_or_compile(schema, |error| {
        McpError::invalid_request(
            format!("invalid arguments_schema for {entity_label} '{entity_name}': {error}"),
            None,
        )
    })?;

    let details = compiled
        .iter_errors(&Value::Object(args.clone()))
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !details.is_empty() {
        return Err(McpError::invalid_params(
            format!(
                "{entity_label} arguments validation failed for '{entity_name}': {}",
                details.join("; ")
            ),
            None,
        ));
    }

    Ok(())
}
