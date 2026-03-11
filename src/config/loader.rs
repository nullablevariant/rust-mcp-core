//! Config loading, environment variable expansion, and JSON schema validation.

use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use rmcp::ErrorData as McpError;
use serde_json::Value;

use super::{
    config_schema, McpConfig, OutboundRetryConfig, StreamableHttpSessionMode,
    TopLevelCombinatorsPolicy,
};

/// Load an [`McpConfig`] from the given path or the `MCP_CONFIG_PATH` env var.
///
/// If `path` is `Some`, that file is used directly. Otherwise falls back to
/// the `MCP_CONFIG_PATH` environment variable. The file may be YAML or JSON
/// (detected by extension). Environment variables in `${env:VAR}` syntax are
/// expanded before parsing.
///
/// # Arguments
///
/// * `path` — explicit config file path, or `None` to use `MCP_CONFIG_PATH`
///
/// # Returns
///
/// A validated [`McpConfig`] ready for use with [`run_from_config`](crate::runtime::run_from_config).
///
/// # Errors
///
/// Returns `McpError` if:
/// - No path is provided and `MCP_CONFIG_PATH` is not set
/// - The file cannot be read
/// - Environment variable placeholders are malformed
/// - The config fails JSON schema validation
/// - Deserialization into `McpConfig` fails
pub fn load_mcp_config(path: Option<PathBuf>) -> Result<McpConfig, McpError> {
    let config_path = path.or_else(|| env::var("MCP_CONFIG_PATH").ok().map(PathBuf::from));
    let Some(path) = config_path else {
        return Err(McpError::invalid_request(
            "MCP_CONFIG_PATH is required for config-driven servers".to_owned(),
            None,
        ));
    };
    load_mcp_config_from_path(path)
}

/// Load an [`McpConfig`] from a specific file path.
///
/// Detects YAML vs JSON by file extension. Expands `${env:VAR}` environment
/// variable placeholders, validates against the config JSON schema, and
/// deserializes into [`McpConfig`].
///
/// # Arguments
///
/// * `path` — path to a YAML or JSON config file
///
/// # Returns
///
/// A validated [`McpConfig`].
///
/// # Errors
///
/// Returns `McpError` if the file cannot be read, parsed, or validated.
pub fn load_mcp_config_from_path<P: AsRef<Path>>(path: P) -> Result<McpConfig, McpError> {
    let path = path.as_ref();
    let raw =
        std::fs::read_to_string(path).map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let expanded = expand_env_vars(&raw)?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let value: Value = if ext.eq_ignore_ascii_case("json") {
        serde_json::from_str(&expanded)
            .map_err(|e| McpError::invalid_request(e.to_string(), None))?
    } else {
        serde_yaml::from_str(&expanded)
            .map_err(|e| McpError::invalid_request(e.to_string(), None))?
    };

    validate_config(&value)?;

    let config = serde_json::from_value::<McpConfig>(value)
        .map_err(|e| McpError::invalid_request(e.to_string(), None))?;
    validate_streamable_http_hardening_semantics(&config)?;
    validate_completion_semantics(&config)?;
    validate_outbound_retry_semantics(&config)?;
    validate_input_schema_client_compat(&config)?;
    Ok(config)
}

// Scans for ${env:VAR_NAME} placeholders in raw config text and replaces them
// with the corresponding env var value, or "null" if unset. Non-namespaced
// placeholders are preserved for runtime templating.
fn expand_env_vars(content: &str) -> Result<String, McpError> {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && matches!(chars.peek(), Some('{')) {
            chars.next();
            let mut key = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                key.push(next);
            }
            if !closed {
                return Err(McpError::invalid_request(
                    format!("malformed env placeholder: missing closing `}}` for `${{{key}}}`"),
                    None,
                ));
            }

            if let Some(env_key) = key.strip_prefix("env:") {
                if env_key.is_empty() {
                    return Err(McpError::invalid_request(
                        "malformed env placeholder: empty env var name in `${env:}`".to_owned(),
                        None,
                    ));
                }
                let value = env::var(env_key).unwrap_or_else(|_| "null".to_owned());
                out.push_str(&value);
            } else {
                out.push_str("${");
                out.push_str(&key);
                out.push('}');
            }
        } else {
            out.push(ch);
        }
    }

    Ok(out)
}

// Two-phase validation: first checks the JSON schema, then enforces plugin
// name uniqueness (which JSON schema alone can't express).
fn validate_config(value: &Value) -> Result<(), McpError> {
    let schema = config_schema();
    let compiled = jsonschema::validator_for(schema)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

    let messages = compiled
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    if !messages.is_empty() {
        return Err(McpError::invalid_request(
            format!("config schema validation failed: {}", messages.join("; ")),
            None,
        ));
    }

    validate_plugin_names(value)?;

    Ok(())
}

fn validate_plugin_names(value: &Value) -> Result<(), McpError> {
    let Some(plugins) = value.get("plugins").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    for plugin in plugins {
        let Some(name) = plugin.get("name").and_then(|v| v.as_str()) else {
            return Err(McpError::invalid_request(
                "plugins[].name must be a string".to_owned(),
                None,
            ));
        };
        if !seen.insert(name.to_owned()) {
            return Err(McpError::invalid_request(
                format!("duplicate plugin name: {name}"),
                None,
            ));
        }
    }
    Ok(())
}

fn validate_input_schema_client_compat(config: &McpConfig) -> Result<(), McpError> {
    let policy = config
        .server
        .client_compat
        .input_schema
        .top_level_combinators;
    if policy == TopLevelCombinatorsPolicy::Off {
        return Ok(());
    }

    for (index, tool) in config.tools_items().iter().enumerate() {
        let Some(schema) = tool.input_schema.as_object() else {
            continue;
        };

        let found = ["anyOf", "oneOf", "allOf"]
            .into_iter()
            .filter(|key| schema.contains_key(*key))
            .collect::<Vec<_>>();
        if found.is_empty() {
            continue;
        }

        let detail = format!(
            "tools.{index}.input_schema (tool '{}') uses top-level {}. \
some AI clients reject top-level combinators; prefer object-only constraints \
like `required` and `minProperties`.",
            tool.name,
            found
                .iter()
                .map(|key| format!("`{key}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if policy == TopLevelCombinatorsPolicy::Warn {
            tracing::warn!("{detail}");
            continue;
        }
        return Err(McpError::invalid_request(
            format!("input schema client compatibility check failed: {detail}"),
            None,
        ));
    }

    Ok(())
}

fn validate_completion_semantics(config: &McpConfig) -> Result<(), McpError> {
    let Some(completion) = config.completion.as_ref() else {
        return Ok(());
    };
    if !completion.is_active() {
        return Ok(());
    }
    if completion.providers.is_empty() {
        return Err(McpError::invalid_request(
            "completion config is active but completion.providers is empty; set completion.enabled=false to disable completion"
                .to_owned(),
            None,
        ));
    }
    Ok(())
}

fn validate_streamable_http_hardening_semantics(config: &McpConfig) -> Result<(), McpError> {
    let streamable_http = &config.server.transport.streamable_http;
    let hardening = streamable_http.hardening.as_ref();
    if streamable_http.session_mode == StreamableHttpSessionMode::None
        && hardening
            .and_then(|candidate| candidate.session.as_ref())
            .is_some()
    {
        return Err(McpError::invalid_request(
            "transport.streamable_http.hardening.session requires session_mode=optional|required"
                .to_owned(),
            None,
        ));
    }
    if let Some(rate_limit) = hardening.and_then(|candidate| candidate.rate_limit.as_ref()) {
        if rate_limit.is_active() && !rate_limit.has_any_bucket() {
            return Err(McpError::invalid_request(
                "transport.streamable_http.hardening.rate_limit requires global or per_ip when enabled"
                    .to_owned(),
                None,
            ));
        }
    }
    if let Some(creation_rate) = hardening
        .and_then(|candidate| candidate.session.as_ref())
        .and_then(|session| session.creation_rate.as_ref())
    {
        if creation_rate.is_active() && !creation_rate.has_any_bucket() {
            return Err(McpError::invalid_request(
                "transport.streamable_http.hardening.session.creation_rate requires global or per_ip when enabled"
                    .to_owned(),
                None,
            ));
        }
    }
    Ok(())
}

fn validate_outbound_retry_semantics(config: &McpConfig) -> Result<(), McpError> {
    if let Some(retry) = config
        .outbound_http
        .as_ref()
        .and_then(|outbound| outbound.retry.as_ref())
    {
        validate_retry_config("outbound_http.retry", retry)?;
    }

    for (name, upstream) in &config.upstreams {
        if let Some(retry) = upstream.retry.as_ref() {
            validate_retry_config(&format!("upstreams.{name}.retry"), retry)?;
        }
    }

    for (index, tool) in config.tools_items().iter().enumerate() {
        let Some(execute_http) = tool.execute.as_http() else {
            continue;
        };
        let Some(retry) = execute_http.retry.as_ref() else {
            continue;
        };
        validate_retry_config(&format!("tools.{index}.execute.retry"), retry)?;
    }

    Ok(())
}

fn validate_retry_config(path: &str, retry: &OutboundRetryConfig) -> Result<(), McpError> {
    if retry.max_attempts == 0 {
        return Err(McpError::invalid_request(
            format!("{path}.max_attempts must be >= 1"),
            None,
        ));
    }
    if retry.delay_ms == 0 {
        return Err(McpError::invalid_request(
            format!("{path}.delay_ms must be >= 1"),
            None,
        ));
    }
    if !retry.on_network_errors && retry.on_statuses.is_empty() {
        return Err(McpError::invalid_request(
            format!("{path} must configure on_statuses when on_network_errors is false"),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{env, sync::Mutex};

    use serde_json::json;

    use crate::config::{StreamableHttpSessionMode, TopLevelCombinatorsPolicy};

    use super::{
        expand_env_vars, validate_completion_semantics, validate_input_schema_client_compat,
        validate_plugin_names, validate_retry_config, validate_streamable_http_hardening_semantics,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // Inline-only because `validate_plugin_names` is a private helper that cannot be
    // exercised from integration tests without widening visibility.
    #[test]
    fn validate_plugin_names_rejects_missing_name() {
        // Item 6: missing "name" key — no .get("name") match at all
        let value = json!({ "plugins": [{ "type": "tool" }] });
        let error = validate_plugin_names(&value).expect_err("missing name should fail");
        assert_eq!(
            error.message, "plugins[].name must be a string",
            "expected exact missing-name error message"
        );
        // Verify no "name" key present in input to confirm this is the missing-key branch
        assert!(
            value["plugins"][0].get("name").is_none(),
            "test input must lack a 'name' key to exercise missing-name branch"
        );
    }

    #[test]
    fn validate_plugin_names_rejects_non_string_name() {
        // Item 6: "name" key exists but is not a string — .as_str() returns None
        let value = json!({ "plugins": [{ "name": 123, "type": "tool" }] });
        let error = validate_plugin_names(&value).expect_err("non-string name should fail");
        assert_eq!(
            error.message, "plugins[].name must be a string",
            "expected exact non-string-name error message"
        );
        // Verify "name" key IS present but not a string, confirming non-string branch
        let name_val = value["plugins"][0]
            .get("name")
            .expect("name key should exist");
        assert!(
            !name_val.is_string(),
            "test input must have a non-string 'name' to exercise non-string branch"
        );
    }

    // Inline-only because `expand_env_vars` is private loader logic that cannot
    // be exercised from integration tests without widening visibility.
    #[test]
    fn expand_env_vars_expands_namespaced_only() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        env::set_var("API_BASE_URL", "https://example.com");
        env::set_var("query", "injected");

        let content = r"base_url: ${env:API_BASE_URL}
q: ${query}
root: ${$}
opt: ${limit?}
";
        let expanded = expand_env_vars(content).expect("expand should succeed");
        // Item 2: assert exact line-level outputs, not just containment
        let lines: Vec<&str> = expanded.lines().collect();
        assert_eq!(lines.len(), 4, "expected exactly 4 output lines");
        assert_eq!(
            lines[0], "base_url: https://example.com",
            "env:VAR must be substituted"
        );
        assert_eq!(
            lines[1], "q: ${query}",
            "non-namespaced placeholder must be preserved"
        );
        assert_eq!(
            lines[2], "root: ${$}",
            "non-namespaced placeholder must be preserved"
        );
        assert_eq!(
            lines[3], "opt: ${limit?}",
            "non-namespaced placeholder must be preserved"
        );

        env::remove_var("API_BASE_URL");
        env::remove_var("query");
    }

    // Item 1: tighten to exact error message
    #[test]
    fn expand_env_vars_rejects_empty_namespaced_placeholder() {
        let error = expand_env_vars("x: ${env:}").expect_err("empty env key should fail");
        assert_eq!(
            error.message, "malformed env placeholder: empty env var name in `${env:}`",
            "exact error message for empty env var name"
        );
    }

    // Inline-only because `validate_input_schema_client_compat` is private
    // loader logic and should not be exposed for integration tests.
    #[test]
    fn validate_input_schema_client_compat_rejects_error_policy() {
        let mut config = crate::inline_test_fixtures::base_config();
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators = TopLevelCombinatorsPolicy::Error;
        *config.tools_items_mut() = vec![crate::config::ToolConfig {
            name: "tool.compat".to_owned(),
            title: None,
            description: "compat tool".to_owned(),
            cancellable: true,
            input_schema: json!({
                "type": "object",
                "anyOf": [{"required": ["id"]}]
            }),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
            execute: crate::config::ExecuteConfig::Plugin(crate::config::ExecutePluginConfig {
                plugin: "tool.plugin".to_owned(),
                config: None,
                task_support: crate::config::TaskSupport::Forbidden,
            }),
            response: None,
        }];

        let error = validate_input_schema_client_compat(&config)
            .expect_err("error policy should reject top-level anyOf");
        // Item 1: assert deterministic error tokens including path, tool name, combinator, and prefix
        assert!(
            error
                .message
                .starts_with("input schema client compatibility check failed:"),
            "error must start with compat check prefix, got: {}",
            error.message
        );
        assert!(
            error.message.contains("tools.0.input_schema"),
            "error must reference offending field path, got: {}",
            error.message
        );
        assert!(
            error.message.contains("tool 'tool.compat'"),
            "error must reference tool name, got: {}",
            error.message
        );
        assert!(
            error.message.contains("`anyOf`"),
            "error must reference the combinator found, got: {}",
            error.message
        );
    }

    // Item 3: assert config unchanged and no validation error for non-object schemas
    #[test]
    fn validate_input_schema_client_compat_ignores_non_object_input_schema() {
        let mut config = crate::inline_test_fixtures::base_config();
        config
            .server
            .client_compat
            .input_schema
            .top_level_combinators = TopLevelCombinatorsPolicy::Warn;
        *config.tools_items_mut() = vec![crate::config::ToolConfig {
            name: "tool.compat".to_owned(),
            title: None,
            description: "compat tool".to_owned(),
            cancellable: true,
            input_schema: json!("not-an-object"),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
            execute: crate::config::ExecuteConfig::Plugin(crate::config::ExecutePluginConfig {
                plugin: "tool.plugin".to_owned(),
                config: None,
                task_support: crate::config::TaskSupport::Forbidden,
            }),
            response: None,
        }];
        let result = validate_input_schema_client_compat(&config);
        assert!(
            result.is_ok(),
            "non-object input_schema should be ignored by compat checker"
        );
        // Verify post-conditions: config tool still has non-object schema (no silent mutation)
        assert_eq!(
            config.tools_items().len(),
            1,
            "tool count must be unchanged"
        );
        assert_eq!(
            config.tools_items()[0].input_schema,
            json!("not-an-object"),
            "input_schema must be unchanged"
        );
        // Also verify the policy is still active (Warn), confirming the skip was due to non-object schema
        assert_eq!(
            config
                .server
                .client_compat
                .input_schema
                .top_level_combinators,
            TopLevelCombinatorsPolicy::Warn,
            "policy must remain Warn"
        );
    }

    // Item 1: tighten to exact error message for completion semantics
    #[test]
    fn validate_completion_semantics_rejects_active_completion_without_sources() {
        let mut config = crate::inline_test_fixtures::base_config();
        config.completion = Some(crate::config::CompletionConfig {
            enabled: Some(true),
            providers: Vec::new(),
        });
        let error = validate_completion_semantics(&config)
            .expect_err("active completion without sources should fail");
        assert_eq!(
            error.message,
            "completion config is active but completion.providers is empty; set completion.enabled=false to disable completion",
            "exact error message for active completion without sources"
        );
    }

    // Inline-only because `validate_retry_config` is a private helper and
    // schema-level fixtures cannot reach these branches deterministically.
    // Item 1: tighten to exact error messages including path prefix
    #[test]
    fn validate_retry_config_rejects_zero_bounds() {
        let max_attempts_error = validate_retry_config(
            "outbound_http.retry",
            &crate::config::OutboundRetryConfig {
                max_attempts: 0,
                delay_ms: 200,
                on_network_errors: true,
                on_statuses: vec![429],
            },
        )
        .expect_err("max_attempts=0 should fail");
        assert_eq!(
            max_attempts_error.message, "outbound_http.retry.max_attempts must be >= 1",
            "error must include path and field"
        );

        let delay_error = validate_retry_config(
            "outbound_http.retry",
            &crate::config::OutboundRetryConfig {
                max_attempts: 2,
                delay_ms: 0,
                on_network_errors: true,
                on_statuses: vec![429],
            },
        )
        .expect_err("delay_ms=0 should fail");
        assert_eq!(
            delay_error.message, "outbound_http.retry.delay_ms must be >= 1",
            "error must include path and field"
        );
    }

    // Inline-only because `validate_streamable_http_hardening_semantics` is private
    // loader logic and should not be exposed for integration tests.
    // Item 4: assert strict error contract including rule identifier and offending path
    #[test]
    fn validate_streamable_http_hardening_semantics_rejects_session_config_without_sessions() {
        let mut config = crate::inline_test_fixtures::base_config();
        config.server.transport.streamable_http.session_mode = StreamableHttpSessionMode::None;
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                session: Some(crate::config::StreamableHttpSessionHardeningConfig::default()),
                ..crate::config::StreamableHttpHardeningConfig::default()
            });
        let error = validate_streamable_http_hardening_semantics(&config)
            .expect_err("session hardening should fail when session_mode=none");
        // Assert exact error message to distinguish from rate_limit and creation_rate branches
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.session requires session_mode=optional|required",
            "error must reference session_mode rule and full path"
        );
    }

    #[test]
    fn validate_streamable_http_hardening_semantics_rejects_rate_limit_without_buckets() {
        let mut config = crate::inline_test_fixtures::base_config();
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                rate_limit: Some(crate::config::StreamableHttpRateLimitConfig {
                    enabled: Some(true),
                    global: None,
                    per_ip: None,
                }),
                ..crate::config::StreamableHttpHardeningConfig::default()
            });
        let error = validate_streamable_http_hardening_semantics(&config)
            .expect_err("active hardening.rate_limit should require at least one bucket");
        // Assert exact error message to distinguish from session and creation_rate branches
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.rate_limit requires global or per_ip when enabled",
            "error must reference rate_limit rule and full path"
        );
    }

    #[test]
    fn validate_streamable_http_hardening_semantics_rejects_creation_rate_without_buckets() {
        let mut config = crate::inline_test_fixtures::base_config();
        config.server.transport.streamable_http.hardening =
            Some(crate::config::StreamableHttpHardeningConfig {
                session: Some(crate::config::StreamableHttpSessionHardeningConfig {
                    creation_rate: Some(crate::config::StreamableHttpRateLimitConfig {
                        enabled: Some(true),
                        global: None,
                        per_ip: None,
                    }),
                    ..Default::default()
                }),
                ..crate::config::StreamableHttpHardeningConfig::default()
            });
        let error = validate_streamable_http_hardening_semantics(&config)
            .expect_err("active hardening.session.creation_rate should require a bucket");
        // Assert exact error message to distinguish from session and rate_limit branches
        assert_eq!(
            error.message,
            "transport.streamable_http.hardening.session.creation_rate requires global or per_ip when enabled",
            "error must reference creation_rate rule and full path"
        );
    }
}
