# Transport

rust-mcp-core supports two transport modes: stdio and streamable HTTP.

## Stdio

```yaml
server:
  transport:
    mode: stdio
```

Stdio transport reads JSON-RPC messages from stdin and writes responses to stdout. No additional configuration is needed. This is the simplest transport and works without the `streamable_http` feature.

## Streamable HTTP

```yaml
server:
  host: 0.0.0.0
  port: 3000
  endpoint_path: /mcp
  transport:
    mode: streamable_http
    streamable_http:
      enable_get_stream: true
      enable_sse_resumption: false
      session_mode: optional
      allow_delete_session: false
      sse_keep_alive_ms: 15000
      sse_retry_ms: 2000
      protocol_version_negotiation:
        mode: strict # strict | negotiate
      hardening:
        # Requires `http_hardening` compile-time feature.
        max_request_bytes: 1048576
        catch_panics: true
        sanitize_sensitive_headers: true
        session:
          max_sessions: 2048
          idle_ttl_secs: 900
          max_lifetime_secs: 86400
          creation_rate:
            enabled: true
            global: { capacity: 60, refill_per_sec: 1 }
        rate_limit:
          enabled: true
          global: { capacity: 200, refill_per_sec: 20 }
          per_ip: { capacity: 20, refill_per_sec: 2, key_source: peer_addr }
```

Requires the `streamable_http` compile-time feature.

### Endpoint behavior

The MCP endpoint (default `/mcp`) accepts:

- **POST**: JSON-RPC messages. This is the primary request method.
- **GET**: Opens an SSE stream for server-to-client notifications. Returns HTTP 405 when `enable_get_stream` is false.
- **DELETE**: Terminates the current session. Returns HTTP 405 when `allow_delete_session` is false.

### Manual client requirements

For direct HTTP clients (for example `curl`), streamable HTTP requires MCP handshake sequencing:

- Send `initialize` before calling other methods.
- Send `notifications/initialized` after a successful `initialize`.
- Include `Accept: application/json, text/event-stream` on POST requests.
- For post-init requests, `MCP-Protocol-Version` is optional.
  - `mode: strict` (default): if header is present and unsupported, server returns HTTP 400.
  - `mode: negotiate`: unsupported header values are dropped before rmcp validation.
- When sessions are enabled, capture and reuse `MCP-Session-Id` on subsequent requests.
- Expect SSE-framed responses from the streamable HTTP endpoint.

### Protocol version compatibility (rmcp-bound)

`MCP-Protocol-Version` is validated by rmcp transport before normal request
handling. In `protocol_version_negotiation.mode = negotiate`, rust-mcp-core
drops unsupported header values before the request reaches rmcp.

For client-specific compatibility patterns, see
[AI Client Compatibility](AI_CLIENT_COMPATIBILITY.md).

### Session modes

Configured via `streamable_http.session_mode`:

| Mode | Behavior |
|------|----------|
| `optional` | Sessions are created but the `MCP-Session-Id` header is not required on subsequent requests. Default. |
| `required` | `MCP-Session-Id` is required on GET and DELETE requests. Returns HTTP 400 when missing. |
| `none` | Stateless mode. No session management. |

### SSE configuration

- `sse_keep_alive_ms`: Interval for SSE keep-alive comments to prevent connection timeout. Set to `null` to disable.
- `sse_retry_ms`: The `retry` field sent in SSE events, telling clients how long to wait before reconnecting.
- `enable_sse_resumption`: When false, GET requests with `Last-Event-ID` header return HTTP 400.

### Route policies

The streamable HTTP transport enforces method restrictions at the route level:

- POST is always allowed on the MCP endpoint.
- GET is rejected with 405 when `enable_get_stream` is false.
- DELETE is rejected with 405 when `allow_delete_session` is false.
- SSE resumption (GET with `Last-Event-ID`) is rejected with 400 when `enable_sse_resumption` is false.

### HTTP hardening (`http_hardening` feature)

When compiled with `http_hardening`, streamable HTTP can enforce transport abuse controls via `server.transport.streamable_http.hardening`.

Defaults when `http_hardening` is enabled (even if `hardening` is omitted):
- `max_request_bytes = 1048576` (1 MiB body cap)
- `catch_panics = true` (panic-to-500 guard)
- `sanitize_sensitive_headers = true` (mark sensitive headers for transport logging)

Notes:
- `catch_panics` is a transport-layer panic guard for router/middleware paths.
- Tool execution panics are handled at the core execution boundary and converted to terminal MCP tool errors (`isError: true`) so streamable HTTP requests do not hang without a terminal payload.

Presence-based controls (disabled unless configured):
- `hardening.session` (`max_sessions`, `idle_ttl_secs`, `max_lifetime_secs`)
- `hardening.session.creation_rate` (session-create limiter)
- `hardening.rate_limit` (general inbound request limiter)

Behavior:
- Oversized inbound body returns HTTP `413 Payload Too Large`.
- Inbound rate-limit exceed returns HTTP `429 Too Many Requests`.
- `session_mode: none` rejects `hardening.session` config at load/validation time.
- `hardening.rate_limit` / `hardening.session.creation_rate` require at least one bucket (`global` or `per_ip`) when enabled.

### Scope of hardening

Core transport hardening applies to the MCP endpoint path handled by streamable HTTP (`server.endpoint_path`).

- Built-in MCP endpoint traffic uses the hardening middleware stack.
- `HttpRouterPlugin` custom routes are plugin/consumer-owned unless you explicitly add equivalent middleware.
- `outbound_http` controls outbound request/response transport behavior for HTTP tools and plugin `send(...)` / `send_with(...)` calls.
- `server.response_limits` controls emitted MCP tool-result payload size.

### HTTP router plugins

When the `streamable_http` feature is enabled, HTTP router plugins can add custom routes and middleware to the Axum router. See [Plugin Guide](PLUGINS.md) for details on the `HttpRouterPlugin` trait.

Custom routes must not collide with the MCP endpoint path or the OAuth metadata paths (when auth is enabled). Route collisions are detected at startup and cause a failure.

### TLS

See [Auth](AUTH.md) for TLS termination guidance.
