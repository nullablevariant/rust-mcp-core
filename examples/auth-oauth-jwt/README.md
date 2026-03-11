# auth-oauth-jwt

## Covers
- OAuth JWT/JWKS validation configuration.

## Run
```bash
cargo run --example auth-oauth-jwt
```
This example starts a local mock upstream API automatically, but JWT/JWKS validation still requires a real IdP-issued token and JWKS endpoint.

Default endpoint: `http://127.0.0.1:31943/mcp`.

## Test (JSON-RPC over HTTP)

Set a bearer token first:
```bash
export TOKEN="test-oauth-access-token"
```
`TOKEN` above is a test credential for this README. Replace it with a real OAuth access token from your IdP to pass JWT validation.

Capture a session id from the initialize response headers and reuse it for all subsequent requests:
```bash
INIT_HEADERS=$(mktemp)
curl -sS -D "${INIT_HEADERS}" http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "authorization: Bearer ${TOKEN}" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example-client","version":"0.1.0"}}}'
SESSION_ID=$(awk 'tolower($1)=="mcp-session-id:" {print $2}' "${INIT_HEADERS}" | tr -d '\r')
```
1) Send initialized notification (required by MCP lifecycle):
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "authorization: Bearer ${TOKEN}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
```
2) Call tool:
```bash
curl -sS http://127.0.0.1:31943/mcp \
  -H "content-type: application/json" \
  -H "accept: application/json, text/event-stream" \
  -H "authorization: Bearer ${TOKEN}" \
  -H "MCP-Session-Id: ${SESSION_ID}" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"api.get\",\"arguments\":{\"id\":\"item-1\"}}}"
```
