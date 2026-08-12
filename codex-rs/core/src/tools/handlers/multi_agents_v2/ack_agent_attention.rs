use super::*;
use crate::tools::handlers::multi_agents_spec::create_ack_agent_attention_tool;
use codex_tools::ToolSpec;

const MAX_ATTENTION_REFS: usize = 8;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("ack_agent_attention")
    }

    fn spec(&self) -> ToolSpec {
        create_ack_agent_attention_tool()
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                payload,
                ..
            } = invocation;
            let arguments = function_arguments(payload)?;
            let args: AckAgentAttentionArgs = parse_arguments(&arguments)?;
            if args.attention_refs.is_empty() || args.attention_refs.len() > MAX_ATTENTION_REFS {
                return Err(FunctionCallError::RespondToModel(format!(
                    "attention_refs must contain between 1 and {MAX_ATTENTION_REFS} entries"
                )));
            }
            for attention_ref in &args.attention_refs {
                session
                    .services
                    .agent_control
                    .acknowledge_agent_attention(
                        session.thread_id,
                        &turn.session_source,
                        &turn.sub_id,
                        attention_ref.clone(),
                    )
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
            }
            Ok(boxed_tool_output(AckAgentAttentionResult {
                acknowledged_refs: args.attention_refs,
            }))
        })
    }
}

impl CoreToolRuntime for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AckAgentAttentionArgs {
    attention_refs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AckAgentAttentionResult {
    acknowledged_refs: Vec<String>,
}

impl ToolOutput for AckAgentAttentionResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "ack_agent_attention")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "ack_agent_attention")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "ack_agent_attention")
    }
}
