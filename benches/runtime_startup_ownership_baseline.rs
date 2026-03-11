use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt::Write as _, io::Write};

use rmcp::ErrorData as McpError;
use rust_mcp_core::auth::build_auth_state_with_plugins;
use rust_mcp_core::{load_mcp_config_from_path, Engine, EngineConfig, McpConfig, PluginRegistry};
use uuid::Uuid;

const TOOL_COUNT: usize = 600;
type StartupHarness = (
    Arc<Engine>,
    Arc<McpConfig>,
    Arc<rust_mcp_core::AuthState>,
    Arc<PluginRegistry>,
);

fn build_tools_block(count: usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "    - name: tool.startup.{idx:04}\n      description: startup benchmark tool {idx}\n      input_schema:\n        type: object\n        properties:\n          id:\n            type: integer\n      execute:\n        type: http\n        upstream: bench\n        method: GET\n        path: /bench/{idx:04}\n"
        );
    }
    out
}

fn bench_config_yaml() -> String {
    let tools_block = build_tools_block(TOOL_COUNT);
    format!(
        r"
version: 1
server:
  host: 127.0.0.1
  port: 31943
  endpoint_path: /mcp
  logging:
    level: error
  transport:
    mode: streamable_http
tools:
  items:
{tools_block}
upstreams:
  bench:
    base_url: http://127.0.0.1:31998
plugins: []
"
    )
}

fn load_bench_config() -> McpConfig {
    let path = std::env::temp_dir().join(format!(
        "runtime_startup_ownership_baseline_{}.yaml",
        Uuid::new_v4()
    ));
    if let Err(error) = std::fs::write(&path, bench_config_yaml()) {
        panic!("failed to write benchmark config fixture: {error}");
    }
    let config = match load_mcp_config_from_path(&path) {
        Ok(value) => value,
        Err(error) => panic!("failed to load benchmark config via schema loader: {error}"),
    };
    if let Err(error) = std::fs::remove_file(&path) {
        panic!("failed to remove benchmark config fixture: {error}");
    }
    config
}

fn build_engine_and_support(config: McpConfig) -> Result<StartupHarness, McpError> {
    let config_arc = Arc::new(config.clone());
    let plugins = PluginRegistry::new();
    let auth_state = build_auth_state_with_plugins(&config, None)?;
    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: plugins.clone(),
        list_refresh_handle: None,
    })?;
    Ok((Arc::new(engine), config_arc, auth_state, Arc::new(plugins)))
}

fn percentile(sorted: &[Duration], numerator: usize, denominator: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::from_nanos(0);
    }
    let index = ((sorted.len() - 1) * numerator) / denominator;
    sorted[index]
}

fn run_case<F>(name: &str, iterations: usize, mut op: F)
where
    F: FnMut(),
{
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        op();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let median = percentile(&samples, 1, 2);
    let p95 = percentile(&samples, 95, 100);
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "{name}: iterations={iterations} median_ns={} p95_ns={}",
        median.as_nanos(),
        p95.as_nanos()
    );
}

fn main() {
    if std::env::args().any(|arg| arg == "--list" || arg == "--list-tests") {
        return;
    }

    let config = load_bench_config();
    let (engine, config, auth_state, plugins) = match build_engine_and_support(config) {
        Ok(value) => value,
        Err(error) => panic!("failed to build startup benchmark harness: {error}"),
    };

    let iterations = 400;

    run_case("startup_clone_config_only", iterations, || {
        let cloned = Arc::clone(&config);
        black_box(cloned);
    });

    run_case("startup_clone_engine_only", iterations, || {
        let cloned = Arc::clone(&engine);
        black_box(cloned);
    });

    run_case("startup_handoff_stdio_args", iterations, || {
        black_box(Arc::clone(&engine));
    });

    run_case("startup_handoff_streamable_http_args", iterations, || {
        let cloned_config = Arc::clone(&config);
        let cloned_engine = Arc::clone(&engine);
        let cloned_auth = Arc::clone(&auth_state);
        let cloned_plugins = Arc::clone(&plugins);
        black_box((cloned_config, cloned_engine, cloned_auth, cloned_plugins));
    });
}
