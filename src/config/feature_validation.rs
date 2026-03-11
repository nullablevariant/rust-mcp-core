//! Shared feature-gate validation used by runtime and engine construction paths.

#[cfg(not(feature = "http_tools"))]
use super::ExecuteType;
use super::McpConfig;
#[cfg(not(feature = "tasks_utility"))]
use super::TaskSupport;
use rmcp::ErrorData as McpError;

// Returns the first feature-gate violation in config, if any.
#[cfg(not(all(
    feature = "http_tools",
    feature = "prompts",
    feature = "resources",
    feature = "client_logging",
    feature = "progress_utility",
    feature = "tasks_utility",
    feature = "client_features",
    feature = "http_hardening"
)))]
pub(crate) fn shared_feature_validation_error(config: &McpConfig) -> Option<McpError> {
    #[cfg(not(feature = "http_tools"))]
    if let Some(error) = check_http_tools(config) {
        return Some(error);
    }

    #[cfg(not(feature = "prompts"))]
    if let Some(error) = check_prompts(config) {
        return Some(error);
    }

    #[cfg(not(feature = "resources"))]
    if let Some(error) = check_resources(config) {
        return Some(error);
    }

    #[cfg(not(feature = "client_logging"))]
    if let Some(error) = check_logging(config) {
        return Some(error);
    }

    #[cfg(not(feature = "progress_utility"))]
    if let Some(error) = check_progress(config) {
        return Some(error);
    }

    #[cfg(not(feature = "tasks_utility"))]
    if let Some(error) = check_tasks(config) {
        return Some(error);
    }

    #[cfg(not(feature = "client_features"))]
    if let Some(error) = check_client_features(config) {
        return Some(error);
    }

    #[cfg(not(feature = "http_hardening"))]
    if let Some(error) = check_http_hardening(config) {
        return Some(error);
    }

    None
}

#[cfg(all(
    feature = "http_tools",
    feature = "prompts",
    feature = "resources",
    feature = "client_logging",
    feature = "progress_utility",
    feature = "tasks_utility",
    feature = "client_features",
    feature = "http_hardening"
))]
pub(crate) const fn shared_feature_validation_error(_config: &McpConfig) -> Option<McpError> {
    None
}

#[cfg(not(feature = "http_tools"))]
fn check_http_tools(config: &McpConfig) -> Option<McpError> {
    config
        .tools_items()
        .iter()
        .find(|tool| tool.execute.execute_type() == ExecuteType::Http)
        .map(|tool| {
            McpError::invalid_request(
                format!(
                    "http_tools feature disabled but tool '{}' uses execute.type=http",
                    tool.name
                ),
                None,
            )
        })
}

#[cfg(not(feature = "prompts"))]
fn check_prompts(config: &McpConfig) -> Option<McpError> {
    config.prompts_active().then(|| {
        McpError::invalid_request(
            "prompts feature disabled but prompts config is active".to_owned(),
            None,
        )
    })
}

#[cfg(not(feature = "resources"))]
fn check_resources(config: &McpConfig) -> Option<McpError> {
    config.resources_active().then(|| {
        McpError::invalid_request(
            "resources feature disabled but resources config is active".to_owned(),
            None,
        )
    })
}

#[cfg(not(feature = "client_logging"))]
fn check_logging(config: &McpConfig) -> Option<McpError> {
    config.client_logging_active().then(|| {
        McpError::invalid_request(
            "client_logging feature disabled but logging config is present".to_owned(),
            None,
        )
    })
}

#[cfg(not(feature = "progress_utility"))]
fn check_progress(config: &McpConfig) -> Option<McpError> {
    config.progress_active().then(|| {
        McpError::invalid_request(
            "progress_utility feature disabled but progress config is present".to_owned(),
            None,
        )
    })
}

#[cfg(not(feature = "tasks_utility"))]
fn check_tasks(config: &McpConfig) -> Option<McpError> {
    if config.tasks_active() {
        return Some(McpError::invalid_request(
            "tasks_utility feature disabled but tasks config is active".to_owned(),
            None,
        ));
    }
    config
        .tools_items()
        .iter()
        .find(|tool| tool.execute.task_support() != TaskSupport::Forbidden)
        .map(|tool| {
            McpError::invalid_request(
                format!(
                    "tasks_utility feature disabled but tool '{}' sets execute.task_support",
                    tool.name
                ),
                None,
            )
        })
}

#[cfg(not(feature = "client_features"))]
fn check_client_features(config: &McpConfig) -> Option<McpError> {
    (config.client_roots_active()
        || config.client_sampling_active()
        || config.client_elicitation_active())
    .then(|| {
        McpError::invalid_request(
            "client_features feature disabled but client_features has enabled sections".to_owned(),
            None,
        )
    })
}

#[cfg(not(feature = "http_hardening"))]
fn check_http_hardening(config: &McpConfig) -> Option<McpError> {
    config
        .server
        .transport
        .streamable_http
        .hardening
        .is_some()
        .then(|| {
            McpError::invalid_request(
                "http_hardening feature disabled but server.transport.streamable_http.hardening config is present"
                    .to_owned(),
                None,
            )
        })
}
