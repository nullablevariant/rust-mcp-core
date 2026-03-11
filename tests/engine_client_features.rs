#![cfg(feature = "client_features")]

mod e2e_common;

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::model::ErrorCode;
use rmcp::service::{RoleClient, RoleServer, RunningService};
use rmcp::ServiceExt;
use rust_mcp_core::config::{
    ClientElicitationConfig, ClientFeaturesConfig, ClientRootsConfig, ClientSamplingConfig,
    ElicitationMode,
};
use rust_mcp_core::default_http_client;
use rust_mcp_core::plugins::PluginContext;
use serde_json::json;

fn plugin_context_with_features(
    features: ClientFeaturesConfig,
    request: Option<rmcp::service::RequestContext<rmcp::service::RoleServer>>,
) -> PluginContext {
    PluginContext::new(request, Arc::new(HashMap::new()), default_http_client())
        .with_client_features(features)
}

fn enabled_roots() -> ClientFeaturesConfig {
    ClientFeaturesConfig {
        roots: Some(ClientRootsConfig { enabled: true }),
        ..Default::default()
    }
}

fn enabled_sampling() -> ClientFeaturesConfig {
    ClientFeaturesConfig {
        sampling: Some(ClientSamplingConfig {
            enabled: Some(true),
            allow_tools: false,
        }),
        ..Default::default()
    }
}

fn enabled_elicitation(mode: ElicitationMode) -> ClientFeaturesConfig {
    ClientFeaturesConfig {
        elicitation: Some(ClientElicitationConfig {
            enabled: Some(true),
            mode,
        }),
        ..Default::default()
    }
}

fn capabilities_with_sampling(
    sampling: rmcp::model::SamplingCapability,
) -> rmcp::model::ClientCapabilities {
    let mut capabilities = rmcp::model::ClientCapabilities::default();
    capabilities.sampling = Some(sampling);
    capabilities
}

fn capabilities_with_elicitation(
    elicitation: rmcp::model::ElicitationCapability,
) -> rmcp::model::ClientCapabilities {
    let mut capabilities = rmcp::model::ClientCapabilities::default();
    capabilities.elicitation = Some(elicitation);
    capabilities
}

// ---------------------------------------------------------------------------
// Roots
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_roots_returns_error_when_not_enabled() {
    let ctx = plugin_context_with_features(ClientFeaturesConfig::default(), None);
    let err = ctx.request_roots().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "client_features.roots is not enabled");
}

#[tokio::test]
async fn request_roots_returns_error_when_no_request_context() {
    let ctx = plugin_context_with_features(enabled_roots(), None);
    let err = ctx.request_roots().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "request_roots requires a request context");
}

#[tokio::test]
async fn request_roots_returns_error_when_client_lacks_capability() {
    // Set up a duplex peer with no roots capability
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request =
        request_context_with_capabilities(&service, rmcp::model::ClientCapabilities::default());
    let ctx = plugin_context_with_features(enabled_roots(), Some(request));
    let err = ctx.request_roots().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "client does not support roots capability");
    drop(service);
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_sampling_returns_error_when_not_enabled() {
    let ctx = plugin_context_with_features(ClientFeaturesConfig::default(), None);
    let params = minimal_sampling_params();
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "client_features.sampling is not enabled");
}

#[tokio::test]
async fn request_sampling_returns_error_when_no_request_context() {
    let ctx = plugin_context_with_features(enabled_sampling(), None);
    let params = minimal_sampling_params();
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "request_sampling requires a request context");
}

#[tokio::test]
async fn request_sampling_returns_error_when_client_lacks_capability() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request =
        request_context_with_capabilities(&service, rmcp::model::ClientCapabilities::default());
    let ctx = plugin_context_with_features(enabled_sampling(), Some(request));
    let params = minimal_sampling_params();
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "client does not support sampling capability");
    drop(service);
}

#[tokio::test]
async fn request_sampling_rejects_tools_when_allow_tools_false() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_sampling(rmcp::model::SamplingCapability::default()),
    );
    let ctx = plugin_context_with_features(enabled_sampling(), Some(request));
    let mut params = minimal_sampling_params();
    params.tools = Some(vec![make_test_tool()]);
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        err.message,
        "sampling tools not allowed by server config (allow_tools=false)"
    );
    drop(service);
}

#[tokio::test]
async fn request_sampling_rejects_tools_when_client_lacks_sampling_tools() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    // Client has sampling but no tools sub-capability
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_sampling(rmcp::model::SamplingCapability {
            tools: None,
            context: None,
        }),
    );
    let features = ClientFeaturesConfig {
        sampling: Some(ClientSamplingConfig {
            enabled: Some(true),
            allow_tools: true,
        }),
        ..Default::default()
    };
    let ctx = plugin_context_with_features(features, Some(request));
    let mut params = minimal_sampling_params();
    params.tools = Some(vec![make_test_tool()]);
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        err.message,
        "client does not support sampling tools capability"
    );
    drop(service);
}

// ---------------------------------------------------------------------------
// Elicitation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_elicitation_returns_error_when_not_enabled() {
    let ctx = plugin_context_with_features(ClientFeaturesConfig::default(), None);
    let params = minimal_form_elicitation_params();
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(err.message, "client_features.elicitation is not enabled");
}

#[tokio::test]
async fn request_elicitation_returns_error_when_no_request_context() {
    let ctx = plugin_context_with_features(enabled_elicitation(ElicitationMode::Form), None);
    let params = minimal_form_elicitation_params();
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        err.message,
        "request_elicitation requires a request context"
    );
}

#[tokio::test]
async fn request_elicitation_returns_error_when_client_lacks_capability() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request =
        request_context_with_capabilities(&service, rmcp::model::ClientCapabilities::default());
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Form), Some(request));
    let params = minimal_form_elicitation_params();
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        err.message,
        "client does not support elicitation capability"
    );
    drop(service);
}

#[tokio::test]
async fn request_elicitation_rejects_url_mode_when_config_is_form() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_elicitation(rmcp::model::ElicitationCapability {
            form: Some(rmcp::model::FormElicitationCapability::default()),
            url: Some(rmcp::model::UrlElicitationCapability::default()),
        }),
    );
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Form), Some(request));
    let params = rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
        meta: None,
        message: "please visit".to_owned(),
        url: "https://example.com".to_owned(),
        elicitation_id: "elic-1".to_owned(),
    };
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        err.message,
        "URL elicitation not allowed by server config (mode=form)"
    );
    drop(service);
}

#[tokio::test]
async fn request_elicitation_rejects_form_mode_when_config_is_url() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_elicitation(rmcp::model::ElicitationCapability {
            form: Some(rmcp::model::FormElicitationCapability::default()),
            url: Some(rmcp::model::UrlElicitationCapability::default()),
        }),
    );
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Url), Some(request));
    let params = minimal_form_elicitation_params();
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        err.message,
        "form elicitation not allowed by server config (mode=url)"
    );
    drop(service);
}

// ---------------------------------------------------------------------------
// Elicitation — client capability mode validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_elicitation_rejects_form_when_client_only_supports_url() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    // Client declares url only (form: None, url: Some)
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_elicitation(rmcp::model::ElicitationCapability {
            form: None,
            url: Some(rmcp::model::UrlElicitationCapability::default()),
        }),
    );
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Both), Some(request));
    let params = minimal_form_elicitation_params();
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(err.message, "client does not support form elicitation");
    drop(service);
}

#[tokio::test]
async fn request_elicitation_allows_form_with_empty_capability_backwards_compat() {
    // Per spec: empty `elicitation: {}` (form: None, url: None) = form-only.
    let capabilities = capabilities_with_elicitation(rmcp::model::ElicitationCapability {
        form: None,
        url: None,
    });
    let client = e2e_common::SmokeTestClient::new()
        .with_capabilities(capabilities.clone())
        .with_elicitation_response(rmcp::model::CreateElicitationResult {
            action: rmcp::model::ElicitationAction::Accept,
            content: Some(json!({"name": "ok"})),
        });
    let (service, _client_service) = start_connected_server_and_client(client).await;
    let request = request_context_with_capabilities(&service, capabilities);
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Form), Some(request));
    let result = ctx
        .request_elicitation(minimal_form_elicitation_params())
        .await
        .expect("form request should pass and return client response");
    assert_eq!(result.action, rmcp::model::ElicitationAction::Accept);
    assert_eq!(result.content, Some(json!({"name": "ok"})));
    drop(service);
}

#[tokio::test]
async fn request_elicitation_both_mode_allows_form_request() {
    let capabilities = capabilities_with_elicitation(rmcp::model::ElicitationCapability {
        form: Some(rmcp::model::FormElicitationCapability::default()),
        url: Some(rmcp::model::UrlElicitationCapability::default()),
    });
    let client = e2e_common::SmokeTestClient::new()
        .with_capabilities(capabilities.clone())
        .with_elicitation_response(rmcp::model::CreateElicitationResult {
            action: rmcp::model::ElicitationAction::Accept,
            content: Some(json!({"name": "form"})),
        });
    let (service, _client_service) = start_connected_server_and_client(client).await;
    let request = request_context_with_capabilities(&service, capabilities);
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Both), Some(request));
    let result = ctx
        .request_elicitation(minimal_form_elicitation_params())
        .await
        .expect("form request should succeed in mode=both");
    assert_eq!(result.action, rmcp::model::ElicitationAction::Accept);
    assert_eq!(result.content, Some(json!({"name": "form"})));
    drop(service);
}

#[tokio::test]
async fn request_elicitation_both_mode_allows_url_request() {
    let capabilities = capabilities_with_elicitation(rmcp::model::ElicitationCapability {
        form: Some(rmcp::model::FormElicitationCapability::default()),
        url: Some(rmcp::model::UrlElicitationCapability::default()),
    });
    let client = e2e_common::SmokeTestClient::new()
        .with_capabilities(capabilities.clone())
        .with_elicitation_response(rmcp::model::CreateElicitationResult {
            action: rmcp::model::ElicitationAction::Accept,
            content: Some(json!({"name": "url"})),
        });
    let (service, _client_service) = start_connected_server_and_client(client).await;
    let request = request_context_with_capabilities(&service, capabilities);
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Both), Some(request));
    let params = rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
        meta: None,
        message: "please visit".to_owned(),
        url: "https://example.com".to_owned(),
        elicitation_id: "elic-1".to_owned(),
    };
    let result = ctx
        .request_elicitation(params)
        .await
        .expect("url request should succeed in mode=both");
    assert_eq!(result.action, rmcp::model::ElicitationAction::Accept);
    assert_eq!(result.content, Some(json!({"name": "url"})));
    drop(service);
}

#[tokio::test]
async fn request_elicitation_rejects_url_when_client_only_supports_form() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    // Client declares form only (form: Some, url: None)
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_elicitation(rmcp::model::ElicitationCapability {
            form: Some(rmcp::model::FormElicitationCapability::default()),
            url: None,
        }),
    );
    let ctx =
        plugin_context_with_features(enabled_elicitation(ElicitationMode::Both), Some(request));
    let params = rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
        meta: None,
        message: "please visit".to_owned(),
        url: "https://example.com".to_owned(),
        elicitation_id: "elic-1".to_owned(),
    };
    let err = ctx.request_elicitation(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(err.message, "client does not support URL elicitation");
    drop(service);
}

// ---------------------------------------------------------------------------
// Positive-path validation pass tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_roots_passes_validation_with_capable_client() {
    let capabilities = rmcp::model::ClientCapabilities::builder()
        .enable_roots()
        .build();
    let root = serde_json::from_value(json!({
        "uri": "file:///tmp/test",
        "name": "test root"
    }))
    .expect("root should deserialize");
    let client = e2e_common::SmokeTestClient::new()
        .with_capabilities(capabilities.clone())
        .with_roots(vec![root]);
    let (service, _client_service) = start_connected_server_and_client(client).await;
    let request = request_context_with_capabilities(&service, capabilities);
    let ctx = plugin_context_with_features(enabled_roots(), Some(request));
    let result = ctx
        .request_roots()
        .await
        .expect("roots request should succeed");
    assert_eq!(result.roots.len(), 1);
    assert_eq!(result.roots[0].uri.as_str(), "file:///tmp/test");
    drop(service);
}

#[tokio::test]
async fn request_sampling_passes_validation_with_capable_client() {
    let capabilities = capabilities_with_sampling(rmcp::model::SamplingCapability::default());
    let client = e2e_common::SmokeTestClient::new()
        .with_capabilities(capabilities.clone())
        .with_sampling_response(
            rmcp::model::CreateMessageResult::new(
                rmcp::model::SamplingMessage::new(
                    rmcp::model::Role::Assistant,
                    rmcp::model::SamplingMessageContent::text("sampled"),
                ),
                "test-model".to_owned(),
            )
            .with_stop_reason("end_turn"),
        );
    let (service, _client_service) = start_connected_server_and_client(client).await;
    let request = request_context_with_capabilities(&service, capabilities);
    let ctx = plugin_context_with_features(enabled_sampling(), Some(request));
    let params = minimal_sampling_params();
    let result = ctx
        .request_sampling(params)
        .await
        .expect("sampling request should succeed");
    assert_eq!(result.model, "test-model");
    assert_eq!(result.message.role, rmcp::model::Role::Assistant);
    drop(service);
}

// ---------------------------------------------------------------------------
// Sampling — tool_choice validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_sampling_rejects_tool_choice_when_allow_tools_false() {
    let (_client_io, server_io) = tokio::io::duplex(4096);
    let engine = build_noop_engine();
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let request = request_context_with_capabilities(
        &service,
        capabilities_with_sampling(rmcp::model::SamplingCapability::default()),
    );
    let ctx = plugin_context_with_features(enabled_sampling(), Some(request));
    let mut params = minimal_sampling_params();
    params.tool_choice = Some(serde_json::from_value(serde_json::json!({"mode": "auto"})).unwrap());
    let err = ctx.request_sampling(params).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        err.message,
        "sampling tools not allowed by server config (allow_tools=false)"
    );
    drop(service);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_noop_engine() -> rust_mcp_core::Engine {
    let config: rust_mcp_core::McpConfig = serde_yaml::from_str(
        r"
version: 1
server:
  transport:
    mode: stdio
",
    )
    .expect("config should parse");
    rust_mcp_core::Engine::from_config(rust_mcp_core::EngineConfig {
        config,
        plugins: rust_mcp_core::PluginRegistry::new(),
        list_refresh_handle: None,
    })
    .expect("engine should build")
}

fn request_context_with_capabilities(
    service: &rmcp::service::RunningService<rmcp::service::RoleServer, rust_mcp_core::Engine>,
    capabilities: rmcp::model::ClientCapabilities,
) -> rmcp::service::RequestContext<rmcp::service::RoleServer> {
    let context = rmcp::service::RequestContext {
        peer: service.peer().clone(),
        ct: tokio_util::sync::CancellationToken::new(),
        id: rmcp::model::NumberOrString::Number(1),
        meta: rmcp::model::Meta::default(),
        extensions: rmcp::model::Extensions::default(),
    };
    context
        .peer
        .set_peer_info(rmcp::model::InitializeRequestParams::new(
            capabilities,
            rmcp::model::Implementation::new("test-client", "1.0.0"),
        ));
    context
}

fn minimal_sampling_params() -> rmcp::model::CreateMessageRequestParams {
    rmcp::model::CreateMessageRequestParams::new(
        vec![rmcp::model::SamplingMessage::user_text("Hello")],
        100,
    )
}

fn minimal_form_elicitation_params() -> rmcp::model::CreateElicitationRequestParams {
    rmcp::model::CreateElicitationRequestParams::FormElicitationParams {
        meta: None,
        message: "please provide input".to_owned(),
        requested_schema: rmcp::model::ElicitationSchema::builder()
            .required_string("name")
            .build()
            .expect("schema should build"),
    }
}

async fn start_connected_server_and_client(
    client: e2e_common::SmokeTestClient,
) -> (
    RunningService<RoleServer, rust_mcp_core::Engine>,
    RunningService<RoleClient, e2e_common::SmokeTestClient>,
) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let server_service = rmcp::service::serve_directly(build_noop_engine(), server_io, None);
    let client_service = client
        .serve(client_io)
        .await
        .expect("client service should start");
    (server_service, client_service)
}

fn make_test_tool() -> rmcp::model::Tool {
    serde_json::from_value(serde_json::json!({
        "name": "test",
        "description": "test tool",
        "inputSchema": {
            "type": "object"
        }
    }))
    .expect("tool should deserialize")
}
