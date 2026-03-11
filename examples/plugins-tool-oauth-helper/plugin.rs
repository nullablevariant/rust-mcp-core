use async_trait::async_trait;
use rust_mcp_core::mcp::{CallToolResult, McpError};
use rust_mcp_core::{OutboundHttpRequest, PluginCallParams, ToolPlugin};
use serde_json::json;
use serde_json::Value;

pub(crate) struct OauthHeaderPlugin;

#[async_trait]
impl ToolPlugin for OauthHeaderPlugin {
    fn name(&self) -> &'static str {
        "plugin.oauth_header"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let upstream_name = parse_optional_string_arg(&args, "upstream")?
            .unwrap_or_else(|| "partner_api".to_owned());
        let force_refresh = parse_optional_bool_arg(&args, "force_refresh")?.unwrap_or(false);

        let token = params
            .ctx
            .upstream_access_token(&upstream_name, force_refresh)
            .await?;
        let (header_name, header_value) = params
            .ctx
            .upstream_bearer_header(&upstream_name, force_refresh)
            .await?;
        let upstream_response = params
            .ctx
            .send(
                &upstream_name,
                OutboundHttpRequest {
                    method: "GET".to_owned(),
                    url: "/partners".to_owned(),
                    query: vec![("limit".to_owned(), "2".to_owned())],
                    ..OutboundHttpRequest::default()
                },
            )
            .await?;
        let status = upstream_response.status();
        let upstream_json: Value = upstream_response.json()?;
        let partner_count = upstream_json
            .get("partners")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len);

        Ok(CallToolResult::structured(json!({
            "plugin": self.name(),
            "upstream": upstream_name,
            "force_refresh": force_refresh,
            "token_length": token.as_str().len(),
            "header_name": header_name,
            "header_value": header_value,
            "upstream_status": status,
            "partner_count": partner_count
        })))
    }
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

fn parse_optional_bool_arg(args: &Value, key: &str) -> Result<Option<bool>, McpError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let Some(as_bool) = value.as_bool() else {
        return Err(McpError::invalid_params(
            format!("'{key}' must be a boolean"),
            None,
        ));
    };
    Ok(Some(as_bool))
}
