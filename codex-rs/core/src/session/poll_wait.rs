//! Event-driven suspension between model polls while the task owns awaited work.

use tokio_util::sync::CancellationToken;

use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;

use super::session::Session;
use super::turn_context::TurnContext;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AwaitedPollWaitOutcome {
    Resume,
    AwaitedWorkCleared,
}

/// Park the current logical turn until input arrives or its awaited work is cleared.
///
/// Input and awaited-state subscriptions are installed before predicates are read.
/// Completion ingress queues model-visible input before resolving its awaited token,
/// so the final input recheck closes the cross-channel empty-set race.
pub(super) async fn wait_for_awaited_poll_resume(
    sess: &Session,
    turn_context: &TurnContext,
    cancellation_token: &CancellationToken,
) -> Result<AwaitedPollWaitOutcome> {
    sess.input_queue
        .accept_mailbox_delivery_for_current_turn(&sess.active_turn, &turn_context.sub_id)
        .await;

    let turn_state = sess
        .input_queue
        .turn_state_for_sub_id(&sess.active_turn, &turn_context.sub_id)
        .await;
    let (mut input_activity_rx, mut pending_activity) = sess
        .input_queue
        .subscribe_activity(turn_state.as_deref())
        .await;
    let mut awaited_count_rx = sess.awaited_terminals.subscribe_count();

    loop {
        if pending_activity.take().is_some()
            || sess.input_queue.has_pending_input(&sess.active_turn).await
            || sess.input_queue.has_pending_session_inputs().await
        {
            return Ok(AwaitedPollWaitOutcome::Resume);
        }

        if !sess.has_awaited_terminals().await {
            if sess.input_queue.has_pending_input(&sess.active_turn).await
                || sess.input_queue.has_pending_session_inputs().await
            {
                return Ok(AwaitedPollWaitOutcome::Resume);
            }
            return Ok(AwaitedPollWaitOutcome::AwaitedWorkCleared);
        }

        tokio::select! {
            result = input_activity_rx.changed() => {
                if result.is_err() {
                    return Err(CodexErr::TurnAborted);
                }
                input_activity_rx.borrow_and_update();
            }
            result = awaited_count_rx.changed() => {
                if result.is_err() {
                    return Err(CodexErr::TurnAborted);
                }
                awaited_count_rx.borrow_and_update();
            }
            _ = cancellation_token.cancelled() => {
                return Err(CodexErr::TurnAborted);
            }
        }
    }
}
