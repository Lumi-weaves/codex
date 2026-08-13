use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;

/// A transient model constraint emitted while this task owns an active resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveResourceNoFinish;

/// Buffers assistant messages from one constrained sampling response so their completed phases
/// can be merged and delivered as commentary without losing useful final-answer text.
#[derive(Debug, Default)]
pub(crate) struct ActiveResourceNoFinishBuffer {
    merged: Option<ResponseItem>,
    downgraded_finish: bool,
}

impl ActiveResourceNoFinishBuffer {
    pub(crate) fn push_if_assistant(&mut self, item: ResponseItem) -> Option<ResponseItem> {
        let ResponseItem::Message {
            role,
            content,
            phase,
            ..
        } = &item
        else {
            return Some(item);
        };
        if role != "assistant" {
            return Some(item);
        }

        self.downgraded_finish |= !matches!(phase, Some(MessagePhase::Commentary));
        match self.merged.as_mut() {
            Some(ResponseItem::Message {
                content: merged_content,
                ..
            }) => {
                if !merged_content.is_empty() && !content.is_empty() {
                    merged_content.push(ContentItem::OutputText {
                        text: "\n\n".to_string(),
                    });
                }
                merged_content.extend(content.iter().cloned());
            }
            None => {
                let mut item = item;
                if let ResponseItem::Message { phase, .. } = &mut item {
                    *phase = Some(MessagePhase::Commentary);
                }
                self.merged = Some(item);
            }
            Some(_) => unreachable!("active-resource buffer stores only assistant messages"),
        }
        None
    }

    pub(crate) fn take_commentary(&mut self) -> Option<ResponseItem> {
        self.merged.take()
    }

    pub(crate) fn downgraded_finish(&self) -> bool {
        self.downgraded_finish
    }
}

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
