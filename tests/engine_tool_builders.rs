#![cfg(feature = "http_tools")]

mod engine_common;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use engine_common::load_tool_fixture;
use httpmock::{Method::GET, MockServer};
use rust_mcp_core::engine::Engine;

#[tokio::test]
async fn content_response_mode_fixture() {
    let fixture = load_tool_fixture("engine/engine_content_response_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool ok");

    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert_eq!(result.content.len(), 2);
    let first_text = result.content[0]
        .as_text()
        .expect("first content should be text")
        .text
        .as_str();
    assert_eq!(first_text, "static");
    let second_text = result.content[1]
        .as_text()
        .expect("second content should be text")
        .text
        .as_str();
    assert_eq!(second_text, "5");
}

#[tokio::test]
async fn content_response_without_items_fixture() {
    let fixture = load_tool_fixture("engine/engine_content_no_items_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/empty");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool ok");

    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.is_none());
    assert!(result.content.is_empty());
}

#[tokio::test]
async fn response_fallback_json_text_fixture() {
    let fixture = load_tool_fixture("engine/engine_response_fallback_json_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/fallback");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool ok");

    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .as_ref()
        .expect("structured_content should be present");
    assert_eq!(structured, &serde_json::json!({"ok": true}));
    assert_eq!(result.content.len(), 1);
    let fallback_text = result.content[0]
        .as_text()
        .expect("fallback content should be text")
        .text
        .as_str();
    assert_eq!(fallback_text, "{\"ok\":true}");
}

#[tokio::test]
async fn headers_basic_auth_fixture() {
    let fixture = load_tool_fixture("engine/engine_headers_basic_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let expected_auth = format!("Basic {}", STANDARD.encode("user:pass"));

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path("/headers")
            .header("User-Agent", "mcp-test")
            .header("X-Default", "yes")
            .header("X-Upstream", "ok")
            .header("X-Tool", "tool")
            .header("Authorization", expected_auth);
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("basic auth tool should succeed");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
    assert_eq!(result.content.len(), 1);
    let text = result.content[0]
        .as_text()
        .expect("content should be text")
        .text
        .as_str();
    assert_eq!(text, "{\"ok\":true}");
}

#[tokio::test]
async fn headers_bearer_auth_fixture() {
    let fixture = load_tool_fixture("engine/engine_headers_bearer_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path("/bearer")
            .header("Authorization", "Bearer secret");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("bearer auth tool should succeed");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
    assert_eq!(result.content.len(), 1);
    let text = result.content[0]
        .as_text()
        .expect("content should be text")
        .text
        .as_str();
    assert_eq!(text, "{\"ok\":true}");
}

#[tokio::test]
async fn upstream_user_agent_overrides_global_default_fixture() {
    let fixture = load_tool_fixture("engine/engine_headers_upstream_user_agent_override_fixture");
    let server = MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path("/headers-ua")
            .header("User-Agent", "upstream-agent");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::new(config).expect("engine should build");
    let result = engine
        .execute_tool(&fixture.tool, fixture.args)
        .await
        .expect("tool should succeed");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
}
