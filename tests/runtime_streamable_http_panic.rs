#![cfg(feature = "streamable_http")]

mod config_common;

use std::net::TcpListener;

use async_trait::async_trait;
use config_common::base_config_streamable_http_with_builtin_noop;
use rmcp::{model::CallToolResult, ErrorData as McpError};
use rust_mcp_core::{
    config::{
        ExecuteConfig, ExecutePluginConfig, McpConfig, PluginConfig, TaskSupport, ToolConfig,
    },
    plugins::{PluginCallParams, PluginRegistry, PluginType, ToolPlugin},
    runtime,
};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout, Duration, Instant};

struct PanicPlugin;

#[async_trait]
impl ToolPlugin for PanicPlugin {
    fn name(&self) -> &'static str {
        "panic.tool"
    }

    async fn call(
        &self,
        _args: Value,
        _params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        panic!("streamable panic");
    }
}

fn reserve_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback port");
    listener.local_addr().expect("local addr").port()
}

async fn wait_for_server(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("streamable HTTP server did not start on port {port}");
}

fn panic_plugin_config(port: u16) -> McpConfig {
    let mut config = base_config_streamable_http_with_builtin_noop();
    config.server.port = port;
    config.set_tools_items(vec![ToolConfig {
        name: "panic.tool".to_owned(),
        title: None,
        description: "panics".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "panic.tool".to_owned(),
            config: None,
            task_support: TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.plugins = vec![PluginConfig {
        name: "panic.tool".to_owned(),
        plugin_type: PluginType::Tool,
        targets: None,
        config: None,
    }];
    config
}

fn initialize_payload() -> Value {
    json!({
        "jsonrpc":"2.0",
        "id": 1,
        "method":"initialize",
        "params":{
            "protocolVersion":"2025-06-18",
            "capabilities":{},
            "clientInfo":{"name":"test-client","version":"1.0.0"}
        }
    })
}

fn initialized_payload() -> Value {
    json!({
        "jsonrpc":"2.0",
        "method":"notifications/initialized",
        "params":{}
    })
}

fn call_payload() -> Value {
    json!({
        "jsonrpc":"2.0",
        "id": 2,
        "method":"tools/call",
        "params":{
            "name":"panic.tool",
            "arguments":{}
        }
    })
}

async fn initialize_session(client: &reqwest::Client, endpoint: &str) -> String {
    let initialize_response = client
        .post(endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&initialize_payload())
        .send()
        .await
        .expect("initialize request should complete");
    assert_eq!(initialize_response.status(), reqwest::StatusCode::OK);
    initialize_response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("initialize response should include MCP session id")
}

async fn post_session_request(
    client: &reqwest::Client,
    endpoint: &str,
    session_id: &str,
    payload: &Value,
) -> reqwest::Response {
    client
        .post(endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", session_id)
        .header("mcp-protocol-version", "2025-06-18")
        .json(payload)
        .send()
        .await
        .expect("session request should complete")
}

#[tokio::test]
async fn streamable_http_panicking_tool_returns_terminal_error_without_hang() {
    let port = reserve_loopback_port();
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let config = panic_plugin_config(port);

    let registry = PluginRegistry::new()
        .register_tool(PanicPlugin)
        .expect("register panic plugin");
    let server = tokio::spawn(async move { runtime::run_from_config(config, registry).await });
    wait_for_server(port).await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("reqwest client");

    let session_id = initialize_session(&client, &endpoint).await;
    let initialized_response =
        post_session_request(&client, &endpoint, &session_id, &initialized_payload()).await;
    assert_eq!(initialized_response.status(), reqwest::StatusCode::ACCEPTED);

    let call_response = timeout(
        Duration::from_secs(2),
        post_session_request(&client, &endpoint, &session_id, &call_payload()),
    )
    .await
    .expect("tools/call should not hang");
    assert_eq!(call_response.status(), reqwest::StatusCode::OK);

    let body = timeout(Duration::from_secs(2), call_response.text())
        .await
        .expect("tools/call body should complete")
        .expect("tools/call body should read");

    // Parse the SSE data frames to extract the terminal JSON-RPC response.
    // Filter to non-empty data lines (SSE stream may include empty data lines for keep-alive).
    let data_lines: Vec<&str> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| !data.is_empty())
        .collect();
    assert_eq!(
        data_lines.len(),
        1,
        "expected exactly one non-empty SSE data frame (terminal), got {}: {body}",
        data_lines.len()
    );

    let frame: serde_json::Value =
        serde_json::from_str(data_lines[0]).expect("terminal frame should be valid JSON");

    // Assert JSON-RPC structure.
    assert_eq!(
        frame["jsonrpc"], "2.0",
        "terminal frame must be valid JSON-RPC 2.0"
    );
    assert_eq!(
        frame["id"], 2,
        "terminal frame request id must match the tools/call request id"
    );

    // Assert tool result contract.
    let result = &frame["result"];
    assert_eq!(
        result["isError"], true,
        "terminal frame must indicate tool error"
    );
    let content = result["content"]
        .as_array()
        .expect("result.content should be an array");
    assert_eq!(content.len(), 1, "should have exactly one content item");
    assert_eq!(content[0]["type"], "text", "content type should be text");
    let error_text = content[0]["text"]
        .as_str()
        .expect("content text should be a string");
    assert_eq!(
        error_text, "internal server error",
        "panic tool error must be sanitized to generic message"
    );

    server.abort();
    let _ = server.await;
}
