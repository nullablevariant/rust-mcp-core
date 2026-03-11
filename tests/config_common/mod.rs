use rust_mcp_core::config::{
    AuthConfig, ClientFeaturesConfig, McpConfig, OutboundHttpConfig, OutboundRetryConfig,
    ServerSection, StreamableHttpTransportConfig, TransportConfig, TransportMode, UpstreamConfig,
};
#[cfg(feature = "http_tools")]
use rust_mcp_core::config::{
    ExecuteConfig, ExecuteHttpConfig, TaskSupport, ToolConfig, ToolsConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[allow(dead_code)]
#[cfg(feature = "http_tools")]
pub(crate) fn base_config_yaml_stdio_builtin_noop() -> McpConfig {
    serde_yaml::from_str(
        r"
version: 1
server:
  transport:
    mode: stdio
upstreams:
  noop:
    base_url: https://example.com
tools:
  items:
    - name: noop
      description: No-op tool
      input_schema:
        type: object
      execute:
        type: http
        upstream: noop
        method: GET
        path: /
",
    )
    .expect("base config should parse")
}

#[allow(dead_code)]
#[cfg(not(feature = "http_tools"))]
pub(crate) fn base_config_yaml_stdio_builtin_noop() -> McpConfig {
    serde_yaml::from_str(
        r"
version: 1
server:
  transport:
    mode: stdio
",
    )
    .expect("base config should parse")
}

#[allow(dead_code)]
#[cfg(feature = "http_tools")]
pub(crate) fn base_config_streamable_http_with_builtin_noop() -> McpConfig {
    McpConfig {
        version: 1,
        server: ServerSection {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            endpoint_path: "/mcp".to_owned(),
            transport: TransportConfig {
                mode: TransportMode::StreamableHttp,
                streamable_http: StreamableHttpTransportConfig::default(),
            },
            auth: Some(AuthConfig {
                enabled: Some(false),
                ..AuthConfig::default()
            }),
            errors: rust_mcp_core::config::ErrorExposureConfig::default(),
            logging: rust_mcp_core::config::ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: rust_mcp_core::config::ClientCompatConfig::default(),
            info: None,
        },
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::from([(
            "noop".to_owned(),
            UpstreamConfig {
                base_url: "https://example.com".to_owned(),
                headers: HashMap::new(),
                user_agent: None,
                timeout_ms: None,
                max_response_bytes: None,
                retry: None,
                auth: None,
            },
        )]),
        tools: Some(ToolsConfig {
            enabled: None,
            notify_list_changed: false,
            items: vec![ToolConfig {
                name: "tool.noop".to_owned(),
                title: None,
                description: "noop".to_owned(),
                cancellable: true,
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
                execute: ExecuteConfig::Http(ExecuteHttpConfig {
                    upstream: "noop".to_owned(),
                    method: "GET".to_owned(),
                    path: "/".to_owned(),
                    query: None,
                    headers: None,
                    body: None,
                    retry: None,
                    task_support: TaskSupport::Forbidden,
                }),
                response: None,
            }],
        }),
        plugins: Vec::new(),
        outbound_http: None,
    }
}

#[allow(dead_code)]
#[cfg(not(feature = "http_tools"))]
pub(crate) fn base_config_streamable_http_with_builtin_noop() -> McpConfig {
    McpConfig {
        version: 1,
        server: ServerSection {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            endpoint_path: "/mcp".to_owned(),
            transport: TransportConfig {
                mode: TransportMode::StreamableHttp,
                streamable_http: StreamableHttpTransportConfig::default(),
            },
            auth: Some(AuthConfig {
                enabled: Some(false),
                ..AuthConfig::default()
            }),
            errors: rust_mcp_core::config::ErrorExposureConfig::default(),
            logging: rust_mcp_core::config::ServerLoggingConfig::default(),
            response_limits: None,
            client_compat: rust_mcp_core::config::ClientCompatConfig::default(),
            info: None,
        },
        client_logging: None,
        progress: None,
        prompts: None,
        resources: None,
        completion: None,
        tasks: None,
        client_features: ClientFeaturesConfig::default(),
        pagination: None,
        upstreams: HashMap::new(),
        tools: None,
        plugins: Vec::new(),
        outbound_http: None,
    }
}

#[allow(dead_code)]
pub(crate) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[allow(dead_code)]
pub(crate) fn load_config_fixture(name: &str) -> McpConfig {
    rust_mcp_core::load_mcp_config_from_path(fixture_path(name))
        .expect("config fixture should load via schema-validating loader")
}

#[allow(dead_code)]
pub(crate) fn upstream_config(base_url: impl Into<String>) -> UpstreamConfig {
    UpstreamConfig {
        base_url: base_url.into(),
        headers: HashMap::new(),
        user_agent: None,
        timeout_ms: None,
        max_response_bytes: None,
        retry: None,
        auth: None,
    }
}

#[allow(dead_code)]
pub(crate) fn outbound_http_config(
    timeout_ms: Option<u64>,
    max_response_bytes: Option<u64>,
    retry: Option<OutboundRetryConfig>,
) -> OutboundHttpConfig {
    OutboundHttpConfig {
        headers: HashMap::new(),
        user_agent: None,
        timeout_ms,
        max_response_bytes,
        retry,
    }
}
