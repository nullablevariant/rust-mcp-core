#[cfg(any(feature = "auth", feature = "http_tools"))]
use rmcp::model::ErrorCode;
#[cfg(any(feature = "auth", feature = "http_tools"))]
use rust_mcp_core::{HttpClient, OutboundHttpRequest, ReqwestHttpClient};

#[cfg(any(feature = "auth", feature = "http_tools"))]
#[tokio::test]
async fn reqwest_http_client_rejects_oversized_max_response_bytes_setting() {
    let client = ReqwestHttpClient::new(reqwest::Client::new());
    let result = client
        .send(OutboundHttpRequest {
            method: "GET".to_owned(),
            url: "http://localhost/unused".to_owned(),
            max_response_bytes: Some(u64::MAX),
            ..OutboundHttpRequest::default()
        })
        .await;

    let error = result.expect_err("request should fail");
    if usize::try_from(u64::MAX).is_err() {
        // 32-bit platform: rejected at validation with INVALID_REQUEST
        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        assert!(error.message.contains("too large for this platform"));
    } else {
        // 64-bit platform: passes validation but fails on actual HTTP send
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert!(!error.message.is_empty());
        // The reqwest error message should reference connection failure
        // (since localhost/unused is not reachable in typical test environments)
        assert!(
            error.message.contains("error") || error.message.contains("connect"),
            "expected connection-related error, got: {}",
            error.message
        );
    }
}
