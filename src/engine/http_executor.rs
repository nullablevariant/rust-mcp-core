//! HTTP tool execution: template rendering, request building, and response mapping.
use std::fmt;

use rmcp::ErrorData as McpError;
use serde_json::Value;

use crate::http::client::{HttpClient, OutboundHttpRequest};

use super::templating::{render_value, value_to_string, RenderContext};

#[derive(Debug)]
pub(crate) enum HttpExecutionError {
    Template(McpError),
    Request(McpError),
    Response(McpError),
    UpstreamStatus { status: u16 },
    Cancelled,
}

impl HttpExecutionError {
    pub(crate) const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UpstreamStatus { status } => Some(*status),
            Self::Template(_) | Self::Request(_) | Self::Response(_) | Self::Cancelled => None,
        }
    }

    pub(crate) fn into_mcp_error(self) -> McpError {
        match self {
            Self::Template(error) | Self::Request(error) | Self::Response(error) => error,
            Self::UpstreamStatus { status } => {
                McpError::internal_error(format!("upstream returned {status}"), None)
            }
            Self::Cancelled => {
                McpError::internal_error(crate::errors::CANCELLED_ERROR_MESSAGE.to_owned(), None)
            }
        }
    }
}

impl fmt::Display for HttpExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Template(error) | Self::Request(error) | Self::Response(error) => {
                write!(f, "{error}")
            }
            Self::UpstreamStatus { status } => write!(f, "upstream returned {status}"),
            Self::Cancelled => write!(f, "request cancelled"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpRequestTemplate {
    pub method: String,
    pub path: String,
    pub query: Option<Value>,
    pub headers: Option<Value>,
    pub body: Option<Value>,
    pub timeout_ms: Option<u64>,
    pub max_response_bytes: Option<u64>,
}

// Renders template placeholders in path/query/headers/body using tool args,
// builds an OutboundHttpRequest, sends it via the injected HTTP client,
// and returns the response body as JSON. Null query/header values are
// skipped so optional params don't produce empty strings.
pub async fn execute_http(
    client: &dyn HttpClient,
    base_url: &str,
    template: &HttpRequestTemplate,
    args: &Value,
) -> Result<Value, McpError> {
    execute_http_detailed(client, base_url, template, args)
        .await
        .map_err(HttpExecutionError::into_mcp_error)
}

pub(crate) async fn execute_http_detailed(
    client: &dyn HttpClient,
    base_url: &str,
    template: &HttpRequestTemplate,
    args: &Value,
) -> Result<Value, HttpExecutionError> {
    let ctx = RenderContext::new(args, None);
    let path_value = render_value(&Value::String(template.path.clone()), &ctx)
        .map_err(HttpExecutionError::Template)?;
    let path_str = path_value.as_str().ok_or_else(|| {
        HttpExecutionError::Template(McpError::invalid_request(
            "path must render to a string".to_owned(),
            None,
        ))
    })?;

    let base = base_url.trim_end_matches('/');
    let path = if path_str.starts_with('/') {
        path_str.to_owned()
    } else {
        format!("/{path_str}")
    };
    let url = format!("{base}{path}");

    let mut request = OutboundHttpRequest {
        method: template.method.clone(),
        url,
        ..OutboundHttpRequest::default()
    };

    if let Some(timeout_ms) = template.timeout_ms {
        request.timeout_ms = Some(timeout_ms);
    }
    if let Some(max_response_bytes) = template.max_response_bytes {
        request.max_response_bytes = Some(max_response_bytes);
    }

    if let Some(query_template) = template.query.as_ref() {
        let rendered = render_value(query_template, &ctx).map_err(HttpExecutionError::Template)?;
        if let Value::Object(map) = rendered {
            for (key, value) in map {
                if value.is_null() {
                    continue;
                }
                request.query.push((key, value_to_string(&value)));
            }
        }
    }

    if let Some(header_template) = template.headers.as_ref() {
        let rendered = render_value(header_template, &ctx).map_err(HttpExecutionError::Template)?;
        if let Value::Object(map) = rendered {
            for (key, value) in map {
                if value.is_null() {
                    continue;
                }
                request.headers.push((key, value_to_string(&value)));
            }
        }
    }

    if let Some(body_template) = template.body.as_ref() {
        let rendered = render_value(body_template, &ctx).map_err(HttpExecutionError::Template)?;
        request.json_body = Some(rendered);
    }

    let response = client
        .send(request)
        .await
        .map_err(HttpExecutionError::Request)?;

    if !response.is_success() {
        return Err(HttpExecutionError::UpstreamStatus {
            status: response.status(),
        });
    }

    response
        .json::<Value>()
        .map_err(HttpExecutionError::Response)
}

#[cfg(test)]
// Inline tests cover private HttpExecutionError helpers and internal mapping.
mod tests {
    use super::HttpExecutionError;
    use rmcp::{model::ErrorCode, ErrorData as McpError};

    #[test]
    fn http_execution_error_display_formats_all_variants() {
        let upstream = HttpExecutionError::UpstreamStatus { status: 503 };
        let cancelled = HttpExecutionError::Cancelled;
        let template =
            HttpExecutionError::Template(McpError::invalid_request("tpl".to_owned(), None));
        let request =
            HttpExecutionError::Request(McpError::invalid_request("bad".to_owned(), None));
        let response =
            HttpExecutionError::Response(McpError::internal_error("decode".to_owned(), None));

        assert_eq!(upstream.to_string(), "upstream returned 503");
        assert_eq!(cancelled.to_string(), "request cancelled");
        assert_eq!(template.to_string(), "-32600: tpl");
        assert_eq!(request.to_string(), "-32600: bad");
        assert_eq!(response.to_string(), "-32603: decode");
    }

    #[test]
    fn http_execution_error_status_code_maps_only_upstream_status() {
        for status in [401_u16, 429_u16, 503_u16] {
            let upstream = HttpExecutionError::UpstreamStatus { status };
            assert_eq!(upstream.status_code(), Some(status));
        }

        let cancelled = HttpExecutionError::Cancelled;
        let request =
            HttpExecutionError::Request(McpError::invalid_request("bad".to_owned(), None));

        assert_eq!(cancelled.status_code(), None);
        assert_eq!(request.status_code(), None);
    }

    #[test]
    fn http_execution_error_into_mcp_error_maps_cancelled() {
        let error = HttpExecutionError::Cancelled.into_mcp_error();
        assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "request cancelled");
        assert!(error.data.is_none());
    }

    #[test]
    fn http_execution_error_into_mcp_error_preserves_all_variants_contract() {
        let template =
            HttpExecutionError::Template(McpError::invalid_request("tpl".to_owned(), None));
        let request = HttpExecutionError::Request(McpError::new(
            ErrorCode(-32001),
            "request-failure".to_owned(),
            Some(serde_json::json!({"kind":"request"})),
        ));
        let response = HttpExecutionError::Response(McpError::new(
            ErrorCode(-32002),
            "response-failure".to_owned(),
            Some(serde_json::json!({"kind":"response"})),
        ));
        let upstream = HttpExecutionError::UpstreamStatus { status: 502 };

        let template_error = template.into_mcp_error();
        assert_eq!(template_error.code, ErrorCode::INVALID_REQUEST);
        assert_eq!(template_error.message, "tpl");
        assert!(template_error.data.is_none());

        let request_error = request.into_mcp_error();
        assert_eq!(request_error.code, ErrorCode(-32001));
        assert_eq!(request_error.message, "request-failure");
        assert_eq!(
            request_error.data,
            Some(serde_json::json!({"kind":"request"}))
        );

        let response_error = response.into_mcp_error();
        assert_eq!(response_error.code, ErrorCode(-32002));
        assert_eq!(response_error.message, "response-failure");
        assert_eq!(
            response_error.data,
            Some(serde_json::json!({"kind":"response"}))
        );

        let upstream_error = upstream.into_mcp_error();
        assert_eq!(upstream_error.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(upstream_error.message, "upstream returned 502");
        assert!(upstream_error.data.is_none());
    }
}
