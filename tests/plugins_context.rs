#![cfg(feature = "http_tools")]

mod config_common;

use std::{collections::HashMap, sync::Arc};

use config_common::{load_config_fixture, outbound_http_config, upstream_config};
use httpmock::{Method::GET, Method::POST, MockServer};
use rmcp::model::{ErrorCode, Extensions};
use rmcp::ErrorData as McpError;
use rust_mcp_core::{
    config::{OutboundHttpConfig, OutboundRetryConfig, UpstreamAuth, UpstreamConfig},
    engine::{Engine, EngineConfig},
    plugins::{
        ListFeature, ListRefreshHandle, PluginContext, PluginRegistry, PluginSendAuthMode,
        PluginSendOptions,
    },
    OutboundHttpRequest, OutboundHttpResponse, ReqwestHttpClient,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

struct MockListRefreshHandle;

#[async_trait::async_trait]
impl ListRefreshHandle for MockListRefreshHandle {
    async fn refresh_list(&self, feature: ListFeature) -> Result<bool, McpError> {
        Ok(matches!(feature, ListFeature::Tools))
    }
}

fn assert_json_response(response: &OutboundHttpResponse, expected: &Value) {
    let body: Value = response.json().expect("response body should be valid JSON");
    assert_eq!(body, *expected);
}

#[tokio::test]
async fn plugin_context_uses_request_cancellation_and_progress() {
    let fixture = load_config_fixture("plugins/plugin_context_request_fixture");
    let engine = Engine::from_config(EngineConfig {
        config: fixture,
        plugins: PluginRegistry::default(),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut server_service = rmcp::service::serve_directly(engine, server_io, None);

    let mut meta = rmcp::model::Meta::new();
    meta.set_progress_token(rmcp::model::ProgressToken(
        rmcp::model::NumberOrString::Number(7),
    ));

    let request_ct = CancellationToken::new();
    let request = rmcp::service::RequestContext {
        peer: server_service.peer().clone(),
        ct: request_ct.clone(),
        id: rmcp::model::NumberOrString::Number(1),
        meta,
        extensions: Extensions::default(),
    };

    let context = PluginContext::new(
        Some(request),
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    );

    let progress_token = context
        .progress
        .as_ref()
        .expect("progress token should be set from request meta");
    assert_eq!(
        progress_token.0,
        rmcp::model::NumberOrString::Number(7),
        "progress token should preserve the value set in request meta"
    );
    assert!(!context.cancellation.is_cancelled());
    request_ct.cancel();
    assert!(context.cancellation.is_cancelled());

    let _ = server_service.close().await;
}

#[tokio::test]
async fn plugin_context_refresh_list_requires_runtime_handle() {
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    );
    let error = context
        .request_list_refresh(ListFeature::Tools)
        .await
        .expect_err("refresh should fail without runtime handle");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "list refresh unavailable");
}

#[tokio::test]
async fn plugin_context_refresh_list_delegates_to_handle() {
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_list_refresh_handle(Arc::new(MockListRefreshHandle));

    let tools_changed = context
        .request_list_refresh(ListFeature::Tools)
        .await
        .expect("tools refresh");
    assert!(tools_changed);

    let prompts_changed = context
        .request_list_refresh(ListFeature::Prompts)
        .await
        .expect("prompts refresh");
    assert!(!prompts_changed);
}

#[tokio::test]
async fn plugin_http_raw_send_does_not_apply_outbound_default_timeout() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/timeout-default");
            then.status(200)
                .delay(std::time::Duration::from_millis(120))
                .json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(outbound_http_config(Some(20), None, None)));

    let response = context
        .send_raw(OutboundHttpRequest {
            method: "GET".to_owned(),
            url: server.url("/timeout-default"),
            ..OutboundHttpRequest::default()
        })
        .await
        .expect("raw send should not apply outbound default timeout");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_http_raw_send_does_not_apply_outbound_default_max_response() {
    let server = MockServer::start_async().await;
    let payload = serde_json::json!({"payload": "x".repeat(96)});
    let body = payload.to_string();
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/default-max-response");
            then.status(200).body(body.as_str());
        })
        .await;
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(outbound_http_config(None, Some(64), None)));

    let response = context
        .send_raw(OutboundHttpRequest {
            method: "GET".to_owned(),
            url: server.url("/default-max-response"),
            ..OutboundHttpRequest::default()
        })
        .await
        .expect("raw send should not apply outbound default max_response_bytes");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &payload);
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_http_unset_defaults_do_not_apply_extra_limits() {
    let server = MockServer::start_async().await;
    let payload = serde_json::json!({"payload": "x".repeat(4096)});
    let body = payload.to_string();
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/no-defaults");
            then.status(200).body(body.as_str());
        })
        .await;
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send_raw(OutboundHttpRequest {
            method: "GET".to_owned(),
            url: server.url("/no-defaults"),
            ..OutboundHttpRequest::default()
        })
        .await
        .expect("raw send should succeed without defaults");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &payload);
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_applies_upstream_defaults_and_path_join() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/upstream-defaults");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            timeout_ms: Some(500),
            max_response_bytes: Some(16),
            ..upstream_config(server.base_url())
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/upstream-defaults".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("send should apply upstream defaults and join path");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_applies_outbound_defaults_when_upstream_unset() {
    let server = MockServer::start_async().await;
    let body = "x".repeat(96);
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/upstream-outbound-default-max");
            then.status(200).body(body.as_str());
        })
        .await;

    let mut upstreams = HashMap::new();
    upstreams.insert("primary".to_owned(), upstream_config(server.base_url()));
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(outbound_http_config(None, Some(64), None)));

    let result = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/upstream-outbound-default-max".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await;

    let error = result.expect_err("outbound defaults should set request max_response_bytes");
    assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.message,
        "outbound response body exceeds configured max_response_bytes"
    );
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_applies_default_timeout_when_upstream_unset() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/timeout-default-applied");
            then.status(200)
                .delay(std::time::Duration::from_millis(120))
                .body("ok");
        })
        .await;

    let mut upstreams = HashMap::new();
    upstreams.insert("primary".to_owned(), upstream_config(server.base_url()));
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(outbound_http_config(Some(20), None, None)));

    let result = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/timeout-default-applied".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await;

    let error = result.expect_err("default timeout should be applied");
    assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.contains("error sending request"),
        "timeout should produce a reqwest send error, got: {}",
        error.message
    );
    assert!(
        error.message.contains("/timeout-default-applied"),
        "error should reference the request URL path, got: {}",
        error.message
    );
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_prefers_upstream_timeout_over_default() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/timeout-upstream-over-default");
            then.status(200)
                .delay(std::time::Duration::from_millis(120))
                .json_body(serde_json::json!({"ok": true}));
        })
        .await;

    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            timeout_ms: Some(200),
            ..upstream_config(server.base_url())
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(outbound_http_config(Some(20), None, None)));

    let result = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/timeout-upstream-over-default".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await;

    let response = result.expect("upstream timeout (200ms) should win over default (20ms)");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_retries_retryable_status_when_enabled() {
    let server = MockServer::start_async().await;
    let retryable = server
        .mock_async(|when, then| {
            when.method(GET).path("/retry-status");
            then.status(503)
                .json_body(serde_json::json!({"error": "unavailable"}));
        })
        .await;

    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            base_url: server.base_url(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: Some(OutboundRetryConfig {
                max_attempts: 2,
                delay_ms: 1,
                on_network_errors: false,
                on_statuses: vec![503],
            }),
            auth: None,
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/retry-status".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("request should return final upstream response");
    assert_eq!(response.status(), 503);
    assert!(!response.is_success());
    assert_json_response(&response, &serde_json::json!({"error": "unavailable"}));

    retryable.assert_calls_async(2).await;
}

#[tokio::test]
async fn plugin_context_send_does_not_retry_non_idempotent_method() {
    let server = MockServer::start_async().await;
    let first = server
        .mock_async(|when, then| {
            when.method(POST).path("/retry-post");
            then.status(503)
                .json_body(serde_json::json!({"error": "unavailable"}));
        })
        .await;

    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            base_url: server.base_url(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: Some(OutboundRetryConfig {
                max_attempts: 3,
                delay_ms: 1,
                on_network_errors: false,
                on_statuses: vec![503],
            }),
            auth: None,
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "POST".to_owned(),
                url: "/retry-post".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("request should return first response without retry");
    assert_eq!(response.status(), 503);
    assert!(!response.is_success());
    assert_json_response(&response, &serde_json::json!({"error": "unavailable"}));

    first.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_applies_header_defaults_and_upstream_auth() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/headers-merge")
                .header("User-Agent", "plugin-agent")
                .header("X-Default", "yes")
                .header("X-Upstream", "ok")
                .header("X-Request", "plugin")
                .header("Authorization", "Bearer upstream-token");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            base_url: server.base_url(),
            headers: HashMap::from([("X-Upstream".to_owned(), "ok".to_owned())]),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Bearer {
                token: "upstream-token".to_owned(),
            }),
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    )
    .with_outbound_http(Some(OutboundHttpConfig {
        headers: HashMap::from([("X-Default".to_owned(), "yes".to_owned())]),
        user_agent: Some("plugin-agent".to_owned()),
        timeout_ms: None,
        max_response_bytes: None,
        retry: None,
    }));

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/headers-merge".to_owned(),
                headers: vec![("X-Request".to_owned(), "plugin".to_owned())],
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("send with header defaults and upstream auth should succeed");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_request_headers_override_auth_headers() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/headers-override-auth")
                .header("Authorization", "Bearer request-token");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let mut upstreams = HashMap::new();
    upstreams.insert(
        "primary".to_owned(),
        UpstreamConfig {
            base_url: server.base_url(),
            headers: HashMap::new(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: Some(UpstreamAuth::Basic {
                username: "user".to_owned(),
                password: "pass".to_owned(),
            }),
        },
    );
    let context = PluginContext::new(
        None,
        Arc::new(upstreams),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/headers-override-auth".to_owned(),
                headers: vec![(
                    "Authorization".to_owned(),
                    "Bearer request-token".to_owned(),
                )],
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("request-level auth header should override upstream auth");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_with_none_disables_upstream_auth() {
    let server = MockServer::start_async().await;
    let no_auth = server
        .mock_async(|when, then| {
            when.method(GET).path("/headers-no-auth");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let with_auth = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/headers-no-auth")
                .header_exists("authorization");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let mut upstream = upstream_config(server.base_url());
    upstream.auth = Some(UpstreamAuth::Bearer {
        token: "upstream-token".to_owned(),
    });
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::from([("primary".to_owned(), upstream)])),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send_with(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/headers-no-auth".to_owned(),
                ..OutboundHttpRequest::default()
            },
            PluginSendOptions {
                auth: PluginSendAuthMode::None,
            },
        )
        .await
        .expect("send_with auth=None should succeed without auth headers");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    no_auth.assert_calls_async(1).await;
    with_auth.assert_calls_async(0).await;
}

#[tokio::test]
async fn plugin_context_send_with_explicit_auth_overrides_upstream_auth() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/headers-explicit-auth")
                .header("authorization", "Bearer explicit-token");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let mut upstream = upstream_config(server.base_url());
    upstream.auth = Some(UpstreamAuth::Basic {
        username: "user".to_owned(),
        password: "pass".to_owned(),
    });
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::from([("primary".to_owned(), upstream)])),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send_with(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/headers-explicit-auth".to_owned(),
                ..OutboundHttpRequest::default()
            },
            PluginSendOptions {
                auth: PluginSendAuthMode::Explicit {
                    authorization: "Bearer explicit-token".to_owned(),
                },
            },
        )
        .await
        .expect("explicit auth should override upstream auth");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_accepts_absolute_url() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/upstream-absolute");
            then.status(200).json_body(serde_json::json!({"ok": true}));
        })
        .await;
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::from([(
            "primary".to_owned(),
            UpstreamConfig {
                base_url: "https://unused.example".to_owned(),
                headers: HashMap::new(),
                user_agent: None,
                timeout_ms: None,
                max_response_bytes: None,
                retry: None,
                auth: None,
            },
        )])),
        Arc::new(ReqwestHttpClient::default()),
    );

    let response = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: server.url("/upstream-absolute"),
                ..OutboundHttpRequest::default()
            },
        )
        .await
        .expect("absolute URL should bypass base_url and succeed");
    assert_eq!(response.status(), 200);
    assert!(response.is_success());
    assert_json_response(&response, &serde_json::json!({"ok": true}));
    mock.assert_calls_async(1).await;
}

#[tokio::test]
async fn plugin_context_send_rejects_non_absolute_or_non_path_urls() {
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::from([(
            "primary".to_owned(),
            UpstreamConfig {
                base_url: "https://example.com".to_owned(),
                headers: HashMap::new(),
                user_agent: None,
                timeout_ms: None,
                max_response_bytes: None,
                retry: None,
                auth: None,
            },
        )])),
        Arc::new(ReqwestHttpClient::default()),
    );

    let result = context
        .send(
            "primary",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "relative-path-without-slash".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await;

    let error = result.expect_err("invalid upstream URL form should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "plugin upstream request url must be absolute or begin with '/'"
    );
}

#[tokio::test]
async fn plugin_context_send_rejects_unknown_upstream() {
    let context = PluginContext::new(
        None,
        Arc::new(HashMap::new()),
        Arc::new(ReqwestHttpClient::default()),
    );

    let result = context
        .send(
            "missing",
            OutboundHttpRequest {
                method: "GET".to_owned(),
                url: "/anything".to_owned(),
                ..OutboundHttpRequest::default()
            },
        )
        .await;

    let error = result.expect_err("unknown upstream should fail");
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(error.message, "upstream 'missing' not found");
}
