use super::*;
use crate::agent::control::AgentMessageRead;
use crate::tools::handlers::multi_agents_spec::create_read_agent_messages_tool;
use codex_tools::ToolSpec;

const DEFAULT_MESSAGE_READ_MAX_BYTES: usize = 4096;
const MAX_MESSAGE_READ_MAX_BYTES: usize = 6000;
const MAX_MESSAGE_REFS: usize = 8;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("read_agent_messages")
    }

    fn spec(&self) -> ToolSpec {
        create_read_agent_messages_tool()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
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
            let args: ReadAgentMessagesArgs = parse_arguments(&arguments)?;
            if args.message_refs.is_empty() || args.message_refs.len() > MAX_MESSAGE_REFS {
                return Err(FunctionCallError::RespondToModel(format!(
                    "message_refs must contain between 1 and {MAX_MESSAGE_REFS} entries"
                )));
            }
            let max_bytes = args
                .max_bytes
                .unwrap_or(DEFAULT_MESSAGE_READ_MAX_BYTES)
                .clamp(1, MAX_MESSAGE_READ_MAX_BYTES);
            let mut messages = Vec::with_capacity(args.message_refs.len());
            for message_ref in args.message_refs {
                let message = session
                    .services
                    .agent_control
                    .read_agent_message(&turn.session_source, &message_ref, args.offset, max_bytes)
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                if message.state == crate::agent::control::AgentMessageReadState::Available
                    && message.next_offset.is_none()
                {
                    session
                        .services
                        .agent_control
                        .acknowledge_agent_attention(
                            session.thread_id,
                            &turn.session_source,
                            &turn.sub_id,
                            message_ref,
                        )
                        .await
                        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                }
                messages.push(message);
            }
            Ok(boxed_tool_output(ReadAgentMessagesResult { messages }))
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
struct ReadAgentMessagesArgs {
    message_refs: Vec<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ReadAgentMessagesResult {
    messages: Vec<AgentMessageRead>,
}

impl ToolOutput for ReadAgentMessagesResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "read_agent_messages")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "read_agent_messages")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "read_agent_messages")
    }
}
