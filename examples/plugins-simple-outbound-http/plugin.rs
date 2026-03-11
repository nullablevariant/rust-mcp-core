use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{OutboundHttpRequest, PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct SimpleOutboundHttpPlugin;

#[async_trait]
impl ToolPlugin for SimpleOutboundHttpPlugin {
    fn name(&self) -> &'static str {
        "plugin.simple_outbound_http"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let limit = parse_optional_u64_arg(&args, "limit")?;
        let search = parse_optional_string_arg(&args, "search")?;

        let mut query = Vec::new();
        if let Some(limit) = limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        if let Some(search) = search.clone() {
            query.push(("search".to_owned(), search));
        }

        let response = params
            .ctx
            .send(
                "partner_api",
                OutboundHttpRequest {
                    method: "GET".to_owned(),
                    url: "/partners".to_owned(),
                    query,
                    ..OutboundHttpRequest::default()
                },
            )
            .await?;

        let status = response.status();
        let payload: Value = response.json()?;
        let partner_count = payload
            .get("partners")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len);

        Ok(CallToolResult::structured(json!({
            "plugin": self.name(),
            "status": status,
            "partner_count": partner_count,
            "upstream_response": payload
        })))
    }
}

fn parse_optional_u64_arg(args: &Value, key: &str) -> Result<Option<u64>, McpError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let Some(as_u64) = value.as_u64() else {
        return Err(McpError::invalid_params(
            format!("'{key}' must be a positive integer"),
            None,
        ));
    };
    Ok(Some(as_u64))
}

fn parse_optional_string_arg(args: &Value, key: &str) -> Result<Option<String>, McpError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let Some(as_str) = value.as_str() else {
        return Err(McpError::invalid_params(
            format!("'{key}' must be a string"),
            None,
        ));
    };
    Ok(Some(as_str.to_owned()))
}
