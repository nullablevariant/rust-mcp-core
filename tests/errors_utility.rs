use rmcp::model::ErrorCode;
use rust_mcp_core::errors::{cancelled_error, cancelled_tool_result, tool_execution_error_result};

#[test]
fn cancelled_error_has_correct_code() {
    let error = cancelled_error();
    assert_eq!(error.code, ErrorCode(-32000));
}

#[test]
fn cancelled_error_has_correct_message() {
    let error = cancelled_error();
    assert_eq!(error.message, "request cancelled");
}

#[test]
fn cancelled_error_has_no_data() {
    let error = cancelled_error();
    assert!(error.data.is_none());
}

#[test]
fn cancelled_error_full_contract() {
    let error = cancelled_error();
    assert_eq!(error.code, ErrorCode(-32000));
    assert_eq!(error.message, "request cancelled");
    assert!(error.data.is_none());
}

#[test]
fn cancelled_tool_result_is_error() {
    let result = cancelled_tool_result();
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn cancelled_tool_result_contains_message() {
    let result = cancelled_tool_result();
    let text = result
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|t| t.text.as_str())
        .expect("content should have text");
    assert_eq!(text, "request cancelled");
}

#[test]
fn cancelled_tool_result_full_contract() {
    let result = cancelled_tool_result();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.content.len(), 1);
    let text = result
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|t| t.text.as_str())
        .expect("content should have text");
    assert_eq!(text, "request cancelled");
}

#[test]
fn tool_execution_error_result_is_error() {
    let result = tool_execution_error_result("something broke");
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn tool_execution_error_result_contains_provided_message() {
    let result = tool_execution_error_result("custom error message");
    let text = result
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|t| t.text.as_str())
        .expect("content should have text");
    assert_eq!(text, "custom error message");
}

#[test]
fn tool_execution_error_result_accepts_string() {
    let message = String::from("owned message");
    let result = tool_execution_error_result(message);
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn tool_execution_error_result_full_contract() {
    let result = tool_execution_error_result("full contract check");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.content.len(), 1);
    let text = result
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|t| t.text.as_str())
        .expect("content should have text");
    assert_eq!(text, "full contract check");
}
