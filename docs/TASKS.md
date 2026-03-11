# Tasks utility (experimental)

The tasks utility enables long-running tool executions that return results asynchronously. This is an experimental feature in the MCP 2025-11-25 specification.

## Overview

When task-augmented tool calling is enabled, `tools/call` returns a `CreateTaskResult` immediately instead of blocking until the tool completes. Clients poll with `tasks/get` or block with `tasks/result` to retrieve the outcome.

## Configuration

```yaml
tasks:
  enabled: true
  status_notifications: true       # Emit notifications/tasks/status updates
```

## Per-tool task support

Each tool declares its task behavior via `tools.items[].execute.task_support`:

| Mode | Behavior |
|------|----------|
| `forbidden` | Default. Task-augmented calls are rejected for this tool. |
| `optional` | The tool supports both normal and task-augmented calls. |
| `required` | Normal `tools/call` is rejected; clients must use task-augmentation. |

Task support is advertised in the tool's `execution.taskSupport` field during `tools/list`.

## Client interaction flow

1. Client sends `tools/call` with task metadata.
2. Server returns `CreateTaskResult` with a task ID.
3. Client polls with `tasks/get` to check status.
4. Client calls `tasks/result` to block until a terminal status and receive the tool result.
5. Client may call `tasks/cancel` to request cancellation.

## Behavior details

- **Peer isolation**: Each connected client can only see and interact with tasks it created.
- **TTL**: Task TTL is request-scoped via `tools/call.params.task.ttl` (milliseconds). Expired tasks are cleaned up automatically.
- **Cooperative cancellation**: `tasks/cancel` sets the task's cancellation token, which the tool plugin can observe via `ctx.cancellation.is_cancelled()`. Cancelling a task that has already reached a terminal status returns `invalid_params` (-32602).
- **Status notifications**: When `tasks.status_notifications` is true, the server may emit `notifications/tasks/status` on selected status transitions. Clients must not rely on receiving every transition and should continue polling with `tasks/get`.

## Why not the SDK's task handler?

The rmcp SDK provides `OperationProcessor` + `#[task_handler]` as a quick-start scaffold for task support. rust-mcp-core uses its own `TaskStore` because this library requires stricter behavior (peer isolation, deterministic lifecycle metadata, TTL/cleanup, and cooperative cancellation).  

The table below reflects this project's current comparison against SDK scaffolding behavior as evaluated with rmcp v1.1.0:

| Concern | SDK `#[task_handler]` | rust-mcp-core `TaskStore` |
|---------|----------------------|---------------------------|
| Peer isolation | None (any client can access any task) | Enforced per-connection |
| Timestamps | Fake `created_at` on every query | Stored once at creation |
| Task IDs | Uses JSON-RPC request ID (predictable) | Random UUIDs |
| Result waiting | `sleep(100ms)` polling loop | Notification-based wake-up |
| TTL/expiry | None | Configurable with automatic cleanup |
| Cancellation | None | Cooperative via `CancellationToken` |
| Status notifications | None | `notifications/tasks/status` |
