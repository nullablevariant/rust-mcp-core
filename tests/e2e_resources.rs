#![cfg(feature = "resources")]

mod e2e_common;

use e2e_common::{build_engine, make_minimal_config, spawn_e2e, SmokeTestClient};
use rmcp::model::ReadResourceRequestParams;
use rust_mcp_core::{
    config::{ResourceContentConfig, ResourceItemConfig, ResourceProviderConfig, ResourcesConfig},
    plugins::PluginRegistry,
};

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "End-to-end resource flow assertions are intentionally kept in one test."
)]
async fn e2e_resources_list_and_read() {
    let mut config = make_minimal_config();
    config.resources = Some(ResourcesConfig {
        enabled: None,
        notify_list_changed: false,
        clients_can_subscribe: false,
        pagination: None,
        providers: vec![ResourceProviderConfig::Inline {
            items: Some(vec![ResourceItemConfig {
                uri: "resource://hello".to_owned(),
                name: "hello".to_owned(),
                title: None,
                description: None,
                mime_type: Some("text/plain".to_owned()),
                size: None,
                icons: None,
                annotations: None,
                content: Some(ResourceContentConfig {
                    text: Some("hello world".to_owned()),
                    blob: None,
                }),
            }]),
            templates: None,
        }],
    });

    let engine = build_engine(config, PluginRegistry::new());
    let client = SmokeTestClient::new();
    let (client_service, _server) = spawn_e2e(engine, client).await;

    let resources = client_service
        .peer()
        .list_resources(None)
        .await
        .expect("list_resources");
    assert_eq!(
        resources.resources.len(),
        1,
        "should have exactly 1 resource listed"
    );
    assert_eq!(
        resources.resources[0].uri.as_str(),
        "resource://hello",
        "resource URI should match"
    );
    assert_eq!(
        resources.resources[0].name, "hello",
        "resource name should be 'hello'"
    );
    assert_eq!(
        resources.resources[0].mime_type.as_deref(),
        Some("text/plain"),
        "resource mime_type should be 'text/plain'"
    );

    let result = client_service
        .peer()
        .read_resource(ReadResourceRequestParams::new("resource://hello"))
        .await
        .expect("read_resource hello");

    assert_eq!(
        result.contents.len(),
        1,
        "read_resource should return exactly 1 content entry"
    );
    let content = &result.contents[0];
    match content {
        rmcp::model::ResourceContents::TextResourceContents {
            uri,
            text,
            mime_type,
            ..
        } => {
            assert_eq!(uri.as_str(), "resource://hello", "content URI should match");
            assert_eq!(text, "hello world", "content text should match exactly");
            assert_eq!(
                mime_type.as_deref(),
                Some("text/plain"),
                "content mime_type should be 'text/plain'"
            );
        }
        other @ rmcp::model::ResourceContents::BlobResourceContents { .. } => {
            panic!("expected text content, got: {other:?}")
        }
    }
}
