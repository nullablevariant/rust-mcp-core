use async_trait::async_trait;
use rust_mcp_core::mcp::{
    CallToolResult, CreateElicitationRequestParams, CreateMessageRequestParams, ElicitationSchema,
    McpError, Role, SamplingMessage, SamplingMessageContent,
};
use rust_mcp_core::{PluginCallParams, ToolPlugin};
use serde_json::{json, Value};

pub(crate) struct ClientFeaturesAdvancedPlugin;

#[async_trait]
impl ToolPlugin for ClientFeaturesAdvancedPlugin {
    fn name(&self) -> &'static str {
        "client.features.advanced"
    }

    async fn call(
        &self,
        args: Value,
        params: PluginCallParams,
    ) -> Result<CallToolResult, McpError> {
        let ctx = params.ctx;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("roots");

        match mode {
            "roots" => {
                let roots = ctx.request_roots().await?;
                Ok(CallToolResult::structured(
                    json!({"roots": roots.roots.len()}),
                ))
            }
            "sampling" => {
                let result = ctx
                    .request_sampling(CreateMessageRequestParams::new(
                        vec![SamplingMessage::new(
                            Role::User,
                            SamplingMessageContent::text("Summarize this message"),
                        )],
                        128,
                    ))
                    .await?;
                Ok(CallToolResult::structured(json!({
                    "sampling_model": result.model,
                    "sampling_role": format!("{:?}", result.message.role)
                })))
            }
            "elicitation" => {
                let result = ctx
                    .request_elicitation(CreateElicitationRequestParams::FormElicitationParams {
                        meta: None,
                        message: "Please provide your name".to_owned(),
                        requested_schema: ElicitationSchema::builder()
                            .required_string("name")
                            .build()
                            .map_err(|err| {
                                McpError::internal_error(
                                    format!("failed to build schema: {err}"),
                                    None,
                                )
                            })?,
                    })
                    .await?;
                Ok(CallToolResult::structured(json!({
                    "elicitation_action": format!("{:?}", result.action)
                })))
            }
            _ => Err(McpError::invalid_params(
                "mode must be one of: roots, sampling, elicitation".to_owned(),
                None,
            )),
        }
    }
}
