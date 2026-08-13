use super::ContextualUserFragment;

/// A transient model constraint emitted while this task owns an active resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveResourceNoFinish;

impl ContextualUserFragment for ActiveResourceNoFinish {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn requires_separate_message(&self) -> bool {
        true
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        "no-finish".to_string()
    }
}
