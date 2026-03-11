# AI Client Compatibility

This guide documents practical compatibility constraints observed with AI MCP
clients and how to configure servers for reliable interoperability.

These items are mostly **client/runtime constraints**, not MCP spec violations.

Last verified: 2026-03-03

## Built-in protocol version negotiation

Use this streamable HTTP config:

```yaml
server:
  transport:
    mode: streamable_http
    streamable_http:
      protocol_version_negotiation:
        mode: strict # strict | negotiate
```

Expected semantics:

- `protocol_version_negotiation` omitted => strict behavior.
- `mode: strict` => no header normalization.
- `mode: negotiate`:
  - missing `MCP-Protocol-Version` => pass through
  - known rmcp version => pass through
  - unknown version => drop header

Known versions should be sourced from rmcp directly:

- `rmcp::model::ProtocolVersion::KNOWN_VERSIONS`

## Known limitations: Claude Code

Observed with Claude Code `2.1.63`:

- Rejects `tools.items[].input_schema` with top-level combinators (`anyOf`, `oneOf`,
  `allOf`) with errors like:
  - `input_schema does not support oneOf, allOf, or anyOf at the top level`
- Can reject `structuredContent` arrays when it expects a record/object, with
  errors like:
  - `Invalid input: expected record, received array`

Recommended Claude-compatible patterns:

- Use object-only input schemas (`required` + `minProperties`) instead of
  top-level combinators.
- Enable config linting to catch this at startup:
  - `server.client_compat.input_schema.top_level_combinators: warn|error`
- For list endpoints, wrap arrays under an object key:
  - `response: { type: structured, template: { data: ${$} } }`
- If Claude sends unsupported protocol headers (for example
  `MCP-Protocol-Version: 2025-11-25`), use built-in protocol version
  negotiation (`strict | negotiate`).

For concrete error signatures and remediation steps, see
[Troubleshooting](TROUBLESHOOTING.md).

## 1) `input_schema` top-level combinators

Some clients reject tool schemas when `input_schema` has top-level:

- `anyOf`
- `oneOf`
- `allOf`

Example of a commonly rejected shape:

```yaml
input_schema:
  type: object
  properties:
    task_id: { type: integer }
    title: { type: string }
  required: [task_id]
  anyOf:
    - required: [title]
```

Compatibility-safe pattern:

```yaml
input_schema:
  type: object
  additionalProperties: false
  properties:
    task_id: { type: integer }
    title: { type: string }
    description: { type: string }
  required: [task_id]
  minProperties: 2
```

## 2) `structuredContent` shape expectations

Some clients expect `structuredContent` to be a JSON object (record), not an
array.

If your upstream returns an array, avoid:

```yaml
response:
  type: structured
  template: ${$}
```

Use an object wrapper:

```yaml
response:
  type: structured
  template:
    data: ${$}
```

or:

```yaml
response:
  type: structured
  template:
    items: ${$}
```

## 3) `MCP-Protocol-Version` header mismatch

`rmcp` validates `MCP-Protocol-Version` at transport layer.
With the current stack (`rmcp 1.1.0`), supported header value is
`2025-06-18`.

If a client sends a newer value (for example `2025-11-25`) by default, requests
can fail with HTTP 400 before normal MCP handling.

Mitigation options:

- strict mode: require supported header only
- built-in negotiation mode: drop unsupported protocol header

## 4) HTTP `Accept` requirements (streamable HTTP)

For streamable HTTP POST requests, clients must send:

- `Accept: application/json, text/event-stream`

Missing/incorrect `Accept` can cause 406 responses and downstream transport
errors (for example “Unexpected content type: None”).

## Quick compatibility checklist

- Prefer object-only `input_schema` constraints.
- Wrap array structured content in an object field.
- Normalize or drop unsupported `MCP-Protocol-Version` headers when needed.
- Ensure streamable HTTP clients send the required `Accept` header.
