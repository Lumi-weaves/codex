use codex_protocol::AgentPath;
use serde_json::json;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InterAgentCompletionMessage {
    task_name: AgentPath,
    sender: AgentPath,
    body: CompletionBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionBody {
    Inline(String),
    Checkpoint {
        checkpoint_ref: String,
        status: &'static str,
        approximate_bytes: usize,
    },
}

impl InterAgentCompletionMessage {
    pub(crate) fn new(task_name: AgentPath, sender: AgentPath, payload: impl Into<String>) -> Self {
        Self {
            task_name,
            sender,
            body: CompletionBody::Inline(payload.into()),
        }
    }

    pub(crate) fn checkpoint(
        task_name: AgentPath,
        sender: AgentPath,
        checkpoint_ref: String,
        status: &'static str,
        approximate_bytes: usize,
    ) -> Self {
        Self {
            task_name,
            sender,
            body: CompletionBody::Checkpoint {
                checkpoint_ref,
                status,
                approximate_bytes,
            },
        }
    }
}

impl ContextualUserFragment for InterAgentCompletionMessage {
    fn role(&self) -> &'static str {
        "assistant"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        match &self.body {
            CompletionBody::Inline(payload) => format!(
                "Message Type: FINAL_ANSWER\nTask name: {}\nSender: {}\nPayload:\n{}",
                self.task_name, self.sender, payload,
            ),
            CompletionBody::Checkpoint {
                checkpoint_ref,
                status,
                approximate_bytes,
            } => {
                let event = json!({
                    "kind": "completion",
                    "checkpoint_ref": checkpoint_ref,
                    "status": status,
                    "approximate_bytes": approximate_bytes,
                    "read_hint": "Call read_agent_checkpoints with one or more checkpoint refs to inspect selected payloads.",
                });
                format!(
                    "Message Type: AGENT_ATTENTION\nTask name: {}\nSender: {}\nEvent:\n{}",
                    self.task_name, self.sender, event,
                )
            }
        }
    }
}
