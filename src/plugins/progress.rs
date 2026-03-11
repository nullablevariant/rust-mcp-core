//! Progress notification state and rate limiting.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

use crate::McpError;

#[derive(Clone, Debug)]
#[doc(hidden)]
pub(crate) struct ProgressState {
    enabled: bool,
    notification_interval: Duration,
}

#[derive(Default)]
pub(crate) struct ProgressStateInner {
    last_sent_at: Option<Instant>,
    last_progress: Option<f64>,
}

impl ProgressState {
    pub(crate) const fn new(enabled: bool, notification_interval_ms: u64) -> Self {
        Self {
            enabled,
            notification_interval: Duration::from_millis(notification_interval_ms),
        }
    }

    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) async fn should_send(
        &self,
        tracker: &Arc<Mutex<ProgressStateInner>>,
        progress: f64,
    ) -> Result<bool, McpError> {
        if !self.enabled {
            return Ok(false);
        }
        if !progress.is_finite() {
            return Err(McpError::invalid_params(
                "progress must be a finite number".to_owned(),
                None,
            ));
        }

        let mut inner = tracker.lock().await;
        let now = Instant::now();
        if self.notification_interval > Duration::ZERO {
            if let Some(last_sent_at) = inner.last_sent_at {
                if now.duration_since(last_sent_at) < self.notification_interval {
                    return Ok(false);
                }
            }
        }

        if let Some(last_progress) = inner.last_progress {
            if progress <= last_progress {
                return Err(McpError::invalid_params(
                    "progress must increase with each notification".to_owned(),
                    None,
                ));
            }
        }

        inner.last_sent_at = Some(now);
        inner.last_progress = Some(progress);
        Ok(true)
    }
}

impl Default for ProgressState {
    fn default() -> Self {
        Self::new(false, 250)
    }
}
