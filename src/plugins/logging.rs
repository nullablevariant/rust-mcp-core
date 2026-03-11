//! Plugin logging state and helpers for server/client log emission.

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use serde_json::Value;

use crate::log_safety::{truncate_json_for_log, truncate_string_for_log};
use crate::mcp::LoggingLevel;

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct ClientLoggingState {
    enabled: bool,
    min_level: Arc<AtomicU8>,
}

impl ClientLoggingState {
    pub fn new(enabled: bool, min_level: LoggingLevel) -> Self {
        Self {
            enabled,
            min_level: Arc::new(AtomicU8::new(logging_level_rank(min_level))),
        }
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn min_level(&self) -> LoggingLevel {
        u8_to_logging_level(self.min_level.load(Ordering::Relaxed))
    }

    pub fn set_min_level(&self, level: LoggingLevel) {
        self.min_level
            .store(logging_level_rank(level), Ordering::Relaxed);
    }

    pub fn should_notify(&self, level: LoggingLevel) -> bool {
        self.enabled && logging_level_rank(level) >= logging_level_rank(self.min_level())
    }
}

impl Default for ClientLoggingState {
    fn default() -> Self {
        Self::new(false, LoggingLevel::Info)
    }
}

/// Selects where a log event is emitted.
///
/// Pass one or both variants to [`PluginContext::log_event`] to control
/// whether the message goes to the server's tracing output, the MCP client
/// via `notifications/message`, or both.
///
/// [`PluginContext::log_event`]: crate::PluginContext::log_event
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogChannel {
    /// Emit to the server's tracing subscriber (controlled by `server.logging.level`).
    Server,
    /// Send an MCP `notifications/message` to the client (controlled by `client_logging.level`
    /// and `logging/setLevel`).
    Client,
}

/// Outcome of a [`PluginContext::log_event`] call.
///
/// [`PluginContext::log_event`]: crate::PluginContext::log_event
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogResult {
    /// `true` if the message was written to the server tracing output.
    pub server_logged: bool,
    /// `true` if an MCP `notifications/message` was sent to the client.
    pub client_notified: bool,
}

pub(crate) fn build_notification_payload(message: String, data: Option<Value>) -> Value {
    match data {
        Some(Value::Object(mut object)) => {
            object
                .entry("message".to_owned())
                .or_insert(Value::String(message));
            Value::Object(object)
        }
        Some(value) => Value::Object(
            [
                ("message".to_owned(), Value::String(message)),
                ("details".to_owned(), value),
            ]
            .into_iter()
            .collect(),
        ),
        None => Value::String(message),
    }
}

pub(crate) fn log_to_server(
    level: LoggingLevel,
    message: &str,
    data: Option<&Value>,
    max_payload_bytes: u64,
) {
    dispatch_log(level, message, data, max_payload_bytes);
}

// Dispatches a plugin log event to the appropriate tracing level.
// Each log-level path is isolated in its own helper to stay within
// the cognitive-complexity threshold despite tracing-macro expansion.
fn dispatch_log(level: LoggingLevel, message: &str, data: Option<&Value>, max_payload_bytes: u64) {
    match level {
        LoggingLevel::Debug => log_debug(message, data, max_payload_bytes),
        LoggingLevel::Info | LoggingLevel::Notice => log_info(message, data, max_payload_bytes),
        LoggingLevel::Warning => log_warn(message, data, max_payload_bytes),
        LoggingLevel::Error
        | LoggingLevel::Critical
        | LoggingLevel::Alert
        | LoggingLevel::Emergency => log_error(message, data, max_payload_bytes),
    }
}

fn log_debug(message: &str, data: Option<&Value>, max_payload_bytes: u64) {
    let fields = prepare_log_fields(message, data, max_payload_bytes);
    if let Some(data) = fields.data.as_ref() {
        tracing::debug!(
            message = %fields.message.value,
            message_bytes = fields.message.original_bytes,
            message_truncated = fields.message.truncated,
            data = %data.value,
            data_bytes = data.original_bytes,
            data_truncated = data.truncated,
            "plugin log"
        );
        return;
    }
    tracing::debug!(
        message = %fields.message.value,
        message_bytes = fields.message.original_bytes,
        message_truncated = fields.message.truncated,
        "plugin log"
    );
}

fn log_info(message: &str, data: Option<&Value>, max_payload_bytes: u64) {
    let fields = prepare_log_fields(message, data, max_payload_bytes);
    if let Some(data) = fields.data.as_ref() {
        tracing::info!(
            message = %fields.message.value,
            message_bytes = fields.message.original_bytes,
            message_truncated = fields.message.truncated,
            data = %data.value,
            data_bytes = data.original_bytes,
            data_truncated = data.truncated,
            "plugin log"
        );
        return;
    }
    tracing::info!(
        message = %fields.message.value,
        message_bytes = fields.message.original_bytes,
        message_truncated = fields.message.truncated,
        "plugin log"
    );
}

fn log_warn(message: &str, data: Option<&Value>, max_payload_bytes: u64) {
    let fields = prepare_log_fields(message, data, max_payload_bytes);
    if let Some(data) = fields.data.as_ref() {
        tracing::warn!(
            message = %fields.message.value,
            message_bytes = fields.message.original_bytes,
            message_truncated = fields.message.truncated,
            data = %data.value,
            data_bytes = data.original_bytes,
            data_truncated = data.truncated,
            "plugin log"
        );
        return;
    }
    tracing::warn!(
        message = %fields.message.value,
        message_bytes = fields.message.original_bytes,
        message_truncated = fields.message.truncated,
        "plugin log"
    );
}

fn log_error(message: &str, data: Option<&Value>, max_payload_bytes: u64) {
    let fields = prepare_log_fields(message, data, max_payload_bytes);
    if let Some(data) = fields.data.as_ref() {
        tracing::error!(
            message = %fields.message.value,
            message_bytes = fields.message.original_bytes,
            message_truncated = fields.message.truncated,
            data = %data.value,
            data_bytes = data.original_bytes,
            data_truncated = data.truncated,
            "plugin log"
        );
        return;
    }
    tracing::error!(
        message = %fields.message.value,
        message_bytes = fields.message.original_bytes,
        message_truncated = fields.message.truncated,
        "plugin log"
    );
}

struct PreparedLogFields {
    message: crate::log_safety::TruncatedField,
    data: Option<crate::log_safety::TruncatedField>,
}

fn prepare_log_fields(
    message: &str,
    data: Option<&Value>,
    max_payload_bytes: u64,
) -> PreparedLogFields {
    PreparedLogFields {
        message: truncate_string_for_log(message, max_payload_bytes),
        data: data.map(|value| truncate_json_for_log(value, max_payload_bytes)),
    }
}

pub(crate) const fn logging_level_rank(level: LoggingLevel) -> u8 {
    match level {
        LoggingLevel::Debug => 0,
        LoggingLevel::Info => 1,
        LoggingLevel::Notice => 2,
        LoggingLevel::Warning => 3,
        LoggingLevel::Error => 4,
        LoggingLevel::Critical => 5,
        LoggingLevel::Alert => 6,
        LoggingLevel::Emergency => 7,
    }
}

fn u8_to_logging_level(level: u8) -> LoggingLevel {
    match level {
        0 => LoggingLevel::Debug,
        1 => LoggingLevel::Info,
        2 => LoggingLevel::Notice,
        3 => LoggingLevel::Warning,
        4 => LoggingLevel::Error,
        5 => LoggingLevel::Critical,
        6 => LoggingLevel::Alert,
        7 => LoggingLevel::Emergency,
        // Invalid values are unreachable through public API usage.
        _ => {
            tracing::warn!("invalid logging level value in client logging state: {level}");
            LoggingLevel::Info
        }
    }
}

// Inline tests are required here because private helpers are not externally reachable.
#[cfg(test)]
mod tests {
    use super::{
        build_notification_payload, log_to_server, logging_level_rank, truncate_json_for_log,
        u8_to_logging_level,
    };
    use crate::mcp::LoggingLevel;
    use crate::plugins::logging::ClientLoggingState;
    use serde_json::{json, Value};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::Level;

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture_logs<F>(operation: F) -> String
    where
        F: FnOnce(),
    {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_buffer = Arc::clone(&buffer);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::TRACE)
            .without_time()
            .with_ansi(false)
            .with_writer(move || LogWriter(Arc::clone(&writer_buffer)))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        operation();

        let bytes = buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn build_notification_payload_with_none_data_returns_string() {
        let payload = build_notification_payload("hello".to_owned(), None);
        assert_eq!(payload, Value::String("hello".to_owned()));
    }

    #[test]
    fn build_notification_payload_with_object_data_inserts_message() {
        let data = json!({"key": "value"});
        let payload = build_notification_payload("hello".to_owned(), Some(data));
        let object = payload.as_object().expect("should be object");
        assert_eq!(
            object.get("message"),
            Some(&Value::String("hello".to_owned()))
        );
        assert_eq!(object.get("key"), Some(&Value::String("value".to_owned())));
    }

    #[test]
    fn build_notification_payload_with_object_does_not_overwrite_existing_message() {
        let data = json!({"message": "existing"});
        let payload = build_notification_payload("new".to_owned(), Some(data));
        let object = payload.as_object().expect("should be object");
        assert_eq!(
            object.get("message"),
            Some(&Value::String("existing".to_owned()))
        );
    }

    #[test]
    fn build_notification_payload_with_object_preserves_existing_details() {
        let data = json!({
            "message": "existing",
            "details": "provided",
        });
        let payload = build_notification_payload("new".to_owned(), Some(data));
        let object = payload.as_object().expect("should be object");
        assert_eq!(
            object.get("message"),
            Some(&Value::String("existing".to_owned()))
        );
        assert_eq!(
            object.get("details"),
            Some(&Value::String("provided".to_owned()))
        );
    }

    #[test]
    fn build_notification_payload_with_nested_message_preserves_inner_message() {
        let data = json!({
            "details": {
                "message": "inner",
            }
        });
        let payload = build_notification_payload("outer".to_owned(), Some(data));
        let object = payload.as_object().expect("should be object");
        assert_eq!(
            object.get("message"),
            Some(&Value::String("outer".to_owned()))
        );
        assert_eq!(
            object.get("details"),
            Some(&json!({
                "message": "inner",
            }))
        );
    }

    #[test]
    fn build_notification_payload_with_non_object_data_wraps_as_details() {
        let data = json!(42);
        let payload = build_notification_payload("hello".to_owned(), Some(data));
        let object = payload.as_object().expect("should be object");
        assert_eq!(
            object.get("message"),
            Some(&Value::String("hello".to_owned()))
        );
        assert_eq!(object.get("details"), Some(&json!(42)));
    }

    #[test]
    fn logging_level_rank_ordering_is_monotonically_increasing() {
        let levels = [
            LoggingLevel::Debug,
            LoggingLevel::Info,
            LoggingLevel::Notice,
            LoggingLevel::Warning,
            LoggingLevel::Error,
            LoggingLevel::Critical,
            LoggingLevel::Alert,
            LoggingLevel::Emergency,
        ];
        for window in levels.windows(2) {
            assert!(
                logging_level_rank(window[0]) < logging_level_rank(window[1]),
                "{:?} should rank below {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn logging_level_rank_debug_is_zero() {
        assert_eq!(logging_level_rank(LoggingLevel::Debug), 0);
    }

    #[test]
    fn logging_level_rank_emergency_is_seven() {
        assert_eq!(logging_level_rank(LoggingLevel::Emergency), 7);
    }

    #[test]
    fn log_to_server_does_not_panic_for_all_levels() {
        let levels = [
            LoggingLevel::Debug,
            LoggingLevel::Info,
            LoggingLevel::Notice,
            LoggingLevel::Warning,
            LoggingLevel::Error,
            LoggingLevel::Critical,
            LoggingLevel::Alert,
            LoggingLevel::Emergency,
        ];
        for level in &levels {
            log_to_server(*level, "test message", None, 4096);
            log_to_server(*level, "test message", Some(&json!({"key": "value"})), 4096);
        }
    }

    #[test]
    fn log_to_server_emits_expected_level_and_fields() {
        let cases = [
            (LoggingLevel::Debug, "DEBUG"),
            (LoggingLevel::Info, " INFO"),
            (LoggingLevel::Notice, " INFO"),
            (LoggingLevel::Warning, " WARN"),
            (LoggingLevel::Error, "ERROR"),
        ];
        for (level, level_token) in cases {
            let logs = capture_logs(|| {
                log_to_server(level, "hello", Some(&json!({"key": "value"})), 4096);
            });
            assert!(logs.contains(level_token), "missing level token: {logs}");
            assert!(
                logs.contains("plugin log hello"),
                "missing message text: {logs}"
            );
            assert!(
                logs.contains("message_bytes=5"),
                "missing message bytes: {logs}"
            );
            assert!(
                logs.contains("message_truncated=false"),
                "missing message truncation marker: {logs}"
            );
            assert!(logs.contains("data_bytes=15"), "missing data bytes: {logs}");
            assert!(
                logs.contains("data_truncated=false"),
                "missing data truncation marker: {logs}"
            );
        }
    }

    #[test]
    fn truncate_json_for_log_marks_truncated_payload() {
        let payload = json!({"message": "abcdefghijklmnopqrstuvwxyz"});
        let result = truncate_json_for_log(&payload, 8);
        assert!(result.truncated);
        assert!(result.original_bytes > 8);
        assert_eq!(result.value, "{\"messag...");
        assert_eq!(result.value.len(), 11);
        assert!(result.value.ends_with("..."));
    }

    #[test]
    fn min_level_round_trips_for_each_level() {
        let levels = [
            LoggingLevel::Debug,
            LoggingLevel::Info,
            LoggingLevel::Notice,
            LoggingLevel::Warning,
            LoggingLevel::Error,
            LoggingLevel::Critical,
            LoggingLevel::Alert,
            LoggingLevel::Emergency,
        ];
        let state = ClientLoggingState::new(true, LoggingLevel::Info);
        for level in levels {
            state.set_min_level(level);
            assert_eq!(state.min_level(), level);
        }
    }

    #[test]
    fn should_notify_uses_min_level_threshold() {
        let state = ClientLoggingState::new(true, LoggingLevel::Info);
        state.set_min_level(LoggingLevel::Error);
        assert!(!state.should_notify(LoggingLevel::Warning));
        assert!(state.should_notify(LoggingLevel::Error));
        assert!(state.should_notify(LoggingLevel::Emergency));
    }

    #[test]
    fn should_notify_disabled_state_never_notifies() {
        let state = ClientLoggingState::new(false, LoggingLevel::Debug);
        assert!(!state.should_notify(LoggingLevel::Debug));
        assert!(!state.should_notify(LoggingLevel::Emergency));
    }

    #[test]
    fn u8_to_logging_level_known_values_map_exactly() {
        let expected = [
            (0, LoggingLevel::Debug),
            (1, LoggingLevel::Info),
            (2, LoggingLevel::Notice),
            (3, LoggingLevel::Warning),
            (4, LoggingLevel::Error),
            (5, LoggingLevel::Critical),
            (6, LoggingLevel::Alert),
            (7, LoggingLevel::Emergency),
        ];
        for (raw, level) in expected {
            assert_eq!(u8_to_logging_level(raw), level);
        }
    }

    #[test]
    fn u8_to_logging_level_invalid_value_defaults_to_info() {
        assert_eq!(u8_to_logging_level(8), LoggingLevel::Info);
        assert_eq!(u8_to_logging_level(255), LoggingLevel::Info);
    }
}
