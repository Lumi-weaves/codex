//! Core-private events produced by background machinery and admitted through
//! the session submission loop so they are serialized with external
//! submissions (user input, interrupts, shutdown). These are not client
//! `Op`s: they never cross the app-server protocol surface.

use crate::context::UnifiedExecCompletionEvent;
use crate::context::UnifiedExecOutputAvailableEvent;

/// Internal session event produced inside the process.
#[derive(Debug)]
pub(crate) enum InternalSessionEvent {
    /// A background unified-exec terminal process finished without a
    /// synchronous observation of its exit. Carries the bounded,
    /// model-visible completion fragment.
    UnifiedExecCompletion(UnifiedExecCompletionEvent),
    /// An interactive background terminal produced unread output and may need
    /// model input. Carries a bounded, model-visible attention fragment.
    UnifiedExecOutputAvailable(UnifiedExecOutputAvailableEvent),
}
