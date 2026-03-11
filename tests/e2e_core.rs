mod e2e_common;

#[cfg(feature = "http_tools")]
use e2e_common::make_noop_tool;
use e2e_common::{build_engine, make_minimal_config, spawn_e2e, SmokeTestClient};
use rmcp::model::{ClientRequest, PingRequest};
#[cfg(feature = "http_tools")]
use rust_mcp_core::config::PaginationConfig;
#[cfg(all(feature = "prompts", feature = "resources"))]
use rust_mcp_core::config::{
    PromptItemConfig, PromptProviderConfig, PromptTemplateConfig, PromptTemplateMessageConfig,
    PromptsConfig, ResourceContentConfig, ResourceItemConfig, ResourceProviderConfig,
    ResourcesConfig,
};
use rust_mcp_core::plugins::PluginRegistry;
#[cfg(all(feature = "prompts", feature = "resources"))]
use serde_json::json;

#[tokio::test]
async fn e2e_ping() {
    let config = make_minimal_config();
    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let result = client_service
        .peer()
        .send_request(ClientRequest::PingRequest(PingRequest::default()))
        .await
        .expect("ping should succeed");

    // Assert the ping response is the expected empty result variant
    assert!(
        matches!(result, rmcp::model::ServerResult::EmptyResult(..)),
        "ping should return EmptyResult, got: {result:?}"
    );
}

#[tokio::test]
#[cfg(all(
    feature = "completion",
    feature = "prompts",
    feature = "resources",
    feature = "tasks_utility",
    feature = "client_logging"
))]
#[allow(clippy::too_many_lines)]
async fn e2e_initialize_capabilities() {
    let mut config = make_minimal_config();
    config.client_logging = Some(rust_mcp_core::config::ClientLoggingConfig::default());
    config.completion = Some(rust_mcp_core::config::CompletionConfig {
        enabled: Some(true),
        providers: vec![rust_mcp_core::config::CompletionProviderConfig::Inline {
            name: "status".to_owned(),
            values: vec!["open".to_owned(), "done".to_owned()],
        }],
    });
    config.tasks = Some(rust_mcp_core::config::TasksConfig {
        enabled: Some(true),
        ..rust_mcp_core::config::TasksConfig::default()
    });
    config.prompts = Some(rust_mcp_core::config::PromptsConfig {
        enabled: None,
        notify_list_changed: false,
        pagination: None,
        providers: vec![],
    });
    config.resources = Some(rust_mcp_core::config::ResourcesConfig {
        enabled: None,
        notify_list_changed: false,
        clients_can_subscribe: false,
        pagination: None,
        providers: vec![rust_mcp_core::config::ResourceProviderConfig::Inline {
            items: Some(vec![rust_mcp_core::config::ResourceItemConfig {
                uri: "resource://dummy".to_owned(),
                name: "dummy".to_owned(),
                title: None,
                description: None,
                mime_type: None,
                size: None,
                icons: None,
                annotations: None,
                content: Some(rust_mcp_core::config::ResourceContentConfig {
                    text: Some("x".to_owned()),
                    blob: None,
                }),
            }]),
            templates: None,
        }],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let peer_info = client_service
        .peer()
        .peer_info()
        .expect("peer_info should be set");

    // Logging: present as an empty object (no sub-fields to assert beyond presence)
    let logging = peer_info
        .capabilities
        .logging
        .as_ref()
        .expect("logging capability should be present");
    assert!(
        logging.is_empty(),
        "logging capability should be an empty object, got: {logging:?}"
    );

    // Completion: present as an empty object
    let completions = peer_info
        .capabilities
        .completions
        .as_ref()
        .expect("completion capability should be present");
    assert!(
        completions.is_empty(),
        "completions capability should be an empty object, got: {completions:?}"
    );

    // Tasks: present with list + cancel capabilities enabled (TaskCapabilities defaults)
    let tasks_cap = peer_info
        .capabilities
        .tasks
        .as_ref()
        .expect("tasks capability should be present");
    assert!(
        tasks_cap.list.is_some(),
        "tasks.list should be present (TaskCapabilities default has list=true)"
    );
    assert!(
        tasks_cap.cancel.is_some(),
        "tasks.cancel should be present (TaskCapabilities default has cancel=true)"
    );

    // Prompts: present with list_changed=false (config default)
    let prompts_cap = peer_info
        .capabilities
        .prompts
        .as_ref()
        .expect("prompts capability should be present");
    assert!(
        prompts_cap.list_changed.is_none(),
        "prompts.list_changed should be None when notify_list_changed is false"
    );

    // Resources: present with list_changed=false and subscribe=false (config default)
    let resources_cap = peer_info
        .capabilities
        .resources
        .as_ref()
        .expect("resources capability should be present");
    assert!(
        resources_cap.list_changed.is_none(),
        "resources.list_changed should be None when notify_list_changed is false"
    );
    assert!(
        resources_cap.subscribe.is_none(),
        "resources.subscribe should be None when clients_can_subscribe is false"
    );

    // Tools: present with list_changed=false (config default)
    let tools_cap = peer_info
        .capabilities
        .tools
        .as_ref()
        .expect("tools capability should be present");
    assert!(
        tools_cap.list_changed.is_none(),
        "tools.list_changed should be None when tools.notify_list_changed is false"
    );
}

#[tokio::test]
#[cfg(feature = "http_tools")]
async fn e2e_pagination() {
    let mut config = make_minimal_config();
    config.pagination = Some(PaginationConfig { page_size: 1 });
    let mut tool2 = make_noop_tool();
    tool2.name = "noop2".to_owned();
    config.tools_items_mut().push(tool2);

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let page1 = client_service
        .peer()
        .list_tools(None)
        .await
        .expect("list_tools page 1");
    assert_eq!(page1.tools.len(), 1, "page 1 should have 1 tool");
    assert!(
        page1.next_cursor.is_some(),
        "page 1 should have next_cursor"
    );

    let page2 = client_service
        .peer()
        .list_tools(Some(
            rmcp::model::PaginatedRequestParams::default().with_cursor(page1.next_cursor),
        ))
        .await
        .expect("list_tools page 2");
    assert_eq!(page2.tools.len(), 1, "page 2 should have 1 tool");
    assert!(
        page2.next_cursor.is_none(),
        "page 2 should have no next_cursor"
    );
}

#[tokio::test]
#[cfg(all(feature = "http_tools", feature = "prompts", feature = "resources"))]
#[allow(
    clippy::too_many_lines,
    reason = "Covers cross-capability disabled-mode behavior end-to-end in one scenario."
)]
async fn e2e_disabled_tools_prompts_resources_are_not_advertised_or_listed() {
    let mut config = make_minimal_config();
    config.tools = Some(rust_mcp_core::config::ToolsConfig {
        enabled: Some(false),
        notify_list_changed: false,
        items: vec![make_noop_tool()],
    });
    config.prompts = Some(PromptsConfig {
        enabled: Some(false),
        notify_list_changed: false,
        pagination: None,
        providers: vec![PromptProviderConfig::Inline {
            items: vec![PromptItemConfig {
                name: "disabled.prompt".to_owned(),
                title: None,
                description: None,
                icons: None,
                arguments_schema: json!({"type":"object"}),
                template: PromptTemplateConfig {
                    messages: vec![PromptTemplateMessageConfig {
                        role: rust_mcp_core::config::PromptMessageRoleConfig::User,
                        content: json!("disabled"),
                    }],
                },
                completions: None,
            }],
        }],
    });
    config.resources = Some(ResourcesConfig {
        enabled: Some(false),
        notify_list_changed: false,
        clients_can_subscribe: false,
        pagination: None,
        providers: vec![ResourceProviderConfig::Inline {
            items: Some(vec![ResourceItemConfig {
                uri: "resource://disabled".to_owned(),
                name: "disabled".to_owned(),
                title: None,
                description: None,
                mime_type: Some("text/plain".to_owned()),
                size: None,
                icons: None,
                annotations: None,
                content: Some(ResourceContentConfig {
                    text: Some("disabled".to_owned()),
                    blob: None,
                }),
            }]),
            templates: None,
        }],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let peer_info = client_service
        .peer()
        .peer_info()
        .expect("peer_info should be set");
    let tools_cap = peer_info
        .capabilities
        .tools
        .as_ref()
        .expect("tools capability should remain present with tools disabled");
    assert!(
        tools_cap.list_changed.is_none(),
        "disabled tools should not advertise list_changed"
    );
    assert!(
        peer_info.capabilities.prompts.is_none(),
        "prompts capability should be absent when prompts are disabled"
    );
    assert!(
        peer_info.capabilities.resources.is_none(),
        "resources capability should be absent when resources are disabled"
    );

    let tools = client_service
        .peer()
        .list_tools(None)
        .await
        .expect("list_tools should succeed when tools are disabled");
    assert!(
        tools.tools.is_empty(),
        "disabled tools config should produce empty tools catalog"
    );

    let prompts = client_service
        .peer()
        .list_prompts(None)
        .await
        .expect("list_prompts should succeed when prompts are disabled");
    assert!(
        prompts.prompts.is_empty(),
        "disabled prompts config should produce empty prompts catalog"
    );

    let resources = client_service
        .peer()
        .list_resources(None)
        .await
        .expect("list_resources should succeed when resources are disabled");
    assert!(
        resources.resources.is_empty(),
        "disabled resources config should produce empty resources catalog"
    );
}
