# plugins-tool-custom

## Covers
- Custom tool plugin wiring with `execute.type=plugin`.

## Run
```bash
cargo run --example plugins-tool-custom
```

Default endpoint: `http://127.0.0.1:31943/mcp`.

## Test (JSON-RPC over HTTP)
Copy/paste these commands in order:

```bash
URL="http://127.0.0.1:31943/mcp"
ACCEPT="application/json, text/event-stream"
MCP_PROTOCOL_VERSION="2025-06-18"
```

1) Initialize and capture session id:
```bash
INIT_HEADERS=$(mktemp)
curl -sS -D "${INIT_HEADERS}" "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"0.1.0"}}}'

SESSION_ID=$(awk 'tolower($1)=="mcp-session-id:" {print $2}' "${INIT_HEADERS}" | tr -d '\r')
echo "SESSION_ID=${SESSION_ID}"
```

2) Send initialized notification (required by MCP lifecycle):
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -H "MCP-Protocol-Version: ${MCP_PROTOCOL_VERSION}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
```

3) Call `reports.aggregate`:
```bash
curl -sS "${URL}" \
  -H "content-type: application/json" \
  -H "accept: ${ACCEPT}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -H "MCP-Protocol-Version: ${MCP_PROTOCOL_VERSION}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"reports.aggregate","arguments":{"project_id":1}}}'
```
