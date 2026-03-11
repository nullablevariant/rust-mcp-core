use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::{fmt::Write as _, io::Write as _};

use async_trait::async_trait;
use rmcp::model::{
    AnnotateAble, GetPromptResult, Prompt, PromptMessage, PromptMessageRole, RawResource,
    ReadResourceResult, ResourceContents,
};
use rmcp::ErrorData as McpError;
use rust_mcp_core::runtime::{build_runtime, Runtime};
use rust_mcp_core::{
    load_mcp_config_from_path, ListFeature, McpConfig, PluginCallParams, PluginRegistry,
    PromptEntry, PromptPlugin, ResourceEntry, ResourcePlugin,
};
use serde_json::{json, Value};
use uuid::Uuid;

const TOOL_COUNT: usize = 500;
const INLINE_PROMPT_COUNT: usize = 150;
const PLUGIN_PROMPT_COUNT: usize = 150;
const INLINE_RESOURCE_COUNT: usize = 150;
const INLINE_RESOURCE_TEMPLATE_COUNT: usize = 150;
const PLUGIN_RESOURCE_COUNT: usize = 150;
const PLUGIN_RESOURCE_TEMPLATE_COUNT: usize = 150;

#[derive(Clone, Debug)]
struct TogglePromptPlugin {
    changed: Arc<AtomicBool>,
}

#[async_trait]
impl PromptPlugin for TogglePromptPlugin {
    fn name(&self) -> &'static str {
        "bench.prompt"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        let suffix = if self.changed.load(Ordering::Relaxed) {
            "changed"
        } else {
            "base"
        };
        let mut entries = Vec::with_capacity(PLUGIN_PROMPT_COUNT);
        for idx in 0..PLUGIN_PROMPT_COUNT {
            let prompt_name = format!("prompt.plugin.{idx}.{suffix}");
            let prompt_description = format!("benchmark-{suffix}-{idx}");
            entries.push(PromptEntry {
                prompt: Prompt::new(prompt_name, Some(prompt_description), None),
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "index": { "type": "integer" },
                    },
                }),
                completions: None,
            });
        }
        Ok(entries)
    }

    async fn get(
        &self,
        _name: &str,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::Assistant,
            "bench",
        )])
        .with_description("bench"))
    }
}

#[derive(Clone, Debug)]
struct ToggleResourcePlugin {
    changed: Arc<AtomicBool>,
}

#[async_trait]
impl ResourcePlugin for ToggleResourcePlugin {
    fn name(&self) -> &'static str {
        "bench.resource"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        let suffix = if self.changed.load(Ordering::Relaxed) {
            "changed"
        } else {
            "base"
        };
        let mut entries = Vec::with_capacity(PLUGIN_RESOURCE_COUNT);
        for idx in 0..PLUGIN_RESOURCE_COUNT {
            entries.push(ResourceEntry {
                resource: RawResource::new(
                    format!("resource://bench/{suffix}/{idx}"),
                    format!("bench-{idx}"),
                )
                .no_annotation(),
            });
        }
        Ok(entries)
    }

    async fn read(
        &self,
        uri: &str,
        _params: PluginCallParams,
    ) -> Result<ReadResourceResult, McpError> {
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: uri.to_owned(),
                mime_type: Some("text/plain".to_owned()),
                text: "ok".to_owned(),
                meta: None,
            },
        ]))
    }

    async fn subscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }

    async fn unsubscribe(&self, _uri: &str, _params: PluginCallParams) -> Result<(), McpError> {
        Ok(())
    }
}

#[derive(Debug)]
struct BenchHarness {
    runtime: Runtime,
    prompt_changed: Arc<AtomicBool>,
    resource_changed: Arc<AtomicBool>,
    config_a: McpConfig,
    config_b: McpConfig,
}

fn build_tools_block(count: usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "    - name: tool.bench.{idx:04}\n      description: benchmark tool {idx}\n      input_schema:\n        type: object\n        properties:\n          id:\n            type: integer\n      execute:\n        type: http\n        upstream: bench\n        method: GET\n        path: /bench/{idx:04}\n"
        );
    }
    out
}

fn build_inline_prompts_block(count: usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "      - name: inline.prompt.{idx:04}\n        description: inline benchmark prompt {idx}\n        arguments_schema:\n          type: object\n          properties:\n            text:\n              type: string\n        template:\n          messages:\n            - role: assistant\n              content: \"inline prompt {idx}\"\n"
        );
    }
    out
}

fn build_inline_resource_items_block(count: usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "      - uri: resource://inline/{idx}\n        name: inline-resource-{idx}\n        mime_type: text/plain\n        content:\n          text: \"inline resource {idx}\"\n"
        );
    }
    out
}

fn build_resource_templates_block(count: usize, prefix: &str) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "      - uri_template: resource://{prefix}/template/{idx}/{{id}}\n        name: {prefix}-template-{idx}\n        mime_type: text/plain\n        arguments_schema:\n          type: object\n          properties:\n            id:\n              type: string\n"
        );
    }
    out
}

fn bench_config_yaml() -> String {
    let tools_block = build_tools_block(TOOL_COUNT);
    let inline_prompts_block = build_inline_prompts_block(INLINE_PROMPT_COUNT);
    let inline_resources_block = build_inline_resource_items_block(INLINE_RESOURCE_COUNT);
    let inline_resource_templates_block =
        build_resource_templates_block(INLINE_RESOURCE_TEMPLATE_COUNT, "inline");
    let plugin_resource_templates_block =
        build_resource_templates_block(PLUGIN_RESOURCE_TEMPLATE_COUNT, "plugin");

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
    mode: stdio
prompts:
  notify_list_changed: false
  providers:
    - type: inline
      items:
{inline_prompts_block}    - type: plugin
      plugin: bench.prompt
resources:
  notify_list_changed: false
  clients_can_subscribe: false
  providers:
    - type: inline
      items:
{inline_resources_block}      templates:
{inline_resource_templates_block}    - type: plugin
      plugin: bench.resource
      templates:
{plugin_resource_templates_block}tools:
  items:
{tools_block}upstreams:
  bench:
    base_url: http://127.0.0.1:31998
plugins:
  - name: bench.prompt
    type: prompt
  - name: bench.resource
    type: resource
"
    )
}

fn load_bench_config() -> McpConfig {
    let path =
        std::env::temp_dir().join(format!("runtime_split_lock_bench_{}.yaml", Uuid::new_v4()));
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

fn make_harness(runtime: &tokio::runtime::Runtime) -> BenchHarness {
    let config_a = load_bench_config();
    let mut config_b = config_a.clone();
    config_b.set_tools_notify_list_changed(true);

    let prompt_changed = Arc::new(AtomicBool::new(false));
    let resource_changed = Arc::new(AtomicBool::new(false));

    let plugins = match PluginRegistry::new()
        .register_prompt(TogglePromptPlugin {
            changed: Arc::clone(&prompt_changed),
        })
        .and_then(|registry| {
            registry.register_resource(ToggleResourcePlugin {
                changed: Arc::clone(&resource_changed),
            })
        }) {
        Ok(value) => value,
        Err(error) => panic!("failed to register benchmark plugins: {error}"),
    };

    let runtime_instance = match runtime.block_on(build_runtime(config_a.clone(), plugins)) {
        Ok(value) => value,
        Err(error) => panic!("failed to build benchmark runtime: {error}"),
    };

    BenchHarness {
        runtime: runtime_instance,
        prompt_changed,
        resource_changed,
        config_a,
        config_b,
    }
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

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(value) => value,
        Err(error) => panic!("failed to build tokio runtime: {error}"),
    };
    let harness = make_harness(&rt);
    let iterations = 100;

    // Warmup to avoid first-call cold-path skew.
    if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Tools)) {
        panic!("warmup tools refresh failed: {error}");
    }
    if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Prompts)) {
        panic!("warmup prompts refresh failed: {error}");
    }
    if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Resources)) {
        panic!("warmup resources refresh failed: {error}");
    }

    run_case("refresh_tools_no_change", iterations, || {
        if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Tools)) {
            panic!("refresh tools failed: {error}");
        }
    });

    run_case("refresh_prompts_no_change", iterations, || {
        harness.prompt_changed.store(false, Ordering::Relaxed);
        if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Prompts)) {
            panic!("refresh prompts no-change failed: {error}");
        }
    });

    run_case("refresh_prompts_changed", iterations, || {
        harness.prompt_changed.fetch_xor(true, Ordering::Relaxed);
        if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Prompts)) {
            panic!("refresh prompts changed failed: {error}");
        }
    });

    run_case("refresh_resources_no_change", iterations, || {
        harness.resource_changed.store(false, Ordering::Relaxed);
        if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Resources)) {
            panic!("refresh resources no-change failed: {error}");
        }
    });

    run_case("refresh_resources_changed", iterations, || {
        harness.resource_changed.fetch_xor(true, Ordering::Relaxed);
        if let Err(error) = rt.block_on(harness.runtime.refresh_list(ListFeature::Resources)) {
            panic!("refresh resources changed failed: {error}");
        }
    });

    let reload_toggle = AtomicBool::new(false);
    run_case("reload_config_alternating", iterations, || {
        let config = if reload_toggle.fetch_xor(true, Ordering::Relaxed) {
            harness.config_b.clone()
        } else {
            harness.config_a.clone()
        };
        if let Err(error) = rt.block_on(harness.runtime.reload_config(config)) {
            panic!("reload config failed: {error}");
        }
    });
}
