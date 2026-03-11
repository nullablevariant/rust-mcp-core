//! Shared utility helpers.

// Normalize an MCP endpoint path to a canonical form.
//
// Ensures the path starts with `/`, strips trailing slashes, and defaults
// empty input to `"/mcp"`.
//
// # Arguments
//
// * `path` — raw endpoint path string from config
//
// # Returns
//
// A normalized path string (e.g., `"/mcp"`, `"/custom/path"`).
#[doc(hidden)]
pub fn normalize_endpoint_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/mcp".to_owned()
    } else if trimmed == "/" {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.trim_end_matches('/').to_owned()
    } else {
        format!("/{}", trimmed.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_endpoint_path;

    #[test]
    fn normalize_endpoint_path_cases() {
        assert_eq!(normalize_endpoint_path(""), "/mcp");
        assert_eq!(normalize_endpoint_path("/"), "/");
        assert_eq!(normalize_endpoint_path("mcp/"), "/mcp");
        assert_eq!(normalize_endpoint_path("/mcp/"), "/mcp");
        assert_eq!(normalize_endpoint_path(" /foo/bar/ "), "/foo/bar");
        assert_eq!(normalize_endpoint_path("/mcp"), "/mcp");
        assert_eq!(normalize_endpoint_path("mcp"), "/mcp");
        assert_eq!(normalize_endpoint_path("  /mcp  "), "/mcp");
    }

    #[test]
    fn normalize_endpoint_path_is_idempotent() {
        let cases = ["", "/", "mcp/", "/mcp/", " /foo/bar/ ", "/custom/path"];
        for input in cases {
            let once = normalize_endpoint_path(input);
            let twice = normalize_endpoint_path(&once);
            assert_eq!(twice, once);
        }
    }
}
