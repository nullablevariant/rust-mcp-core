#![cfg(feature = "http_tools")]

mod engine_common;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use engine_common::load_config_fixture;
use rmcp::{
    model::{ErrorCode, Extensions, Meta, NumberOrString, PaginatedRequestParams},
    ServerHandler,
};
use rust_mcp_core::{config::PaginationConfig, engine::Engine};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn server_handler_list_tools_pagination_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let first_page = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(None)),
        ctx.clone(),
    )
    .await
    .expect("first page");
    assert_eq!(first_page.tools.len(), 2);
    assert_eq!(first_page.tools[0].name, "tool.alpha");
    assert_eq!(first_page.tools[1].name, "tool.bravo");
    assert!(
        first_page.next_cursor.is_some(),
        "first page must have a next_cursor when more items exist"
    );

    let second_page = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(first_page.next_cursor)),
        ctx,
    )
    .await
    .expect("second page");
    assert_eq!(second_page.tools.len(), 1);
    assert_eq!(second_page.tools[0].name, "tool.charlie");
    assert!(
        second_page.next_cursor.is_none(),
        "last page must not have a next_cursor"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_invalid_cursor_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let error = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("not-a-valid-cursor".to_owned()))),
        ctx,
    )
    .await
    .expect_err("invalid cursor should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message, "invalid cursor",
        "base64-decode failure must produce 'invalid cursor' message"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_cursor_missing_prefix_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let invalid_cursor = STANDARD.encode("missing-prefix");
    let error = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some(invalid_cursor))),
        ctx,
    )
    .await
    .expect_err("cursor without v1 prefix should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message, "invalid cursor",
        "missing-prefix cursor must produce 'invalid cursor' message"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_cursor_out_of_range_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let invalid_cursor = STANDARD.encode("v1:999");
    let error = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some(invalid_cursor))),
        ctx,
    )
    .await
    .expect_err("out-of-range cursor should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message, "invalid cursor",
        "out-of-range cursor must produce 'invalid cursor' message"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_cursor_non_numeric_offset_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let invalid_cursor = STANDARD.encode("v1:not-a-number");
    let error = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some(invalid_cursor))),
        ctx,
    )
    .await
    .expect_err("cursor with non-numeric offset should fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message, "invalid cursor",
        "non-numeric offset cursor must produce 'invalid cursor' message"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_cursor_at_end_returns_empty_page_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
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

    let end_cursor = STANDARD.encode("v1:3");
    let page = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some(end_cursor))),
        ctx,
    )
    .await
    .expect("cursor at list end should return empty page");
    assert!(page.tools.is_empty());
    assert!(page.next_cursor.is_none());

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_cursor_ignored_when_pagination_disabled_fixture() {
    let fixture = load_config_fixture("engine/engine_list_tools_fixture");
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

    let result = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("ignored-cursor".to_owned()))),
        ctx,
    )
    .await
    .expect("cursor should be ignored when pagination is disabled");
    assert_eq!(result.tools.len(), 1);
    assert_eq!(
        result.tools[0].name, "tool.meta",
        "disabled pagination must return all tools; fixture has one tool named tool.meta"
    );
    assert!(
        result.next_cursor.is_none(),
        "disabled pagination must not return a cursor"
    );

    let _ = server_service.close().await;
}

#[tokio::test]
async fn server_handler_list_tools_page_size_zero_disables_pagination_fixture() {
    let mut fixture = load_config_fixture("engine/engine_list_tools_paginated_fixture");
    fixture.config.pagination = Some(PaginationConfig { page_size: 0 });
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

    let result = ServerHandler::list_tools(
        server_service.service(),
        Some(PaginatedRequestParams::default().with_cursor(Some("ignored-cursor".to_owned()))),
        ctx,
    )
    .await
    .expect("cursor should be ignored when page_size=0");
    assert_eq!(result.tools.len(), 3);
    let tool_names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(
        tool_names,
        vec!["tool.alpha", "tool.bravo", "tool.charlie"],
        "page_size=0 must return all tools in fixture order"
    );
    assert!(
        result.next_cursor.is_none(),
        "page_size=0 must not return a cursor"
    );

    let _ = server_service.close().await;
}
