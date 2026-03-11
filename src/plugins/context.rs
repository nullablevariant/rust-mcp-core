//! Plugin execution context providing cancellation, progress, logging, and client feature helpers.

use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::{OutboundHttpConfig, UpstreamAuth, UpstreamConfig};
use crate::http::client::{OutboundHttpRequest, OutboundHttpResponse, SharedHttpClient};
#[cfg(feature = "client_features")]
use crate::mcp::{
    CreateElicitationRequestParams, CreateElicitationResult, CreateMessageRequestParams,
    CreateMessageResult, ElicitationAction, ListRootsResult,
};
use crate::mcp::{LoggingLevel, ProgressToken};
use crate::rmcp_internal::{
    LoggingMessageNotificationParam, ProgressNotificationParam, RequestContext, RoleServer,
};
use crate::McpError;
#[cfg(feature = "http_tools")]
use crate::{
    auth::outbound::token_manager::OutboundTokenManager, config::UpstreamOauth2AuthConfig,
};

use super::logging::{
    build_notification_payload, log_to_server, ClientLoggingState, LogChannel, LogResult,
};
use super::progress::{ProgressState, ProgressStateInner};
use super::traits::{ListFeature, ListRefreshHandle};

/// Auth behavior for [`PluginContext::send_with`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PluginSendAuthMode {
    /// Inherit auth from the configured upstream (`bearer`/`basic`/`oauth2`).
    #[default]
    Inherit,
    /// Disable upstream auth for this request.
    None,
    /// Use an explicit `Authorization` header value for this request.
    Explicit { authorization: String },
}

/// Optional per-call outbound controls for [`PluginContext::send_with`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PluginSendOptions {
    /// Auth behavior override for this call.
    pub auth: PluginSendAuthMode,
}

struct PluginOauth2ExecutionContext<'a> {
    #[cfg(feature = "http_tools")]
    auth: &'a UpstreamOauth2AuthConfig,
    bearer: String,
    retry_on_401_once: bool,
    #[cfg(not(feature = "http_tools"))]
    _marker: std::marker::PhantomData<&'a ()>,
}

#[derive(Clone, Copy)]
struct PluginRequestHeadersParams<'a> {
    upstream: &'a UpstreamConfig,
    auth_mode: &'a PluginSendAuthMode,
    oauth2_bearer: Option<&'a str>,
    request_headers: &'a [(String, String)],
}

/// Redacted access token wrapper for plugin OAuth helper methods.
///
/// The token value is intentionally hidden in `Debug` output.
#[derive(Clone)]
pub struct PluginAccessToken {
    value: String,
}

impl PluginAccessToken {
    /// Returns the raw bearer token string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Consumes the wrapper and returns the raw bearer token string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.value
    }
}

impl std::fmt::Debug for PluginAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginAccessToken")
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Parameters for [`PluginContext::log_event`].
///
/// Groups the logging call arguments to satisfy the 4-parameter limit.
/// Construct with struct literal syntax; `data` and `channels` have sensible defaults.
///
/// # Examples
///
/// ```rust,no_run
/// use rust_mcp_core::{
///     mcp::{LoggingLevel, McpError},
///     LogChannel, LogEventParams, PluginContext,
/// };
///
/// async fn emit_log(ctx: &PluginContext) -> Result<(), McpError> {
///     let _result = ctx
///         .log_event(LogEventParams {
///             level: LoggingLevel::Info,
///             message: "something happened".to_owned(),
///             data: None,
///             channels: &[LogChannel::Server],
///         })
///         .await?;
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct LogEventParams<'a> {
    /// Severity level for the log entry.
    pub level: LoggingLevel,
    /// Human-readable log message.
    pub message: String,
    /// Optional structured data payload attached to the log entry.
    pub data: Option<Value>,
    /// Which channel(s) to emit to: [`LogChannel::Server`], [`LogChannel::Client`], or both.
    pub channels: &'a [LogChannel],
}

/// Context passed to plugin `call`/`list`/`get`/`read` methods during execution.
///
/// Provides access to:
/// - **Cancellation** — a [`CancellationToken`] wired to the client request lifecycle
/// - **Progress** — [`notify_progress`](Self::notify_progress) for long-running operations
/// - **Logging** — [`log_event`](Self::log_event) for server and/or client log emission
/// - **HTTP** — outbound helpers: [`send`](Self::send),
///   [`send_with`](Self::send_with), and [`send_raw`](Self::send_raw)
/// - **Upstreams** — named upstream base URLs from config
/// - **Client features** — [`request_roots`](Self::request_roots),
///   [`request_sampling`](Self::request_sampling), [`request_elicitation`](Self::request_elicitation)
///   (requires `client_features` feature)
/// - **List refresh** — [`request_list_refresh`](Self::request_list_refresh) to trigger
///   `notifications/list_changed`
#[derive(Clone)]
pub struct PluginContext {
    request: Option<RequestContext<RoleServer>>,
    pub cancellation: CancellationToken,
    pub progress: Option<ProgressToken>,
    pub upstreams: Arc<HashMap<String, UpstreamConfig>>,
    http: SharedHttpClient,
    outbound_http: Option<OutboundHttpConfig>,
    #[cfg(feature = "http_tools")]
    outbound_token_manager: Option<OutboundTokenManager>,
    client_logging: ClientLoggingState,
    server_log_payload_max_bytes: u64,
    progress_state: ProgressState,
    progress_tracker: Arc<Mutex<ProgressStateInner>>,
    list_refresh_handle: Option<Arc<dyn ListRefreshHandle>>,
    #[cfg(feature = "client_features")]
    client_features: crate::config::ClientFeaturesConfig,
}

impl PluginContext {
    #[doc(hidden)]
    pub fn new(
        request: Option<RequestContext<RoleServer>>,
        upstreams: Arc<HashMap<String, UpstreamConfig>>,
        http: SharedHttpClient,
    ) -> Self {
        let cancellation = request
            .as_ref()
            .map(|ctx| ctx.ct.clone())
            .unwrap_or_default();
        let progress = request
            .as_ref()
            .and_then(|ctx| ctx.meta.get_progress_token());
        Self {
            request,
            cancellation,
            progress,
            upstreams,
            http,
            outbound_http: None,
            #[cfg(feature = "http_tools")]
            outbound_token_manager: None,
            client_logging: ClientLoggingState::default(),
            server_log_payload_max_bytes: 4096,
            progress_state: ProgressState::default(),
            progress_tracker: Arc::new(Mutex::new(ProgressStateInner::default())),
            list_refresh_handle: None,
            #[cfg(feature = "client_features")]
            client_features: crate::config::ClientFeaturesConfig::default(),
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn with_outbound_http(mut self, config: Option<OutboundHttpConfig>) -> Self {
        self.outbound_http = config;
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub fn with_client_logging_state(mut self, state: ClientLoggingState) -> Self {
        self.client_logging = state;
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub const fn with_server_log_payload_max_bytes(mut self, max_bytes: u64) -> Self {
        self.server_log_payload_max_bytes = max_bytes;
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub(crate) const fn with_progress_state(mut self, state: ProgressState) -> Self {
        self.progress_state = state;
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub const fn with_progress(mut self, enabled: bool, notification_interval_ms: u64) -> Self {
        self.progress_state = ProgressState::new(enabled, notification_interval_ms);
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub fn with_list_refresh_handle(mut self, handle: Arc<dyn ListRefreshHandle>) -> Self {
        self.list_refresh_handle = Some(handle);
        self
    }

    #[cfg(feature = "http_tools")]
    #[must_use]
    #[doc(hidden)]
    pub(crate) fn with_outbound_token_manager(
        mut self,
        token_manager: OutboundTokenManager,
    ) -> Self {
        self.outbound_token_manager = Some(token_manager);
        self
    }

    #[cfg(feature = "client_features")]
    #[must_use]
    #[doc(hidden)]
    pub const fn with_client_features(
        mut self,
        config: crate::config::ClientFeaturesConfig,
    ) -> Self {
        self.client_features = config;
        self
    }

    /// Requests a list-changed refresh for tools/prompts/resources.
    ///
    /// Returns `Ok(true)` when a notification is emitted, `Ok(false)` when the
    /// target feature is currently not active.
    pub async fn request_list_refresh(&self, feature: ListFeature) -> Result<bool, McpError> {
        let Some(handle) = self.list_refresh_handle.as_ref() else {
            return Err(McpError::invalid_request(
                "list refresh unavailable".to_owned(),
                None,
            ));
        };
        handle.refresh_list(feature).await
    }

    /// Emits a plugin log event to server logs, client notifications, or both.
    pub async fn log_event(&self, params: LogEventParams<'_>) -> Result<LogResult, McpError> {
        let LogEventParams {
            level,
            message,
            data,
            channels,
        } = params;
        let notify_server = channels.contains(&LogChannel::Server);
        let notify_client = channels.contains(&LogChannel::Client);
        let mut result = LogResult::default();

        if notify_server {
            log_to_server(
                level,
                &message,
                data.as_ref(),
                self.server_log_payload_max_bytes,
            );
            result.server_logged = true;
        }

        if notify_client && self.client_logging.should_notify(level) {
            let Some(request) = self.request.as_ref() else {
                tracing::warn!("client log requested without request context");
                return Ok(result);
            };

            let payload = build_notification_payload(message, data);
            request
                .peer
                .notify_logging_message(LoggingMessageNotificationParam {
                    level,
                    logger: None,
                    data: payload,
                })
                .await
                .map_err(|error| {
                    McpError::internal_error(
                        format!("failed to send logging notification: {error}"),
                        None,
                    )
                })?;
            result.client_notified = true;
        }

        Ok(result)
    }

    /// Sends a progress notification when progress reporting is enabled and throttling allows it.
    ///
    /// Returns `Ok(true)` when a notification is sent and `Ok(false)` when progress
    /// is unavailable or suppressed by throttling.
    pub async fn notify_progress(
        &self,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    ) -> Result<bool, McpError> {
        let Some(request) = self.request.as_ref() else {
            return Ok(false);
        };
        let Some(progress_token) = self.progress.clone() else {
            return Ok(false);
        };
        if let Some(total_value) = total {
            if !total_value.is_finite() {
                return Err(McpError::invalid_params(
                    "total must be a finite number".to_owned(),
                    None,
                ));
            }
        }
        if !self
            .progress_state
            .should_send(&self.progress_tracker, progress)
            .await?
        {
            return Ok(false);
        }

        request
            .peer
            .notify_progress(ProgressNotificationParam {
                progress_token,
                progress,
                total,
                message,
            })
            .await
            .map_err(|error| {
                McpError::internal_error(
                    format!("failed to send progress notification: {error}"),
                    None,
                )
            })?;
        Ok(true)
    }

    /// Fetches an OAuth2 access token for a configured upstream.
    ///
    /// The upstream must exist and use `upstreams.<name>.auth.type=oauth2`.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when:
    /// - upstream name is unknown,
    /// - upstream auth is not oauth2,
    /// - OAuth helper is unavailable in this context.
    ///
    /// Propagates token resolution/exchange errors from the outbound OAuth path.
    #[cfg(feature = "http_tools")]
    pub async fn upstream_access_token(
        &self,
        upstream_name: &str,
        force_refresh: bool,
    ) -> Result<PluginAccessToken, McpError> {
        let auth = self.oauth2_upstream_auth(upstream_name)?;
        let token_manager = self.outbound_token_manager.as_ref().ok_or_else(|| {
            McpError::invalid_request("outbound oauth2 helper unavailable".to_owned(), None)
        })?;
        let token = token_manager
            .access_token(upstream_name, auth, force_refresh)
            .await?;
        Ok(PluginAccessToken {
            value: secrecy::ExposeSecret::expose_secret(&token).to_owned(),
        })
    }

    /// Builds an `Authorization` header pair using an upstream OAuth2 access token.
    ///
    /// Returns `("Authorization", "Bearer <token>")`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`PluginContext::upstream_access_token`].
    #[cfg(feature = "http_tools")]
    pub async fn upstream_bearer_header(
        &self,
        upstream_name: &str,
        force_refresh: bool,
    ) -> Result<(String, String), McpError> {
        let token = self
            .upstream_access_token(upstream_name, force_refresh)
            .await?;
        Ok((
            "Authorization".to_owned(),
            format!("Bearer {}", token.as_str()),
        ))
    }

    /// Sends an upstream-bound outbound request using default behavior.
    ///
    /// Equivalent to [`PluginContext::send_with`] with
    /// `PluginSendOptions { auth: PluginSendAuthMode::Inherit }`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`PluginContext::send_with`].
    pub async fn send(
        &self,
        upstream_name: &str,
        request: OutboundHttpRequest,
    ) -> Result<OutboundHttpResponse, McpError> {
        self.send_with(upstream_name, request, PluginSendOptions::default())
            .await
    }

    /// Sends an upstream-bound outbound request with per-call policy overrides.
    ///
    /// If `request.url` starts with `/`, the upstream base URL is prepended.
    /// If `request.url` is absolute (`http://` or `https://`), it is used as-is.
    ///
    /// For unset request limits, timeout/size fallback order is:
    /// request -> upstream -> `outbound_http`.
    ///
    /// Retry fallback order is:
    /// upstream -> global outbound retry.
    /// Retries apply only to idempotent methods (`GET`, `HEAD`, `OPTIONS`, `DELETE`).
    ///
    /// Header merge precedence is:
    /// - `outbound_http.user_agent`
    /// - `outbound_http.headers`
    /// - `upstreams.<name>.headers`
    /// - auth layer (depends on `options.auth`)
    /// - explicit request headers (final override)
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when:
    /// - `upstream_name` does not exist,
    /// - `request.url` is neither absolute nor path-absolute,
    /// - OAuth2 helper support is required but unavailable for this build/runtime.
    ///
    /// Propagates outbound transport/send failures from the shared HTTP client.
    pub async fn send_with(
        &self,
        upstream_name: &str,
        mut request: OutboundHttpRequest,
        options: PluginSendOptions,
    ) -> Result<OutboundHttpResponse, McpError> {
        let upstream = self.upstreams.get(upstream_name).ok_or_else(|| {
            McpError::invalid_request(format!("upstream '{upstream_name}' not found"), None)
        })?;
        let oauth2_context = self
            .resolve_plugin_oauth2_context(upstream_name, upstream, &options.auth)
            .await?;
        let original_request_headers = request.headers.clone();

        request.url = crate::http::outbound_pipeline::resolve_upstream_url(
            &upstream.base_url,
            request.url.as_str(),
        )?;
        request.timeout_ms = crate::http::outbound_pipeline::resolve_timeout_ms(
            request.timeout_ms,
            upstream,
            self.outbound_http.as_ref(),
        );
        request.max_response_bytes = crate::http::outbound_pipeline::resolve_max_response_bytes(
            request.max_response_bytes,
            upstream,
            self.outbound_http.as_ref(),
        );
        request.headers = self.build_upstream_request_headers(PluginRequestHeadersParams {
            upstream,
            auth_mode: &options.auth,
            oauth2_bearer: oauth2_context
                .as_ref()
                .map(|context| context.bearer.as_str()),
            request_headers: &original_request_headers,
        });

        let retry = crate::http::outbound_pipeline::resolve_retry_config(
            None,
            upstream,
            self.outbound_http.as_ref(),
        );
        let cancellation = self.cancellation.clone();
        let response = self
            .execute_upstream_http_with_retry(request.clone(), retry, Some(&cancellation))
            .await?;

        if response.status() == 401
            && oauth2_context
                .as_ref()
                .is_some_and(|context| context.retry_on_401_once)
        {
            #[cfg(feature = "http_tools")]
            if let Some(context) = oauth2_context.as_ref() {
                let token_manager = self.outbound_token_manager.as_ref().ok_or_else(|| {
                    McpError::invalid_request("outbound oauth2 helper unavailable".to_owned(), None)
                })?;
                let refreshed = token_manager
                    .access_token(upstream_name, context.auth, true)
                    .await?;
                let refreshed_bearer = secrecy::ExposeSecret::expose_secret(&refreshed).to_owned();
                request.headers = self.build_upstream_request_headers(PluginRequestHeadersParams {
                    upstream,
                    auth_mode: &options.auth,
                    oauth2_bearer: Some(refreshed_bearer.as_str()),
                    request_headers: &original_request_headers,
                });
                return self
                    .execute_upstream_http_with_retry(request, retry, Some(&cancellation))
                    .await;
            }
        }

        Ok(response)
    }

    /// Sends a raw outbound request through the shared HTTP client.
    ///
    /// This path does not apply upstream URL resolution, defaults, auth injection,
    /// or retry policy.
    ///
    /// # Errors
    ///
    /// Propagates outbound transport/send failures from the shared HTTP client.
    pub async fn send_raw(
        &self,
        request: OutboundHttpRequest,
    ) -> Result<OutboundHttpResponse, McpError> {
        self.http.send(request).await
    }

    fn build_upstream_request_headers(
        &self,
        params: PluginRequestHeadersParams<'_>,
    ) -> Vec<(String, String)> {
        let PluginRequestHeadersParams {
            upstream,
            auth_mode,
            oauth2_bearer,
            request_headers,
        } = params;
        let auth = match auth_mode {
            PluginSendAuthMode::Inherit => {
                crate::http::outbound_pipeline::PluginRequestAuth::inherit(oauth2_bearer)
            }
            PluginSendAuthMode::None => crate::http::outbound_pipeline::PluginRequestAuth::none(),
            PluginSendAuthMode::Explicit { authorization } => {
                crate::http::outbound_pipeline::PluginRequestAuth::explicit(authorization)
            }
        };
        crate::http::outbound_pipeline::build_request_headers(
            self.outbound_http.as_ref(),
            upstream,
            auth,
            request_headers,
        )
    }

    async fn execute_upstream_http_with_retry(
        &self,
        request: OutboundHttpRequest,
        retry: Option<&crate::config::OutboundRetryConfig>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<OutboundHttpResponse, McpError> {
        crate::http::outbound_pipeline::execute_with_retry(
            crate::http::outbound_pipeline::RetryExecutionParams {
                method: request.method.as_str(),
                retry,
                cancellation,
                cancelled_error: crate::errors::cancelled_error,
            },
            || {
                let request = request.clone();
                self.http.send(request)
            },
            |result, retry| match result {
                Ok(response) => retry.on_statuses.contains(&response.status()),
                Err(error) => {
                    error.code == crate::mcp::ErrorCode::INTERNAL_ERROR && retry.on_network_errors
                }
            },
        )
        .await
    }

    #[cfg(feature = "http_tools")]
    async fn resolve_plugin_oauth2_context<'a>(
        &self,
        upstream_name: &str,
        upstream: &'a UpstreamConfig,
        auth_mode: &PluginSendAuthMode,
    ) -> Result<Option<PluginOauth2ExecutionContext<'a>>, McpError> {
        if !matches!(auth_mode, PluginSendAuthMode::Inherit) {
            return Ok(None);
        }
        let Some(UpstreamAuth::Oauth2(auth)) = upstream.auth.as_ref() else {
            return Ok(None);
        };
        let retry_on_401_once =
            crate::auth::outbound::config::OutboundOauth2RefreshPolicy::from_auth(auth)
                .retry_on_401_once;
        let token_manager = self.outbound_token_manager.as_ref().ok_or_else(|| {
            McpError::invalid_request("outbound oauth2 helper unavailable".to_owned(), None)
        })?;
        let token = token_manager
            .access_token(upstream_name, auth.as_ref(), false)
            .await?;
        Ok(Some(PluginOauth2ExecutionContext {
            auth,
            bearer: secrecy::ExposeSecret::expose_secret(&token).to_owned(),
            retry_on_401_once,
        }))
    }

    #[cfg(not(feature = "http_tools"))]
    #[allow(
        clippy::unused_async,
        reason = "Keep helper signature consistent across http_tools feature variants"
    )]
    async fn resolve_plugin_oauth2_context<'a>(
        &self,
        _upstream_name: &str,
        upstream: &'a UpstreamConfig,
        auth_mode: &PluginSendAuthMode,
    ) -> Result<Option<PluginOauth2ExecutionContext<'a>>, McpError> {
        if matches!(auth_mode, PluginSendAuthMode::Inherit)
            && matches!(upstream.auth, Some(UpstreamAuth::Oauth2(_)))
        {
            return Err(McpError::invalid_request(
                "upstream oauth2 auth for plugin outbound requests requires the http_tools feature"
                    .to_owned(),
                None,
            ));
        }
        Ok(None)
    }

    #[cfg(feature = "http_tools")]
    fn oauth2_upstream_auth(
        &self,
        upstream_name: &str,
    ) -> Result<&UpstreamOauth2AuthConfig, McpError> {
        let upstream = self.upstreams.get(upstream_name).ok_or_else(|| {
            McpError::invalid_request(format!("upstream '{upstream_name}' not found"), None)
        })?;
        let auth = upstream.auth.as_ref().ok_or_else(|| {
            McpError::invalid_request(
                format!("upstream '{upstream_name}' does not use oauth2 auth"),
                None,
            )
        })?;
        match auth {
            UpstreamAuth::Oauth2(config) => Ok(config.as_ref()),
            _ => Err(McpError::invalid_request(
                format!("upstream '{upstream_name}' does not use oauth2 auth"),
                None,
            )),
        }
    }

    #[cfg(feature = "client_features")]
    pub async fn request_roots(&self) -> Result<ListRootsResult, McpError> {
        if !self.client_features.roots_active() {
            return Err(McpError::invalid_request(
                "client_features.roots is not enabled".to_owned(),
                None,
            ));
        }
        let request = self.request.as_ref().ok_or_else(|| {
            McpError::invalid_request("request_roots requires a request context".to_owned(), None)
        })?;
        let peer_info = request
            .peer
            .peer_info()
            .ok_or_else(|| McpError::internal_error("peer info unavailable".to_owned(), None))?;
        if peer_info.capabilities.roots.is_none() {
            return Err(McpError::invalid_request(
                "client does not support roots capability".to_owned(),
                None,
            ));
        }
        request.peer.list_roots().await.map_err(|error| {
            tracing::warn!("list_roots RPC failed: {error}");
            McpError::internal_error(format!("list_roots failed: {error}"), None)
        })
    }

    #[cfg(feature = "client_features")]
    pub async fn request_sampling(
        &self,
        params: CreateMessageRequestParams,
    ) -> Result<CreateMessageResult, McpError> {
        if !self.client_features.sampling_active() {
            return Err(McpError::invalid_request(
                "client_features.sampling is not enabled".to_owned(),
                None,
            ));
        }
        let request = self.request.as_ref().ok_or_else(|| {
            McpError::invalid_request(
                "request_sampling requires a request context".to_owned(),
                None,
            )
        })?;
        let peer_info = request
            .peer
            .peer_info()
            .ok_or_else(|| McpError::internal_error("peer info unavailable".to_owned(), None))?;
        if peer_info.capabilities.sampling.is_none() {
            return Err(McpError::invalid_request(
                "client does not support sampling capability".to_owned(),
                None,
            ));
        }
        let has_tools =
            params.tools.as_ref().is_some_and(|t| !t.is_empty()) || params.tool_choice.is_some();
        if has_tools && !self.client_features.sampling_allow_tools() {
            return Err(McpError::invalid_params(
                "sampling tools not allowed by server config (allow_tools=false)".to_owned(),
                None,
            ));
        }
        if has_tools && !request.peer.supports_sampling_tools() {
            return Err(McpError::invalid_params(
                "client does not support sampling tools capability".to_owned(),
                None,
            ));
        }
        request.peer.create_message(params).await.map_err(|error| {
            tracing::warn!("create_message RPC failed: {error}");
            McpError::internal_error(format!("create_message failed: {error}"), None)
        })
    }

    #[cfg(feature = "client_features")]
    pub async fn request_elicitation(
        &self,
        params: CreateElicitationRequestParams,
    ) -> Result<CreateElicitationResult, McpError> {
        use crate::config::ElicitationMode;

        if !self.client_features.elicitation_active() {
            return Err(McpError::invalid_request(
                "client_features.elicitation is not enabled".to_owned(),
                None,
            ));
        }
        let request = self.request.as_ref().ok_or_else(|| {
            McpError::invalid_request(
                "request_elicitation requires a request context".to_owned(),
                None,
            )
        })?;
        let peer_info = request
            .peer
            .peer_info()
            .ok_or_else(|| McpError::internal_error("peer info unavailable".to_owned(), None))?;
        let elicitation_cap = peer_info.capabilities.elicitation.as_ref().ok_or_else(|| {
            McpError::invalid_request(
                "client does not support elicitation capability".to_owned(),
                None,
            )
        })?;

        // Determine if the request is form or url mode
        let is_url_request = matches!(
            params,
            CreateElicitationRequestParams::UrlElicitationParams { .. }
        );
        let is_form_request = !is_url_request;

        // Config mode validation
        let config_mode = self
            .client_features
            .elicitation_mode()
            .unwrap_or(ElicitationMode::Form);
        if is_url_request && config_mode == ElicitationMode::Form {
            return Err(McpError::invalid_params(
                "URL elicitation not allowed by server config (mode=form)".to_owned(),
                None,
            ));
        }
        if is_form_request && config_mode == ElicitationMode::Url {
            return Err(McpError::invalid_params(
                "form elicitation not allowed by server config (mode=url)".to_owned(),
                None,
            ));
        }

        // Client capability mode validation
        // Per spec: empty `elicitation: {}` = form-only (backwards compat)
        let client_supports_form = elicitation_cap.form.is_some() || elicitation_cap.url.is_none();
        let client_supports_url = elicitation_cap.url.is_some();

        if is_form_request && !client_supports_form {
            return Err(McpError::invalid_params(
                "client does not support form elicitation".to_owned(),
                None,
            ));
        }
        if is_url_request && !client_supports_url {
            return Err(McpError::invalid_params(
                "client does not support URL elicitation".to_owned(),
                None,
            ));
        }

        let result = request
            .peer
            .create_elicitation(params)
            .await
            .map_err(|error| {
                tracing::warn!("create_elicitation RPC failed: {error}");
                McpError::internal_error(format!("create_elicitation failed: {error}"), None)
            })?;

        match result.action {
            ElicitationAction::Decline => {
                tracing::info!("client declined elicitation request");
            }
            ElicitationAction::Cancel => {
                tracing::info!("client cancelled elicitation request");
            }
            ElicitationAction::Accept => {}
        }

        Ok(result)
    }
}

impl std::fmt::Debug for PluginContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("PluginContext");
        debug
            .field("has_request", &self.request.is_some())
            .field("cancellation", &self.cancellation)
            .field("progress", &self.progress)
            .field("upstream_count", &self.upstreams.len())
            .field("has_http_client", &true);
        debug.field("has_outbound_http", &self.outbound_http.is_some());
        #[cfg(feature = "http_tools")]
        {
            debug.field(
                "has_outbound_token_manager",
                &self.outbound_token_manager.is_some(),
            );
        }
        debug
            .field("client_logging_enabled", &self.client_logging.enabled())
            .field("progress_enabled", &self.progress_state.enabled())
            // progress_tracker inner state is not Debug
            .field("has_progress_tracker", &true)
            .field(
                "has_list_refresh_handle",
                &self.list_refresh_handle.is_some(),
            )
            .finish_non_exhaustive()
    }
}
