#![cfg(feature = "http_tools")]

mod engine_common;

use engine_common::load_tool_fixture;
use rmcp::{
    model::{CallToolRequestParams, Extensions, Meta, NumberOrString},
    ServerHandler,
};
use rust_mcp_core::{
    engine::{Engine, EngineConfig},
    plugins::PluginRegistry,
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn server_handler_list_and_call_tool_fixture() {
    let fixture = load_tool_fixture("engine/engine_server_handler_fixture");
    let server = httpmock::prelude::MockServer::start();

    let mut config = fixture.config;
    if let Some(upstream) = config.upstreams.get_mut("api") {
        upstream.base_url = server.base_url();
    }

    let _mock = server.mock(|when, then| {
        when.method(httpmock::prelude::GET).path("/ping");
        then.status(200).json_body(serde_json::json!({"ok": true}));
    });

    let engine = Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::default(),
        list_refresh_handle: None,
    })
    .expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut server_service = rmcp::service::serve_directly(engine, server_io, None);

    let ctx = rmcp::service::RequestContext {
        peer: server_service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    };

    let list_result = ServerHandler::list_tools(server_service.service(), None, ctx.clone())
        .await
        .expect("list tools");
    assert_eq!(
        list_result.tools.len(),
        1,
        "fixture defines exactly one tool"
    );
    assert_eq!(
        list_result.tools[0].name.as_ref(),
        "api.ping",
        "tool name should match fixture"
    );
    assert_eq!(
        list_result.tools[0].description.as_deref(),
        Some("ping"),
        "tool description should match fixture"
    );

    let call_params =
        CallToolRequestParams::new(fixture.tool).with_arguments(serde_json::Map::new());

    let call_result = ServerHandler::call_tool(server_service.service(), call_params, ctx)
        .await
        .expect("call tool");
    assert_eq!(
        call_result.is_error,
        Some(false),
        "call_tool should indicate success"
    );
    assert_eq!(
        call_result.structured_content,
        Some(serde_json::json!({"ok": true})),
        "structured content should match upstream response"
    );

    let _ = server_service.close().await;
}
