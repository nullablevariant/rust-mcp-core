# Examples Index

All examples are isolated and self-contained:
- `main.rs` wires runtime + config loading
- `config/mcp_config.yml` defines the example server behavior
- plugin-driven examples include `plugin.rs`

Each example has its own README with scope and JSON-RPC test steps.

For streamable HTTP examples, post-init requests may include `MCP-Protocol-Version: 2025-06-18`.
If provided with an unsupported value, rmcp returns HTTP 400.

## Local mock harness

Endpoint-dependent examples use a shared local harness at `examples/_shared/mock_harness.rs`.
This keeps README validation deterministic without external APIs/IdPs for primary tool-flow checks.

External-by-design auth example:
- `auth-oauth-jwt` (requires real JWT + JWKS validation path)

## Endpoint Dependency Audit (Phase 10)

Local mock harness-backed (deterministic HTTP validation path):
- `auth-all-mode`
- `auth-bearer`
- `auth-oauth-introspection`
- `auth-oauth-jwt` (upstream calls are local; auth validation still real IdP/JWKS)
- `core-minimal`
- `plugins-auth-custom`
- `plugins-router-http`
- `plugins-simple-outbound-http`
- `plugins-tool-oauth-helper`
- `tools-crud-http`
- `tools-http-post`
- `tools-output-modes`
- `tools-rich-content`
- `tools-templating`
- `tools-upstream-auth`
- `tools-web-search`
- `utility-list-changed`
- `utility-pagination`

No external HTTP dependency in primary flow:
- `plugins-tool-custom`
- `plugins-tool-filesystem`
- `plugins-tool-client-features`
- `plugins-tool-client-features-advanced`
- `prompts-inline-plugin`
- `resources-inline-plugin`
- `resources-subscribe-updated`
- `utility-cancellation`
- `utility-completion`
- `utility-logging`
- `utility-progress`
- `utility-tasks`
- `utility-tasks-advanced`

## Core and Auth
- [core-minimal](core-minimal/README.md)
- [auth-bearer](auth-bearer/README.md)
- [auth-oauth-jwt](auth-oauth-jwt/README.md)
- [auth-oauth-introspection](auth-oauth-introspection/README.md)
- [auth-all-mode](auth-all-mode/README.md)

## Tools
- [tools-web-search](tools-web-search/README.md)
- [tools-crud-http](tools-crud-http/README.md)
- [tools-http-post](tools-http-post/README.md)
- [tools-templating](tools-templating/README.md)
- [tools-output-modes](tools-output-modes/README.md)
- [tools-upstream-auth](tools-upstream-auth/README.md)
- [tools-rich-content](tools-rich-content/README.md)

## Plugin Wiring
- [plugins-tool-custom](plugins-tool-custom/README.md)
- [plugins-tool-filesystem](plugins-tool-filesystem/README.md)
- [plugins-auth-custom](plugins-auth-custom/README.md)
- [plugins-router-http](plugins-router-http/README.md)
- [plugins-simple-outbound-http](plugins-simple-outbound-http/README.md)
- [plugins-tool-client-features](plugins-tool-client-features/README.md)
- [plugins-tool-client-features-advanced](plugins-tool-client-features-advanced/README.md)
- [plugins-tool-oauth-helper](plugins-tool-oauth-helper/README.md)

## Prompts and Resources
- [prompts-inline-plugin](prompts-inline-plugin/README.md)
- [resources-inline-plugin](resources-inline-plugin/README.md)
- [resources-subscribe-updated](resources-subscribe-updated/README.md)

## Utilities
- [utility-logging](utility-logging/README.md)
- [utility-progress](utility-progress/README.md)
- [utility-cancellation](utility-cancellation/README.md)
- [utility-completion](utility-completion/README.md)
- [utility-pagination](utility-pagination/README.md)
- [utility-list-changed](utility-list-changed/README.md)
- [utility-tasks](utility-tasks/README.md)
- [utility-tasks-advanced](utility-tasks-advanced/README.md)
