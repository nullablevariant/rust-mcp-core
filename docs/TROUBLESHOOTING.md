# Troubleshooting

This guide captures common operational errors and fixes for rust-mcp-core
deployments.

## AI client compatibility errors

### `tools.<n>.custom.input_schema: input_schema does not support oneOf, allOf, or anyOf at the top level`

Cause:
- client-side schema subset validation (observed in Claude Code).

Fix:
- remove top-level combinators from `tools.items[].input_schema`
- use object-only constraints (`required`, `minProperties`, typed fields)

See also: [AI Client Compatibility](AI_CLIENT_COMPATIBILITY.md).

### `Invalid input: expected record, received array` at `structuredContent`

Cause:
- client expects object/record `structuredContent`.
- tool returned array structured content (for example `response: { type: structured, template: ${$} }`
  on list endpoints).

Fix:
- wrap array under an object key:
  - `response: { type: structured, template: { data: ${$} } }`
  - or `response: { type: structured, template: { items: ${$} } }`

### `Bad Request: Unsupported MCP-Protocol-Version: 2025-11-25` (HTTP 400)

Cause:
- rmcp transport rejects unknown `MCP-Protocol-Version`.
- current rmcp stack supports `2025-06-18`.

Fix:
- enable built-in `protocol_version_negotiation.mode: negotiate`, or configure
  clients to send a supported protocol header value.

### `Unexpected content type: None` (MCP worker transport error)

Cause (observed):
- streamable HTTP handshake/request failed and response had no valid MCP content
  type (commonly 406/4xx paths).
- common triggers:
  - missing `Accept: application/json, text/event-stream` on POST
  - auth failure (`401 Unauthorized`), for example wrong/mismatched bearer token
  - stale client/session state after restarting MCP server

Fix:
- ensure client sends required `Accept` header
- verify auth token values match between client and server config/environment
- reinitialize MCP session (or restart client MCP connection) after server restart
- verify endpoint/path and headers with a direct `curl initialize` check

## Streamable HTTP header checks

For manual clients (`curl`, custom MCP transports):

- POST requests must include:
  - `Accept: application/json, text/event-stream`
  - `Content-Type: application/json`
- initialize sequence:
  1. `initialize`
  2. `notifications/initialized`
  3. reuse `MCP-Session-Id` where required by session mode

## Transport hardening errors (`http_hardening`)

### HTTP 413 `Payload Too Large`

Cause:
- request body exceeded `server.transport.streamable_http.hardening.max_request_bytes`.

Fix:
- reduce request payload size, or raise `max_request_bytes` for your deployment.

### HTTP 429 `Too Many Requests`

Cause:
- inbound limiter triggered:
  - `hardening.rate_limit` (general requests), or
  - `hardening.session.creation_rate` (new sessions).

Fix:
- increase bucket settings (`capacity`, `refill_per_sec`), or disable limiter (`enabled: false` / omit section) if appropriate.
- for `per_ip` policies behind a proxy, ensure `key_source` matches your network topology (`peer_addr` vs `x_forwarded_for`).

### Session creation fails after threshold (max sessions)

Cause:
- `hardening.session.max_sessions` reached.
- Current transport behavior surfaces this as HTTP `500 Internal Server Error` on the session-creation request path (not `429`).

Fix:
- raise `max_sessions`, shorten `idle_ttl_secs`/`max_lifetime_secs`, or reduce concurrent client session usage.

### Startup fails: `...hardening.rate_limit requires global or per_ip...`

Cause:
- limiter is active but no bucket (`global`/`per_ip`) is configured.

Fix:
- define at least one bucket, or set `enabled: false`.

## Auth upstream diagnostics

These issues are typically visible in server logs, not client payloads (unless
`server.errors.expose_internal_details=true`).

### OAuth2 token exchange failures (outbound)

Expected server log message patterns include:
- `oauth2 token endpoint returned an error response: <error>[: <description>] [(<error_uri>)]`
- `oauth2 token request failed: <transport error>`
- `oauth2 token endpoint returned invalid JSON: <parse error>`
- `oauth2 token exchange failed: <other>`

What to check:
- client credentials / grant type
- token endpoint URL
- audience/scope settings
- mTLS cert/key/CA settings when mTLS is enabled

### Token introspection failures (inbound provider)

Expected message patterns include:
- `token introspection endpoint returned an error response: <error>[: <description>] [(<error_uri>)]`
- `token introspection request failed: <transport error>`
- `token introspection endpoint returned invalid JSON: <parse error>`
- `token introspection exchange failed: <other>`

What to check:
- introspection URL reachability
- introspection client auth method and secrets
- timeout/response-size settings on outbound auth calls

### OIDC discovery / JWKS fetch failures

Expected provider diagnostics:
- discovery candidate failures are logged at `debug`:
  - request/send errors
  - non-success status
  - parse failures
- discovery terminal failure is logged at `warn`:
  - `auth discovery failed for all candidates`
- JWKS failures are logged at `warn`:
  - request/send errors
  - non-success status
  - parse failures

Operational note:
- server log payload fields are capped by `server.logging.log_payload_max_bytes`
  in both redacted and detailed client-error modes.
