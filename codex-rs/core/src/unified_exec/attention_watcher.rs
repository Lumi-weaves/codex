use std::sync::Arc;

use tokio::time::Duration;

use super::UnifiedExecProcess;
use crate::context::UNIFIED_EXEC_ATTENTION_OUTPUT_EXCERPT_MAX_BYTES;
use crate::context::UnifiedExecOutputAvailableEvent;
use crate::context::decode_lossy_one_for_one;
use crate::session::SessionIngress;
use crate::session::session::Session;

/// Quiet period used to combine a burst of interactive-terminal output into
/// one attention event.
pub(crate) const TTY_ATTENTION_DEBOUNCE: Duration = Duration::from_millis(350);
/// Maximum time a continuous output stream can postpone its first attention
/// event. The unread watermark still prevents repeated wakes until the model
/// drains that batch.
pub(crate) const TTY_ATTENTION_MAX_DELAY: Duration = Duration::from_secs(2);

/// Watch an interactive background terminal for unread output.
///
/// This is deliberately TTY-only. Non-interactive commands keep their current
/// completion-only wake behavior, while a TTY opts into output-as-attention
/// semantics because it may eventually request model input. The process owns a
/// single unread-attention watermark, so a chatty terminal cannot wake again
/// until `exec_command` or `write_stdin` drains the previous output batch.
pub(crate) fn spawn_tty_attention_watcher(
    process: Arc<UnifiedExecProcess>,
    session_ref: Arc<Session>,
    process_id: i32,
) {
    // Subscribe before the task is spawned. Output that races with task
    // scheduling is retained in this receiver, while output older than the
    // subscription is still discovered through the process's monotonic
    // produced/observed offsets when the first activity is handled.
    let mut output_rx = process.output_receiver();
    let exit_token = process.cancellation_token();

    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;

        loop {
            let received = tokio::select! {
                biased;
                _ = exit_token.cancelled() => return,
                received = output_rx.recv() => received,
            };
            match received {
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }

            // Extend the quiet period for every additional chunk. This keeps
            // prompts and short bursts together. A separate hard deadline
            // prevents a continuously chatty TTY from postponing attention
            // forever.
            let quiet = tokio::time::sleep(TTY_ATTENTION_DEBOUNCE);
            let maximum_delay = tokio::time::sleep(TTY_ATTENTION_MAX_DELAY);
            tokio::pin!(quiet);
            tokio::pin!(maximum_delay);
            loop {
                tokio::select! {
                    biased;
                    _ = exit_token.cancelled() => return,
                    received = output_rx.recv() => match received {
                        Ok(_) | Err(RecvError::Lagged(_)) => {
                            quiet.as_mut().reset(tokio::time::Instant::now() + TTY_ATTENTION_DEBOUNCE);
                        }
                        Err(RecvError::Closed) => return,
                    },
                    _ = &mut quiet => break,
                    _ = &mut maximum_delay => break,
                }
            }

            // Serialize with the initial exec observation, write_stdin, and
            // the exit watcher. Rechecking after the lock makes completion win
            // whenever exit has already become observable.
            let _interaction_guard = process.interaction_lock().lock_owned().await;
            if exit_token.is_cancelled() || process.has_exited() {
                return;
            }
            let Some(snapshot) = process
                .claim_output_attention(UNIFIED_EXEC_ATTENTION_OUTPUT_EXCERPT_MAX_BYTES)
                .await
            else {
                continue;
            };
            let event = UnifiedExecOutputAvailableEvent::new(
                process_id,
                snapshot.observed_offset,
                snapshot.produced_offset,
                snapshot.total_output_bytes,
                snapshot.omitted_output_bytes,
                decode_lossy_one_for_one(&snapshot.output_excerpt),
            );

            let Some(ingress_tx) = session_ref.internal_session_event_tx.upgrade() else {
                return;
            };
            tokio::select! {
                biased;
                _ = exit_token.cancelled() => return,
                result = ingress_tx.send(SessionIngress::Internal(
                    crate::session::internal_event::InternalSessionEvent::UnifiedExecOutputAvailable(
                        event,
                    ),
                )) => {
                    if result.is_err() {
                        return;
                    }
                }
            }
        }
    });
}
