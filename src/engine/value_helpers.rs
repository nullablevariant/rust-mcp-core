//! JSON value helpers for template rendering and argument extraction.
#[cfg(feature = "http_tools")]
use rmcp::model::{Annotations, Icon, Meta};
use rmcp::{model::JsonObject, ErrorData as McpError};
#[cfg(feature = "http_tools")]
use serde_json::Map;
use serde_json::Value;

use crate::{config::McpConfig, plugins::PluginType};

#[cfg(feature = "http_tools")]
pub(super) fn remove_required_string(
    object: &mut Map<String, Value>,
    keys: &[&str],
    message: &str,
) -> Result<String, McpError> {
    remove_optional_string(object, keys)
        .ok_or_else(|| McpError::invalid_params(message.to_owned(), None))
}

#[cfg(feature = "http_tools")]
pub(super) fn remove_required_text(
    object: &mut Map<String, Value>,
    keys: &[&str],
    message: &str,
) -> Result<String, McpError> {
    for key in keys {
        if let Some(value) = object.remove(*key) {
            return Ok(match value {
                Value::String(value) => value,
                other => other.to_string(),
            });
        }
    }
    Err(McpError::invalid_params(message.to_owned(), None))
}

#[cfg(feature = "http_tools")]
pub(super) fn remove_optional_string(
    object: &mut Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        if let Some(value) = object.remove(*key) {
            if let Some(value) = value.as_str() {
                return Some(value.to_owned());
            }
            return None;
        }
    }
    None
}

#[cfg(feature = "http_tools")]
pub(super) fn remove_optional_meta(
    object: &mut Map<String, Value>,
    key: &str,
    error_message: &str,
) -> Result<Option<Meta>, McpError> {
    object
        .remove(key)
        .map(serde_json::from_value::<Meta>)
        .transpose()
        .map_err(|error| McpError::invalid_params(format!("{error_message}: {error}"), None))
}

#[cfg(feature = "http_tools")]
pub(super) fn remove_optional_icons(
    object: &mut Map<String, Value>,
    key: &str,
    error_message: &str,
) -> Result<Option<Vec<Icon>>, McpError> {
    object
        .remove(key)
        .map(normalize_icons_value)
        .transpose()?
        .map(serde_json::from_value::<Vec<Icon>>)
        .transpose()
        .map_err(|error| McpError::invalid_params(format!("{error_message}: {error}"), None))
}

#[cfg(feature = "http_tools")]
pub(super) fn remove_optional_annotations(
    object: &mut Map<String, Value>,
    key: &str,
    error_message: &str,
) -> Result<Option<Annotations>, McpError> {
    object
        .remove(key)
        .map(normalize_annotations_value)
        .transpose()?
        .map(serde_json::from_value::<Annotations>)
        .transpose()
        .map_err(|error| McpError::invalid_params(format!("{error_message}: {error}"), None))
}

// Normalizes snake_case "mime_type" to camelCase "mimeType" in icon objects
// so config authors can use either convention.
#[cfg(feature = "http_tools")]
pub(super) fn normalize_icons_value(value: Value) -> Result<Value, McpError> {
    let Value::Array(icons) = value else {
        return Err(McpError::invalid_params(
            "icons must be an array".to_owned(),
            None,
        ));
    };
    let normalized = icons
        .into_iter()
        .map(|icon| {
            let Value::Object(mut icon_object) = icon else {
                return Err(McpError::invalid_params(
                    "icon entries must be objects".to_owned(),
                    None,
                ));
            };
            if let Some(mime_type) = icon_object.remove("mime_type") {
                icon_object
                    .entry("mimeType".to_owned())
                    .or_insert(mime_type);
            }
            Ok(Value::Object(icon_object))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(normalized))
}

// Normalizes snake_case "last_modified" to camelCase "lastModified" in
// annotations so config authors can use either convention.
#[cfg(feature = "http_tools")]
pub(super) fn normalize_annotations_value(value: Value) -> Result<Value, McpError> {
    let Value::Object(mut annotations) = value else {
        return Err(McpError::invalid_params(
            "annotations must be an object".to_owned(),
            None,
        ));
    };
    if let Some(last_modified) = annotations.remove("last_modified") {
        annotations
            .entry("lastModified".to_owned())
            .or_insert(last_modified);
    }
    Ok(Value::Object(annotations))
}

pub(super) fn value_to_object(value: &Value, label: &str) -> Result<JsonObject, McpError> {
    match value {
        Value::Object(map) => Ok(map.clone()),
        _ => Err(McpError::invalid_request(
            format!("{label} must be an object"),
            None,
        )),
    }
}

// Shallow-merge plugin default config with per-tool override config.
// Per-tool `execute.config` keys override plugin-level `plugins[].config` keys.
pub(super) fn merge_plugin_config(
    config: &McpConfig,
    plugin_type: PluginType,
    plugin_name: &str,
    plugin_config: Option<&Value>,
) -> Value {
    let plugin_default = config
        .plugins
        .iter()
        .find(|p| p.name == plugin_name && p.plugin_type == plugin_type)
        .and_then(|p| p.config.as_ref());

    match (plugin_default, plugin_config) {
        (Some(Value::Object(base)), Some(Value::Object(overrides))) => {
            let mut merged = base.clone();
            for (key, value) in overrides {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        (_, Some(value)) | (Some(value), None) => value.clone(),
        (None, None) => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::merge_plugin_config;

    use crate::config::PluginConfig;
    use crate::inline_test_fixtures::base_config;
    use crate::plugins::PluginType;
    use serde_json::json;

    #[test]
    fn merge_plugin_config_filters_by_tool_type() {
        let mut config = base_config();
        config.plugins = vec![
            PluginConfig {
                name: "shared_name".to_owned(),
                plugin_type: PluginType::Auth,
                targets: None,
                config: Some(json!({"source": "auth"})),
            },
            PluginConfig {
                name: "shared_name".to_owned(),
                plugin_type: PluginType::Tool,
                targets: None,
                config: Some(json!({"source": "tool"})),
            },
        ];
        let result = merge_plugin_config(&config, PluginType::Tool, "shared_name", None);
        assert_eq!(result, json!({"source": "tool"}));

        let prompt_result = merge_plugin_config(&config, PluginType::Prompt, "shared_name", None);
        assert_eq!(prompt_result, json!(null));
    }

    #[test]
    fn merge_plugin_config_overrides_object_keys() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "plugin.echo".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: Some(json!({"timeout": 1000, "mode": "default"})),
        }];

        let result = merge_plugin_config(
            &config,
            PluginType::Tool,
            "plugin.echo",
            Some(&json!({"mode": "override", "retry": 2})),
        );

        assert_eq!(
            result,
            json!({"timeout": 1000, "mode": "override", "retry": 2})
        );
    }

    #[test]
    fn merge_plugin_config_uses_tool_config_for_non_objects() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "plugin.echo".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: Some(json!({"timeout": 1000})),
        }];

        let result = merge_plugin_config(
            &config,
            PluginType::Tool,
            "plugin.echo",
            Some(&json!("raw-value")),
        );
        assert_eq!(result, json!("raw-value"));
    }

    #[test]
    fn merge_plugin_config_uses_plugin_default_when_execute_config_absent() {
        let mut config = base_config();
        config.plugins = vec![PluginConfig {
            name: "plugin.echo".to_owned(),
            plugin_type: PluginType::Tool,
            targets: None,
            config: Some(json!({"timeout": 1000, "mode": "default"})),
        }];

        let result = merge_plugin_config(&config, PluginType::Tool, "plugin.echo", None);
        assert_eq!(result, json!({"timeout": 1000, "mode": "default"}));
    }

    #[test]
    fn merge_plugin_config_returns_null_when_plugin_default_and_execute_config_absent() {
        let config = base_config();
        let result = merge_plugin_config(&config, PluginType::Tool, "plugin.echo", None);
        assert_eq!(result, json!(null));
    }
}
