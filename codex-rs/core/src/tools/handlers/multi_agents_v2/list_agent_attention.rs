use super::*;
use crate::agent::control::AgentAttentionItem;
use crate::tools::handlers::multi_agents_spec::create_list_agent_attention_tool;
use codex_tools::ToolSpec;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_agent_attention")
    }

    fn spec(&self) -> ToolSpec {
        create_list_agent_attention_tool()
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move {
            let ToolInvocation {
                session, payload, ..
            } = invocation;
            let arguments = function_arguments(payload)?;
            let args: ListAgentAttentionArgs = parse_arguments(&arguments)?;
            let items = session
                .services
                .agent_control
                .list_agent_attention(session.thread_id, args.include_read)
                .await
                .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
            Ok(boxed_tool_output(ListAgentAttentionResult { items }))
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
struct ListAgentAttentionArgs {
    #[serde(default)]
    include_read: bool,
}

#[derive(Debug, Serialize)]
struct ListAgentAttentionResult {
    items: Vec<AgentAttentionItem>,
}

impl ToolOutput for ListAgentAttentionResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "list_agent_attention")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "list_agent_attention")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "list_agent_attention")
    }
}
