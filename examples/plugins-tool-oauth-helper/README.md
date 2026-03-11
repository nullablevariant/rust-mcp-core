# plugins-tool-oauth-helper

## Covers
- Plugin usage of outbound OAuth2 helpers on `PluginContext`.
- `upstream_access_token(...)` and `upstream_bearer_header(...)` with upstream-name lookup.
- Built-in outbound upstream HTTP helper via `send(...)` (upstream oauth2 auth is auto-applied).

## Run
```bash
cargo run --example plugins-tool-oauth-helper
```
This example starts local mock upstream + oauth token endpoints automatically.

Server endpoint: `http://127.0.0.1:31963/mcp`.

## Test (JSON-RPC over HTTP)

```bash
URL="http://127.0.0.1:31963/mcp"
ACCEPT="application/json, text/event-stream"
```

1) Initialize + capture session id:
```bash
INIT_HEADERS=$(mktemp)
curl -sS -D "${INIT_HEADERS}" "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"0.1.0"}}}'
SESSION_ID=$(awk 'tolower($1)=="mcp-session-id:" {print $2}' "${INIT_HEADERS}" | tr -d '\r')
```

2) Send initialized notification:
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
```

3) Call plugin tool (cached-token path):
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"partner.oauth_header","arguments":{"upstream":"partner_api","force_refresh":false}}}'
```
Expected: response contains `"isError":false`, `"header_value":"Bearer mock-access-token"`, and `"upstream_status":200`.

4) Call again with `force_refresh=true`:
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"partner.oauth_header","arguments":{"upstream":"partner_api","force_refresh":true}}}'
```
Expected: response contains `"isError":false`, `"force_refresh":true`, and `"partner_count":2`.
