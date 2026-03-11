use std::path::PathBuf;

#[path = "../_shared/mock_harness.rs"]
mod mock_harness;

use rust_mcp_core::McpError;
use rust_mcp_core::{load_mcp_config_from_path, runtime, PluginRegistry};

fn config_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("utility-list-changed")
        .join("config")
        .join(file)
}

#[tokio::main]
async fn main() -> Result<(), McpError> {
    mock_harness::bootstrap_mock_env().await.map_err(|error| {
        McpError::internal_error(
            format!("failed to start example mock harness: {error}"),
            None,
        )
    })?;
    let initial = load_mcp_config_from_path(config_path("mcp_config.yml"))?;
    let runtime = runtime::build_runtime(initial, PluginRegistry::new()).await?;

    let runtime_for_reload = runtime.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        match load_mcp_config_from_path(config_path("mcp_config_reload.yml")) {
            Ok(updated) => {
                if let Err(error) = runtime_for_reload.reload_config(updated).await {
                    tracing::warn!("list-changed example reload failed: {}", error);
                }
            }
            Err(error) => {
                tracing::warn!(
                    "list-changed example failed to load reload config: {}",
                    error
                );
            }
        }
    });

    runtime.run().await
}
