# Plugin Guide

Plugins extend rust-mcp-core with custom behavior that cannot be expressed in configuration alone. This document covers the plugin traits, registration, the `PluginContext` API, and conventions plugin authors should follow.

Design boundary:
- This library is not a domain-specific toolbox. Plugins provide your app-specific integration logic.
- For dynamic data, plugin authors must implement their own change-detection strategy (filesystem watcher, polling, webhooks, cache invalidation, etc.) and call `request_list_refresh(...)` when needed.

## Plugin types

| Type | Trait | Purpose |
|------|-------|---------|
| Tool | `ToolPlugin` | Custom tool execution logic |
| Auth | `AuthPlugin` | Additional token validation beyond built-in JWT/introspection |
| Prompt | `PromptPlugin` | Dynamic prompt providers backed by external data |
| Resource | `ResourcePlugin` | Dynamic resource providers backed by external data |
| Completion | `CompletionPlugin` | Custom autocompletion sources for prompt/resource arguments |
| HTTP Router | `HttpRouterPlugin` | Custom Axum routes and middleware on the HTTP transport |

## Imports and trait entry points

All plugin traits are exported from the crate root, and MCP model types are exported from `rust_mcp_core::mcp`:

```rust
use rust_mcp_core::{
    AuthPlugin, CompletionPlugin, McpError, PluginCallParams, PluginContext,
    PromptPlugin, ResourcePlugin, ToolPlugin,
};
use rust_mcp_core::mcp::{
    CallToolResult, CompleteRequestParams, CompletionInfo, GetPromptResult,
    Prompt, ReadResourceResult, Resource,
};
```

`ToolPlugin`, `AuthPlugin`, `PromptPlugin`, `ResourcePlugin`, and `CompletionPlugin` are the primary extension interfaces.

## Registration and allowlisting

Any plugin that is **referenced by config** must be both **registered** in the `PluginRegistry` and **declared** in the config `plugins` array. If a referenced plugin is missing from either side, startup fails with a clear error.

```rust
// Registration in code
let plugins = PluginRegistry::new()
    .register_tool(MyToolPlugin)?;

// Declaration in config
// plugins:
//   - name: my.tool
//     type: tool
//     config:
//       timeout_ms: 5000
```

The `name` returned by the plugin's `name()` method must match the name in the config `plugins` entry. The config entry's `type` must match the plugin kind (`tool`, `auth`, `prompt`, `resource`, `completion`, `http_router`).

If a plugin is registered in code but not declared in config `plugins`, the server logs a warning and ignores that extra registration.

## Tool plugins

```rust
#[async_trait]
pub trait ToolPlugin: Send + Sync {
    fn name(&self) -> &str;

    async fn call(
        &self,
        args: Value,           // Parsed input arguments
        params: PluginCallParams, // Bundled config + execution context
    ) -> Result<CallToolResult, McpError>;
}
```

### Error return conventions

- Return `Ok(CallToolResult::error("message"))` for **tool execution failures** (user-facing errors like "item not found" or "validation failed"). These are normal tool results with `isError: true`.
- Return `Err(McpError)` for **internal execution faults** (database connection lost, invariant violation, panic-mapped failure). Core maps these to `CallToolResult { isError: true }`, with message visibility controlled by `server.errors.expose_internal_details`.
- Protocol/framework failures (unknown tool, malformed request, method-not-found) remain JSON-RPC errors.

### Config merging

Tool plugins receive a merged config value. The merge works as follows:

1. The base config comes from the `plugins[].config` entry in the config file.
2. The override config comes from `tools.items[].execute.config` on the specific tool definition.
3. If both are JSON objects, keys from the tool-level config override keys from the plugin-level config (shallow merge).
4. If the tool-level config is not an object (or is absent), the plugin-level config is used as-is.

```yaml
plugins:
  - name: reports.aggregate
    type: tool
    config:
      timeout_ms: 5000      # Base config for all tools using this plugin
      format: json

tools:
  items:
    - name: reports.daily
      execute:
        type: plugin
        plugin: reports.aggregate
        config:
          format: csv          # Overrides "json" for this specific tool
```

The plugin receives `params.config = {"timeout_ms": 5000, "format": "csv"}`.

### Accessing config and context

`PluginCallParams` bundles both runtime config and request context:

```rust
let PluginCallParams { config, ctx } = params;
```

- `config`: merged plugin config value (see merge rules above).
- `ctx`: `PluginContext` with cancellation/progress/logging/client feature helpers.

## PluginContext API

`PluginContext` is available on `PluginCallParams.ctx` for plugin calls and provides access to cancellation, progress, logging, client features, upstreams, and list refresh.

### Public fields

| Field | Type | Description |
|-------|------|-------------|
| `cancellation` | `CancellationToken` | Cancellation token for the current request (see below) |
| `progress` | `Option<ProgressToken>` | Progress token from request metadata, if the client sent one |
| `upstreams` | `Arc<HashMap<String, UpstreamConfig>>` | Configured upstream definitions from the config file |

Note: the raw request context is intentionally internal; use `PluginContext` helper methods instead of relying on rmcp request internals.

#### Built-in outbound HTTP path

- Use `ctx.send(...)` for plugin requests that target configured upstreams.
- Use `ctx.send_with(...)` when you need per-call auth overrides (`inherit`/`none`/`explicit`).
- Use `ctx.send_raw(...)` for low-level calls that should bypass upstream/default/retry policy.
- `send(...)` resolves the upstream base URL and applies timeout/max-response defaults in order: request -> upstream -> `outbound_http`.
- Runnable example: `examples/plugins-simple-outbound-http/README.md`.

```rust
use rust_mcp_core::OutboundHttpRequest;

let response = ctx
    .send(
        "partner_api",
        OutboundHttpRequest {
            method: "GET".to_owned(),
            url: "/partners".to_owned(),
            ..OutboundHttpRequest::default()
        },
    )
    .await?;
```

Per-call auth override example:

```rust
use rust_mcp_core::{OutboundHttpRequest, PluginSendAuthMode, PluginSendOptions};

let response = ctx
    .send_with(
        "partner_api",
        OutboundHttpRequest {
            method: "GET".to_owned(),
            url: "/partners".to_owned(),
            ..OutboundHttpRequest::default()
        },
        PluginSendOptions {
            auth: PluginSendAuthMode::None,
        },
    )
    .await?;
```

`PluginSendAuthMode` options:
- `Inherit` (default): apply upstream bearer/basic/oauth2 auth.
- `None`: skip upstream auth for this request.
- `Explicit { authorization }`: set `Authorization` explicitly for this request.

#### HTTP responsibility boundary

- `send(...)` and `send_with(...)` use the same core outbound stack as built-in HTTP tools.
- `send_raw(...)` uses the shared HTTP client directly and intentionally bypasses upstream/default/retry policy.
- Plugins that use core helpers inherit core outbound guardrails.
- Plugins that construct their own HTTP clients bypass core outbound policy controls and are responsible for equivalent safeguards.

### Cancellation

The `cancellation` field is a `tokio_util::sync::CancellationToken`. Its behavior depends on the tool's `cancellable` setting (defaults to `true`):

**When `cancellable = true` (default):**
- The token is the live cancellation token from the MCP request.
- The engine races the plugin's `call()` against the token. If the client cancels the request, the engine aborts the plugin call and returns `isError: true` with message `"request cancelled"`.
- Plugins that perform long-running work in loops **should** check `ctx.cancellation.is_cancelled()` to cooperate with cancellation and clean up promptly, but this is optional. The engine handles the abort externally.

**When `cancellable = false`:**
- The engine replaces the token with a fresh one that never fires.
- The plugin runs to completion regardless of client cancellation.
- Use this for operations that must not be interrupted (e.g., writes that must be atomic).

### Progress notifications

```rust
ctx.notify_progress(25.0, Some(100.0), Some("Indexing...".to_string())).await?;
```

- Returns `Ok(false)` silently if the client did not send a `progressToken` in request metadata.
- Rate-limited by `progress.notification_interval_ms` config. Calls within the interval window are silently skipped.
- Progress values must strictly increase per request. Non-increasing or non-finite values return `invalid_params`.

### Logging

```rust
ctx.log_event(LogEventParams {
    level: LoggingLevel::Info,
    message: "operation completed".to_owned(),
    data: Some(json!({"count": 42})),
    channels: &[LogChannel::Server, LogChannel::Client],
}).await?;
```

- `LogChannel::Server` writes to the server's tracing output (controlled by `server.logging.level`).
- `LogChannel::Client` sends an MCP `notifications/message` to the connected client (controlled by `client_logging.level` config and runtime `logging/setLevel` adjustments).
- You can send to both channels, one, or neither.

### Client features

These methods are available when the `client_features` feature is enabled. Each method checks both the server-side config (`client_features.X.enabled`) and the client's advertised capabilities before making the request. If either check fails, the method returns an error.

```rust
// Request filesystem roots from the client
let roots = ctx.request_roots().await?;

// Request LLM generation from the client
let result = ctx.request_sampling(params).await?;

// Request structured user input (form mode) or out-of-band interaction (URL mode)
let result = ctx.request_elicitation(params).await?;
```

For sampling, the framework also validates whether tool use is allowed (`client_features.sampling.allow_tools`) and whether the client supports sampling tools.

For elicitation, the framework validates the request mode (form vs URL) against both the server config (`client_features.elicitation.mode`) and the client's elicitation capabilities.

### List refresh

Plugins that manage dynamic data can trigger list-changed notifications:

```rust
ctx.request_list_refresh(ListFeature::Tools).await?;
```

Behavior contract:

- Returns `Ok(true)` when list payload changed and cache was updated.
- Returns `Ok(false)` when payload did not change (or stale refresh result is discarded during reload).
- Returns `Err(invalid_request(\"list refresh unavailable\"))` when no runtime refresh handle is attached.
- If changed, list-changed notifications are emitted only when corresponding notify flags are enabled:
  - tools: `tools.notify_list_changed`
  - prompts: `prompts.notify_list_changed`
  - resources: `resources.notify_list_changed`

Supported features: `Tools`, `Prompts`, `Resources`.

Important: `request_list_refresh(...)` does **not** discover changes by itself. It only asks the runtime to recompute/diff/notify. Your plugin must decide **when** to call it.

### Outbound OAuth2 helpers (`http_tools`)

When `http_tools` is enabled, tool/prompt/resource/completion plugins can reuse upstream OAuth2 token acquisition from `PluginContext`:

```rust
let token = params.ctx.upstream_access_token("partner_api", false).await?;
let (header_name, header_value) = params
    .ctx
    .upstream_bearer_header("partner_api", false)
    .await?;
```

Rules:
- The upstream name must reference `upstreams.<name>.auth.type: oauth2`.
- `upstream_access_token` returns a redacted wrapper (`PluginAccessToken`), not raw secrets in `Debug`.
- `upstream_bearer_header` returns `("Authorization", "Bearer <token>")`.
- `force_refresh=true` bypasses the fresh cache check and forces one token refresh attempt.

Error semantics:
- Unknown upstream -> `invalid_request("upstream '<name>' not found")`
- Non-oauth2 upstream -> `invalid_request("upstream '<name>' does not use oauth2 auth")`
- OAuth resolution/exchange errors propagate from the outbound OAuth2 path.

Runnable example: `examples/plugins-tool-oauth-helper/README.md`.

## Auth plugins

```rust
#[async_trait]
pub trait AuthPlugin: Send + Sync {
    fn name(&self) -> &str;

    async fn validate(&self, params: AuthPluginValidateParams<'_>) -> AuthPluginDecision;
}
```

Auth plugins are regular auth providers (`server.auth.providers[].type: plugin`) in the ordered provider chain.
They receive `AuthPluginValidateParams` (token, claims, headers, plugin config) and return:
- `AuthPluginDecision::Accept` to allow the request,
- `AuthPluginDecision::Reject` to deny it,
- `AuthPluginDecision::Abstain` to continue to the next provider.

## Prompt plugins

```rust
#[async_trait]
pub trait PromptPlugin: Send + Sync {
    fn name(&self) -> &str;

    async fn list(&self, params: PluginCallParams) -> Result<Vec<PromptEntry>, McpError>;

    async fn get(
        &self,
        name: &str,
        args: Value,
        params: PluginCallParams,
    ) -> Result<GetPromptResult, McpError>;
}
```

Prompt plugins provide dynamic prompt lists and handle `prompts/get` requests. The `list()` method returns prompt metadata including argument schemas and completion bindings. The `get()` method renders a specific prompt with the provided arguments.

## Resource plugins

```rust
#[async_trait]
pub trait ResourcePlugin: Send + Sync {
    fn name(&self) -> &str;

    async fn list(&self, params: PluginCallParams) -> Result<Vec<ResourceEntry>, McpError>;

    async fn read(&self, uri: &str, params: PluginCallParams) -> Result<ReadResourceResult, McpError>;

    async fn subscribe(&self, uri: &str, params: PluginCallParams) -> Result<(), McpError>;

    async fn unsubscribe(&self, uri: &str, params: PluginCallParams) -> Result<(), McpError>;
}
```

Resource plugins provide dynamic resource lists, handle `resources/read`, and manage subscriptions. The framework calls `subscribe`/`unsubscribe` when clients use `resources/subscribe` and `resources/unsubscribe`.

## Completion plugins

```rust
#[async_trait]
pub trait CompletionPlugin: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(
        &self,
        req: &CompleteRequestParams,
        params: PluginCallParams,
    ) -> Result<CompletionInfo, McpError>;
}
```

Completion plugins provide autocompletion values for prompt and resource template arguments. They are wired via `completion.providers[].type: plugin`.

## MCP model cheat sheet (plugin-facing)

These are the most common rmcp model types plugin authors touch, re-exported via `rust_mcp_core::mcp`:

| Type | Key fields / notes |
|------|---------------------|
| `Prompt` | `name: String`, `title: Option<String>`, `description: Option<String>`, `arguments: Option<Vec<PromptArgument>>`, `icons`, `_meta` |
| `Resource` (`Annotated<RawResource>`) | `uri: String`, `name: String`, `title`, `description`, `mime_type`, `size: Option<u32>`, `icons`, `_meta` |
| `ResourceTemplate` (`Annotated<RawResourceTemplate>`) | `uri_template: String`, `name: String`, `title`, `description`, `mime_type`, `icons`, `_meta` |
| `ReadResourceResult` | wraps resource contents (`text`/`blob`) returned by `ResourcePlugin::read` |

Usage guidance:

- Prefer importing from `rust_mcp_core::mcp` instead of adding a direct `rmcp` dependency in plugin crates.
- `size` is bytes and typed as `Option<u32>`.
- Serialized wire field names are camelCase (e.g. `mimeType`) even though Rust fields are snake_case.

## HTTP router plugins

HTTP router plugins add custom Axum routes and middleware to the HTTP transport. They are only available when the `streamable_http` feature is enabled. Common use cases include health check endpoints, CORS headers, request logging, rate limiting, and applying auth to custom routes.

```rust
pub trait HttpRouterPlugin: Send + Sync {
    fn name(&self) -> &str;

    fn apply(
        &self,
        ctx: &RuntimeContext,
        targets: &[HttpRouterTarget],
        config: &Value,
    ) -> Result<Vec<HttpRouterOp>, McpError>;
}
```

Each plugin receives a `RuntimeContext` with auth summary information and returns one `HttpRouterOp` per target. Ops are either `Route(Router)` (mount a new Axum route) or `Wrap(RouterTransform)` (apply middleware to an existing path).

### Target types

Targets are configured in the config `plugins[].targets` array:

```yaml
plugins:
  - name: healthz
    type: http_router
    targets:
      - type: route
        path: /health          # Mount a new route
      - type: wrap
        path: /mcp             # Apply middleware to the MCP endpoint
      - type: wrap
        path: "*"              # Apply middleware to all routes
```

- **Route targets** mount a new Axum `Router` at the given path. The path must not collide with the MCP endpoint or OAuth metadata paths.
- **Wrap targets** apply a `RouterTransform` (a function that takes a `Router` and returns a `Router`) to an existing path. Use `"*"` to wrap all routes. Wraps are applied outermost-first in plugin declaration order.

### Auth on custom routes

The `RuntimeContext` provides `ctx.auth_wrap()` which returns the server's configured auth middleware as a `RouterTransform`. Use this to protect custom routes with the same auth rules as the MCP endpoint:

```rust
fn apply(&self, ctx: &RuntimeContext, targets: &[HttpRouterTarget], _config: &Value)
    -> Result<Vec<HttpRouterOp>, McpError>
{
    let mut ops = Vec::new();
    for target in targets {
        match target.target_type {
            HttpRouterTargetType::Route => {
                let router = Router::new().route("/", get(|| async { "ok" }));
                ops.push(HttpRouterOp::Route(router));
            }
            HttpRouterTargetType::Wrap => {
                let wrap = ctx.auth_wrap().ok_or_else(|| {
                    McpError::invalid_request("auth not enabled".into(), None)
                })?;
                ops.push(HttpRouterOp::Wrap(wrap));
            }
        }
    }
    Ok(ops)
}
```

`auth_wrap()` returns `None` when auth is not active (for example `server.auth` is absent or `server.auth.enabled=false`).

### Route collision detection

Custom route paths must not collide with:
- The MCP endpoint path (default `/mcp`)
- The OAuth protected resource metadata paths (when auth is enabled)
- Other plugin route paths

Collisions are detected at startup and cause a clear error.

For a working example with a health check route and wrap middleware, see [plugins-router-http](../examples/plugins-router-http/README.md).
