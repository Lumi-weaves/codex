//! Session-owned registry of awaited background unified-exec terminals.
//!
//! When the model is handed a live ("yielded") terminal, the session records
//! its process id here. A regular turn stays active at later poll boundaries
//! until the awaited work produces model-visible input or is disposed. The
//! completion ingress queues that input before resolving the token;
//! synchronous observation/disposal resolves it directly. A watch channel
//! wakes the parked turn for resolution paths that do not enqueue input.
//!
//! The registry also owns the one-shot idle claim for an awaited batch. It
//! retains the most recent held final message and the authoritative upstream
//! `ThreadIdleCause`, then lets exactly one safe-boundary caller consume them
//! after the last token resolves and no continuation remains. The session
//! layer (`Session` methods in `session/mod.rs`) owns status transitions and
//! lifecycle emission. Exited unified-exec entries are filtered before
//! completion admission, so this registry never consults `list_processes()`.

use std::collections::HashSet;

use codex_extension_api::ThreadIdleCause;
use tokio::sync::Mutex;
use tokio::sync::watch;

/// Session-scoped awaited-terminal registry (see module docs).
pub(crate) struct AwaitedTerminals {
    state: Mutex<AwaitedTerminalState>,
    count_tx: watch::Sender<usize>,
}

#[derive(Default)]
struct AwaitedTerminalState {
    /// Process ids of live terminals the session is currently awaiting.
    awaited: HashSet<i32>,
    /// Final status held back while an awaited batch remains in flight.
    waiting_final: Option<WaitingFinal>,
    /// One-shot ownership token for the thread-idle lifecycle that belongs to
    /// the current awaited batch. It survives resolution while a continuation
    /// is queued and is consumed only once the session is truly idle.
    idle_claim_pending: bool,
    /// Most recent terminal cause observed while the claim is pending.
    pending_idle_cause: Option<ThreadIdleCause>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WaitingFinal {
    pub(crate) last_agent_message: Option<String>,
    pub(crate) idle_cause: ThreadIdleCause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AwaitedIdleClaim {
    pub(crate) waiting_final: Option<WaitingFinal>,
    pub(crate) idle_cause: Option<ThreadIdleCause>,
}

impl AwaitedTerminals {
    pub(crate) fn new() -> Self {
        let (count_tx, _count_rx) = watch::channel(0);
        Self {
            state: Mutex::new(AwaitedTerminalState::default()),
            count_tx,
        }
    }

    /// Register a live terminal id that was yielded to the model.
    ///
    /// Expected to be called while a turn is active (from the tool-result
    /// path that hands the model a still-running terminal), so the session is
    /// already `Running`. Idempotent for a duplicate id.
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) async fn register(&self, process_id: i32) {
        let count = {
            let mut state = self.state.lock().await;
            if !state.awaited.insert(process_id) {
                None
            } else {
                if !state.idle_claim_pending {
                    state.idle_claim_pending = true;
                }
                Some(state.awaited.len())
            }
        };
        if let Some(count) = count {
            self.count_tx.send_replace(count);
        }
    }

    /// Resolve a terminal id after its completion was admitted or it was
    /// observed/disposed synchronously. Returns whether the id was awaited.
    pub(crate) async fn resolve(&self, process_id: i32) -> bool {
        let count = {
            let mut state = self.state.lock().await;
            state
                .awaited
                .remove(&process_id)
                .then_some(state.awaited.len())
        };
        if let Some(count) = count {
            self.count_tx.send_replace(count);
            true
        } else {
            false
        }
    }

    /// Snapshot of the currently awaited terminal ids (unsorted).
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) async fn ids(&self) -> Vec<i32> {
        self.state.lock().await.awaited.iter().copied().collect()
    }

    /// Number of currently awaited terminal ids.
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) async fn count(&self) -> usize {
        self.state.lock().await.awaited.len()
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.state.lock().await.awaited.is_empty()
    }

    /// Resolve every awaited terminal id at once (cleanup/disposal).
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) async fn clear(&self) -> bool {
        let mut state = self.state.lock().await;
        let changed = !state.awaited.is_empty();
        state.awaited.clear();
        drop(state);
        if changed {
            self.count_tx.send_replace(0);
        }
        changed
    }

    /// Subscribe before reading awaited state when a task may need to park.
    /// The count is only a wake hint; callers re-read the authoritative set.
    pub(crate) fn subscribe_count(&self) -> watch::Receiver<usize> {
        self.count_tx.subscribe()
    }

    /// Attach the current task's terminal cause to an outstanding awaited
    /// batch. The returned boolean is the task tail's ownership token: when
    /// true, ordinary idle emission must not race the batch's one-shot claim.
    pub(crate) async fn note_idle_cause_if_pending(&self, cause: ThreadIdleCause) -> bool {
        let mut state = self.state.lock().await;
        if !state.idle_claim_pending {
            return false;
        }
        state.pending_idle_cause = Some(cause);
        if let Some(waiting_final) = state.waiting_final.as_mut() {
            waiting_final.idle_cause = cause;
        }
        true
    }

    /// Atomically hold a `TurnComplete` final status when terminals remain.
    /// A message-less auxiliary completion preserves the latest real final
    /// message while still updating the cause.
    pub(crate) async fn hold_final_if_awaited(
        &self,
        last_agent_message: Option<String>,
        fallback_cause: ThreadIdleCause,
    ) -> bool {
        let mut state = self.state.lock().await;
        if state.awaited.is_empty() {
            state.waiting_final = None;
            return false;
        }

        let idle_cause = state.pending_idle_cause.unwrap_or(fallback_cause);
        match state.waiting_final.as_mut() {
            Some(waiting_final) => {
                if last_agent_message.is_some() {
                    waiting_final.last_agent_message = last_agent_message;
                }
                waiting_final.idle_cause = idle_cause;
            }
            None => {
                state.waiting_final = Some(WaitingFinal {
                    last_agent_message,
                    idle_cause,
                });
            }
        }
        true
    }

    /// Atomically consume the awaited batch's idle claim after all ids have
    /// resolved. A running session must have a held final before it may claim;
    /// interrupted/errored sessions may claim without one.
    pub(crate) async fn claim_idle_if_resolved(
        &self,
        allow_without_waiting_final: bool,
    ) -> Option<AwaitedIdleClaim> {
        let mut state = self.state.lock().await;
        if !state.awaited.is_empty()
            || !state.idle_claim_pending
            || (state.waiting_final.is_none() && !allow_without_waiting_final)
        {
            return None;
        }

        state.idle_claim_pending = false;
        Some(AwaitedIdleClaim {
            waiting_final: state.waiting_final.take(),
            idle_cause: state.pending_idle_cause.take(),
        })
    }
}

impl Default for AwaitedTerminals {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_query_resolve_and_clear() {
        let registry = AwaitedTerminals::new();
        assert!(registry.is_empty().await);
        assert_eq!(registry.count().await, 0);

        registry.register(41).await;
        registry.register(42).await;
        registry.register(42).await; // duplicate is idempotent
        assert!(!registry.is_empty().await);
        assert_eq!(registry.count().await, 2);
        let mut ids = registry.ids().await;
        ids.sort_unstable();
        assert_eq!(ids, vec![41, 42]);

        assert!(registry.resolve(41).await);
        assert_eq!(registry.count().await, 1);
        assert!(!registry.resolve(99).await, "unknown id resolves to false");

        assert!(registry.clear().await);
        assert!(!registry.clear().await, "clearing twice is unchanged");
        assert!(registry.is_empty().await);
        assert_eq!(registry.count().await, 0);
    }

    #[tokio::test]
    async fn waiting_final_keeps_most_recent_message_and_cause() {
        let registry = AwaitedTerminals::new();
        registry.register(42).await;
        assert!(
            registry
                .note_idle_cause_if_pending(ThreadIdleCause::Completed)
                .await
        );
        assert!(
            registry
                .hold_final_if_awaited(Some("first".to_string()), ThreadIdleCause::Completed,)
                .await
        );
        assert!(
            registry
                .hold_final_if_awaited(Some("second".to_string()), ThreadIdleCause::Completed,)
                .await
        );
        assert!(registry.resolve(42).await);
        assert_eq!(
            registry.claim_idle_if_resolved(false).await,
            Some(AwaitedIdleClaim {
                waiting_final: Some(WaitingFinal {
                    last_agent_message: Some("second".to_string()),
                    idle_cause: ThreadIdleCause::Completed,
                }),
                idle_cause: Some(ThreadIdleCause::Completed),
            })
        );

        // A message-less suppressed completion must not erase the retained
        // final message.
        registry.register(43).await;
        assert!(
            registry
                .hold_final_if_awaited(Some("third".to_string()), ThreadIdleCause::Completed,)
                .await
        );
        assert!(
            registry
                .note_idle_cause_if_pending(ThreadIdleCause::Failed)
                .await
        );
        assert!(
            registry
                .hold_final_if_awaited(None, ThreadIdleCause::Completed)
                .await
        );
        assert!(registry.resolve(43).await);
        assert_eq!(
            registry.claim_idle_if_resolved(false).await,
            Some(AwaitedIdleClaim {
                waiting_final: Some(WaitingFinal {
                    last_agent_message: Some("third".to_string()),
                    idle_cause: ThreadIdleCause::Failed,
                }),
                idle_cause: Some(ThreadIdleCause::Failed),
            })
        );
        assert_eq!(registry.claim_idle_if_resolved(true).await, None);
    }
}
