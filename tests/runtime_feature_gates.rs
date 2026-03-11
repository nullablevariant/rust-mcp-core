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
    not(feature = "http_hardening"),
))]

mod config_common;

use crate::config_common::base_config_yaml_stdio_builtin_noop as base_config;
#[cfg(any(not(feature = "prompts"), not(feature = "resources")))]
use crate::config_common::load_config_fixture;
use rmcp::{model::ErrorCode, ErrorData as McpError};
#[cfg(not(feature = "auth"))]
use rust_mcp_core::config::AuthProviderConfig;
#[cfg(any(not(feature = "streamable_http"), not(feature = "completion")))]
use rust_mcp_core::config::PluginConfig;
#[cfg(not(feature = "streamable_http"))]
use rust_mcp_core::config::TransportMode;
#[cfg(not(feature = "http_tools"))]
use rust_mcp_core::config::{
    ExecuteConfig, ExecuteHttpConfig, TaskSupport, ToolConfig, UpstreamConfig,
};
#[cfg(any(not(feature = "streamable_http"), not(feature = "completion")))]
use rust_mcp_core::plugins::PluginType;
use rust_mcp_core::{config::McpConfig, runtime::build_runtime, PluginRegistry};

async fn runtime_error(config: McpConfig) -> McpError {
    build_runtime(config, PluginRegistry::new())
        .await
        .expect_err("runtime should fail with validation error")
}

#[cfg(any(not(feature = "prompts"), not(feature = "resources")))]
async fn runtime_ok(config: McpConfig) {
    build_runtime(config, PluginRegistry::new())
        .await
        .expect("runtime should build successfully");
}

#[cfg(not(feature = "auth"))]
#[tokio::test]
async fn auth_feature_gate_rejects_non_none_mode() {
    let mut config = base_config();
    let auth = config.server.auth_mut_or_insert();
    auth.providers = vec![AuthProviderConfig::bearer("static", "token")];
    auth.enabled = Some(true);
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "auth feature disabled but server.auth is active"
    );
}

#[cfg(not(feature = "streamable_http"))]
#[tokio::test]
async fn streamable_feature_gate_rejects_streamable_transport() {
    let mut config = base_config();
    config.server.transport.mode = TransportMode::StreamableHttp;
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "streamable_http feature disabled but server.transport.mode=streamable_http"
    );
}

#[cfg(not(feature = "streamable_http"))]
#[tokio::test]
async fn streamable_feature_gate_rejects_http_router_plugins() {
    let mut config = base_config();
    config.plugins.push(PluginConfig {
        name: "router".to_owned(),
        plugin_type: PluginType::HttpRouter,
        targets: None,
        config: None,
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "streamable_http feature disabled but plugins include type=http_router"
    );
}

#[cfg(not(feature = "http_tools"))]
#[tokio::test]
async fn http_tools_feature_gate_rejects_http_tools() {
    let mut config = base_config();
    config.set_tools_items(vec![ToolConfig {
        name: "noop".to_owned(),
        title: None,
        description: "No-op tool".to_owned(),
        cancellable: true,
        input_schema: serde_json::json!({"type":"object"}),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
        execute: ExecuteConfig::Http(ExecuteHttpConfig {
            upstream: "api".to_owned(),
            method: "GET".to_owned(),
            path: "/items".to_owned(),
            query: None,
            headers: None,
            body: None,
            retry: None,
            task_support: TaskSupport::Forbidden,
        }),
        response: None,
    }]);
    config.upstreams.insert(
        "api".to_owned(),
        UpstreamConfig {
            base_url: "http://example.com".to_owned(),
            headers: std::collections::HashMap::default(),
            user_agent: None,
            timeout_ms: None,
            max_response_bytes: None,
            retry: None,
            auth: None,
        },
    );
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "http_tools feature disabled but tool 'noop' uses execute.type=http"
    );
}

#[cfg(not(feature = "prompts"))]
#[tokio::test]
async fn prompts_feature_gate_rejects_prompts_config() {
    let config: McpConfig = serde_yaml::from_str(
        r"
version: 1
server:
  transport:
    mode: stdio
prompts:
  providers:
    - type: inline
      items:
        - name: prompt.one
          arguments_schema:
            type: object
          template:
            messages:
              - role: user
                content: hello
",
    )
    .expect("config should parse");
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "prompts feature disabled but prompts config is active"
    );
}

#[cfg(not(feature = "prompts"))]
#[tokio::test]
async fn prompts_feature_gate_allows_disabled_prompts_config() {
    let config = load_config_fixture("runtime/runtime_feature_gates_prompts_disabled_fixture");
    runtime_ok(config).await;
}

#[cfg(not(feature = "resources"))]
#[tokio::test]
async fn resources_feature_gate_rejects_resources_config() {
    let config: McpConfig = serde_yaml::from_str(
        r"
version: 1
server:
  transport:
    mode: stdio
resources:
  providers:
    - type: inline
      items:
        - uri: resource://hello
          name: hello
          content:
            text: hello
",
    )
    .expect("config should parse");
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "resources feature disabled but resources config is active"
    );
}

#[cfg(not(feature = "resources"))]
#[tokio::test]
async fn resources_feature_gate_allows_disabled_resources_config() {
    let config = load_config_fixture("runtime/runtime_feature_gates_resources_disabled_fixture");
    runtime_ok(config).await;
}

#[cfg(not(feature = "client_logging"))]
#[tokio::test]
async fn logging_feature_gate_rejects_enabled_logging_config() {
    let mut config = base_config();
    config.client_logging = Some(rust_mcp_core::config::ClientLoggingConfig::default());
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "client_logging feature disabled but logging config is present"
    );
}

#[cfg(not(feature = "progress_utility"))]
#[tokio::test]
async fn progress_feature_gate_rejects_enabled_progress_config() {
    let mut config = base_config();
    config.progress = Some(rust_mcp_core::config::ProgressConfig::default());
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "progress_utility feature disabled but progress config is present"
    );
}

#[cfg(not(feature = "completion"))]
#[tokio::test]
async fn completion_feature_gate_rejects_completion_config() {
    let mut config = base_config();
    config.completion = Some(rust_mcp_core::config::CompletionConfig {
        enabled: Some(true),
        providers: vec![rust_mcp_core::config::CompletionProviderConfig::Inline {
            name: "status".to_owned(),
            values: vec!["open".to_owned(), "done".to_owned()],
        }],
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "completion feature disabled but completion config is active"
    );
}

#[cfg(not(feature = "completion"))]
#[tokio::test]
async fn completion_feature_gate_rejects_completion_plugin_config() {
    let mut config = base_config();
    config.plugins.push(PluginConfig {
        name: "completion.plugin".to_owned(),
        plugin_type: PluginType::Completion,
        targets: None,
        config: None,
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "completion feature disabled but plugins include type=completion"
    );
}

#[cfg(not(feature = "tasks_utility"))]
#[tokio::test]
async fn tasks_feature_gate_rejects_enabled_tasks_config() {
    let mut config = base_config();
    config.tasks = Some(rust_mcp_core::config::TasksConfig {
        enabled: Some(true),
        ..rust_mcp_core::config::TasksConfig::default()
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "tasks_utility feature disabled but tasks config is active"
    );
}

#[cfg(not(feature = "client_features"))]
#[tokio::test]
async fn client_features_gate_rejects_enabled_roots_when_disabled() {
    let mut config = base_config();
    config.client_features.roots = Some(rust_mcp_core::config::ClientRootsConfig { enabled: true });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "client_features feature disabled but client_features has enabled sections"
    );
}

#[cfg(not(feature = "client_features"))]
#[tokio::test]
async fn client_features_gate_rejects_enabled_sampling_when_disabled() {
    let mut config = base_config();
    config.client_features.sampling = Some(rust_mcp_core::config::ClientSamplingConfig {
        enabled: Some(true),
        allow_tools: false,
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "client_features feature disabled but client_features has enabled sections"
    );
}

#[cfg(not(feature = "client_features"))]
#[tokio::test]
async fn client_features_gate_rejects_enabled_elicitation_when_disabled() {
    let mut config = base_config();
    config.client_features.elicitation = Some(rust_mcp_core::config::ClientElicitationConfig {
        enabled: Some(true),
        mode: rust_mcp_core::config::ElicitationMode::Form,
    });
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "client_features feature disabled but client_features has enabled sections"
    );
}

#[cfg(not(feature = "http_hardening"))]
#[tokio::test]
async fn http_hardening_feature_gate_rejects_hardening_config() {
    let mut config = base_config();
    config.server.transport.streamable_http.hardening =
        Some(rust_mcp_core::config::StreamableHttpHardeningConfig::default());
    let error = runtime_error(config).await;
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert_eq!(
        error.message,
        "http_hardening feature disabled but server.transport.streamable_http.hardening config is present"
    );
}
