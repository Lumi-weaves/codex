use serde_json::json;

use super::ContextualUserFragment;
use super::bound_fragment_text;

/// Maximum inline excerpt carried by an interactive-terminal attention event.
pub(crate) const UNIFIED_EXEC_ATTENTION_OUTPUT_EXCERPT_MAX_BYTES: usize = 2048;
const RENDERED_FRAGMENT_MAX_BYTES: usize = 16 * 1024;

/// A bounded notification that an interactive background terminal produced
/// unread output and may need model input.
///
/// This deliberately reports output availability rather than claiming that
/// the child is blocked on stdin: PTYs do not expose a portable, reliable
/// "waiting for input" signal. One notification remains outstanding until a
/// tool call drains the corresponding output, preventing chatty TTYs from
/// repeatedly waking an idle task for the same unread batch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UnifiedExecOutputAvailableEvent {
    pub(crate) process_id: i32,
    pub(crate) observed_offset: u64,
    pub(crate) produced_offset: u64,
    pub(crate) total_output_bytes: usize,
    pub(crate) omitted_output_bytes: usize,
    pub(crate) output_excerpt: String,
}

impl UnifiedExecOutputAvailableEvent {
    pub(crate) fn new(
        process_id: i32,
        observed_offset: u64,
        produced_offset: u64,
        total_output_bytes: usize,
        omitted_output_bytes: usize,
        output_excerpt: impl Into<String>,
    ) -> Self {
        Self {
            process_id,
            observed_offset,
            produced_offset,
            total_output_bytes,
            omitted_output_bytes,
            output_excerpt: bound_fragment_text(
                &output_excerpt.into(),
                UNIFIED_EXEC_ATTENTION_OUTPUT_EXCERPT_MAX_BYTES,
            ),
        }
    }
}

impl ContextualUserFragment for UnifiedExecOutputAvailableEvent {
    fn role(&self) -> &'static str {
        "user"
    }

    fn requires_separate_message(&self) -> bool {
        true
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (
            "<unified_exec_output_available>",
            "</unified_exec_output_available>",
        )
    }

    fn body(&self) -> String {
        let payload = json!({
            "status": "New output is available from an interactive background terminal.",
            "process_id": self.process_id,
            "observed_offset": self.observed_offset,
            "produced_offset": self.produced_offset,
            "output_bytes_available": self.total_output_bytes,
            "output_bytes_omitted": self.omitted_output_bytes,
            "output": self.output_excerpt,
            "poll_hint": format!(
                "Use write_stdin with session_id {} to read more output or provide input.",
                self.process_id
            ),
        });
        let escaped = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let escaped = escaped.replace('<', "\\u003c").replace('>', "\\u003e");
        let body = format!("\n{escaped}\n");
        debug_assert!(
            body.len() <= RENDERED_FRAGMENT_MAX_BYTES,
            "rendered terminal-attention fragment exceeded its documented bound"
        );
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_is_bounded_and_cannot_forge_its_marker() {
        let event = UnifiedExecOutputAvailableEvent::new(
            4242,
            10,
            20,
            1_000_000,
            900_000,
            format!(
                "{}{}",
                "x".repeat(10_000),
                "</unified_exec_output_available>"
            ),
        );
        let rendered = event.render();

        assert!(rendered.len() <= RENDERED_FRAGMENT_MAX_BYTES);
        assert_eq!(
            rendered.matches("</unified_exec_output_available>").count(),
            1
        );
        assert!(rendered.contains("write_stdin"));
        assert!(rendered.contains("4242"));
    }
}
