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

use std::sync::Mutex;
use std::sync::MutexGuard;

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

    /// Register a live terminal id that was yielded to the model. Returns true
    /// only when this insertion transitions the set from empty to active.
    ///
    /// Expected to be called while a turn is active (from the tool-result
    /// path that hands the model a still-running terminal), so the session is
    /// already `Running`. Idempotent for a duplicate id.
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) fn register(&self, process_id: i32) -> bool {
        let mut state = self.lock_state();
        let was_empty = state.awaited.is_empty();
        state.awaited.insert(process_id) && was_empty
    }

    /// Resolve a terminal id after its completion was admitted or it was
    /// observed/disposed synchronously. Returns whether the id was awaited.
    pub(crate) fn resolve(&self, process_id: i32) -> bool {
        self.lock_state().awaited.remove(&process_id)
    }

    /// Snapshot of the currently awaited terminal ids (unsorted).
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) fn ids(&self) -> Vec<i32> {
        self.lock_state().awaited.iter().copied().collect()
    }

    /// Number of currently awaited terminal ids.
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) fn count(&self) -> usize {
        self.lock_state().awaited.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock_state().awaited.is_empty()
    }

    /// Resolve every awaited terminal id at once (cleanup/disposal).
    // Integration API for the unified-exec tool-result path; exercised by
    // session tests today.
    #[allow(dead_code)]
    pub(crate) fn clear(&self) -> bool {
        let mut state = self.lock_state();
        let changed = !state.awaited.is_empty();
        state.awaited.clear();
        changed
    }

    /// Record the final message of a `TurnComplete` that was held non-final
    /// because awaited terminals remain.
    ///
    /// Only a message-bearing completion replaces the retained message: a
    /// message-less auxiliary completion (for example a standalone user-shell
    /// turn) must not erase the model's final words from an earlier
    /// suppressed turn. The most recent waiting *message* therefore always
    /// wins while `None` never clobbers a retained message.
    pub(crate) fn note_waiting_final(&self, last_agent_message: Option<String>) {
        if let Some(message) = last_agent_message {
            self.lock_state().waiting_final_message = Some(message);
        }
    }

    /// Drop the retained waiting final message because a real final status
    /// superseded it.
    pub(crate) fn clear_waiting_final(&self) {
        self.lock_state().waiting_final_message = None;
    }

    /// Take (and clear) the retained waiting final message, if any.
    pub(crate) fn take_waiting_final(&self) -> Option<String> {
        self.lock_state().waiting_final_message.take()
    }

    fn lock_state(&self) -> MutexGuard<'_, AwaitedTerminalState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_query_resolve_and_clear() {
        let registry = AwaitedTerminals::new();
        assert!(registry.is_empty());
        assert_eq!(registry.count(), 0);

        assert!(registry.register(41), "first id enters active state");
        assert!(!registry.register(42), "additional id keeps active state");
        assert!(!registry.register(42), "duplicate is idempotent");
        assert!(!registry.is_empty());
        assert_eq!(registry.count(), 2);
        let mut ids = registry.ids();
        ids.sort_unstable();
        assert_eq!(ids, vec![41, 42]);

        assert!(registry.resolve(41));
        assert_eq!(registry.count(), 1);
        assert!(!registry.resolve(99), "unknown id resolves to false");

        assert!(registry.clear());
        assert!(!registry.clear(), "clearing twice is unchanged");
        assert!(registry.is_empty());
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn waiting_final_message_keeps_most_recent_message() {
        let registry = AwaitedTerminals::new();
        assert_eq!(registry.take_waiting_final(), None);

        registry.note_waiting_final(Some("first".to_string()));
        registry.note_waiting_final(Some("second".to_string()));
        assert_eq!(registry.take_waiting_final(), Some("second".to_string()));

        // A message-less suppressed completion must not erase the retained
        // final message.
        registry.note_waiting_final(Some("third".to_string()));
        registry.note_waiting_final(None);
        assert_eq!(registry.take_waiting_final(), Some("third".to_string()));

        registry.clear_waiting_final();
        assert_eq!(registry.take_waiting_final(), None);
    }
}
