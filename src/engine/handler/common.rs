//! Shared handler helpers for pagination and config access.
pub(super) fn global_page_size(config: &crate::config::McpConfig) -> Option<usize> {
    config
        .pagination
        .as_ref()
        .map(|pagination| pagination.page_size)
        .filter(|page_size| *page_size > 0)
        .and_then(|page_size| usize::try_from(page_size).ok())
}
