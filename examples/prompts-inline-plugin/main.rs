mod plugin;

use std::path::PathBuf;

use plugin::PromptCatalogPlugin;
use rust_mcp_core::McpError;
use rust_mcp_core::{load_mcp_config_from_path, runtime, PluginRegistry};

fn config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("prompts-inline-plugin")
        .join("config")
        .join("mcp_config.yml")
}

#[tokio::main]
async fn main() -> Result<(), McpError> {
    let config = load_mcp_config_from_path(config_path())?;
    let plugins = PluginRegistry::new().register_prompt(PromptCatalogPlugin)?;
    runtime::run_from_config(config, plugins).await
}
