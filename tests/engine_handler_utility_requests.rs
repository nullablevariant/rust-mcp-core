#![cfg(feature = "http_tools")]

mod engine_common;

use engine_common::load_tool_fixture;
use rmcp::{
    model::{Extensions, Meta, NumberOrString},
    ServerHandler,
};
use rust_mcp_core::engine::Engine;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn server_handler_ping_returns_ok() {
    let fixture = load_tool_fixture("engine/engine_server_handler_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");

    let (server_io, _client_io) = tokio::io::duplex(1024);
    let mut server_service = rmcp::service::serve_directly(engine, server_io, None);

    let ctx = rmcp::service::RequestContext {
        peer: server_service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: Meta::default(),
        extensions: Extensions::default(),
    };

    ServerHandler::ping(server_service.service(), ctx)
        .await
        .expect("ping should succeed");

    let _ = server_service.close().await;
}
