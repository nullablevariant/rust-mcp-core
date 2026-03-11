//! Handler implementations for logging/setLevel, completion/complete, and ping.
#[cfg(feature = "client_logging")]
use rmcp::model::SetLevelRequestParams;
#[cfg(feature = "completion")]
use rmcp::model::{CompleteRequestParams, CompleteResult};
#[cfg(feature = "client_features")]
use rmcp::service::NotificationContext;
#[cfg(any(feature = "completion", feature = "client_logging"))]
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
#[cfg(any(feature = "completion", feature = "client_logging"))]
use rmcp::ErrorData as McpError;
#[cfg(feature = "client_logging")]
use tracing::warn;

use super::super::orchestration::Engine;
#[cfg(any(feature = "completion", feature = "client_logging"))]
use crate::errors::cancelled_error;

impl Engine {
    #[cfg(all(
        feature = "completion",
        any(feature = "prompts", feature = "resources")
    ))]
    pub(super) async fn handle_complete_request(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.completion_active() {
            return Err(McpError::method_not_found::<
                rmcp::model::CompleteRequestMethod,
            >());
        }
        let completion = self
            .complete_request(&request, Some(context.clone()))
            .await?;
        Ok(CompleteResult::new(completion))
    }

    #[cfg(all(
        feature = "completion",
        not(any(feature = "prompts", feature = "resources"))
    ))]
    pub(super) fn handle_complete_request(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.config.completion_active() {
            return Err(McpError::method_not_found::<
                rmcp::model::CompleteRequestMethod,
            >());
        }
        let completion = Self::complete_request(request, Some(context))?;
        Ok(CompleteResult::new(completion))
    }

    #[cfg(feature = "client_logging")]
    pub(super) fn handle_set_level_request(
        &self,
        request: &SetLevelRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        if context.ct.is_cancelled() {
            return Err(cancelled_error());
        }
        if !self.client_logging.enabled() {
            warn!("logging/setLevel called while logging config is inactive");
            return Ok(());
        }
        self.client_logging.set_min_level(request.level);
        Ok(())
    }

    #[cfg(feature = "client_features")]
    pub(super) fn handle_roots_list_changed(&self, _context: NotificationContext<RoleServer>) {
        if self.config.client_roots_active() {
            tracing::info!("received roots list changed notification from client");
        }
    }
}
