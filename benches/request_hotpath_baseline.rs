//! Request-path benchmark baseline for tools/prompts/resources/completion.
use std::hint::black_box;
use std::time::{Duration, Instant};
use std::{fmt::Write as _, io::Write as _};

use async_trait::async_trait;
use rmcp::{
    model::{
        AnnotateAble, ArgumentInfo, CallToolRequestParams, CallToolResult, CompleteRequestParams,
        Extensions, GetPromptRequestParams, GetPromptResult, Meta, NumberOrString, Prompt,
        PromptMessage, PromptMessageRole, ReadResourceRequestParams, ReadResourceResult, Reference,
        ResourceContents,
    },
    service::{RequestContext, RoleServer, RunningService},
    ErrorData as McpError, ServerHandler,
};
use rust_mcp_core::{
    load_mcp_config_from_path, Engine, EngineConfig, PluginCallParams, PluginRegistry, PromptEntry,
    PromptPlugin, ResourceEntry, ResourcePlugin, ToolPlugin,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ITERATIONS: usize = 120;
const FILLER_TOOL_COUNT: usize = 300;
const INLINE_PROMPT_COUNT: usize = 220;
const PLUGIN_PROMPT_COUNT: usize = 220;
const INLINE_RESOURCE_COUNT: usize = 220;
const PLUGIN_RESOURCE_COUNT: usize = 220;
const INLINE_TEMPLATE_COUNT: usize = 220;
const PLUGIN_TEMPLATE_COUNT: usize = 220;

type BenchService = RunningService<RoleServer, Engine>;

#[derive(Debug)]
struct BenchToolPlugin;

#[async_trait]
impl ToolPlugin for BenchToolPlugin {
    fn name(&self) -> &'static str {
        "bench.tool"
    }

    async fn call(
        &self,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let mut result = CallToolResult::default();
        result.structured_content = Some(json!({
            "plugin": "bench.tool",
            "args": args,
        }));
        Ok(result)
    }
}

#[derive(Debug)]
struct BenchPromptPlugin;

#[async_trait]
impl PromptPlugin for BenchPromptPlugin {
    fn name(&self) -> &'static str {
        "bench.prompt"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError> {
        let mut entries = Vec::with_capacity(PLUGIN_PROMPT_COUNT);
        for idx in 0..PLUGIN_PROMPT_COUNT {
            entries.push(PromptEntry {
                prompt: Prompt::new(
                    format!("prompt.plugin.{idx:04}"),
                    Some(format!("plugin prompt {idx}")),
                    None,
                ),
                arguments_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "index": { "type": "integer" },
                    }
                }),
                completions: None,
            });
        }
        Ok(entries)
    }

    async fn get(
        &self,
        name: &str,
        args: Value,
        _params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::Assistant,
            format!("{name}:{args}"),
        )])
        .with_description("bench plugin prompt"))
    }
}

#[derive(Debug)]
struct BenchResourcePlugin;

#[async_trait]
impl ResourcePlugin for BenchResourcePlugin {
    fn name(&self) -> &'static str {
        "bench.resource"
    }

    async fn list(&self, _params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError> {
        let mut entries = Vec::with_capacity(PLUGIN_RESOURCE_COUNT);
        for idx in 0..PLUGIN_RESOURCE_COUNT {
            entries.push(ResourceEntry {
                resource: rmcp::model::RawResource::new(
                    format!("resource://plugin/item/{idx:04}"),
                    format!("plugin-item-{idx:04}"),
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
                text: "plugin-read".to_owned(),
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

fn build_filler_tools_block() -> String {
    let mut out = String::new();
    for idx in 0..FILLER_TOOL_COUNT {
        let _ = write!(
            out,
            "    - name: tool.filler.{idx:04}\n      description: filler tool {idx}\n      input_schema:\n        type: object\n        properties:\n          id:\n            type: integer\n      execute:\n        type: http\n        upstream: bench\n        method: GET\n        path: /filler/{idx:04}\n"
        );
    }
    out
}

fn build_inline_prompts_block() -> String {
    let mut out = String::new();
    for idx in 0..INLINE_PROMPT_COUNT {
        let _ = write!(
            out,
            "      - name: prompt.inline.{idx:04}\n        description: inline prompt {idx}\n        arguments_schema:\n          type: object\n          properties:\n            text:\n              type: string\n            index:\n              type: integer\n        template:\n          messages:\n            - role: assistant\n              content: \"inline prompt {idx}\"\n"
        );
    }
    out
}

fn build_inline_resources_block() -> String {
    let mut out = String::new();
    for idx in 0..INLINE_RESOURCE_COUNT {
        let _ = write!(
            out,
            "      - uri: resource://inline/item/{idx:04}\n        name: inline-item-{idx:04}\n        mime_type: text/plain\n        content:\n          text: \"inline item {idx}\"\n"
        );
    }
    out
}

fn build_templates_block(prefix: &str, count: usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let _ = write!(
            out,
            "      - uri_template: resource://{prefix}/template/{idx:04}/{{id}}\n        name: {prefix}-template-{idx:04}\n        mime_type: text/plain\n        arguments_schema:\n          type: object\n          properties:\n            id:\n              type: string\n"
        );
    }
    out
}

#[allow(clippy::too_many_lines)]
fn bench_config_yaml() -> String {
    let inline_prompts = build_inline_prompts_block();
    let inline_resources = build_inline_resources_block();
    let inline_templates = build_templates_block("inline", INLINE_TEMPLATE_COUNT);
    let plugin_templates = build_templates_block("plugin", PLUGIN_TEMPLATE_COUNT);
    let filler_tools = build_filler_tools_block();
    format!(
        r#"
version: 1
server:
  host: 127.0.0.1
  port: 31947
  endpoint_path: /mcp
  logging:
    level: error
  transport:
    mode: stdio
completion:
  enabled: true
  providers:
    - name: countries
      type: inline
      values:
        - us
        - uk
        - ua
        - ug
        - uz
    - name: resource_ids
      type: inline
      values:
        - item-001
        - item-002
        - item-010
        - item-100
prompts:
  notify_list_changed: false
  providers:
    - type: inline
      items:
{inline_prompts}      - name: prompt.complete.target
        description: completion target
        arguments_schema:
          type: object
          properties:
            country:
              type: string
          required:
            - country
        completions:
          country: countries
        template:
          messages:
            - role: assistant
              content: "Country {{country}}"
    - type: plugin
      plugin: bench.prompt
resources:
  notify_list_changed: false
  clients_can_subscribe: false
  providers:
    - type: inline
      items:
{inline_resources}      templates:
{inline_templates}    - type: plugin
      plugin: bench.resource
      templates:
{plugin_templates}      - uri_template: resource://plugin/template/0100/{{id}}
        name: plugin-template-complete-target
        mime_type: text/plain
        arguments_schema:
          type: object
          properties:
            id:
              type: string
          required:
            - id
        completions:
          id: resource_ids
tools:
  items:
    - name: tool.bench.no_schema
      description: tool benchmark no schema
      input_schema:
        type: object
        properties:
          value:
            type: string
      execute:
        type: plugin
        plugin: bench.tool
    - name: tool.bench.with_schema
      description: tool benchmark with output schema
      input_schema:
        type: object
        properties:
          value:
            type: string
      output_schema:
        type: object
        properties:
          plugin:
            type: string
          args:
            type: object
        required:
          - plugin
          - args
      execute:
        type: plugin
        plugin: bench.tool
{filler_tools}upstreams:
  bench:
    base_url: http://127.0.0.1:31998
plugins:
  - name: bench.tool
    type: tool
  - name: bench.prompt
    type: prompt
  - name: bench.resource
    type: resource
"#
    )
}

fn load_bench_config() -> rust_mcp_core::McpConfig {
    let path =
        std::env::temp_dir().join(format!("request_hotpath_baseline_{}.yaml", Uuid::new_v4()));
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

fn request_context(service: &BenchService, id: i64) -> RequestContext<RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(id),
        meta: Meta::default(),
        extensions: Extensions::default(),
    }
}

fn build_service(rt: &tokio::runtime::Runtime) -> BenchService {
    let config = load_bench_config();
    let plugins = match PluginRegistry::new()
        .register_tool(BenchToolPlugin)
        .and_then(|registry| registry.register_prompt(BenchPromptPlugin))
        .and_then(|registry| registry.register_resource(BenchResourcePlugin))
    {
        Ok(value) => value,
        Err(error) => panic!("failed to register benchmark plugins: {error}"),
    };
    let engine = match Engine::from_config(EngineConfig {
        config,
        plugins,
        list_refresh_handle: None,
    }) {
        Ok(value) => value,
        Err(error) => panic!("failed to build benchmark engine: {error}"),
    };
    rt.block_on(async {
        let (server_io, _client_io) = tokio::io::duplex(1024 * 1024);
        rmcp::service::serve_directly(engine, server_io, None)
    })
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

#[allow(clippy::too_many_lines)]
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
    let mut service = build_service(&rt);

    // Warmup pass to avoid cold-path skew in the measured samples.
    rt.block_on(async {
        let mut warmup_tool_args = serde_json::Map::new();
        warmup_tool_args.insert("value".to_owned(), Value::String("warmup".to_owned()));
        let _ = ServerHandler::call_tool(
            service.service(),
            CallToolRequestParams::new("tool.bench.with_schema").with_arguments(warmup_tool_args),
            request_context(&service, 1),
        )
        .await;

        let mut warmup_prompt_args = serde_json::Map::new();
        warmup_prompt_args.insert("text".to_owned(), Value::String("warmup".to_owned()));
        warmup_prompt_args.insert("index".to_owned(), Value::Number(219_u64.into()));
        let _ = ServerHandler::get_prompt(
            service.service(),
            {
                let mut request = GetPromptRequestParams::new("prompt.plugin.0219");
                request.arguments = Some(warmup_prompt_args);
                request
            },
            request_context(&service, 2),
        )
        .await;

        let _ = ServerHandler::read_resource(
            service.service(),
            ReadResourceRequestParams::new("resource://plugin/item/0219"),
            request_context(&service, 3),
        )
        .await;

        let _ = ServerHandler::complete(
            service.service(),
            CompleteRequestParams::new(
                Reference::for_prompt("prompt.complete.target"),
                ArgumentInfo {
                    name: "country".to_owned(),
                    value: "u".to_owned(),
                },
            ),
            request_context(&service, 4),
        )
        .await;
    });

    run_case("tools_call_with_output_schema", ITERATIONS, || {
        let mut args = serde_json::Map::new();
        args.insert("value".to_owned(), Value::String("abc".to_owned()));
        let result = rt.block_on(async {
            ServerHandler::call_tool(
                service.service(),
                CallToolRequestParams::new("tool.bench.with_schema").with_arguments(args),
                request_context(&service, 11),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("tools_call_with_output_schema failed: {error}"),
        }
    });

    run_case("tools_call_without_output_schema", ITERATIONS, || {
        let mut args = serde_json::Map::new();
        args.insert("value".to_owned(), Value::String("abc".to_owned()));
        let result = rt.block_on(async {
            ServerHandler::call_tool(
                service.service(),
                CallToolRequestParams::new("tool.bench.no_schema").with_arguments(args),
                request_context(&service, 12),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("tools_call_without_output_schema failed: {error}"),
        }
    });

    run_case("prompts_get_large_sets", ITERATIONS, || {
        let mut args = serde_json::Map::new();
        args.insert("text".to_owned(), Value::String("hello".to_owned()));
        args.insert("index".to_owned(), Value::Number(219_u64.into()));
        let result = rt.block_on(async {
            ServerHandler::get_prompt(
                service.service(),
                {
                    let mut request = GetPromptRequestParams::new("prompt.plugin.0219");
                    request.arguments = Some(args);
                    request
                },
                request_context(&service, 13),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("prompts_get_large_sets failed: {error}"),
        }
    });

    run_case("resources_read_uri_hit_large_sets", ITERATIONS, || {
        let result = rt.block_on(async {
            ServerHandler::read_resource(
                service.service(),
                ReadResourceRequestParams::new("resource://plugin/item/0219"),
                request_context(&service, 14),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("resources_read_uri_hit_large_sets failed: {error}"),
        }
    });

    run_case("resources_read_template_hit_large_sets", ITERATIONS, || {
        let result = rt.block_on(async {
            ServerHandler::read_resource(
                service.service(),
                ReadResourceRequestParams::new("resource://plugin/template/0100/item-100"),
                request_context(&service, 15),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("resources_read_template_hit_large_sets failed: {error}"),
        }
    });

    run_case("completion_prompt_argument", ITERATIONS, || {
        let result = rt.block_on(async {
            ServerHandler::complete(
                service.service(),
                CompleteRequestParams::new(
                    Reference::for_prompt("prompt.complete.target"),
                    ArgumentInfo {
                        name: "country".to_owned(),
                        value: "u".to_owned(),
                    },
                ),
                request_context(&service, 16),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("completion_prompt_argument failed: {error}"),
        }
    });

    run_case("completion_resource_template_argument", ITERATIONS, || {
        let result = rt.block_on(async {
            ServerHandler::complete(
                service.service(),
                CompleteRequestParams::new(
                    Reference::for_resource("resource://plugin/template/0100/{id}"),
                    ArgumentInfo {
                        name: "id".to_owned(),
                        value: "item-1".to_owned(),
                    },
                ),
                request_context(&service, 17),
            )
            .await
        });
        match result {
            Ok(value) => {
                black_box(value);
            }
            Err(error) => panic!("completion_resource_template_argument failed: {error}"),
        }
    });

    let _ = rt.block_on(service.close());
}
