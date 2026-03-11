use std::path::PathBuf;

use rust_mcp_core::config::load_mcp_config_from_path;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn invalid_json_returns_error_fixture() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_invalid_json_fixture.json",
    ))
    .expect_err("invalid JSON fixture must fail to load");
    // JSON parse errors produce invalid_request with serde_json error text.
    // The fixture has a missing closing brace so the error references EOF/unexpected end.
    assert!(
        error.message.contains("EOF")
            || error.message.contains("expected")
            || error.message.contains("line")
            || error.message.contains("column"),
        "JSON parse error must reference parse location or expected token, got: {}",
        error.message
    );
    // Must NOT be a schema validation error — it fails earlier at JSON parse
    assert!(
        !error.message.starts_with("config schema validation failed"),
        "error must be a parse error, not a schema validation error"
    );
}

#[test]
fn invalid_yaml_returns_error_fixture() {
    let error = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_invalid_yaml_fixture",
    ))
    .expect_err("invalid YAML fixture must fail to load");
    // YAML parse errors produce invalid_request with serde_yaml error text.
    // The fixture has a malformed line so the error references a line/column.
    assert!(
        error.message.contains("at line")
            || error.message.contains("expected")
            || error.message.contains("did not find"),
        "YAML parse error must reference parse location or expected token, got: {}",
        error.message
    );
    // Must NOT be a schema validation error — it fails earlier at YAML parse
    assert!(
        !error.message.starts_with("config schema validation failed"),
        "error must be a parse error, not a schema validation error"
    );
}

#[test]
fn missing_config_file_returns_error() {
    let error = load_mcp_config_from_path(fixture_path("config_loader/does_not_exist_fixture"))
        .expect_err("missing config file must fail to load");
    // Missing file errors use internal_error with std::io::Error text.
    // On Linux this produces "No such file or directory".
    assert!(
        error.message.contains("No such file or directory") || error.message.contains("not found"),
        "missing file error must reference file-not-found, got: {}",
        error.message
    );
}

#[test]
fn missing_env_var_substitutes_null_fixture() {
    std::env::remove_var("MCP_MISSING_PLUGIN_CONFIG");
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_missing_env_substitution_fixture",
    ))
    .expect("missing env var should substitute null and load successfully");

    // Verify null-substitution: the fixture has `config: ${env:MCP_MISSING_PLUGIN_CONFIG}` on a
    // plugin entry. When MCP_MISSING_PLUGIN_CONFIG is unset, it substitutes to "null", so the
    // plugin's config field should be None after deserialization.
    assert_eq!(
        config.plugins.len(),
        1,
        "fixture must have exactly 1 plugin"
    );
    assert_eq!(
        config.plugins[0].name, "plugin.echo",
        "plugin name must match fixture"
    );
    assert!(
        config.plugins[0].config.is_none(),
        "plugin config must be None after null substitution"
    );
}
