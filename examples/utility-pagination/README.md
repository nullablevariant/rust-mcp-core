# utility-pagination

## Covers
- Cursor-based pagination for `tools/list`, `prompts/list`, `resources/list`, and `resources/templates/list`.

## Run
```bash
cargo run --example utility-pagination
```
This example starts a local mock upstream automatically for `tool.one` and `tool.two`.

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
2) Request first page:
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
```
3) Repeat with returned `nextCursor`:
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"cursor":"<nextCursor>"}}'
```
