#![cfg(feature = "http_tools")]

mod engine_common;

use engine_common::load_config_fixture;
use rmcp::ServerHandler;
use rust_mcp_core::engine::Engine;

#[test]
fn get_info_exposes_tools_capability_fixture() {
    let fixture = load_config_fixture("engine/engine_get_info_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let info = engine.get_info();
    let tools_cap = info
        .capabilities
        .tools
        .expect("tools capability should be present");
    assert_eq!(
        tools_cap.list_changed, None,
        "list_changed should be None when not enabled"
    );
}

#[test]
fn get_info_sets_tools_list_changed_capability_when_enabled() {
    let fixture = load_config_fixture("engine/engine_get_info_fixture");
    let mut config = fixture.config;
    config.set_tools_notify_list_changed(true);
    let engine = Engine::new(config).expect("engine should build");
    let info = engine.get_info();
    assert_eq!(
        info.capabilities
            .tools
            .and_then(|capability| capability.list_changed),
        Some(true)
    );
}

#[test]
fn get_info_uses_build_defaults_when_server_info_omitted() {
    let fixture = load_config_fixture("engine/engine_get_info_fixture");
    let engine = Engine::new(fixture.config).expect("engine should build");
    let info = engine.get_info();
    assert_eq!(info.server_info.name, "rmcp");
    assert_eq!(info.server_info.version, "1.1.0");
    assert!(info.server_info.title.is_none());
    assert!(info.server_info.description.is_none());
    assert!(info.server_info.website_url.is_none());
    assert!(info.server_info.icons.is_none());
    assert!(info.instructions.is_none());
}

#[test]
fn get_info_reflects_server_info_config() {
    let fixture = load_config_fixture("engine/engine_get_info_fixture");
    let mut config = fixture.config;
    config.server.info = Some(rust_mcp_core::config::ServerInfoConfig {
        name: Some("My MCP Server".to_owned()),
        version: Some("2.0.0".to_owned()),
        title: Some("My Server Title".to_owned()),
        description: Some("A test server".to_owned()),
        website_url: Some("https://example.com".to_owned()),
        icons: Some(vec![rust_mcp_core::config::IconConfig {
            src: "https://example.com/icon.png".to_owned(),
            mime_type: Some("image/png".to_owned()),
            sizes: Some(vec!["64x64".to_owned()]),
        }]),
        instructions: Some("Use this server for testing".to_owned()),
    });
    let engine = Engine::new(config).expect("engine should build");
    let info = engine.get_info();
    assert_eq!(info.server_info.name, "My MCP Server");
    assert_eq!(info.server_info.version, "2.0.0");
    assert_eq!(info.server_info.title.as_deref(), Some("My Server Title"));
    assert_eq!(
        info.server_info.description.as_deref(),
        Some("A test server")
    );
    assert_eq!(
        info.server_info.website_url.as_deref(),
        Some("https://example.com")
    );
    let icons = info.server_info.icons.expect("icons should be set");
    assert_eq!(icons.len(), 1);
    assert_eq!(icons[0].src, "https://example.com/icon.png");
    assert_eq!(icons[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(icons[0].sizes.as_deref(), Some(&["64x64".to_owned()][..]));
    assert_eq!(
        info.instructions.as_deref(),
        Some("Use this server for testing")
    );
}

#[test]
fn get_info_falls_back_to_build_defaults_for_name_and_version() {
    let fixture = load_config_fixture("engine/engine_get_info_fixture");
    let mut config = fixture.config;
    config.server.info = Some(rust_mcp_core::config::ServerInfoConfig {
        name: None,
        version: None,
        title: Some("Custom Title".to_owned()),
        description: None,
        website_url: None,
        icons: None,
        instructions: None,
    });
    let engine = Engine::new(config).expect("engine should build");
    let info = engine.get_info();
    assert_eq!(info.server_info.name, "rmcp");
    assert_eq!(info.server_info.version, "1.1.0");
    assert_eq!(info.server_info.title.as_deref(), Some("Custom Title"));
    assert!(
        info.server_info.description.is_none(),
        "description should remain None when not set"
    );
    assert!(
        info.server_info.website_url.is_none(),
        "website_url should remain None when not set"
    );
    assert!(
        info.server_info.icons.is_none(),
        "icons should remain None when not set"
    );
}
