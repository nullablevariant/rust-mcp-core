//! Runtime bootstrap: transport selection, Axum router composition, and server lifecycle.

mod core;
mod list_cache;
mod runtime_checks;

pub use core::{build_runtime, run_from_config, Runtime};
