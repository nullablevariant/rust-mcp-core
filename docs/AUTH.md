# Auth

rust-mcp-core has two auth surfaces:

- **Inbound auth (`server.auth.*`)**: validates client bearer tokens on incoming streamable HTTP MCP requests.
- **Outbound auth (`upstreams.<name>.auth`)**: injects/acquires credentials when HTTP tools (or plugin outbound helpers) call upstream APIs.

This document covers both.

## Inbound auth (`server.auth.*`)

### Activation semantics

- `server.auth` absent -> auth disabled.
- `server.auth.enabled: false` -> auth disabled (providers/oauth config ignored).
- `server.auth` present with `enabled` omitted/`true` -> auth enabled and `providers` must be non-empty.

### Provider model

Auth is provider-driven and evaluated in list order (`server.auth.providers[]`).

Supported provider variants:

- `type: bearer`
  - static token comparison against `Authorization: Bearer <token>`.
- `type: jwks`
  - JWT/JWKS validation using issuer/discovery + key material.
- `type: introspection`
  - OAuth token introspection endpoint validation.
- `type: plugin`
  - delegates decision to registered auth plugin.

Provider outcome behavior:

- `accept` -> authenticated
- `reject` -> request rejected
- `abstain` (plugin only) -> continue to next provider

No provider match/accept -> rejected.

### OAuth metadata/challenge settings

`server.auth.oauth` controls protected-resource metadata and challenge details:

- `resource` (required when `oauth` block present)
- `public_url` (optional; external URL used for metadata/challenge correctness behind proxies)
- `client_metadata_document_url` (optional)
- `scope_in_challenges` (optional, default `true`)

When OAuth-capable providers (`jwks`/`introspection`) are active, `server.auth.oauth` is required.

### Example inbound auth config

```yaml
server:
  auth:
    enabled: true
    providers:
      - name: static
        type: bearer
        token: ${env:MCP_BEARER_TOKEN}

      - name: okta
        type: jwks
        issuer: https://okta.example.com/
        discovery_url: https://okta.example.com/.well-known/openid-configuration
        jwks_url: https://okta.example.com/.well-known/jwks.json
        audiences: [api://mcp]
        required_scopes: [mcp.read]
        required_claims:
          tenant: acme
        algorithms: [RS256]
        clock_skew_sec: 60
        enable_oidc_discovery: true
        allow_well_known_fallback: true

      - name: corp-idp
        type: introspection
        issuer: https://corp.example.com/
        introspection_url: https://corp.example.com/oauth2/introspect
        client_id: ${env:INTROSPECTION_CLIENT_ID}
        client_secret: ${env:INTROSPECTION_CLIENT_SECRET}
        auth_method: basic

      - name: custom
        type: plugin
        plugin: auth.custom.validator

    oauth:
      public_url: https://mcp.example.com
      resource: https://mcp.example.com/mcp
      client_metadata_document_url: https://mcp.example.com/.well-known/oauth-client-metadata
      scope_in_challenges: true
```

## WWW-Authenticate responses

When auth is active and authentication fails:

- **401 Unauthorized** includes `WWW-Authenticate: Bearer`.
- When `server.auth.oauth` is active, 401 challenges include `resource_metadata`.
- When `scope_in_challenges=true`, 401/403 challenges include required `scope` where applicable.
- **403 Forbidden** for insufficient scope uses `error="insufficient_scope"`.

## Protected resource metadata

When `server.auth.oauth` is active, core exposes:

- `/.well-known/oauth-protected-resource`
- `/.well-known/oauth-protected-resource{endpoint_path}` (unless endpoint path is `/`)

Payload includes:

- `resource`
- `authorization_servers` (derived from configured providers)
- `oauth_client_metadata_document_url` (when configured)

## TLS deployment

rust-mcp-core serves plain HTTP. For production, terminate TLS at a reverse proxy/load balancer.
If using OAuth metadata/challenges in production, set `server.auth.oauth.public_url` to the externally reachable HTTPS origin.

## Outbound upstream auth (`upstreams.<name>.auth`)

Outbound auth applies only to HTTP execution paths (`tools.items[].execute.type=http` and plugin `ctx.send(...)` / `ctx.send_with(...)`).

Supported variants:

| Type | Behavior |
|------|----------|
| `none` | No auth injection. |
| `bearer` | Injects `Authorization: Bearer <token>`. |
| `basic` | Injects HTTP Basic auth header. |
| `oauth2` | Acquires token from token endpoint, caches in memory, injects bearer token, and optionally retries once on `401` after refresh. |

### OAuth2 upstream auth

Supported grants:

- `client_credentials`
- `refresh_token` (bootstrap refresh token supplied in config)

Token-endpoint client auth:

- `auth_method: basic` (HTTP Basic)
- `auth_method: request_body` (credentials in form body)

Secret/cert values support `inline`, `env`, and `path` sources.

### Token-endpoint mTLS

mTLS is enabled by presence of `auth.mtls` and requires `client_cert` + `client_key`.

### Header precedence (HTTP tools and plugin send/send_with)

Later layers override earlier layers:

1. `outbound_http.headers` + `outbound_http.user_agent`
2. `upstreams.<name>.headers` + `upstreams.<name>.user_agent`
3. static upstream auth injection (`bearer`/`basic`)
4. upstream oauth2 bearer injection
5. request-local headers (`tools.items[].execute.headers` or plugin request headers)

Notes:
- request-local headers can override `Authorization`.
- oauth2 bearer injection overrides static `Authorization` before request-local headers apply.

### Plugin OAuth2 helpers

With `http_tools` enabled, plugins can reuse upstream OAuth2 handling:

- `ctx.upstream_access_token("<upstream>", force_refresh)`
- `ctx.upstream_bearer_header("<upstream>", force_refresh)`

These reuse the same upstream token manager/cache used by HTTP tools.
