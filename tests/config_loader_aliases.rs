use std::path::PathBuf;

use rust_mcp_core::config::load_mcp_config_from_path;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn load_mcp_config_aliases_fixture() {
    let error =
        load_mcp_config_from_path(fixture_path("config_loader/config_loader_aliases_fixture"))
            .expect_err("aliases fixture with camelCase keys should fail validation");
    // The aliases fixture uses camelCase keys (e.g. inputSchema instead of input_schema).
    // Schema validation rejects it because required field input_schema is missing from the tool.
    assert!(
        error
            .message
            .starts_with("config schema validation failed:"),
        "error must be a schema validation failure, got: {}",
        error.message
    );
    assert!(
        error.message.contains("input_schema"),
        "error must reference the missing required field 'input_schema', got: {}",
        error.message
    );
}

#[test]
fn load_mcp_config_empty_placeholder_fixture() {
    let config = load_mcp_config_from_path(fixture_path(
        "config_loader/config_loader_empty_placeholder_fixture",
    ))
    .expect("config should load");

    assert_eq!(config.server.host, "${}");
}
