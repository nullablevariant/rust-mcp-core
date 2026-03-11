use std::path::PathBuf;

#[path = "../_shared/mock_harness.rs"]
mod mock_harness;

use rust_mcp_core::McpError;
use rust_mcp_core::{load_mcp_config_from_path, runtime, PluginRegistry};

fn config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("tools-templating")
        .join("config")
        .join("mcp_config.yml")
}

#[tokio::main]
async fn main() -> Result<(), McpError> {
    mock_harness::bootstrap_mock_env().await.map_err(|error| {
        McpError::internal_error(
            format!("failed to start example mock harness: {error}"),
            None,
        )
    })?;
    let config = load_mcp_config_from_path(config_path())?;
    runtime::run_from_config(config, PluginRegistry::new()).await
}
