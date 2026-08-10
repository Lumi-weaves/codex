//! Core-private events produced by background machinery and admitted through
//! the session submission loop so they are serialized with external
//! submissions (user input, interrupts, shutdown). These are not client
//! `Op`s: they never cross the app-server protocol surface.

use crate::context::UnifiedExecCompletionEvent;

/// Internal session event produced inside the process.
#[derive(Debug)]
pub(crate) enum InternalSessionEvent {
    /// A background unified-exec terminal process finished without a
    /// synchronous observation of its exit. Carries the bounded,
    /// model-visible completion fragment.
    UnifiedExecCompletion(UnifiedExecCompletionEvent),
}
