//! Cursor-based pagination for list endpoints.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rmcp::ErrorData as McpError;

#[cfg(feature = "http_tools")]
use crate::config::ContentFallback;

#[cfg(feature = "http_tools")]
pub(super) const fn content_fallback_str(value: &ContentFallback) -> &str {
    match value {
        ContentFallback::Text => "text",
        ContentFallback::JsonText => "json_text",
    }
}

// Slices a list by cursor + page_size. Cursors are versioned base64-encoded
// offsets ("v1:<n>") so the format can evolve without breaking clients.
pub(super) fn paginate_items<T: Clone>(
    items: &[T],
    cursor: Option<String>,
    page_size: usize,
) -> Result<(Vec<T>, Option<String>), McpError> {
    let start = if let Some(cursor) = cursor {
        decode_pagination_cursor(&cursor)?
    } else {
        0
    };

    if start > items.len() {
        return Err(McpError::invalid_params("invalid cursor".to_owned(), None));
    }

    let end = std::cmp::min(start.saturating_add(page_size), items.len());
    let next_cursor = if end < items.len() {
        Some(encode_pagination_cursor(end))
    } else {
        None
    };

    Ok((items[start..end].to_vec(), next_cursor))
}

pub(super) fn encode_pagination_cursor(offset: usize) -> String {
    STANDARD.encode(format!("v1:{offset}"))
}

pub(super) fn decode_pagination_cursor(cursor: &str) -> Result<usize, McpError> {
    let decoded = STANDARD
        .decode(cursor)
        .map_err(|_| McpError::invalid_params("invalid cursor".to_owned(), None))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| McpError::invalid_params("invalid cursor".to_owned(), None))?;
    let Some(offset) = decoded.strip_prefix("v1:") else {
        return Err(McpError::invalid_params("invalid cursor".to_owned(), None));
    };
    offset
        .parse::<usize>()
        .map_err(|_| McpError::invalid_params("invalid cursor".to_owned(), None))
}
