# Config Reload

This library supports config reload as an explicit API primitive via
`Runtime::reload_config(new_config)`.

## What reload does

`reload_config(...)`:
- runs the same schema/runtime validation path used at startup,
- rebuilds runtime state from the provided `McpConfig`,
- atomically swaps to the new state only if rebuild succeeds,
- evaluates tool/prompt/resource list changes and emits list-changed notifications when enabled.

## What reload does not do

Core does **not**:
- watch config files,
- trigger reload automatically,
- re-read dotenv files automatically.

Hosts must decide when to reload and provide the new `McpConfig` input.

## Failure semantics

If reload fails (schema/validation/build/runtime checks), `reload_config(...)` returns `Err(...)` and
the previous runtime state remains active.

## Recommended integration patterns

Typical host-managed triggers:
- file watcher (`notify`, fs events, etc.),
- admin HTTP endpoint (for example `POST /admin/reload`),
- signal handler (for example `SIGHUP`),
- plugin/admin tool path that calls into a host-owned runtime handle.

## List-changed behavior on reload

On successful reload, core compares old vs new lists:
- tools
- prompts
- resources/resource templates

If a list changed and the corresponding notify flag is enabled, core emits the matching
`notifications/*/list_changed` event.

## Environment variables and `.env`

Config interpolation reads current process env at config-load time.
If you rely on `.env` files, re-read/re-apply them before calling your config loader and then
`reload_config(...)`.
