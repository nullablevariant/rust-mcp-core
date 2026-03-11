# rust-mcp-core

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-837_passed-brightgreen.svg)]()
[![Coverage](https://img.shields.io/badge/coverage-95.20%25-brightgreen.svg)]()
[![cargo-deny](https://img.shields.io/badge/cargo--deny-passing-brightgreen.svg)]()
[![cargo-audit](https://img.shields.io/badge/cargo--audit-passing-brightgreen.svg)]()

A config-driven MCP server core built on the official Rust SDK ([rmcp](https://github.com/modelcontextprotocol/rust-sdk)). Define tools, auth, prompts, resources, and HTTP behavior in YAML or JSON configuration -- the library handles execution, validation, and protocol compliance with minimal Rust code.

Fully implements the [Model Context Protocol specification (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25).

## Tested AI CLI compatibility

This library has been tested and verified with:

- Claude Code
- Codex CLI
- Gemini CLI

## What this is for

- **Reduced boilerplate.** Stand up a spec-compliant MCP server by writing configuration instead of protocol plumbing. A working server with HTTP tools needs only a YAML file and a few lines of Rust.
- **Built-in HTTP API tooling.** Define outbound HTTP tool calls entirely in config with URL templating, header injection, query parameters, and structured response mapping.
- **Authentication out of the box.** Supports inbound bearer token, JWT/JWKS validation, and OAuth token introspection with scope enforcement and `WWW-Authenticate` challenges. Also supports outbound upstream auth for HTTP tools (`none`, `bearer`, `basic`, `oauth2`) including OAuth2 client-credentials/refresh-token grants with optional mTLS at the token endpoint.
- **Extensible plugin system.** When config-driven behavior is not enough, register plugins for custom tool logic, auth validation, prompt/resource providers, completion providers, and HTTP router extensions.
- **Both transports.** Works with stdio and streamable HTTP transports.
- **Works with the official SDK.** This library builds on [rmcp](https://github.com/modelcontextprotocol/rust-sdk) and uses its transport runtime, `ServerHandler` trait, and MCP type definitions directly -- no reimplementation.

## What this is not

- **Not a toolbox.** This library does not include built-in tools like file servers, database connectors, or shell executors. The only built-in execution type is outbound HTTP calls. Any other tool behavior must be provided through a tool plugin.

## How this complements the rmcp SDK

The [rmcp SDK](https://github.com/modelcontextprotocol/rust-sdk) provides low-level MCP protocol primitives: transport, message framing, the `ServerHandler` trait, and type definitions. rust-mcp-core builds on top of that to provide a config-driven server framework.

What the SDK provides (used directly, not reimplemented):
- Transport runtime (stdio + streamable HTTP)
- `ServerHandler` trait and JSON-RPC dispatch
- All MCP model types (`Tool`, `Prompt`, `Resource`, `Task`, etc.)
- Default handler implementations (e.g., `ping`)

What rust-mcp-core adds:
- Config loading with `${env:ENV}` expansion and JSON schema validation
- Tool execution engine (HTTP tools with templating + plugin tools)
- Output schema validation and structured content rendering
- Plugin registries (tool, auth, prompt, resource, completion, HTTP router)
- Config-driven prompts, resources, and completion providers
- Auth middleware (bearer, JWT/JWKS, introspection, scope enforcement)
- Task store with peer isolation, TTL, cooperative cancellation, and status notifications

The complementary relationship is clearest with auth. The SDK provides **client-side** OAuth (PKCE flows, token acquisition, credential storage, automatic refresh). rust-mcp-core provides **server-side** auth (token validation, scope enforcement, `WWW-Authenticate` challenges, and the protected resource metadata endpoint). An rmcp client obtains a token and sends it; a rust-mcp-core server receives and validates it.

## MCP specification compliance

This library implements the following capabilities from the [MCP 2025-11-25 specification](https://modelcontextprotocol.io/specification/2025-11-25):

### Server capabilities

| Capability | Description |
|------------|-------------|
| **Tools** | Config-driven and plugin-driven tool definitions. Supports input/output schema validation, list-changed notifications, and pagination. |
| **Prompts** | Inline (config-driven) and plugin-driven prompt providers with argument validation, template rendering, and list-changed notifications. |
| **Resources** | Inline and plugin-driven resource providers with subscribe/unsubscribe support and list-changed notifications. |
| **Completion** | Autocompletion for prompt and resource template arguments from inline value lists or plugin sources. |
| **Logging** | Structured log messages via `notifications/message` with syslog severity levels. Clients control notification threshold via `logging/setLevel`. |
| **Progress** | Long-running operation tracking via `notifications/progress` with rate limiting and monotonic progress enforcement. |
| **Cancellation** | In-progress request termination. Cancellable tools are aborted automatically; non-cancellable tools receive a fresh token and run to completion. |
| **Tasks** | Experimental task utility for long-running operations. Supports task-augmented tool calls, polling, deferred result retrieval, cooperative cancellation, TTL, peer isolation, and status notifications. |
| **Pagination** | Cursor-based pagination for `tools/list` and other list operations. |

### Server-initiated client features

These require a registered plugin to invoke. The framework handles config validation and client capability negotiation; plugin code calls helpers via `params.ctx` (`params: PluginCallParams`).

| Feature | Description |
|---------|-------------|
| **Sampling** | Request LLM text/image/audio generation from the client, optionally with tool use. |
| **Roots** | Query the client for filesystem root boundaries. |
| **Elicitation** | Request structured user input (form mode) or trigger out-of-band interactions like OAuth flows (URL mode). |

### Protocol fundamentals

- Capability negotiation during `initialize` handshake
- Ping/pong for connection health
- Feature-gate enforcement: disabled features return `method-not-found` and are omitted from capability advertisement

## Compile-time features

All features are enabled by default. Disable with `default-features = false` and enable selectively.

| Feature | Description | Implies |
|---------|-------------|---------|
| `streamable_http` | HTTP transport (`server.transport.mode=streamable_http`) and HTTP router plugin surface | -- |
| `http_hardening` | Streamable HTTP hardening middleware (`max_request_bytes`, inbound rate limits, session abuse controls, panic/sensitive-header guards) | `streamable_http` |
| `auth` | Auth middleware: bearer, JWT/JWKS, OAuth introspection, scope enforcement | `streamable_http` |
| `http_tools` | Built-in outbound HTTP tool execution (`tools.items[].execute.type=http`) plus upstream auth (`none`, `bearer`, `basic`, `oauth2`) | -- |
| `prompts` | `prompts/list` + `prompts/get` capability | -- |
| `resources` | `resources/list`, `resources/read`, `resources/templates/list`, subscribe/unsubscribe | -- |
| `completion` | `completion/complete` for prompt/resource argument autocompletion | -- |
| `client_logging` | `logging/setLevel` + `notifications/message` | -- |
| `progress_utility` | `notifications/progress` via `params.ctx.notify_progress(...)` | -- |
| `tasks_utility` | Experimental: task-augmented `tools/call`, `tasks/get`, `tasks/list`, `tasks/cancel` | -- |
| `client_features` | Server-initiated client helpers via `params.ctx`: `request_roots()`, `request_sampling()`, `request_elicitation()` | -- |

Minimal build (stdio + plugin tools only):

```bash
cargo build --no-default-features
```

If config references a disabled feature, startup fails with a clear error message.

## Installation

From [crates.io](https://crates.io/crates/rust-mcp-core):

```toml
[dependencies]
rust-mcp-core = "0.1"
```

From GitHub:

```toml
[dependencies]
rust-mcp-core = { git = "https://github.com/nullablevariant/rust-mcp-core" }
```

## Quick start

### Configuration

Create a YAML config file. This example defines one HTTP tool and one inline prompt over streamable HTTP transport. Stdio transport is also supported by setting `transport.mode: stdio`.
If you want a full copy/paste starter with every supported field, use [`mcp_config.template.yml`](mcp_config.template.yml).

```yaml
version: 1
server:
  host: 0.0.0.0
  port: 3000
  endpoint_path: /mcp
  logging:
    level: info
  transport:
    mode: streamable_http  # also supports: stdio
  auth:
    enabled: false

client_logging:
  level: info

upstreams:
  api:
    base_url: ${env:API_BASE_URL}

tools:
  items:
    - name: api.list_items
      description: List items from the API
      input_schema:
        type: object
        properties:
          query:
            type: string
      execute:
        type: http
        upstream: api
        method: GET
        path: /items
        query:
          q: "${query}"
      response:
        type: structured
        template:
          items: "${$.items}"
        fallback: text
```

### Usage

```rust
use std::path::PathBuf;
use rust_mcp_core::{load_mcp_config_from_path, runtime, PluginRegistry};
use rust_mcp_core::McpError;

#[tokio::main]
async fn main() -> Result<(), McpError> {
    let config = load_mcp_config_from_path(PathBuf::from("config/mcp_config.yml"))?;
    let plugins = PluginRegistry::new();
    runtime::run_from_config(config, plugins).await
}
```

With a custom tool plugin:

```rust
use rust_mcp_core::{load_mcp_config_from_path, runtime, McpError, PluginRegistry};

#[tokio::main]
async fn main() -> Result<(), McpError> {
    let config = load_mcp_config_from_path("config/mcp_config.yml".into())?;
    let plugins = PluginRegistry::new()
        .register_tool(MyToolPlugin)?;
    runtime::run_from_config(config, plugins).await
}
```

Any plugin referenced by config (for example via `tools.items[].execute.plugin` or provider plugin fields) must be both declared in config `plugins[]` and registered in `PluginRegistry`. Extra registered plugins that are not declared are ignored with a warning. See [Plugin Guide](docs/PLUGINS.md) for the full plugin contract.

### Config reload (consumer-managed)

`runtime::run_from_config(...)` is a convenience entrypoint and does **not** expose reload control.

If you need config reload:
- use `runtime::build_runtime(...)`,
- keep the returned `runtime` handle,
- load updated config input yourself,
- call `runtime.reload_config(new_config).await`.

`rust-mcp-core` does not automatically watch config files or trigger reloads.

```rust
use std::path::PathBuf;
use rust_mcp_core::{load_mcp_config_from_path, runtime, McpError, PluginRegistry};

#[tokio::main]
async fn main() -> Result<(), McpError> {
    let initial = load_mcp_config_from_path(PathBuf::from("config/mcp_config.yml"))?;
    let runtime = runtime::build_runtime(initial, PluginRegistry::new()).await?;

    // Consumer-owned trigger (file watcher, signal, admin endpoint, etc.)
    let updated = load_mcp_config_from_path(PathBuf::from("config/mcp_config.reload.yml"))?;
    runtime.reload_config(updated).await?;

    runtime.run().await
}
```

### Upstream auth for HTTP tools

`upstreams.<name>.auth` controls outbound auth for `tools.items[].execute.type=http`:

- `type: none` -> no auth header injected.
- `type: bearer` -> injects `Authorization: Bearer <token>`.
- `type: basic` -> injects HTTP Basic auth.
- `type: oauth2` -> acquires/caches access tokens via `client_credentials` or `refresh_token` grant, injects bearer token, and can retry once on `401` after forced refresh.

Token-endpoint secrets support `inline`, `env`, and `path` sources. Optional token-endpoint mTLS is configured via `auth.mtls` (`client_cert`, `client_key`, optional `ca_cert`). See [Auth](docs/AUTH.md) and [Config Schema](docs/CONFIG_SCHEMA.md) for full field-level details.

### Streamable HTTP request flow (manual clients)

When `server.transport.mode=streamable_http`, manual clients (for example `curl`) should follow this flow:

- Send `initialize` first, capture `MCP-Session-Id` from response headers when sessions are enabled.
- Send `notifications/initialized` on the same endpoint with the same session header.
- Include `Accept: application/json, text/event-stream` on POST requests.
- If a client sends `MCP-Protocol-Version` on HTTP requests:
  - default `server.transport.streamable_http.protocol_version_negotiation.mode: strict` keeps the header; unsupported values return HTTP `400`.
  - `mode: negotiate` keeps known RMCP versions and drops unknown versions before rmcp validation.
- Reuse the `MCP-Session-Id` header on subsequent requests in session modes that require it.
- Expect streamable HTTP responses to be SSE-framed.

Why this exists:
- `rust-mcp-core` is coupled to RMCP's known protocol version set.
- Some AI clients send a newer date-stamped `MCP-Protocol-Version` header than the bundled RMCP supports.
- In `strict` mode this fails fast with `400` (explicit mismatch).
- In `negotiate` mode unknown header values are removed so requests can continue without forcing a reverse proxy/header rewrite layer.
- This is a compatibility layer for HTTP headers only; MCP capability/version negotiation still happens in normal `initialize` flow.

For client-specific interoperability constraints (for example schema subset
limits, `structuredContent` shape expectations, protocol-header normalization,
and `Accept` requirements), see
[AI Client Compatibility](docs/AI_CLIENT_COMPATIBILITY.md).

### Streamable HTTP hardening (`http_hardening`)

With `http_hardening` enabled, core can enforce:
- inbound request body cap (`max_request_bytes`, HTTP `413` on exceed),
- general inbound rate limiting (`rate_limit`, HTTP `429` on exceed),
- session abuse controls (`max_sessions`, `idle_ttl_secs`, `max_lifetime_secs`, `creation_rate`),
- panic-to-500 and sensitive-header transport guards.

See [Transport](docs/TRANSPORT.md) and [Config Schema](docs/CONFIG_SCHEMA.md) for field-level behavior and validation rules.

## Plugin logging helper

Tool plugins can emit server logs and/or MCP client notifications:

```rust
let ctx = params.ctx;
ctx.log_event(rust_mcp_core::LogEventParams {
    level: rust_mcp_core::mcp::LoggingLevel::Info,
    message: "synced records".to_owned(),
    data: Some(serde_json::json!({"count": 42})),
    channels: &[rust_mcp_core::LogChannel::Server, rust_mcp_core::LogChannel::Client],
}).await?;
```

- `LogChannel::Server` writes to the server's tracing output (controlled by `server.logging.level`).
- `LogChannel::Client` sends an MCP `notifications/message` to the client (controlled by `client_logging.level` and `logging/setLevel`).
- `logging/setLevel` does **not** change server tracing verbosity. It only updates client notification filtering.
- On config reload, `server.logging.level` updates are applied when rust-mcp-core owns the global tracing subscriber. If a host app already set the global subscriber, rust-mcp-core logs a warning and cannot override it.

## Plugin outbound OAuth2 helper (`http_tools`)

When `http_tools` is enabled and an upstream uses `auth.type: oauth2`, plugins can reuse native token acquisition via `PluginContext`:

```rust
let token = params.ctx.upstream_access_token("partner_api", false).await?;
let (name, value) = params
    .ctx
    .upstream_bearer_header("partner_api", false)
    .await?;
```

`upstream_bearer_header` returns `("Authorization", "Bearer <token>")`. `force_refresh=true` forces one refresh attempt through the same outbound OAuth2 token manager used by HTTP tools.

For outbound requests to configured upstreams, plugins can use the built-in helper path:

```rust
let response = params.ctx.send(
    "partner_api",
    rust_mcp_core::OutboundHttpRequest {
        method: "GET".to_owned(),
        url: "/partners".to_owned(),
        ..rust_mcp_core::OutboundHttpRequest::default()
    },
).await?;
```

This path applies the same outbound default-resolution behavior as built-in HTTP tools.
Use `send_with(...)` for per-call auth overrides and `send_raw(...)` when you intentionally want to bypass upstream/default/retry policy.
`send_with(...)` supports `PluginSendAuthMode::{Inherit, None, Explicit}`.

See `examples/plugins-tool-oauth-helper/README.md` for a complete runnable flow.

## Documentation

| Document | Description |
|----------|-------------|
| [Plugin Guide](docs/PLUGINS.md) | Plugin trait signatures/imports, `PluginContext` behavior (including `request_list_refresh`), and MCP model cheat sheet |
| [Auth](docs/AUTH.md) | Auth modes, token validation, OAuth metadata, TLS deployment |
| [Tasks](docs/TASKS.md) | Task utility behavior, `task_support` modes, SDK comparison |
| [Transport](docs/TRANSPORT.md) | Streamable HTTP options, session modes, protocol negotiation, and hardening controls |
| [Config Schema](docs/CONFIG_SCHEMA.md) | Full field reference, defaults, validation rules, env expansion |
| [Config Reload](docs/CONFIG_RELOAD.md) | Consumer-managed reload flow, trigger patterns, and failure semantics |
| [AI Client Compatibility](docs/AI_CLIENT_COMPATIBILITY.md) | Practical MCP client constraints and compatibility patterns |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Common runtime/client errors and concrete fixes |

## Examples

Runnable examples live under [`examples/`](examples/) with per-example configs in
`examples/<name>/config/mcp_config.yml`.

- Browse all examples: [examples/README.md](examples/README.md)
- Minimal core: [examples/core-minimal/README.md](examples/core-minimal/README.md)
- Auth flows: [auth-bearer](examples/auth-bearer/README.md), [auth-oauth-jwt](examples/auth-oauth-jwt/README.md), [auth-oauth-introspection](examples/auth-oauth-introspection/README.md), [auth-all-mode](examples/auth-all-mode/README.md)
- HTTP tools: [tools-web-search](examples/tools-web-search/README.md), [tools-crud-http](examples/tools-crud-http/README.md), [tools-http-post](examples/tools-http-post/README.md), [tools-templating](examples/tools-templating/README.md), [tools-output-modes](examples/tools-output-modes/README.md), [tools-upstream-auth](examples/tools-upstream-auth/README.md), [tools-rich-content](examples/tools-rich-content/README.md)
- Plugin wiring: [plugins-tool-custom](examples/plugins-tool-custom/README.md), [plugins-tool-filesystem](examples/plugins-tool-filesystem/README.md), [plugins-auth-custom](examples/plugins-auth-custom/README.md), [plugins-router-http](examples/plugins-router-http/README.md), [plugins-simple-outbound-http](examples/plugins-simple-outbound-http/README.md)
- Prompts/resources/tasks/client features: [prompts-inline-plugin](examples/prompts-inline-plugin/README.md), [resources-inline-plugin](examples/resources-inline-plugin/README.md), [resources-subscribe-updated](examples/resources-subscribe-updated/README.md), [utility-tasks](examples/utility-tasks/README.md), [utility-tasks-advanced](examples/utility-tasks-advanced/README.md), [plugins-tool-client-features](examples/plugins-tool-client-features/README.md), [plugins-tool-client-features-advanced](examples/plugins-tool-client-features-advanced/README.md)
- Utility behavior: [utility-logging](examples/utility-logging/README.md), [utility-progress](examples/utility-progress/README.md), [utility-cancellation](examples/utility-cancellation/README.md), [utility-completion](examples/utility-completion/README.md), [utility-pagination](examples/utility-pagination/README.md), [utility-list-changed](examples/utility-list-changed/README.md)

## Config schema

See the full schema and validation rules in [CONFIG_SCHEMA.md](docs/CONFIG_SCHEMA.md).
