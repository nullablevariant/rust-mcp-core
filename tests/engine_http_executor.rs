#![cfg(feature = "http_tools")]

use std::path::PathBuf;
use std::time::Duration;

use httpmock::{Method::GET, Method::POST, MockServer};
use rmcp::model::ErrorCode;
use rust_mcp_core::engine::http_executor::{execute_http, HttpRequestTemplate};
use rust_mcp_core::ReqwestHttpClient;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct HttpFixture {
    method: String,
    path: String,
    query: Option<Value>,
    headers: Option<Value>,
    body: Option<Value>,
    args: Value,
    expected_response: Option<Value>,
    timeout_ms: Option<u64>,
    max_response_bytes: Option<u64>,
    expected_path: Option<String>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> HttpFixture {
    let raw = std::fs::read_to_string(fixture_path(name)).expect("fixture should read");
    serde_yaml::from_str(&raw).expect("fixture should parse")
}

#[tokio::test]
async fn executes_get_with_query_fixture() {
    let fixture = load_fixture("http/http_get_fixture");
    let server = MockServer::start();
    let expected = fixture.expected_response.clone().unwrap();

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path(&fixture.path)
            .query_param("q", "test")
            .header("X-Client", "mcp");
        then.status(200)
            .json_body(fixture.expected_response.clone().unwrap());
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: None,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("http GET request should succeed");

    assert_eq!(response, expected);
    assert_eq!(
        response.get("ok"),
        Some(&Value::Bool(true)),
        "response should contain ok: true"
    );
    assert!(
        response.get("items").unwrap().as_array().unwrap().len() == 1,
        "response should contain exactly one item"
    );
}

#[tokio::test]
async fn executes_post_with_body_fixture() {
    let fixture = load_fixture("http/http_post_fixture");
    let server = MockServer::start();
    let expected = fixture.expected_response.clone().unwrap();

    let _mock = server.mock(|when, then| {
        when.method(POST)
            .path(&fixture.path)
            .header("X-Client", "mcp")
            .json_body(serde_json::json!({"name": "alpha"}));
        then.status(200)
            .json_body(fixture.expected_response.clone().unwrap());
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: None,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("http POST request should succeed");

    assert_eq!(response, expected);
    assert_eq!(
        response.get("id"),
        Some(&serde_json::json!(123)),
        "response should contain id: 123"
    );
    assert_eq!(
        response.get("name"),
        Some(&Value::String("alpha".to_owned())),
        "response should contain name: alpha"
    );
}

#[tokio::test]
async fn path_not_string_returns_error_fixture() {
    let fixture = load_fixture("http/http_path_not_string_fixture");
    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let error = execute_http(&client, "http://localhost", &template, &fixture.args)
        .await
        .expect_err("path rendering to non-string should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_REQUEST,
        "path-not-string should produce INVALID_REQUEST code"
    );
    assert_eq!(
        error.message, "path must render to a string",
        "error message should be exact path validation message"
    );
}

#[tokio::test]
async fn path_without_slash_is_prefixed_fixture() {
    let fixture = load_fixture("http/http_path_without_slash_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET).path(&expected_path);
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("path without slash should be auto-prefixed and succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
    assert_eq!(
        expected_path, "/items",
        "fixture expected_path should confirm slash was prefixed"
    );
}

#[tokio::test]
async fn timeout_and_bool_query_fixture() {
    let fixture = load_fixture("http/http_timeout_bool_query_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET).path(&expected_path);
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("request with timeout and bool query should succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
}

#[tokio::test]
async fn timeout_expiry_returns_error() {
    let server = MockServer::start();

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/slow");
        then.status(200)
            .json_body(serde_json::json!({"ok": true}))
            .delay(Duration::from_millis(500));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: "GET".to_owned(),
        path: "/slow".to_owned(),
        query: None,
        headers: None,
        body: None,
        timeout_ms: Some(50),
        max_response_bytes: None,
    };

    let error = execute_http(
        &client,
        &server.base_url(),
        &template,
        &serde_json::json!({}),
    )
    .await
    .expect_err("request should time out");
    assert_eq!(
        error.code,
        ErrorCode::INTERNAL_ERROR,
        "timeout error should produce INTERNAL_ERROR code"
    );
    assert!(
        error.message.contains("error sending request"),
        "timeout should manifest as a send error, got: {}",
        error.message
    );
}

#[tokio::test]
async fn number_query_value_fixture() {
    let fixture = load_fixture("http/http_query_number_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path(&expected_path)
            .query_param("count", "3");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("request with numeric query value should succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
}

#[tokio::test]
async fn null_query_and_header_values_are_skipped_fixture() {
    let fixture = load_fixture("http/http_null_query_header_values_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET)
            .path(&expected_path)
            .query_param("keep", "value")
            .header("X-Keep", "keep");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("request with null query/header values should succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
}

#[tokio::test]
async fn non_object_query_and_headers_are_ignored_fixture() {
    let fixture = load_fixture("http/http_non_object_query_header_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET).path(&expected_path);
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("request with non-object query/headers should succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
}

#[tokio::test]
async fn all_null_query_values_skip_query_serialization_fixture() {
    let fixture = load_fixture("http/http_query_all_null_fixture");
    let server = MockServer::start();
    let expected_path = fixture.expected_path.clone().expect("expected_path");

    let _mock = server.mock(|when, then| {
        when.method(GET).path(&expected_path);
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: fixture.timeout_ms,
        max_response_bytes: fixture.max_response_bytes,
    };

    let response = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect("request with all-null query values should succeed");
    assert_eq!(
        response,
        serde_json::json!({"ok": true}),
        "response payload should match mock body"
    );
}
