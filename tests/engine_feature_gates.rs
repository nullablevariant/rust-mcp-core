#![cfg(any(
    not(feature = "auth"),
    not(feature = "streamable_http"),
    not(feature = "http_tools"),
    not(feature = "prompts"),
    not(feature = "resources"),
    not(feature = "completion"),
    not(feature = "client_logging"),
    not(feature = "progress_utility"),
    not(feature = "tasks_utility"),
    not(feature = "client_features"),
))]

mod config_common;

#[cfg(not(feature = "prompts"))]
use rmcp::model::GetPromptRequestParams;
#[cfg(not(feature = "resources"))]
use rmcp::model::ReadResourceRequestParams;
#[cfg(not(feature = "completion"))]
use rmcp::model::{ArgumentInfo, CompleteRequestParams, Reference};
#[cfg(not(feature = "tasks_utility"))]
use rmcp::model::{CancelTaskParams, GetTaskInfoParams};
#[cfg(not(feature = "client_logging"))]
use rmcp::model::{LoggingLevel, SetLevelRequestParams};
use rmcp::{
    model::{ErrorCode, NumberOrString},
    service::{RequestContext, RoleServer, RunningService},
    ServerHandler,
};
use rust_mcp_core::{
    config::McpConfig,
    engine::{Engine, EngineConfig},
    PluginRegistry,
};
use tokio_util::sync::CancellationToken;

fn base_config() -> McpConfig {
    #[cfg(feature = "http_tools")]
    {
        crate::config_common::base_config_yaml_stdio_builtin_noop()
    }
    #[cfg(not(feature = "http_tools"))]
    {
        serde_yaml::from_str(
            r"
version: 1
server:
  transport:
    mode: stdio
",
        )
        .expect("feature-gate base config should parse")
    }
}

fn request_context(service: &RunningService<RoleServer, Engine>) -> RequestContext<RoleServer> {
    RequestContext {
        peer: service.peer().clone(),
        ct: CancellationToken::new(),
        id: NumberOrString::Number(1),
        meta: rmcp::model::Meta::default(),
        extensions: rmcp::model::Extensions::default(),
    }
}

fn build_engine(config: McpConfig) -> Engine {
    Engine::from_config(EngineConfig {
        config,
        plugins: PluginRegistry::new(),
        list_refresh_handle: None,
    })
    .expect("engine should build")
}

#[cfg(not(feature = "client_logging"))]
#[tokio::test]
async fn logging_methods_return_method_not_found_when_feature_disabled() {
    let engine = build_engine(base_config());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let error = ServerHandler::set_level(
        service.service(),
        SetLevelRequestParams::new(LoggingLevel::Info),
        request_context(&service),
    )
    .await
    .expect_err("set_level should be unavailable");
    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        error.message, "logging/setLevel",
        "set_level method_not_found should identify the logging/setLevel method"
    );
    let _ = service.cancel().await;
}

#[cfg(not(feature = "prompts"))]
#[tokio::test]
async fn prompt_methods_return_method_not_found_when_feature_disabled() {
    let engine = build_engine(base_config());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let service = rmcp::service::serve_directly(engine, server_io, None);
    let list_error =
        ServerHandler::list_prompts(service.service(), None, request_context(&service))
            .await
            .expect_err("list_prompts should be unavailable");
    assert_eq!(list_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        list_error.message, "prompts/list",
        "list_prompts method_not_found should identify prompts/list"
    );

    let get_error = ServerHandler::get_prompt(
        service.service(),
        GetPromptRequestParams::new("missing"),
        request_context(&service),
    )
    .await
    .expect_err("get_prompt should be unavailable");
    assert_eq!(get_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        get_error.message, "prompts/get",
        "get_prompt method_not_found should identify prompts/get"
    );
    let _ = service.cancel().await;
}

#[cfg(not(feature = "resources"))]
#[tokio::test]
async fn resource_methods_return_method_not_found_when_feature_disabled() {
    let engine = build_engine(base_config());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let service = rmcp::service::serve_directly(engine, server_io, None);

    let list_error =
        ServerHandler::list_resources(service.service(), None, request_context(&service))
            .await
            .expect_err("list_resources should be unavailable");
    assert_eq!(list_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        list_error.message, "resources/list",
        "list_resources method_not_found should identify resources/list"
    );

    let templates_error =
        ServerHandler::list_resource_templates(service.service(), None, request_context(&service))
            .await
            .expect_err("list_resource_templates should be unavailable");
    assert_eq!(templates_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        templates_error.message, "resources/templates/list",
        "list_resource_templates method_not_found should identify resources/templates/list"
    );

    let read_error = ServerHandler::read_resource(
        service.service(),
        ReadResourceRequestParams::new("resource://missing"),
        request_context(&service),
    )
    .await
    .expect_err("read_resource should be unavailable");
    assert_eq!(read_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        read_error.message, "resources/read",
        "read_resource method_not_found should identify resources/read"
    );
    let _ = service.cancel().await;
}

#[cfg(not(feature = "completion"))]
#[tokio::test]
async fn completion_method_returns_method_not_found_when_feature_disabled() {
    let engine = build_engine(base_config());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let service = rmcp::service::serve_directly(engine, server_io, None);

    let error = ServerHandler::complete(
        service.service(),
        CompleteRequestParams::new(
            Reference::for_prompt("missing"),
            ArgumentInfo {
                name: "value".to_owned(),
                value: "m".to_owned(),
            },
        ),
        request_context(&service),
    )
    .await
    .expect_err("complete should be unavailable");

    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        error.message, "completion/complete",
        "complete method_not_found should identify completion/complete"
    );
    let _ = service.cancel().await;
}

#[cfg(not(feature = "tasks_utility"))]
#[tokio::test]
async fn tasks_methods_return_method_not_found_when_feature_disabled() {
    let engine = build_engine(base_config());
    let (_client_io, server_io) = tokio::io::duplex(1024);
    let service = rmcp::service::serve_directly(engine, server_io, None);

    let get_error = ServerHandler::get_task_info(
        service.service(),
        GetTaskInfoParams {
            meta: None,
            task_id: "task-1".to_owned(),
        },
        request_context(&service),
    )
    .await
    .expect_err("get_task_info should be unavailable");
    assert_eq!(get_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        get_error.message, "tasks/get",
        "get_task_info method_not_found should identify tasks/get"
    );

    let cancel_error = ServerHandler::cancel_task(
        service.service(),
        CancelTaskParams {
            meta: None,
            task_id: "task-1".to_owned(),
        },
        request_context(&service),
    )
    .await
    .expect_err("cancel_task should be unavailable");
    assert_eq!(cancel_error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(
        cancel_error.message, "tasks/cancel",
        "cancel_task method_not_found should identify tasks/cancel"
    );
    let _ = service.cancel().await;
}
