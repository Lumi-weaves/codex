//! Implements the MultiAgentV2 collaboration tool surface.

use crate::agent::AgentStatus;
use crate::agent::agent_resolver::resolve_agent_target;
use crate::context::ContextualUserFragment;
use crate::context::InterAgentMessage;
use crate::context::InterAgentMessageType;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::multi_agents_common::*;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_protocol::AgentPath;
use codex_protocol::items::CollabAgentTool;
use codex_protocol::items::CollabAgentToolCallItem;
use codex_protocol::items::CollabAgentToolCallStatus;
use codex_protocol::items::SubAgentActivityItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::InterAgentAttention;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::SubAgentActivityKind;
use codex_tools::ToolName;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;

pub(crate) use followup_task::Handler as FollowupTaskHandler;
pub(crate) use interrupt_agent::Handler as InterruptAgentHandler;
pub(crate) use list_agent_attention::Handler as ListAgentAttentionHandler;
pub(crate) use list_agents::Handler as ListAgentsHandler;
pub(crate) use read_agent_checkpoints::Handler as ReadAgentCheckpointsHandler;
pub(crate) use read_agent_messages::Handler as ReadAgentMessagesHandler;
pub(crate) use send_message::Handler as SendMessageHandler;
pub(crate) use spawn::Handler as SpawnAgentHandler;
pub(crate) use wait::Handler as WaitAgentHandler;

mod followup_task;
mod interrupt_agent;
mod list_agent_attention;
mod list_agents;
mod message_tool;
mod read_agent_checkpoints;
mod read_agent_messages;
mod send_message;
mod spawn;
pub(crate) mod wait;

pub(crate) async fn emit_sub_agent_activity(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    item: SubAgentActivityItem,
) {
    let item = TurnItem::SubAgentActivity(item);
    session.emit_turn_item_started(turn, &item).await;
    session.emit_turn_item_completed(turn, item).await;
}

fn communication_from_tool_message(
    author: AgentPath,
    recipient: AgentPath,
    message: String,
    source: &crate::tools::context::ToolCallSource,
    trigger_turn: bool,
) -> InterAgentCommunication {
    if !matches!(
        source,
        crate::tools::context::ToolCallSource::DirectPlaintextMessage
    ) {
        return InterAgentCommunication::new_encrypted(
            author,
            recipient,
            Vec::new(),
            message,
            trigger_turn,
        );
    }
    let message_type = if trigger_turn {
        InterAgentMessageType::NewTask
    } else {
        InterAgentMessageType::Message
    };
    let content =
        InterAgentMessage::new(message_type, recipient.clone(), author.clone(), message).render();
    InterAgentCommunication::new(author, recipient, Vec::new(), content, trigger_turn)
}

fn communication_from_message_ref(
    author: AgentPath,
    recipient: AgentPath,
    message_ref: &str,
    approximate_bytes: usize,
) -> InterAgentCommunication {
    let event = serde_json::json!({
        "kind": "message",
        "message_ref": message_ref,
        "approximate_bytes": approximate_bytes,
        "read_hint": "Call read_agent_messages with one or more message refs to inspect selected payloads.",
    });
    let content = format!(
        "Message Type: AGENT_ATTENTION\nTask name: {recipient}\nSender: {author}\nEvent:\n{event}"
    );
    let mut communication =
        InterAgentCommunication::new(author, recipient, Vec::new(), content, false);
    communication.attention = Some(Box::new(InterAgentAttention {
        reference: message_ref.to_string(),
        acknowledged: false,
    }));
    communication
}
