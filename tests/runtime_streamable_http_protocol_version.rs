#![cfg(all(
    feature = "streamable_http",
    any(feature = "auth", feature = "http_tools")
))]

mod config_common;

use std::net::TcpListener;

use config_common::base_config_streamable_http_with_builtin_noop;
use rust_mcp_core::config::ProtocolVersionNegotiationMode;
use rust_mcp_core::{runtime, PluginRegistry};
use serde_json::json;
use tokio::time::{sleep, Duration, Instant};

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

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "integration test with sequential protocol steps"
)]
async fn streamable_http_protocol_version_strict_rejects_unknown_header() {
    let mut config = base_config_streamable_http_with_builtin_noop();
    let port = reserve_loopback_port();
    config.server.port = port;
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let initialize = json!({
        "jsonrpc":"2.0",
        "id": 1,
        "method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"test-client","version":"1.0.0"}
        }
    });

    let server =
        tokio::spawn(async move { runtime::run_from_config(config, PluginRegistry::new()).await });
    wait_for_server(port).await;

    let client = reqwest::Client::new();

    let initialize_response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .expect("initialize request should complete");
    assert_eq!(initialize_response.status(), reqwest::StatusCode::OK);
    let session_id = initialize_response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("initialize response should include MCP session id");

    let initialized = json!({
        "jsonrpc":"2.0",
        "method":"notifications/initialized",
        "params":{}
    });

    let invalid = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .header("mcp-protocol-version", "9999-01-01")
        .json(&initialized)
        .send()
        .await
        .expect("invalid protocol version request should complete");
    assert_eq!(
        invalid.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "strict mode must reject unknown protocol version with 400"
    );
    let invalid_body = invalid
        .text()
        .await
        .expect("invalid protocol version response body should read");
    assert!(
        !invalid_body.is_empty(),
        "strict rejection should include a response body, got empty"
    );
    assert!(
        invalid_body.contains("Unsupported MCP-Protocol-Version"),
        "strict rejection body should include unsupported-version token, got: {invalid_body}"
    );
    assert!(
        invalid_body.contains("9999-01-01"),
        "strict rejection body should include rejected version token, got: {invalid_body}"
    );

    let missing = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&initialized)
        .send()
        .await
        .expect("missing protocol header request should complete");
    assert_eq!(
        missing.status(),
        reqwest::StatusCode::ACCEPTED,
        "strict mode should accept requests with missing protocol version header"
    );

    server.abort();
    let _ = server.await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "integration test with sequential protocol steps"
)]
async fn streamable_http_protocol_version_negotiate_drops_unknown_header() {
    let mut config = base_config_streamable_http_with_builtin_noop();
    config
        .server
        .transport
        .streamable_http
        .protocol_version_negotiation
        .mode = ProtocolVersionNegotiationMode::Negotiate;
    let port = reserve_loopback_port();
    config.server.port = port;
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let initialize = json!({
        "jsonrpc":"2.0",
        "id": 1,
        "method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"test-client","version":"1.0.0"}
        }
    });

    let server =
        tokio::spawn(async move { runtime::run_from_config(config, PluginRegistry::new()).await });
    wait_for_server(port).await;

    let client = reqwest::Client::new();

    let initialize_response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .expect("initialize request should complete");
    assert_eq!(initialize_response.status(), reqwest::StatusCode::OK);
    let session_id = initialize_response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("initialize response should include MCP session id");

    let initialized = json!({
        "jsonrpc":"2.0",
        "method":"notifications/initialized",
        "params":{}
    });

    let with_unknown = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .header("mcp-protocol-version", "9999-01-01")
        .json(&initialized)
        .send()
        .await
        .expect("unknown protocol version request should complete");
    assert_eq!(
        with_unknown.status(),
        reqwest::StatusCode::ACCEPTED,
        "negotiate mode should drop unknown protocol version and accept request"
    );

    // Verify session remains usable after negotiation: a follow-up tools/list
    // request with a valid protocol version header should succeed.
    let tools_list = json!({
        "jsonrpc":"2.0",
        "id": 2,
        "method":"tools/list",
        "params":{}
    });
    let followup = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .header("mcp-protocol-version", "2025-06-18")
        .json(&tools_list)
        .send()
        .await
        .expect("follow-up request should complete");
    assert_eq!(
        followup.status(),
        reqwest::StatusCode::OK,
        "session should remain usable after negotiate dropped unknown header"
    );
    let followup_body = followup
        .text()
        .await
        .expect("follow-up response body should read");
    assert!(
        followup_body.contains("\"result\""),
        "follow-up tools/list should return a valid JSON-RPC result, got: {followup_body}"
    );

    server.abort();
    let _ = server.await;
}
