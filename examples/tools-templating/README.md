# tools-templating

## Covers
- Query templating with defaults and CSV expansion.

## Run
```bash
cargo run --example tools-templating
```
This example starts a local mock upstream automatically.

Default endpoint: `http://127.0.0.1:31943/mcp`.

## Test (JSON-RPC over HTTP)

Capture a session id from the initialize response headers and reuse it for all subsequent requests:
```bash
INIT_HEADERS=$(mktemp)
curl -sS -D "${INIT_HEADERS}" http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"0.1.0"}}}'
SESSION_ID=$(awk 'tolower($1)=="mcp-session-id:" {print $2}' "${INIT_HEADERS}" | tr -d '\r')
```
1) Send initialized notification (required by MCP lifecycle):
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
```
2) Call `search.advanced` without `page` (uses `page|default(1)` and `per_page|default(25)`):
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search.advanced","arguments":{"query":"mcp","tags":["rust","sdk"]}}}'
```
Expected fragments in the response:
- `"query":"mcp"`
- `"query_params"`
- `"q":"mcp"`
- `"page":"1"`
- `"per_page":"25"`
- `"tags":"rust,sdk"`

3) Call `search.advanced` with explicit `page`/`per_page` overrides:
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search.advanced","arguments":{"query":"mcp","tags":["rust"],"page":3,"per_page":5}}}'
```
Expected fragments in the response:
- `"query":"mcp"`
- `"query_params"`
- `"q":"mcp"`
- `"page":"3"`
- `"per_page":"5"`
- `"tags":"rust"`
