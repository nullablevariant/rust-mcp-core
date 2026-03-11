#![cfg(feature = "http_tools")]

use std::path::PathBuf;

use httpmock::{Method::GET, MockServer};
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

fn template_from_fixture(fixture: HttpFixture) -> (HttpRequestTemplate, Value) {
    (
        HttpRequestTemplate {
            method: fixture.method,
            path: fixture.path,
            query: fixture.query,
            headers: fixture.headers,
            body: fixture.body,
            timeout_ms: None,
            max_response_bytes: None,
        },
        fixture.args,
    )
}

#[tokio::test]
async fn invalid_method_returns_error_fixture() {
    let fixture = load_fixture("http/http_invalid_method_fixture");
    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: None,
        max_response_bytes: None,
    };

    let error = execute_http(&client, "http://localhost", &template, &fixture.args)
        .await
        .expect_err("request with invalid method should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_REQUEST,
        "invalid method should produce INVALID_REQUEST error code"
    );
    assert!(
        error.message.contains("invalid HTTP method"),
        "error message should contain 'invalid HTTP method', got: {}",
        error.message
    );
}

#[tokio::test]
async fn upstream_error_status_returns_error_fixture() {
    let fixture = load_fixture("http/http_error_status_fixture");
    let server = MockServer::start();

    let _mock = server.mock(|when, then| {
        when.method(GET).path(&fixture.path);
        then.status(500);
    });

    let client = ReqwestHttpClient::default();
    let template = HttpRequestTemplate {
        method: fixture.method,
        path: fixture.path,
        query: fixture.query,
        headers: fixture.headers,
        body: fixture.body,
        timeout_ms: None,
        max_response_bytes: None,
    };

    let error = execute_http(&client, &server.base_url(), &template, &fixture.args)
        .await
        .expect_err("upstream 500 should produce error");
    assert_eq!(
        error.code,
        ErrorCode::INTERNAL_ERROR,
        "upstream status error should produce INTERNAL_ERROR code"
    );
    assert_eq!(
        error.message, "upstream returned 500",
        "error message should include status code"
    );
}

#[tokio::test]
async fn path_render_error_returns_error_fixture() {
    let fixture = load_fixture("http/http_path_render_error_fixture");
    let (template, args) = template_from_fixture(fixture);
    let client = ReqwestHttpClient::default();

    let error = execute_http(&client, "http://localhost", &template, &args)
        .await
        .expect_err("path render error should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_PARAMS,
        "path render error should produce INVALID_PARAMS code"
    );
    assert!(
        error.message.contains("missing required value for 'id'"),
        "error message should reference missing 'id' field, got: {}",
        error.message
    );
}

#[tokio::test]
async fn query_render_error_returns_error_fixture() {
    let fixture = load_fixture("http/http_query_render_error_fixture");
    let (template, args) = template_from_fixture(fixture);
    let client = ReqwestHttpClient::default();

    let error = execute_http(&client, "http://localhost", &template, &args)
        .await
        .expect_err("query render error should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_PARAMS,
        "query render error should produce INVALID_PARAMS code"
    );
    assert!(
        error.message.contains("missing required value for 'id'"),
        "error message should reference missing 'id' field, got: {}",
        error.message
    );
}

#[tokio::test]
async fn headers_render_error_returns_error_fixture() {
    let fixture = load_fixture("http/http_headers_render_error_fixture");
    let (template, args) = template_from_fixture(fixture);
    let client = ReqwestHttpClient::default();

    let error = execute_http(&client, "http://localhost", &template, &args)
        .await
        .expect_err("headers render error should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_PARAMS,
        "headers render error should produce INVALID_PARAMS code"
    );
    assert!(
        error.message.contains("missing required value for 'id'"),
        "error message should reference missing 'id' field, got: {}",
        error.message
    );
}

#[tokio::test]
async fn body_render_error_returns_error_fixture() {
    let fixture = load_fixture("http/http_body_render_error_fixture");
    let (template, args) = template_from_fixture(fixture);
    let client = ReqwestHttpClient::default();

    let error = execute_http(&client, "http://localhost", &template, &args)
        .await
        .expect_err("body render error should fail");
    assert_eq!(
        error.code,
        ErrorCode::INVALID_PARAMS,
        "body render error should produce INVALID_PARAMS code"
    );
    assert!(
        error.message.contains("missing required value for 'id'"),
        "error message should reference missing 'id' field, got: {}",
        error.message
    );
}

#[tokio::test]
async fn non_json_response_returns_error_fixture() {
    let fixture = load_fixture("http/http_non_json_response_fixture");
    let (template, args) = template_from_fixture(fixture);
    let server = MockServer::start();

    let _mock = server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).body("not-json");
    });

    let client = ReqwestHttpClient::default();
    let error = execute_http(&client, &server.base_url(), &template, &args)
        .await
        .expect_err("non-JSON response should fail to parse");
    assert_eq!(
        error.code,
        ErrorCode::INTERNAL_ERROR,
        "non-JSON parse error should produce INTERNAL_ERROR code"
    );
    assert!(
        error.message.contains("expected"),
        "error message should describe JSON parse failure, got: {}",
        error.message
    );
}

#[tokio::test]
async fn send_error_returns_error() {
    let fixture = load_fixture("http/http_non_json_response_fixture");
    let (template, args) = template_from_fixture(fixture);
    let client = ReqwestHttpClient::default();

    let error = execute_http(&client, "http://127.0.0.1:1", &template, &args)
        .await
        .expect_err("send to unreachable host should fail");
    assert_eq!(
        error.code,
        ErrorCode::INTERNAL_ERROR,
        "transport/send error should produce INTERNAL_ERROR code"
    );
    assert!(
        error.message.contains("error sending request")
            || error.message.contains("connection refused")
            || error.message.contains("Connection refused"),
        "error message should indicate send/connection failure, got: {}",
        error.message
    );
}
