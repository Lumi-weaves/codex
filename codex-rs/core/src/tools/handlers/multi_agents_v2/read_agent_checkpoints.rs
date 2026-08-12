use super::*;
use crate::agent::control::AgentCheckpointRead;
use crate::tools::handlers::multi_agents_spec::create_read_agent_checkpoints_tool;
use codex_tools::ToolSpec;

const DEFAULT_CHECKPOINT_READ_MAX_BYTES: usize = 4096;
const MAX_CHECKPOINT_READ_MAX_BYTES: usize = 6000;
const MAX_CHECKPOINT_REFS: usize = 8;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("read_agent_checkpoints")
    }

    fn spec(&self) -> ToolSpec {
        create_read_agent_checkpoints_tool()
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
            let args: ReadAgentCheckpointsArgs = parse_arguments(&arguments)?;
            if args.checkpoint_refs.is_empty() || args.checkpoint_refs.len() > MAX_CHECKPOINT_REFS {
                return Err(FunctionCallError::RespondToModel(format!(
                    "checkpoint_refs must contain between 1 and {MAX_CHECKPOINT_REFS} entries"
                )));
            }
            let max_bytes = args
                .max_bytes
                .unwrap_or(DEFAULT_CHECKPOINT_READ_MAX_BYTES)
                .clamp(1, MAX_CHECKPOINT_READ_MAX_BYTES);
            let mut checkpoints = Vec::with_capacity(args.checkpoint_refs.len());
            for checkpoint_ref in args.checkpoint_refs {
                let checkpoint = session
                    .services
                    .agent_control
                    .read_agent_checkpoint(
                        &turn.session_source,
                        &checkpoint_ref,
                        args.offset,
                        max_bytes,
                    )
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                checkpoints.push(checkpoint);
            }
            Ok(boxed_tool_output(ReadAgentCheckpointsResult {
                checkpoints,
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
struct ReadAgentCheckpointsArgs {
    checkpoint_refs: Vec<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ReadAgentCheckpointsResult {
    checkpoints: Vec<AgentCheckpointRead>,
}

impl ToolOutput for ReadAgentCheckpointsResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "read_agent_checkpoints")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "read_agent_checkpoints")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "read_agent_checkpoints")
    }
}
