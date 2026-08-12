use codex_protocol::AgentPath;
use codex_protocol::protocol::AgentStatus;
use codex_utils_output_truncation::approx_token_count;

use super::COMPLETION_MESSAGE_MAX_TOKENS;
use super::ERROR_NEXT_ACTION;
use super::format_inter_agent_checkpoint_message;
use super::format_inter_agent_completion_message;

#[test]
fn error_completion_message_stays_below_manual_review_threshold() {
    let message = format_inter_agent_completion_message(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("valid agent path"),
        &AgentStatus::Errored("stream disconnected ".repeat(1_000)),
    )
    .expect("error status should produce a completion message");

    assert!(approx_token_count(&message) < COMPLETION_MESSAGE_MAX_TOKENS);
    assert!(message.contains(ERROR_NEXT_ACTION));
}

#[test]
fn checkpoint_completion_hides_payload_behind_stable_ref() {
    let child_thread_id = codex_protocol::ThreadId::new();
    let message = format_inter_agent_checkpoint_message(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("valid agent path"),
        child_thread_id,
        "turn-1",
        &AgentStatus::Completed(Some("PRIVATE-CHECKPOINT-PAYLOAD".to_string())),
    )
    .expect("completed status should produce attention");

    assert!(message.contains("Message Type: AGENT_ATTENTION"));
    assert!(message.contains(&format!("agent-checkpoint:{child_thread_id}:turn-1")));
    assert!(message.contains("read_agent_checkpoints"));
    assert!(!message.contains("PRIVATE-CHECKPOINT-PAYLOAD"));
}
