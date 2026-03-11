# plugins-simple-outbound-http

## Covers
- Tool plugin outbound HTTP calls through the built-in `PluginContext` helper: `send(...)`.
- Shared outbound policy application (`outbound_http` + `upstreams.<name>`) without a plugin-owned client.

This example uses rust-mcp-core's built-in outbound HTTP handling. If `http_hardening` is enabled, inbound streamable HTTP hardening still applies on the MCP endpoint for this server.

## Run
```bash
cargo run --example plugins-simple-outbound-http
```

Server endpoint: `http://127.0.0.1:31965/mcp`.

## Test (JSON-RPC over HTTP)

```bash
URL="http://127.0.0.1:31965/mcp"
ACCEPT="application/json, text/event-stream"
MCP_PROTOCOL_VERSION="2025-06-18"
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
  -H "MCP-Protocol-Version: ${MCP_PROTOCOL_VERSION}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
```

3) Call plugin tool:
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -H "MCP-Protocol-Version: ${MCP_PROTOCOL_VERSION}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"partner.list_via_plugin","arguments":{"limit":2,"search":"contoso"}}}'
```
Expected: response contains `"isError":false`, `"status":200`, and `"partner_count":2`.
