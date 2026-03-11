//! Helpers for truncating log payloads to prevent amplification.

use serde_json::Value;

pub(crate) struct TruncatedField {
    pub(crate) value: String,
    pub(crate) original_bytes: usize,
    pub(crate) truncated: bool,
}

pub(crate) fn truncate_string_for_log(input: &str, max_bytes: u64) -> TruncatedField {
    if max_bytes == 0 {
        return TruncatedField {
            value: input.to_owned(),
            original_bytes: input.len(),
            truncated: false,
        };
    }

    let max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    if input.len() <= max_bytes {
        return TruncatedField {
            value: input.to_owned(),
            original_bytes: input.len(),
            truncated: false,
        };
    }

    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = input[..end].to_owned();
    truncated.push_str("...");
    TruncatedField {
        value: truncated,
        original_bytes: input.len(),
        truncated: true,
    }
}

pub(crate) fn truncate_json_for_log(value: &Value, max_bytes: u64) -> TruncatedField {
    let serialized = serde_json::to_string(value)
        .unwrap_or_else(|error| format!("{{\"_log_serialize_error\":\"{error}\"}}"));
    truncate_string_for_log(&serialized, max_bytes)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{truncate_json_for_log, truncate_string_for_log};

    #[test]
    fn truncate_string_respects_zero_disable() {
        let result = truncate_string_for_log("abcdef", 0);
        assert_eq!(result.value, "abcdef");
        assert_eq!(result.original_bytes, 6);
        assert!(!result.truncated);
    }

    #[test]
    fn truncate_string_truncates_with_ascii_suffix() {
        let result = truncate_string_for_log("abcdef", 4);
        assert_eq!(result.value, "abcd...");
        assert_eq!(result.original_bytes, 6);
        assert!(result.truncated);
    }

    #[test]
    fn truncate_string_truncates_at_utf8_boundaries() {
        let two_byte = truncate_string_for_log("éé", 3);
        assert_eq!(two_byte.value, "é...");
        assert_eq!(two_byte.original_bytes, "éé".len());
        assert!(two_byte.truncated);

        let four_byte = truncate_string_for_log("🦀🦀", 5);
        assert_eq!(four_byte.value, "🦀...");
        assert_eq!(four_byte.original_bytes, "🦀🦀".len());
        assert!(four_byte.truncated);
    }

    #[test]
    fn truncate_json_truncates_large_payloads() {
        let value = json!({
            "message": "very long payload",
            "nested": { "value": "more data" }
        });
        let result = truncate_json_for_log(&value, 10);
        assert!(result.truncated);
        assert!(result.original_bytes > 10);
        assert_eq!(result.value, "{\"message\"...");
        assert_eq!(result.value.len(), 13);
        assert!(result.value.ends_with("..."));
    }
}
