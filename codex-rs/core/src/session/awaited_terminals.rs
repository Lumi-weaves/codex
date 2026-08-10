//! Session-owned registry of awaited background unified-exec terminals.
//!
//! When the model is handed a live ("yielded") terminal, the session records
//! its process id here so a later `TurnComplete` cannot finalize the session
//! while that terminal is still being awaited in the background. The
//! completion ingress resolves the token after the model-visible completion
//! fragment is queued; synchronous observation/disposal resolves it directly.
//!
//! The registry is deliberately dumb state plus the narrow bookkeeping needed
//! to restore a truthful final status: the most recent `last_agent_message`
//! from a `TurnComplete` whose final status was held back. The session layer
//! (`Session` methods in `session/mod.rs`) owns the status-transition policy:
//! keep `AgentStatus::Running` while awaited ids remain, suppress V2 parent
//! completion notification, and restore `Completed` only when the last token
//! is resolved with no continuation (no active turn, no pending session
//! inputs). Exited unified-exec entries are filtered before completion
//! admission, so this registry never consults `list_processes()`.

use std::collections::HashSet;

use tokio::sync::Mutex;

/// Session-scoped awaited-terminal registry (see module docs).
#[derive(Default)]
pub(crate) struct AwaitedTerminals {
    state: Mutex<AwaitedTerminalState>,
}

#[derive(Default)]
struct AwaitedTerminalState {
    /// Process ids of live terminals the session is currently awaiting.
    awaited: HashSet<i32>,
    /// Most recent `last_agent_message` from a `TurnComplete` that was held
    /// back (kept non-final) because awaited terminals remained. Retained
    /// only while a final completion is being suppressed; `None` when nothing
    /// is being held back, or when the suppressed completion carried no
    /// message. Used to restore `Completed` when the last terminal is
    /// resolved with no continuation.
    waiting_final_message: Option<String>,
}

impl AwaitedTerminals {
    pub(crate) fn new() -> Self {
        Self::default()
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
        self.state.lock().await.awaited.insert(process_id);
    }

    /// Resolve a terminal id after its completion was admitted or it was
    /// observed/disposed synchronously. Returns whether the id was awaited.
    pub(crate) async fn resolve(&self, process_id: i32) -> bool {
        self.state.lock().await.awaited.remove(&process_id)
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
    pub(crate) async fn clear(&self) {
        self.state.lock().await.awaited.clear();
    }

    /// Record the final message of a `TurnComplete` that was held non-final
    /// because awaited terminals remain.
    ///
    /// Only a message-bearing completion replaces the retained message: a
    /// message-less auxiliary completion (for example a standalone user-shell
    /// turn) must not erase the model's final words from an earlier
    /// suppressed turn. The most recent waiting *message* therefore always
    /// wins while `None` never clobbers a retained message.
    pub(crate) async fn note_waiting_final(&self, last_agent_message: Option<String>) {
        if let Some(message) = last_agent_message {
            self.state.lock().await.waiting_final_message = Some(message);
        }
    }

    /// Drop the retained waiting final message because a real final status
    /// superseded it.
    pub(crate) async fn clear_waiting_final(&self) {
        self.state.lock().await.waiting_final_message = None;
    }

    /// Take (and clear) the retained waiting final message, if any.
    pub(crate) async fn take_waiting_final(&self) -> Option<String> {
        self.state.lock().await.waiting_final_message.take()
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

        registry.clear().await;
        assert!(registry.is_empty().await);
        assert_eq!(registry.count().await, 0);
    }

    #[tokio::test]
    async fn waiting_final_message_keeps_most_recent_message() {
        let registry = AwaitedTerminals::new();
        assert_eq!(registry.take_waiting_final().await, None);

        registry.note_waiting_final(Some("first".to_string())).await;
        registry
            .note_waiting_final(Some("second".to_string()))
            .await;
        assert_eq!(
            registry.take_waiting_final().await,
            Some("second".to_string())
        );

        // A message-less suppressed completion must not erase the retained
        // final message.
        registry.note_waiting_final(Some("third".to_string())).await;
        registry.note_waiting_final(None).await;
        assert_eq!(
            registry.take_waiting_final().await,
            Some("third".to_string())
        );

        registry.clear_waiting_final().await;
        assert_eq!(registry.take_waiting_final().await, None);
    }
}
