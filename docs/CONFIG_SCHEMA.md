# Config Schema

This document describes the full configuration schema accepted by `load_mcp_config`. It is derived from `src/config/schema/config.schema.json` and runtime validation rules in core.

All keys are `snake_case`.
Unknown keys are rejected by schema validation for owned config objects.

## Environment variable expansion

- `${env:ENV_NAME}` placeholders are expanded before schema validation.
- Missing env vars are replaced with the literal string `null` (which then may fail schema validation if the field is not nullable).
- Expansion happens for both JSON and YAML.

## Startup vs reload validation

- Schema + runtime validation is applied both at startup and on `Runtime::reload_config(...)`.
- Reload is consumer-triggered. Core does not include built-in file watching.
- `.env` files are not automatically re-read by core during reload.
  If your host uses dotenv-style files, re-read and re-apply them before rebuilding config input.
- On reload failure, the previous runtime state remains active.

## Top-level object

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `version` | integer | yes | none | Config version. Must be >= 1. |
| `server` | object | no | `{}` | Server settings (host/port/path, transport, auth, logging, client compatibility, and core hardening controls). |
| `client_logging` | object or null | no | `null` (disabled) | MCP client notification logging settings; section presence enables capability. |
| `progress` | object or null | no | `null` (disabled) | MCP progress notification settings; section presence enables helper sends. |
| `pagination` | object or null | no | `null` | List pagination settings. |
| `prompts` | object or null | no | `null` | Prompt providers and `prompts/list`/`prompts/get` settings. |
| `resources` | object or null | no | `null` | Resource providers and `resources/list`/`resources/read`/`resources/templates/list` settings. |
| `completion` | object or null | no | `null` (disabled) | Completion utility settings (`completion/complete` provider registry). Presence enables unless `enabled: false`. |
| `tasks` | object or null | no | `null` (disabled) | Experimental tasks utility settings (`tasks/get`, `tasks/list`, `tasks/result`, `tasks/cancel`). Presence enables unless `enabled: false`. |
| `client_features` | object | no | `{}` | Client feature helpers for server-initiated roots, sampling, and elicitation requests. |
| `outbound_http` | object | no | unset | Global outbound HTTP policy (`headers`, `user_agent`, `timeout_ms`, `max_response_bytes`, `retry`). |
| `upstreams` | object | no | `{}` | Named HTTP upstream definitions. |
| `tools` | object or null | no | `null` | Tool configuration (`enabled`, `notify_list_changed`, `items`). |
| `plugins` | array | no | `[]` | Unified plugin allowlist and configuration. |

## Compile-time feature gates vs runtime schema

Schema validation accepts the full config shape regardless of build features. Runtime startup/reload then enforces compile-time feature gates.

If a disabled feature is referenced, core fails fast with `invalid_request`:

- `auth` disabled + active `server.auth` config (`enabled` omitted/true with providers)
- `streamable_http` disabled + `server.transport.mode=streamable_http`
- `streamable_http` disabled + any `plugins[].type=http_router`
- `http_hardening` disabled + `server.transport.streamable_http.hardening` section present
- `http_tools` disabled + any `tools.items[].execute.type=http`
- `prompts` disabled + active `prompts` config is present
- `resources` disabled + active `resources` config is present
- `completion` disabled + active completion config is present
- `completion` disabled + any `plugins[].type=completion`
- `completion` disabled + prompt/resource completion mappings configured
- `client_logging` disabled + `client_logging` section is present
- `progress_utility` disabled + `progress` section is present
- `tasks_utility` disabled + active `tasks` config is present
- `tasks_utility` disabled + any `tools.items[].execute.task_support` set to `optional` or `required`
- `client_features` disabled + active client feature sections are present (`roots`, `sampling`, `elicitation`)

Behavior when feature is disabled:
- Capability is omitted from `initialize` response (`prompts`, `resources`, `completion`, `logging`, `tasks`).
- Utility/request methods return method-not-found (`-32601`) for disabled surfaces.

## `server`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `host` | string | no | `127.0.0.1` | Bind host for streamable HTTP transport. |
| `port` | integer | no | `3000` | Bind port for streamable HTTP transport. |
| `endpoint_path` | string | no | `/mcp` | Base path for MCP HTTP endpoint. Use `/` only if you want the MCP endpoint at root (no redirect from `/` to other paths). |
| `logging` | object | no | `{ level: info, log_payload_max_bytes: 4096 }` | Server-side tracing level and payload truncation controls. |
| `transport` | object | no | `{ mode: streamable_http }` | Transport settings. |
| `auth` | object | no | `null` (disabled) | Auth settings. |
| `client_compat` | object | no | `{ input_schema: { top_level_combinators: warn } }` | Client-compatibility lint policies for config authoring. |
| `errors` | object | no | `{ expose_internal_details: false }` | Client-facing error exposure controls. |
| `info` | object or null | no | `null` | Server identity fields and initialize instructions. |
| `response_limits` | object or null | no | `null` | Optional tool response size caps by channel and total size. |

### `server.transport`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `mode` | enum | no | `streamable_http` | `streamable_http` or `stdio`. |
| `streamable_http` | object | no | `{enable_get_stream: true, enable_sse_resumption: false, session_mode: optional, allow_delete_session: false}` | Streamable HTTP behavior controls. |

#### `server.transport.streamable_http`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enable_get_stream` | boolean | no | `true` | Enables GET SSE endpoint. If `false`, GET returns 405. |
| `enable_sse_resumption` | boolean | no | `false` | Allows `Last-Event-ID` based resume on GET. |
| `session_mode` | enum | no | `optional` | `none`, `optional`, or `required`. `none` runs stateless mode. |
| `allow_delete_session` | boolean | no | `false` | Enables DELETE session termination endpoint when session mode is stateful. |
| `sse_keep_alive_ms` | integer or null | no | `null` | Streamable HTTP SSE keep-alive interval (ms). Must be > 0. |
| `sse_retry_ms` | integer or null | no | `null` | Streamable HTTP SSE retry interval (ms). Must be > 0. |
| `protocol_version_negotiation` | object | no | `{ mode: strict }` | MCP protocol-header handling behavior for streamable HTTP requests. |
| `hardening` | object | no | unset | Optional transport hardening controls (requires `http_hardening` feature at runtime). |

#### `server.transport.streamable_http.protocol_version_negotiation`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `mode` | enum | no | `strict` | `strict` keeps header as-is (unsupported values return 400 via rmcp). `negotiate` drops unsupported `MCP-Protocol-Version` headers before rmcp validation. |

#### `server.transport.streamable_http.hardening`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_request_bytes` | integer | no | `1048576` | Max inbound request body size in bytes (`>=1`). |
| `catch_panics` | boolean | no | `true` | Enables panic-to-500 transport guard. |
| `sanitize_sensitive_headers` | boolean | no | `true` | Redacts sensitive headers from transport logging. |
| `session` | object | no | unset | Optional session abuse controls. |
| `rate_limit` | object | no | unset | Optional general inbound rate limiting controls. |

#### `server.transport.streamable_http.hardening.session`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_sessions` | integer or null | no | `null` | Optional max concurrent sessions (`>=1`). |
| `idle_ttl_secs` | integer or null | no | `null` | Optional idle timeout in seconds (`>=1`). |
| `max_lifetime_secs` | integer or null | no | `null` | Optional absolute max session lifetime in seconds (`>=1`). |
| `creation_rate` | object | no | unset | Optional session-creation rate limiter. |

#### `server.transport.streamable_http.hardening.session.creation_rate`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` | Explicit override for this limiter when section is present. |
| `global` | object | no | unset | Global token bucket (`capacity`, `refill_per_sec`, both `>=1`). |
| `per_ip` | object | no | unset | Per-IP token bucket (`capacity`, `refill_per_sec`, `key_source`). |

Validation rule:
- when active (`enabled` not `false`), at least one bucket (`global` or `per_ip`) must be set.

#### `server.transport.streamable_http.hardening.rate_limit`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` | Explicit override for this limiter when section is present. |
| `global` | object | no | unset | Global token bucket (`capacity`, `refill_per_sec`, both `>=1`). |
| `per_ip` | object | no | unset | Per-IP token bucket (`capacity`, `refill_per_sec`, `key_source`). |

Validation rule:
- when active (`enabled` not `false`), at least one bucket (`global` or `per_ip`) must be set.

### `server.auth`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Set `false` to disable auth while keeping providers configured. |
| `providers` | array | no | `[]` | Ordered auth providers (`bearer`, `jwks`, `introspection`, `plugin`). |
| `oauth` | object or null | no | `null` | OAuth metadata/challenge settings. Required when oauth-capable providers (`jwks`/`introspection`) are configured and auth is active. |

#### `server.auth.providers[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | yes | none | Provider identifier (must be unique). |
| `type` | enum | yes | none | `bearer`, `jwks`, `introspection`, `plugin`. |

Variant fields:

- `type=bearer`:
  - required: `token`
- `type=jwks`:
  - required: one of `issuer` or `discovery_url`
  - optional: `jwks_url`, `audiences`, `required_scopes`, `required_claims`, `algorithms`, `clock_skew_sec`, `enable_oidc_discovery`, `allow_well_known_fallback`
- `type=introspection`:
  - required: `introspection_url`
  - optional: `issuer`, `allow_missing_iss`, `client_id`, `client_secret`, `auth_method`, `audiences`, `required_scopes`, `required_claims`
- `type=plugin`:
  - required: `plugin`
  - optional: `allow_missing_iss`, `required_scopes`

#### `server.auth.oauth`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `public_url` | string or null | no | derived from host/port | Externally reachable server URL used in metadata/challenges. |
| `resource` | string | yes | none | Protected resource identifier advertised to OAuth clients. |
| `client_metadata_document_url` | string or null | no | `null` | Optional client metadata document URL surfaced in protected-resource metadata. |
| `scope_in_challenges` | boolean | no | `true` | Include required `scope` in `WWW-Authenticate` challenges. |

Activation semantics:
- `server.auth` omitted => auth disabled.
- `server.auth.enabled=false` => auth disabled; providers/oauth are ignored.
- `server.auth.enabled` omitted/`true` => auth enabled and `providers` must be non-empty.

### `server.client_compat`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_schema` | object | no | `{ top_level_combinators: warn }` | Tool input schema compatibility policy settings. |

#### `server.client_compat.input_schema`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `top_level_combinators` | enum | no | `warn` | `off`, `warn`, or `error`. Controls how `tools.items[].input_schema` top-level `anyOf`/`oneOf`/`allOf` are handled. |

### `server.errors`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expose_internal_details` | boolean | no | `false` | Controls client-visible internal error detail only. When `false`, client responses are generic while server logs retain detailed causes. |

### `server.logging`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `level` | enum | no | `info` | Server tracing level (`error`, `warn`, `info`, `debug`, `trace`). |
| `log_payload_max_bytes` | integer | no | `4096` | Max bytes logged per payload field (`0` disables truncation). Applies to server logs regardless of `server.errors.expose_internal_details`. |

#### Error visibility vs server log truncation

`server.errors.expose_internal_details` and `server.logging.log_payload_max_bytes` are
orthogonal:

- `server.errors.expose_internal_details` controls what clients receive.
- `server.logging.log_payload_max_bytes` controls server log payload capping.

Behavior matrix:

| `server.errors.expose_internal_details` | Client-facing internal error text | Server logs include internal cause | Server logs capped by `server.logging.log_payload_max_bytes` |
| --- | --- | --- | --- |
| `false` | redacted (`internal server error`) | yes | yes |
| `true` | detailed | yes | yes |

### `server.response_limits`

All fields are optional. Omitted fields are uncapped.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `text_bytes` | integer or null | no | `null` | Max bytes allowed for text content payloads. |
| `structured_bytes` | integer or null | no | `null` | Max bytes allowed for structured content payloads (JSON object/array). |
| `binary_bytes` | integer or null | no | `null` | Max bytes allowed for binary content payloads. |
| `other_bytes` | integer or null | no | `null` | Max bytes for non-text/non-structured/non-binary content. |
| `total_bytes` | integer or null | no | `null` | Max aggregate bytes across all tool result channels. |

## `server.info`

Customizes the `serverInfo` (Implementation) fields in the initialize response. All fields are optional; omitted fields fall back to build-time defaults from `CARGO_CRATE_NAME` / `CARGO_PKG_VERSION`.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | no | build crate name | Server name. |
| `version` | string | no | build crate version | Server version. |
| `title` | string or null | no | `null` | Optional display title. |
| `description` | string or null | no | `null` | Human-readable server description. |
| `website_url` | string or null | no | `null` | Server website URL (wire: `websiteUrl`). |
| `icons` | array or null | no | `null` | Server icons. Each icon has `src` (required), `mime_type` (optional, wire: `mimeType`), `sizes` (optional, array of WxH strings). |
| `instructions` | string or null | no | `null` | Optional initialize-level instructions returned in the initialize response. |

## `client_logging`

Controls MCP logging utility behavior (`logging/setLevel` and
`notifications/message`). This does **not** control server tracing output.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `level` | enum | no | `info` | Initial minimum client notification level. |

Enum values:
- `debug`
- `info`
- `notice`
- `warning`
- `error`
- `critical`
- `alert`
- `emergency`

## `progress`

Controls MCP progress utility behavior (`notifications/progress`).

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `notification_interval_ms` | integer | no | `250` | Minimum interval between emitted progress notifications per request (`0` disables rate limiting). |

## `tasks` (experimental)

Controls MCP tasks utility behavior (`tasks/get`, `tasks/list`, `tasks/result`,
`tasks/cancel`, and optional `notifications/tasks/status`).

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep tasks active; set `false` to disable while keeping sibling config. |
| `capabilities` | object | no | `{ list: true, cancel: true }` | Per-capability toggles for listing and cancelling tasks. |
| `capabilities.list` | boolean | no | `true` | Enables `tasks/list` when tasks config is active. |
| `capabilities.cancel` | boolean | no | `true` | Enables `tasks/cancel` when tasks config is active. |
| `status_notifications` | boolean | no | `false` | Emits optional `notifications/tasks/status` on status transitions. |

## `client_features`

Controls server-initiated client feature requests (roots, sampling, elicitation). These helpers are exposed on `PluginContext` so tool plugins can call client features with built-in capability checking, config validation, and error logging.

**Plugin required**: Yes. The config enables and validates the feature, but actual calls to `request_roots()`, `request_sampling()`, or `request_elicitation()` must be made from a tool plugin via `PluginContext`.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `roots` | object or null | no | `null` (disabled) | Roots list request settings. Section requires explicit `enabled`. |
| `sampling` | object or null | no | `null` (disabled) | LLM sampling request settings. Presence enables unless `enabled: false`. |
| `elicitation` | object or null | no | `null` (disabled) | User elicitation request settings. Presence enables unless `enabled: false`. |

### `client_features.roots`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | yes (when `roots` section is present) | none | Explicit roots helper toggle. |

### `client_features.sampling`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep sampling active; set `false` to disable while preserving sibling settings. |
| `allow_tools` | boolean | no | `false` | When `true`, allows `tools` and `tool_choice` in sampling requests. If `false`, requests containing tools or tool_choice are rejected with `-32602`. Even when `true`, tools are only sent if the client declared `sampling.tools` capability. |

### `client_features.elicitation`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep elicitation active; set `false` to disable while preserving `mode`. |
| `mode` | enum | no | `form` | `form`, `url`, or `both`. Controls which elicitation modes are permitted. `form` = schema-based form elicitation only. `url` = URL-based elicitation only. `both` = either mode allowed. Requests that don't match the configured mode are rejected with `-32602`. Additionally, client capability is checked: empty `elicitation: {}` is treated as form-only (backwards compatible per spec). |

## `pagination`

Controls MCP pagination utility behavior for list endpoints.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `page_size` | integer | yes (if `pagination` is set) | none | Page size for paginated list responses. Must be `>= 0`. |

## `prompts`

Controls MCP prompts behavior (`prompts/list`, `prompts/get`).

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep prompts active; set `false` to disable while preserving provider config. |
| `notify_list_changed` | boolean | no | `false` | Enables `notifications/prompts/list_changed` on reload/refresh and debounced registry mutations. |
| `pagination` | object or null | no | `null` | Prompt-specific pagination override. If omitted, falls back to global `pagination`. |
| `providers` | array | yes (when prompts is effectively active) | none | Prompt providers (`inline` and/or `plugin`). Must be non-empty when active. |

### `prompts.pagination`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `page_size` | integer | yes (if `prompts.pagination` is set) | none | Prompt list page size. Must be `>= 0`. `0` disables pagination for prompts. |

### `prompts.providers[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | enum | yes | none | `inline` or `plugin`. |
| `items` | array | conditional | unset | Required when `type=inline`. |
| `plugin` | string | conditional | unset | Required when `type=plugin`. Must match a `plugins[]` entry with `type=prompt`. |
| `config` | object or null | no | `null` | Optional provider-level plugin config override (`type=plugin` only). Shallow-merged over `plugins[].config`. |

### `prompts.providers[].items[]` (inline)

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | yes | none | Prompt name. |
| `title` | string | no | unset | Optional display title. |
| `description` | string | no | unset | Optional prompt description. |
| `icons` | array | no | unset | Prompt icons (rmcp `Icon`). |
| `arguments_schema` | object | yes | none | JSON Schema for prompt args. Also used to derive MCP `PromptArgument[]`. |
| `completions` | object | no | unset | Optional map of `argument_name -> completion_ref`. Every key must exist in `arguments_schema.properties`. |
| `template` | object | yes | none | Prompt template definition. |

### `prompts.providers[].items[].template`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `messages` | array | yes | none | Ordered prompt messages. Must be non-empty. |

### `prompts.providers[].items[].template.messages[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `role` | enum | yes | none | `user` or `assistant`. |
| `content` | string or object | yes | none | Message content. String is shorthand for `{ type: text, text: "<string>" }`. |

#### `messages[].content` object (typed content)

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | enum | yes | none | `text`, `image`, `resource`, `resource_link`. |
| `text` | string | conditional | unset | Required for `type=text`; optional for `type=resource` (`text` or `blob` required there). |
| `data` | string | conditional | unset | Required for `type=image` (base64 image bytes). |
| `mime_type` | string | conditional | unset | Required for `type=image`; optional for `resource`/`resource_link`. |
| `uri` | string | conditional | unset | Required for `type=resource` and `type=resource_link`. |
| `blob` | string | conditional | unset | Optional base64 payload for `type=resource` (`text` or `blob` required). |
| `name` | string | no | unset | Optional display name for `type=resource_link`. |
| `title` | string | no | unset | Optional title for `type=resource_link`. |
| `description` | string | no | unset | Optional description for `type=resource_link`. |
| `size` | integer | no | unset | Optional size for `type=resource_link` (`>= 0`). |
| `_meta` | object | no | unset | Optional rmcp metadata object. |
| `annotations` | object | no | unset | Optional rmcp annotations object. |
| `content_meta` | object | no | unset | Optional metadata for embedded `resource` content object. |

## `resources`

Controls MCP resources behavior (`resources/list`, `resources/read`,
`resources/templates/list`).

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep resources active; set `false` to disable while preserving provider config. |
| `notify_list_changed` | boolean | no | `false` | Enables `notifications/resources/list_changed` on reload/refresh and debounced registry mutations. |
| `clients_can_subscribe` | boolean | no | `false` | Enables `resources/subscribe` + `resources/unsubscribe` capability. |
| `pagination` | object or null | no | `null` | Resource-specific pagination override. If omitted, falls back to global `pagination`. |
| `providers` | array | yes (when resources is effectively active) | none | Resource providers (`inline` and/or `plugin`). Must be non-empty when active. |

### `resources.pagination`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `page_size` | integer | yes (if `resources.pagination` is set) | none | Resource list page size. Must be `>= 0`. `0` disables pagination for resources/templates. |

### `resources.providers[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | enum | yes | none | `inline` or `plugin`. |
| `items` | array | conditional | unset | Optional for `inline`; each item is a static resource. |
| `templates` | array | conditional | unset | Optional for both provider types; advertised in `resources/templates/list`. |
| `plugin` | string | conditional | unset | Required when `type=plugin`. Must match a `plugins[]` entry with `type=resource`. |
| `config` | object or null | no | `null` | Optional provider-level plugin config override (`type=plugin` only). Shallow-merged over `plugins[].config`. |

Notes:
- `type=inline` must define at least one of `items` or `templates`.
- `type=plugin` may define `templates`, `list`, or both.

### `resources.providers[].items[]` (inline)

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `uri` | string | yes | none | Resource URI. Must parse as a URI. |
| `name` | string | yes | none | Resource name. |
| `title` | string | no | unset | Optional display title. |
| `description` | string | no | unset | Optional description. |
| `mime_type` | string | no | unset | Optional MIME type. |
| `size` | integer | no | unset | Optional content size in bytes (`>= 0`). |
| `icons` | array | no | unset | Resource icons (rmcp `Icon`). |
| `annotations` | object | no | unset | Optional annotations (`audience`, `priority`, `last_modified`). |
| `content` | object | no | unset | Optional inline content. Requires `text` or `blob` when present. |

#### `resources.providers[].items[].annotations`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `audience` | array | no | unset | Optional audience roles: `user`, `assistant`. |
| `priority` | number | no | unset | Optional priority between `0.0` and `1.0`. |
| `last_modified` | string | no | unset | Optional RFC 3339 timestamp. |

#### `resources.providers[].items[].content`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `text` | string | conditional | unset | Inline text resource body. |
| `blob` | string | conditional | unset | Inline base64 resource body. |

### `resources.providers[].templates[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `uri_template` | string | yes | none | URI template (RFC 6570 style path placeholders). |
| `name` | string | yes | none | Template name. |
| `title` | string | no | unset | Optional display title. |
| `description` | string | no | unset | Optional description. |
| `mime_type` | string | no | unset | Optional MIME type. |
| `icons` | array | no | unset | Template icons (rmcp `Icon`). |
| `annotations` | object | no | unset | Optional annotations (`audience`, `priority`, `last_modified`). |
| `arguments_schema` | object | yes | none | JSON Schema used to validate URI-derived template arguments. |
| `completions` | object | no | unset | Optional map of `argument_name -> completion_ref`. Keys must exist in `arguments_schema.properties`. |

## `completion`

Controls MCP completion utility behavior (`completion/complete`).

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override. Omit to keep completion active; set `false` to disable while preserving provider config. |
| `providers` | array | no | `[]` | Named completion providers (`inline` or `plugin`). |

Activation semantics:
- `completion` omitted/null => disabled
- `completion: { enabled: false }` => disabled
- `completion` present and active => `providers` must be non-empty
- `completion: {}` is invalid (active section without providers)

### `completion.providers[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | yes | none | Provider name referenced by `prompts[].completions` or `resources[].templates[].completions`. Must be unique. |
| `type` | enum | yes | none | `inline` or `plugin`. |
| `values` | array | conditional | unset | Required when `type=inline`; static completion candidates. |
| `plugin` | string | conditional | unset | Required when `type=plugin`; must match `plugins[]` entry with `type=completion`. |
| `config` | object or null | no | `null` | Optional per-provider override for plugin providers. Shallow-merged over `plugins[].config`. |

## `outbound_http`

Global outbound HTTP policy shared by HTTP tools and plugin upstream-aware outbound calls.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `headers` | object | no | `{}` | Default headers applied before upstream/tool/plugin request headers. |
| `user_agent` | string | no | unset | Default `User-Agent` header value. |
| `timeout_ms` | integer | no | unset | Default outbound timeout in milliseconds. |
| `max_response_bytes` | integer | no | unset | Default outbound response body cap in bytes. |
| `retry` | object | no | unset | Global outbound retry policy. Presence enables retry logic. |

## `upstreams`

`upstreams` is a map of upstream name -> config. Each entry defines an HTTP base URL and optional auth/headers/timeout/response-size override.

### `upstreams.<name>`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `base_url` | string | yes | none | Base URL for HTTP requests. |
| `headers` | object | no | `{}` | Static headers applied to every request. |
| `user_agent` | string | no | unset | Per-upstream `User-Agent` override. |
| `timeout_ms` | integer | no | unset | Request timeout in milliseconds. |
| `max_response_bytes` | integer | no | unset | Per-upstream response body cap in bytes (overrides `outbound_http.max_response_bytes`). |
| `retry` | object | no | unset | Per-upstream retry policy override. |
| `auth` | object | no | unset | Upstream auth configuration. |

#### `outbound_http.retry`, `upstreams.<name>.retry`, `tools.items[].execute.retry`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_attempts` | integer | yes (when retry present) | none | Total attempt budget (initial attempt included). Must be `>= 1`. |
| `delay_ms` | integer | yes (when retry present) | none | Delay between retry attempts in milliseconds. Must be `>= 1`. |
| `on_network_errors` | boolean | no | `true` | Retries transport/network failures when `true`. |
| `on_statuses` | array | no | `[]` | HTTP status codes that trigger retries (`100..=599`). |

Validation rules:
- Retry is enabled by presence (`retry.enabled` does not exist).
- `tools.items[].execute.retry` is only valid when `tools.items[].execute.type=http`.
- `on_network_errors=false` requires non-empty `on_statuses` (no no-op retry config).

## `tools`

Each tool entry defines how the server exposes and executes an MCP tool.
Templating is central to HTTP tool configuration. See [Templating](#templating)
for placeholder syntax and behavior.

### `tools`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | boolean | no | `true` (when section present) | Optional override for hybrid activation. |
| `notify_list_changed` | boolean | no | `false` | Enables `tools` capability `listChanged=true` and tool list-changed signaling on reload/refresh and debounced registry mutations. |
| `items` | array | yes (when tools is effectively active) | none | Tool definitions. |

### `tools.items[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | yes | none | Tool name. Must pass rmcp tool name validation. |
| `title` | string | no | unset | Optional display title. |
| `description` | string | yes | none | Tool description. |
| `cancellable` | boolean | no | `true` | Whether core should abort in-flight execution when the request is cancelled. |
| `input_schema` | object | yes | none | JSON Schema for input arguments. Must be a JSON object. |
| `output_schema` | object | no | unset | JSON Schema for structured output validation. Must be a JSON object. |
| `annotations` | object | no | unset | Tool annotations (rmcp `ToolAnnotations`). |
| `icons` | array | no | unset | Tool icons (rmcp `Icon`). |
| `_meta` | object | no | unset | Tool metadata (rmcp `Meta`). |
| `execute` | object | yes | none | Execution configuration. |
| `response` | object | no | unset | Response shaping configuration. |

#### `tools.items[].execute`

`execute` is a tagged enum (`type=http|plugin`) with strict variant fields.

- `type=http` fields:
  - required: `upstream`, `method`, `path`
  - optional: `query`, `headers`, `body`, `retry`, `task_support`
- `type=plugin` fields:
  - required: `plugin`
  - optional: `config`, `task_support`

Invalid cross-variant fields fail at deserialization (for example `plugin` on `type=http`).

#### `tools.items[].response`

`response` is a tagged enum (`type=structured|content`) with strict variant fields.

- `type=structured`:
  - optional: `template`, `fallback`
- `type=content`:
  - required: `items`

Behavior:
- omitted `response` => structured passthrough (default behavior)
- `type=structured` => structured output path, `output_schema` applies
- `type=content` => content block path, `output_schema` does not apply

#### `tools.items[].response.items[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | enum | yes | none | `text`, `image`, `audio`, `resource_link`, or `resource`. |
| `text` | string | conditional | unset | Required when `type=text`. |
| `data` | string | conditional | unset | Required when `type=image` or `type=audio` (base64 payload). |
| `mime_type` | string | conditional | unset | Required when `type=image` or `type=audio`; optional for links/resources. |
| `_meta` | object | no | unset | Optional protocol metadata on the content block. |
| `annotations` | object | no | unset | Optional annotations (`audience`, `priority`, `last_modified`). |
| `uri` | string | conditional | unset | Required when `type=resource_link`. |
| `name` | string | no | `resource` | Optional display name for `type=resource_link`. |
| `title` | string | no | unset | Optional title for `type=resource_link`. |
| `description` | string | no | unset | Optional description for `type=resource_link`. |
| `size` | integer | no | unset | Optional size for `type=resource_link` (`>= 0`). |
| `icons` | array | no | unset | Optional icons for `type=resource_link`. |
| `resource` | object | conditional | unset | Required when `type=resource`; embedded resource payload. |

#### `tools.items[].response.items[].resource`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `uri` | string | yes | none | Resource URI. |
| `mime_type` | string | no | unset | Optional MIME type. |
| `text` | string | conditional | unset | Required when `blob` is unset. |
| `blob` | string | conditional | unset | Required when `text` is unset. |
| `_meta` | object | no | unset | Optional resource-content metadata. |
| `annotations` | object | no | unset | Optional annotations (mapped to embedded resource annotations). |

## `plugins`

Unified plugin allowlist and configuration. Names must be **globally unique** across all plugin types.

### `plugins[]`

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | yes | none | Plugin name. Must be globally unique and registered in the `PluginRegistry`. |
| `type` | enum | yes | none | `tool`, `auth`, `http_router`, `completion`, `prompt`, or `resource`. |
| `targets` | array | conditional | unset | Required for `http_router`; forbidden for all other types. |
| `config` | object or null | no | `null` | Plugin-level config. Allowed for all plugin types. For tool plugins, shallow-merged with per-tool `execute.config` (per-tool keys override). For prompt plugins, shallow-merged with `prompts.providers[].config` (provider keys override). For resource plugins, shallow-merged with `resources.providers[].config` (provider keys override). For completion plugins, shallow-merged with `completion.providers[].config` (provider keys override). For auth plugins, passed to `AuthPlugin::validate`. Core does not validate object contents. |

#### `plugins[].targets[]`

Only applicable when `type` is `http_router`.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | enum | yes | none | `wrap` or `route`. |
| `path` | string | yes | none | Target path (normalized with leading `/`, trailing `/` trimmed). `*` is only allowed for `wrap`. |

## Validation and runtime rules

### Transport rules

- `server.transport.mode = stdio` requires auth to be disabled (`server.auth` absent or `server.auth.enabled=false`) and no `http_router` plugins.
- `server.transport.mode = streamable_http` requires non-empty `server.host` and `server.endpoint_path`.
- `server.transport.streamable_http.enable_sse_resumption=true` requires:
  - `enable_get_stream=true`
  - `session_mode` is `optional` or `required`
- `server.transport.streamable_http.allow_delete_session=true` requires `session_mode` is `optional` or `required`.
- `server.transport.streamable_http.session_mode=none` runs rmcp in stateless mode.
- `enable_get_stream=false` causes GET requests to the MCP endpoint to return `405`.
- `allow_delete_session=false` causes DELETE requests to the MCP endpoint to return `405`.
- GET requests with `Last-Event-ID` return `400` when `enable_sse_resumption=false`.
- For `session_mode=required`, GET/DELETE requests without `MCP-Session-Id` return `400`.
- `protocol_version_negotiation.mode=strict` preserves `MCP-Protocol-Version` as-is; unsupported values return `400` from rmcp.
- `protocol_version_negotiation.mode=negotiate` drops unsupported `MCP-Protocol-Version` headers before rmcp validation.
- `streamable_http.hardening` is only valid when the crate is compiled with `http_hardening`.
- `streamable_http.session_mode=none` rejects `streamable_http.hardening.session`.
- with `http_hardening` enabled, omitted `streamable_http.hardening` still applies default scalar controls:
  - `max_request_bytes=1048576`
  - `catch_panics=true`
  - `sanitize_sensitive_headers=true`
- `streamable_http.hardening.rate_limit` and `streamable_http.hardening.session.creation_rate` require at least one bucket (`global` or `per_ip`) when enabled.
- oversized request bodies are rejected with HTTP `413`.
- inbound rate-limit violations are rejected with HTTP `429`.
- Requests with unknown/expired `MCP-Session-Id` return `404`.
- `server.endpoint_path` is normalized to a leading `/` and trimmed of trailing `/` for routing (except `/` stays `/`).
- Streamable HTTP POST requests must include `Accept: application/json, text/event-stream`.
- Clients should send `initialize`, then `notifications/initialized`, before normal request traffic.
- Streamable HTTP responses may be SSE-framed.

### Pagination rules

- Pagination is disabled when `pagination` is omitted.
- `pagination.page_size=0` disables pagination (server logs a debug line at startup).
- When pagination is disabled, list endpoints ignore incoming `cursor` and return full lists.
- Invalid cursors on paginated endpoints return JSON-RPC `-32602` (`invalid params`).

### JSON-RPC error mapping rules (core-owned)

These mappings are enforced for rust-mcp-core-owned request handling paths.

- `invalid params` (`-32602`):
  - malformed request params (for example invalid pagination cursor)
  - unknown request targets inside a valid method call (for example unknown tool/prompt name)
- `invalid request` (`-32600`):
  - server misconfiguration/invariant violations detected during request handling
- `method not found` (`-32601`):
  - feature-gated methods when the capability is disabled at runtime/config
- `resource not found` (`-32002`):
  - `resources/read` URI misses after direct + template resolution
- `internal error` (`-32603`):
  - internal execution failures; outward detail exposure is controlled by
    `server.errors.expose_internal_details`

Notes:
- rmcp transport-level pre-dispatch protocol validation/mapping remains owned by rmcp.
- HTTP tool execution failures still return `CallToolResult { is_error: true }` unless an
  explicit JSON-RPC protocol/server error path is hit.

### Tool rules

- `tools.notify_list_changed=true` sets `capabilities.tools.listChanged=true`.
- Tool list-changed notifications are emitted on config reload, explicit `request_list_refresh(...)`, and debounced tool-registry mutation events.
- Registry mutation notifications are debounced with a 1 second per-feature window.
- This library accepts full JSON Schema objects for `tools.items[].input_schema`, but
  downstream AI clients may enforce a subset. For broad compatibility, avoid
  top-level `anyOf`/`oneOf`/`allOf` and prefer object-only constraints
  (`required`, `minProperties`, typed fields). See
  [AI Client Compatibility](AI_CLIENT_COMPATIBILITY.md).
- `server.client_compat.input_schema.top_level_combinators` controls enforcement:
  - `off`: no compatibility check
  - `warn`: emit a startup warning and continue (default)
  - `error`: fail config load when top-level combinators are present
- HTTP tool execution failures (upstream failures/timeouts/response mapping failures) return a `CallToolResult` with `is_error=true`.
- Plugin tool execution failures are always surfaced as tool errors (`CallToolResult` with `is_error=true`):
  - `Ok(CallToolResult::error(...))` remains an explicit tool error.
  - `Err(McpError)` from plugin `call(...)` is converted to a tool error result and sanitized by `server.errors.expose_internal_details`.
- Tool execution panics are caught and converted to terminal tool errors (`is_error=true`) so streamable HTTP callers receive a terminal MCP response instead of a hanging SSE stream.
- JSON-RPC errors remain reserved for protocol/framework failures (unknown tool, invalid request shape, method-not-found, startup/reload misconfiguration).

### Prompt rules

- Prompts capability is enabled only when prompts config is effectively active.
- `prompts.providers` must be non-empty when prompts config is effectively active.
- `prompts.pagination.page_size` overrides global `pagination.page_size` for `prompts/list`.
- If `prompts.pagination` is omitted, global `pagination.page_size` is used for `prompts/list` when present.
- If both prompt/global pagination are absent (or page size `0`), `prompts/list` returns full list and ignores cursor.
- `prompts/get` validates request arguments against each prompt `arguments_schema`; invalid args return `-32602`.
- Core derives MCP prompt arguments from `arguments_schema.properties` and `required`.
- For duplicate prompt names across providers, `prompts/list` may include duplicates and `prompts/get` uses the last provider in config order.
- For plugin providers, `prompts.providers[].config` is shallow-merged over `plugins[].config`.
- `prompts.providers[].items[].completions` (and plugin `PromptEntry.completions`) keys must exist in `arguments_schema.properties`.
- When completion config is active, every prompt completion provider reference must exist in `completion.providers[]`.
- Prompt message content currently supports `text`, `image`, `resource`, and `resource_link` in this library.
- If `prompts.notify_list_changed=true`, list-changed notifications are emitted on config reload, explicit `request_list_refresh(...)`, and debounced prompt-registry mutation events.

### Resource rules

- Resources capability is enabled only when resources config is effectively active.
- `resources.providers` must be non-empty when resources config is effectively active.
- `resources.pagination.page_size` overrides global `pagination.page_size` for both `resources/list` and `resources/templates/list`.
- If `resources.pagination` is omitted, global `pagination.page_size` is used for resources list endpoints when present.
- If both resource/global pagination are absent (or page size `0`), resources list endpoints return full list and ignore cursor.
- `resources.providers[].items[].uri` must parse as a URI.
- `resources.providers[].templates[].uri_template` must parse after placeholder normalization and braces must be balanced.
- Inline providers must define at least one of `items` or `templates`.
- `resources.providers[].templates[].arguments_schema` must compile as JSON Schema.
- `resources.providers[].templates[].completions` keys must exist in `arguments_schema.properties`.
- When completion config is active, every template completion provider reference must exist in `completion.providers[]`.
- `resources/read` prefers exact URI matches from `resources/list`; if no exact resource exists, template matching is attempted.
- For duplicate resource URIs or template URI patterns, list results may contain duplicates and read uses the last provider in config order.
- `resources/read` returns JSON-RPC `-32002` when the URI does not resolve.
- If `resources.clients_can_subscribe=false`, `resources/subscribe` and `resources/unsubscribe` return method-not-found.
- If `resources.notify_list_changed=true`, list-changed notifications are emitted on config reload, explicit `request_list_refresh(...)`, and debounced resource-registry mutation events.
- For plugin providers, `resources.providers[].config` is shallow-merged over `plugins[].config`.

### Completion rules

- Completion capability is advertised only when completion config is active.
- If completion is disabled, `completion/complete` returns method-not-found (`-32601`).
- Prompt/resource completion requests use mappings from:
  - `prompts.providers[].items[].completions`
  - plugin prompt entries (`PromptEntry.completions`)
  - `resources.providers[].templates[].completions`
- `completion/complete` returns `-32602` when prompt/template or argument name is unknown.
- Inline providers use prefix matching on `argument.value`.
- Responses are capped at 100 values. Invalid plugin completion payloads are rejected as internal errors.
- Plugin provider config is shallow-merged (`plugins[].config` overridden by `completion.providers[].config`).

### Auth rules

- Auth is provider-driven:
  - `server.auth` absent => disabled
  - `server.auth.enabled=false` => disabled
  - `server.auth.enabled` omitted/`true` => enabled and `providers` must be non-empty
- Clients authenticate with `Authorization: Bearer <token>`.
- Provider order matters. Core evaluates providers in list order and stops on the first definitive outcome.
- `bearer` compares against configured token.
- `jwks` validates JWT signature/claims (`issuer|discovery_url`, `jwks_url`, `audiences`, `required_scopes`, `required_claims`, `algorithms`, `clock_skew_sec`).
- `introspection` validates via `introspection_url`; `auth_method` controls client auth to introspection endpoint (`basic|post|none`).
- `plugin` delegates auth decisions to an `AuthPlugin`.
- When oauth-capable providers (`jwks`/`introspection`) are configured, `server.auth.oauth.resource` is required.
- Set `server.auth.oauth.public_url` in production so OAuth metadata URLs and challenges are externally correct.
- `required_scopes` uses the `scope` claim; it accepts either space-delimited strings or arrays.
- `audiences` matches the `aud` claim when it is a string or array.
- `required_claims` requires exact string matches.
- Protected resource metadata routes are exposed at:
  - `/.well-known/oauth-protected-resource`
  - `/.well-known/oauth-protected-resource{endpoint_path}` (unless `endpoint_path = /`)
- For 401 auth failures, `WWW-Authenticate` includes `resource_metadata` and may include `scope`.
- For insufficient scope, server returns `403` with `WWW-Authenticate: Bearer error="insufficient_scope"` and optional `scope`.
- Discovery metadata is cached for 10 minutes; JWKS is cached for 10 minutes.
- Discovery candidate priority from `issuer`:
  - OAuth AS metadata: `/.well-known/oauth-authorization-server...`
  - OIDC fallbacks (if enabled): `/.well-known/openid-configuration...`
- `server.auth.oauth.client_metadata_document_url` is optional; when set, it is added to the protected resource metadata payload as `oauth_client_metadata_document_url`.
- If auth is disabled, OAuth metadata routes return 501 and `resource` is omitted.
- If auth is disabled via `enabled=false`, configured providers/oauth settings are ignored.

### Plugin rules

- Plugin names must be globally unique across all types. Duplicate names are rejected at config load and registry registration.
- Every plugin referenced by a tool (`execute.plugin`), auth provider (`type=plugin` `plugin`), or router must be present in `plugins` and registered in the `PluginRegistry`.
- If a plugin is registered but not listed in `plugins`, the server logs a warning and continues.
- If a plugin is listed in `plugins` but not registered, initialization fails.
- `config` is allowed for all plugin types. Core validates only object/null shape and leaves inner validation to plugin/tool implementations.
- `targets` is required for `http_router` and forbidden for all other types.
- Auth plugin `config` is passed to `AuthPlugin::validate` at runtime.
- Tool plugins can call `PluginContext::log_event(LogEventParams { ... })` to emit:
  - server logs (`LogChannel::Server`)
  - client notifications (`LogChannel::Client`)
  - or both.
- `log_event(LogEventParams { channels: &[LogChannel::Client], .. })` sends `notifications/message` only when:
  - `client_logging` section is present
  - event level passes the current threshold
  - request context exists (otherwise no-op + server warning).
- Tool plugins can call `PluginContext::notify_progress(progress, total, message)` to emit `notifications/progress`.

### Logging rules

- `client_logging` section presence declares MCP `logging` capability in server info.
- `client_logging.level` sets the initial client-notification threshold.
- `logging/setLevel` updates that client-notification threshold at runtime.
- `logging/setLevel` never changes `server.logging.level` (server tracing level).
- If logging config is absent, `logging/setLevel` is accepted as a no-op and a server warning is logged.

### Progress rules

- Progress helper sends only when:
  - `progress` section is present
  - request context exists
  - request metadata includes `progressToken`.
- `notification_interval_ms` is applied per request context.
- `progress` must be finite and strictly increasing per request.
- `total` (when present) must be finite.
- If no request context or no progress token exists, `notify_progress(...)` returns `false` and does not emit.

### Tasks rules (experimental)

- Inactive tasks config requires every `tools.items[].execute.task_support` to be `forbidden`.
- `tasks/list` is available only when tasks config is active and `tasks.capabilities.list=true`.
- `tasks/cancel` is available only when tasks config is active and `tasks.capabilities.cancel=true`.
- `tools.items[].execute.task_support` behavior:
  - `forbidden`: task-augmented `tools/call` is rejected (`-32601`).
  - `optional`: both normal and task-augmented calls are accepted.
  - `required`: normal `tools/call` is rejected (`-32601`); task-augmented calls are required.
- `tasks/result` blocks until terminal status (`completed`, `failed`, `cancelled`), then returns the underlying tool result.
- Cancelling a terminal task returns `invalid_params` (`-32602`).
- If `tasks.status_notifications=true`, core emits `notifications/tasks/status` on terminal transitions.
- Current rmcp SDK behavior (v1.1.0): tools expose task support via `_meta.execution.taskSupport`.

### HTTP request rules

- `tools.items[].execute.method` is required for `execute.type=http`.
- `tools.items[].execute.path` must render to a string.
- `query` and `headers` templates must render to objects; null values are omitted.
- Header precedence (later overrides earlier): `outbound_http.headers/user_agent` -> `upstreams.headers/user_agent` -> `upstreams.auth` -> `tools.items[].execute.headers`.
- Timeout precedence: request/tool override -> `upstreams.<name>.timeout_ms` -> `outbound_http.timeout_ms`.
- Response-size precedence: request/tool override -> `upstreams.<name>.max_response_bytes` -> `outbound_http.max_response_bytes` -> built-in default `1048576`.

### Response rules

- `response` omitted => structured passthrough behavior.
- `response.type=structured`:
  - if `template` is omitted, raw upstream JSON is used as structured output.
  - when `output_schema` is set, structured output is validated against it.
- `response.type=content`:
  - renders configured `items` content blocks.
  - `output_schema` does not apply.
- Structured response flow expects upstream JSON; non-JSON upstream payloads fail in structured path.

### Cancellation rules

- `tools.items[].cancellable` defaults to `true`.
- For `cancellable=true`:
  - HTTP tools are aborted using request cancellation token when available.
  - Plugin tools are aborted by core before plugin completion when cancellation is observed.
  - Cancelled tool calls return a tool error result (`is_error=true`) with message `"request cancelled"`.
- For `cancellable=false`:
  - Core does not auto-abort the tool call.
  - Plugins receive a fresh non-cancelled `PluginContext.cancellation` token.
- Cancelled non-tool requests (`logging/setLevel`, `tools/list`) return JSON-RPC error code `-32000` with message `"request cancelled"`.

### HTTP router plugin rules

- Router plugins only apply when `server.transport.mode = streamable_http`.
- Plugin `config` for `http_router` type is opaque to core; plugins must validate it.
- `targets` cannot be empty.
- `route` targets cannot use `*`.
- Route targets cannot collide with the MCP endpoint path or OAuth metadata path.
- Wrap targets may be `*`, the normalized MCP endpoint path, or a previously-declared plugin route.
- To wrap the MCP endpoint, add an explicit `wrap` target using the configured endpoint path (e.g., `/mcp`).
- Wrap order matters: layers are applied in the order listed, with earlier wraps becoming outermost.
- A `*` wrap only applies to routes known at the time it is processed.

## Templating

Use templating to interpolate tool arguments into
`execute.path`/`execute.query`/`execute.headers`/`execute.body`, and
to project the upstream JSON into `response.template` or `response.items`.
Templating is supported in `execute.path`, `execute.query`, `execute.headers`, `execute.body`,
`response.template`, and `response.items`.

- `${field}`: required value (from tool args)
- `${field?}`: optional; omitted if missing
- `${field|default(10)}`: default value if missing
- `${field|csv}`: arrays rendered as CSV
- `${$.path}`: JSON path against upstream response (structured/content templates)
- `$` or `${$}`: entire upstream response

When using `template` and you want the full upstream response, you
can use `${$}` directly:

```
template: ${$}
```

If the upstream response is an array and your client expects
`structuredContent` to be an object, wrap it:

```yaml
template:
  data: ${$}
```

See [AI Client Compatibility](AI_CLIENT_COMPATIBILITY.md) for client-specific
constraints.
