use rust_mcp_core::config::{
    AuthOauthConfig, AuthProviderConfig, ExecuteConfig, ExecuteHttpConfig, ExecutePluginConfig,
    McpConfig, TaskSupport, ToolConfig, ToolsConfig,
};
use serde_json::json;

fn sample_tool(name: &str) -> ToolConfig {
    ToolConfig {
        name: name.to_owned(),
        title: None,
        description: "desc".to_owned(),
        cancellable: true,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Plugin(ExecutePluginConfig {
            plugin: "tool.plugin".to_owned(),
            config: None,
            task_support: TaskSupport::Forbidden,
        }),
        response: None,
    }
}

fn base_config() -> McpConfig {
    serde_yaml::from_str("version: 1").expect("base config should parse")
}

#[test]
fn tools_helpers_follow_hybrid_activation_contract() {
    let mut config = base_config();
    assert!(!config.tools_active());
    assert!(!config.tools_notify_list_changed());
    assert_eq!(config.tools_items().len(), 0);

    config.tools = Some(ToolsConfig {
        enabled: None,
        notify_list_changed: true,
        items: vec![sample_tool("active.tool")],
    });
    assert!(config.tools_active());
    assert!(config.tools_notify_list_changed());
    assert_eq!(config.tools_items().len(), 1);

    config.tools = Some(ToolsConfig {
        enabled: Some(false),
        notify_list_changed: true,
        items: vec![sample_tool("inactive.tool")],
    });
    assert!(!config.tools_active());
    assert!(!config.tools_notify_list_changed());
    assert_eq!(config.tools_items().len(), 0);

    config.tools_items_mut().push(sample_tool("mutable.tool"));
    assert!(!config.tools_active());
    config.set_tools_notify_list_changed(true);
    assert!(!config.tools_notify_list_changed());

    config.tools = Some(ToolsConfig {
        enabled: Some(true),
        notify_list_changed: true,
        items: vec![sample_tool("enabled.tool")],
    });
    assert!(config.tools_active());
    assert!(config.tools_notify_list_changed());
}

#[test]
fn server_auth_helpers_follow_enabled_active_and_oauth_contracts() {
    let mut config = base_config();
    assert!(!config.server.auth_enabled());
    assert!(!config.server.auth_active());
    assert!(!config.server.auth_oauth_enabled());

    {
        let auth = config.server.auth_mut_or_insert();
        assert!(auth.is_enabled());
        assert!(!auth.is_active());
    }
    assert!(config.server.auth_enabled());
    assert!(!config.server.auth_active());
    assert!(!config.server.auth_oauth_enabled());

    config
        .server
        .auth_mut_or_insert()
        .providers
        .push(AuthProviderConfig::bearer("static", "expected-token"));
    assert!(config.server.auth_enabled());
    assert!(config.server.auth_active());
    assert!(!config.server.auth_oauth_enabled());

    config.server.auth_mut_or_insert().oauth = Some(AuthOauthConfig {
        public_url: None,
        resource: "http://example.com/mcp".to_owned(),
        client_metadata_document_url: None,
        scope_in_challenges: true,
    });
    assert!(config.server.auth_oauth_enabled());

    config.server.auth_mut_or_insert().enabled = Some(false);
    assert!(!config.server.auth_enabled());
    assert!(!config.server.auth_active());
    assert!(!config.server.auth_oauth_enabled());
}

#[test]
fn auth_provider_bearer_accessors_return_variant_specific_fields() {
    let bearer = AuthProviderConfig::bearer("static", "token");
    assert_eq!(bearer.bearer_token(), Some("token"));
    assert_eq!(bearer.jwks_url(), None);
    assert_eq!(bearer.introspection_url(), None);
    assert_eq!(bearer.introspection_client_id(), None);
    assert_eq!(bearer.introspection_client_secret(), None);
    assert_eq!(bearer.discovery_url(), None);
    assert_eq!(bearer.audiences(), &[] as &[String]);
    assert_eq!(bearer.required_scopes(), &[] as &[String]);
    assert!(bearer.required_claims().is_empty());
    assert_eq!(bearer.algorithms(), &[] as &[String]);
    assert_eq!(bearer.clock_skew_sec(), None);
    assert_eq!(bearer.plugin_name(), None);
}

#[test]
fn auth_provider_jwks_accessors_return_variant_specific_fields() {
    let mut jwks = AuthProviderConfig::jwks("jwt");
    let jwks_cfg = jwks.as_jwks_mut().expect("expected jwks");
    jwks_cfg.issuer = Some("https://issuer.jwt.example.com".to_owned());
    jwks_cfg.discovery_url =
        Some("https://issuer.jwt.example.com/.well-known/openid-configuration".to_owned());
    jwks_cfg.jwks_url = Some("https://issuer.jwt.example.com/jwks".to_owned());
    jwks_cfg.audiences = vec!["api://mcp".to_owned()];
    jwks_cfg.required_scopes = vec!["mcp.read".to_owned()];
    jwks_cfg
        .required_claims
        .insert("tenant".to_owned(), "acme".to_owned());
    jwks_cfg.algorithms = vec!["RS256".to_owned()];
    jwks_cfg.clock_skew_sec = Some(60);
    assert_eq!(jwks.issuer(), Some("https://issuer.jwt.example.com"));
    assert_eq!(
        jwks.discovery_url(),
        Some("https://issuer.jwt.example.com/.well-known/openid-configuration")
    );
    assert_eq!(jwks.jwks_url(), Some("https://issuer.jwt.example.com/jwks"));
    assert_eq!(jwks.audiences(), ["api://mcp"]);
    assert_eq!(jwks.required_scopes(), ["mcp.read"]);
    assert_eq!(
        jwks.required_claims().get("tenant"),
        Some(&"acme".to_owned())
    );
    assert_eq!(jwks.algorithms(), ["RS256"]);
    assert_eq!(jwks.clock_skew_sec(), Some(60));
    assert_eq!(jwks.introspection_url(), None);
    assert_eq!(jwks.introspection_client_id(), None);
    assert_eq!(jwks.introspection_client_secret(), None);
    assert_eq!(jwks.plugin_name(), None);
    assert_eq!(
        jwks.introspection_client_auth_method(),
        rust_mcp_core::config::IntrospectionClientAuthMethod::Basic
    );
}

#[test]
fn auth_provider_introspection_accessors_return_variant_specific_fields() {
    let mut introspection =
        AuthProviderConfig::introspection("opaque", "https://issuer.opaque.example.com/introspect");
    let introspection_cfg = introspection
        .as_introspection_mut()
        .expect("expected introspection");
    introspection_cfg.client_id = Some("client".to_owned());
    introspection_cfg.client_secret = Some("secret".to_owned());
    introspection_cfg.auth_method = rust_mcp_core::config::IntrospectionClientAuthMethod::Post;
    introspection_cfg.audiences = vec!["api://mcp".to_owned()];
    introspection_cfg.required_scopes = vec!["mcp.read".to_owned()];
    introspection_cfg
        .required_claims
        .insert("tenant".to_owned(), "acme".to_owned());
    assert_eq!(
        introspection.introspection_url(),
        Some("https://issuer.opaque.example.com/introspect")
    );
    assert_eq!(introspection.introspection_client_id(), Some("client"));
    assert_eq!(introspection.introspection_client_secret(), Some("secret"));
    assert_eq!(
        introspection.introspection_client_auth_method(),
        rust_mcp_core::config::IntrospectionClientAuthMethod::Post
    );
    assert_eq!(introspection.audiences(), ["api://mcp"]);
    assert_eq!(introspection.required_scopes(), ["mcp.read"]);
    assert_eq!(
        introspection.required_claims().get("tenant"),
        Some(&"acme".to_owned())
    );
    assert_eq!(introspection.jwks_url(), None);
    assert_eq!(introspection.discovery_url(), None);
    assert_eq!(introspection.algorithms(), &[] as &[String]);
    assert_eq!(introspection.clock_skew_sec(), None);
    assert_eq!(introspection.plugin_name(), None);
}

#[test]
fn auth_provider_plugin_accessors_return_variant_specific_fields() {
    let mut plugin = AuthProviderConfig::plugin("plugin", "auth.custom.validator");
    assert_eq!(plugin.plugin_name(), Some("auth.custom.validator"));
    assert_eq!(plugin.introspection_url(), None);
    assert_eq!(plugin.introspection_client_id(), None);
    assert_eq!(plugin.introspection_client_secret(), None);
    assert_eq!(plugin.bearer_token(), None);
    assert_eq!(
        plugin.introspection_client_auth_method(),
        rust_mcp_core::config::IntrospectionClientAuthMethod::Basic
    );
    assert!(plugin.as_plugin_mut().is_some());
    assert!(plugin.as_bearer_mut().is_none());
    assert!(plugin.as_jwks_mut().is_none());
    assert!(plugin.as_introspection_mut().is_none());
}

#[test]
fn execute_config_accessors_cover_all_variant_paths() {
    let mut http_execute = ExecuteConfig::Http(ExecuteHttpConfig {
        upstream: "api".to_owned(),
        method: "GET".to_owned(),
        path: "/search".to_owned(),
        query: None,
        headers: None,
        body: None,
        retry: None,
        task_support: TaskSupport::Optional,
    });
    assert_eq!(
        http_execute.execute_type(),
        rust_mcp_core::config::ExecuteType::Http
    );
    assert_eq!(http_execute.task_support(), TaskSupport::Optional);
    assert!(http_execute.as_http().is_some());
    assert!(http_execute.as_plugin().is_none());
    assert!(http_execute.as_http_mut().is_some());
    assert!(http_execute.as_plugin_mut().is_none());

    let mut plugin_execute = ExecuteConfig::Plugin(ExecutePluginConfig {
        plugin: "tool.plugin".to_owned(),
        config: Some(json!({"mode":"strict"})),
        task_support: TaskSupport::Required,
    });
    assert_eq!(
        plugin_execute.execute_type(),
        rust_mcp_core::config::ExecuteType::Plugin
    );
    assert_eq!(plugin_execute.task_support(), TaskSupport::Required);
    assert!(plugin_execute.as_http().is_none());
    assert!(plugin_execute.as_plugin().is_some());
    assert!(plugin_execute.as_http_mut().is_none());
    assert!(plugin_execute.as_plugin_mut().is_some());
}
