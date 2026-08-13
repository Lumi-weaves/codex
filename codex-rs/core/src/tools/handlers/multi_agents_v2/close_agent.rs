use super::*;
use crate::tools::handlers::multi_agents_spec::create_close_agent_tool_v2;
use codex_protocol::protocol::CollabAgentRef;
use codex_tools::ToolSpec;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("close_agent")
    }

    fn spec(&self) -> ToolSpec {
        create_close_agent_tool_v2()
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move { handle_close_agent(invocation).await.map(boxed_tool_output) })
    }
}

async fn handle_close_agent(
    invocation: ToolInvocation,
) -> Result<CloseAgentResult, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        payload,
        call_id,
        ..
    } = invocation;
    let arguments = function_arguments(payload)?;
    let args: CloseAgentArgs = parse_arguments(&arguments)?;
    let agent_id = resolve_agent_target(&session, &turn, &args.target).await?;
    let receiver_agent = session
        .services
        .agent_control
        .ensure_agent_known(agent_id)
        .map_err(|err| collab_agent_error(agent_id, err))?;
    if receiver_agent
        .agent_path
        .as_ref()
        .is_some_and(AgentPath::is_root)
    {
        return Err(FunctionCallError::RespondToModel(
            "root is not a spawned agent".to_string(),
        ));
    }
    if agent_id == session.thread_id {
        return Err(FunctionCallError::RespondToModel(
            "an agent cannot close itself; return your result and let the parent close you if needed"
                .to_string(),
        ));
    }
    let previous_status = session.services.agent_control.get_status(agent_id).await;
    let receiver_ref = CollabAgentRef {
        thread_id: agent_id,
        agent_nickname: receiver_agent.agent_nickname,
        agent_role: receiver_agent.agent_role,
    };
    session
        .emit_turn_item_started(
            &turn,
            &TurnItem::CollabAgentToolCall(CollabAgentToolCallItem {
                id: call_id.clone(),
                tool: CollabAgentTool::CloseAgent,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: session.thread_id,
                receiver_thread_ids: vec![agent_id],
                receiver_agents: vec![receiver_ref.clone()],
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: Default::default(),
            }),
        )
        .await;

    let result = Box::pin(session.services.agent_control.close_agent(agent_id)).await;
    let completion_status = if result.is_ok() {
        CollabAgentToolCallStatus::Completed
    } else {
        CollabAgentToolCallStatus::Failed
    };
    session
        .emit_turn_item_completed(
            &turn,
            TurnItem::CollabAgentToolCall(CollabAgentToolCallItem {
                id: call_id,
                tool: CollabAgentTool::CloseAgent,
                status: completion_status,
                sender_thread_id: session.thread_id,
                receiver_thread_ids: vec![agent_id],
                receiver_agents: vec![receiver_ref],
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: [(agent_id, previous_status.clone())].into_iter().collect(),
            }),
        )
        .await;
    result.map_err(|err| collab_agent_error(agent_id, err))?;

    Ok(CloseAgentResult {
        closed: true,
        previous_status,
    })
}

impl CoreToolRuntime for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseAgentArgs {
    target: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CloseAgentResult {
    pub(crate) closed: bool,
    pub(crate) previous_status: AgentStatus,
}

impl ToolOutput for CloseAgentResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "close_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "close_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "close_agent")
    }
}
