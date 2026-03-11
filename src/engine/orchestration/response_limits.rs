//! Response-size limit enforcement for tool results.

use crate::{
    config::ResponseLimitsConfig,
    mcp::{CallToolResult, McpError},
};

#[derive(Default)]
struct ResponseBytes {
    text: u64,
    structured: u64,
    binary: u64,
    other: u64,
    total: u64,
}

pub(super) fn enforce_response_limits(
    result: &CallToolResult,
    limits: Option<ResponseLimitsConfig>,
) -> Result<(), McpError> {
    let Some(limits) = limits else {
        return Ok(());
    };

    let bytes = measure_result_bytes(result)?;

    check_limit(bytes.text, limits.text_bytes, "text_bytes")?;
    check_limit(
        bytes.structured,
        limits.structured_bytes,
        "structured_bytes",
    )?;
    check_limit(bytes.binary, limits.binary_bytes, "binary_bytes")?;
    check_limit(bytes.other, limits.other_bytes, "other_bytes")?;
    check_limit(bytes.total, limits.total_bytes, "total_bytes")?;
    Ok(())
}

fn measure_result_bytes(result: &CallToolResult) -> Result<ResponseBytes, McpError> {
    let mut bytes = ResponseBytes {
        total: to_u64_len(
            serde_json::to_vec(result)
                .map_err(|error| {
                    McpError::internal_error(
                        format!("failed to serialize tool result for size check: {error}"),
                        None,
                    )
                })?
                .len(),
        ),
        ..ResponseBytes::default()
    };

    if let Some(structured) = result.structured_content.as_ref() {
        bytes.structured += to_u64_len(
            serde_json::to_vec(structured)
                .map_err(|error| {
                    McpError::internal_error(
                        format!("failed to serialize structured content for size check: {error}"),
                        None,
                    )
                })?
                .len(),
        );
    }

    for content in &result.content {
        let value = serde_json::to_value(content).map_err(|error| {
            McpError::internal_error(
                format!("failed to serialize content block for size check: {error}"),
                None,
            )
        })?;
        let block_bytes = to_u64_len(
            serde_json::to_vec(&value)
                .map_err(|error| {
                    McpError::internal_error(
                        format!("failed to encode content block for size check: {error}"),
                        None,
                    )
                })?
                .len(),
        );
        match classify_content_block(&value) {
            ContentClass::Text => bytes.text += block_bytes,
            ContentClass::Binary => bytes.binary += block_bytes,
            ContentClass::Other => bytes.other += block_bytes,
        }
    }

    Ok(bytes)
}

enum ContentClass {
    Text,
    Binary,
    Other,
}

fn classify_content_block(value: &serde_json::Value) -> ContentClass {
    let Some(content_type) = value.get("type").and_then(serde_json::Value::as_str) else {
        return ContentClass::Other;
    };

    match content_type {
        "text" => ContentClass::Text,
        "image" | "audio" => ContentClass::Binary,
        "resource" => {
            let Some(resource) = value.get("resource").and_then(serde_json::Value::as_object)
            else {
                return ContentClass::Other;
            };
            if resource.contains_key("text") {
                ContentClass::Text
            } else if resource.contains_key("blob") {
                ContentClass::Binary
            } else {
                ContentClass::Other
            }
        }
        _ => ContentClass::Other,
    }
}

fn check_limit(value: u64, limit: Option<u64>, label: &str) -> Result<(), McpError> {
    let Some(limit) = limit else {
        return Ok(());
    };
    if value <= limit {
        return Ok(());
    }

    Err(McpError::internal_error(
        format!("tool response exceeds configured {label} limit"),
        None,
    ))
}

fn to_u64_len(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use crate::mcp::{CallToolResult, Content, RawResource, ResourceContents};

    use super::enforce_response_limits;

    #[test]
    fn response_limits_reject_text_over_limit() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::text("abcdef")];
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: Some(5),
            structured_bytes: None,
            binary_bytes: None,
            other_bytes: None,
            total_bytes: None,
        };
        let error = enforce_response_limits(&result, Some(limits))
            .expect_err("text over limit should fail");
        assert_eq!(error.code, crate::mcp::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.contains("text_bytes"),
            "error must mention channel name"
        );
        assert!(
            error.message.contains("exceeds configured"),
            "error must mention limit exceeded"
        );
    }

    #[test]
    fn response_limits_allow_when_unset() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::text("abcdef")];
        result.structured_content = Some(serde_json::json!({"ok": true}));
        result.is_error = Some(false);
        enforce_response_limits(&result, None).expect("unset limits should pass");
    }

    // Partial-limit success: setting one channel limit must not affect unrelated channels.
    #[test]
    fn response_limits_partial_limits_pass_unrelated_channels() {
        // Large text content but only binary_bytes is limited — must pass.
        let mut result = CallToolResult::default();
        result.content = vec![Content::text(
            "a]very long text payload that exceeds any small limit",
        )];
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: None,
            binary_bytes: Some(1), // very small, but no binary content
            other_bytes: None,
            total_bytes: None,
        };
        enforce_response_limits(&result, Some(limits))
            .expect("binary limit must not affect text-only content");

        // Large structured content but only text_bytes is limited — must pass.
        let mut result2 = CallToolResult::default();
        result2.structured_content =
            Some(serde_json::json!({"large": "structured payload with lots of data inside"}));
        result2.is_error = Some(false);
        let limits2 = crate::config::ResponseLimitsConfig {
            text_bytes: Some(1), // very small, but no text content blocks
            structured_bytes: None,
            binary_bytes: None,
            other_bytes: None,
            total_bytes: None,
        };
        enforce_response_limits(&result2, Some(limits2))
            .expect("text limit must not affect structured-only content");

        // Text content with only other_bytes limited — must pass.
        let mut result3 = CallToolResult::default();
        result3.content = vec![Content::text("hello world")];
        result3.is_error = Some(false);
        let limits3 = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: None,
            binary_bytes: None,
            other_bytes: Some(1), // very small, but no "other" content
            total_bytes: None,
        };
        enforce_response_limits(&result3, Some(limits3))
            .expect("other limit must not affect text-only content");
    }

    #[test]
    fn response_limits_reject_structured_over_limit() {
        let mut result = CallToolResult::default();
        result.structured_content = Some(serde_json::json!({"value": "abcdef"}));
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: Some(8),
            binary_bytes: None,
            other_bytes: None,
            total_bytes: None,
        };
        let error = enforce_response_limits(&result, Some(limits))
            .expect_err("structured over limit should fail");
        assert_eq!(error.code, crate::mcp::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.contains("structured_bytes"),
            "error must mention channel name"
        );
        assert!(
            error.message.contains("exceeds configured"),
            "error must mention limit exceeded"
        );
    }

    #[test]
    fn response_limits_reject_binary_over_limit() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::image("aGVsbG8=", "image/png")];
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: None,
            binary_bytes: Some(10),
            other_bytes: None,
            total_bytes: None,
        };
        let error = enforce_response_limits(&result, Some(limits))
            .expect_err("binary over limit should fail");
        assert_eq!(error.code, crate::mcp::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.contains("binary_bytes"),
            "error must mention channel name"
        );
        assert!(
            error.message.contains("exceeds configured"),
            "error must mention limit exceeded"
        );
    }

    #[test]
    fn response_limits_reject_other_over_limit() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::resource_link(RawResource::new(
            "file:///tmp/demo.txt",
            "demo",
        ))];
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: None,
            binary_bytes: None,
            other_bytes: Some(10),
            total_bytes: None,
        };
        let error = enforce_response_limits(&result, Some(limits))
            .expect_err("other over limit should fail");
        assert_eq!(error.code, crate::mcp::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.contains("other_bytes"),
            "error must mention channel name"
        );
        assert!(
            error.message.contains("exceeds configured"),
            "error must mention limit exceeded"
        );
    }

    // Boundary trio checks: below/at/above for text_bytes, structured_bytes, and total_bytes.
    #[test]
    fn response_limits_text_boundary_trio() {
        let text_content = "abcdef"; // 6 chars
        let mut result = CallToolResult::default();
        result.content = vec![Content::text(text_content)];
        result.is_error = Some(false);

        // Measure actual text block bytes for precise boundary testing.
        let text_block_bytes =
            serde_json::to_vec(&serde_json::to_value(&result.content[0]).unwrap())
                .unwrap()
                .len() as u64;

        // Below limit: must pass.
        let limits_below = crate::config::ResponseLimitsConfig {
            text_bytes: Some(text_block_bytes + 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_below)).expect("below text limit must pass");

        // At limit: must pass (check_limit uses <=).
        let limits_at = crate::config::ResponseLimitsConfig {
            text_bytes: Some(text_block_bytes),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_at)).expect("at text limit must pass");

        // Above limit: must fail.
        let limits_above = crate::config::ResponseLimitsConfig {
            text_bytes: Some(text_block_bytes - 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_above))
            .expect_err("above text limit must fail");
    }

    #[test]
    fn response_limits_structured_boundary_trio() {
        let mut result = CallToolResult::default();
        result.structured_content = Some(serde_json::json!({"ok": true}));
        result.is_error = Some(false);

        let structured_bytes = serde_json::to_vec(result.structured_content.as_ref().unwrap())
            .unwrap()
            .len() as u64;

        let limits_below = crate::config::ResponseLimitsConfig {
            structured_bytes: Some(structured_bytes + 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_below))
            .expect("below structured limit must pass");

        let limits_at = crate::config::ResponseLimitsConfig {
            structured_bytes: Some(structured_bytes),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_at)).expect("at structured limit must pass");

        let limits_above = crate::config::ResponseLimitsConfig {
            structured_bytes: Some(structured_bytes - 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_above))
            .expect_err("above structured limit must fail");
    }

    #[test]
    fn response_limits_total_boundary_trio() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::text("data")];
        result.is_error = Some(false);

        let total_bytes = serde_json::to_vec(&result).unwrap().len() as u64;

        let limits_below = crate::config::ResponseLimitsConfig {
            total_bytes: Some(total_bytes + 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_below)).expect("below total limit must pass");

        let limits_at = crate::config::ResponseLimitsConfig {
            total_bytes: Some(total_bytes),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_at)).expect("at total limit must pass");

        let limits_above = crate::config::ResponseLimitsConfig {
            total_bytes: Some(total_bytes - 1),
            ..Default::default()
        };
        enforce_response_limits(&result, Some(limits_above))
            .expect_err("above total limit must fail");
    }

    #[test]
    fn response_limits_reject_total_over_limit() {
        let mut result = CallToolResult::default();
        result.content = vec![Content::resource(ResourceContents::text(
            "large payload",
            "file:///tmp/a.txt",
        ))];
        result.structured_content = Some(serde_json::json!({"ok": true}));
        result.is_error = Some(false);
        let limits = crate::config::ResponseLimitsConfig {
            text_bytes: None,
            structured_bytes: None,
            binary_bytes: None,
            other_bytes: None,
            total_bytes: Some(10),
        };
        let error = enforce_response_limits(&result, Some(limits))
            .expect_err("total over limit should fail");
        assert_eq!(error.code, crate::mcp::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.contains("total_bytes"),
            "error must mention channel name"
        );
        assert!(
            error.message.contains("exceeds configured"),
            "error must mention limit exceeded"
        );
    }
}
