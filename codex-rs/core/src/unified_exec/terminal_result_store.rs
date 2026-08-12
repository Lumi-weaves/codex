use super::UnifiedExecProcessManager;
use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug)]
pub(crate) struct TerminalResultInput {
    pub(crate) process_id: i32,
    pub(crate) item_id: String,
    pub(crate) command: String,
    pub(crate) cwd: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) failure_message: Option<String>,
    pub(crate) duration_ms: u64,
    pub(crate) output_bytes_total: usize,
    pub(crate) output_bytes_retained: usize,
    pub(crate) output_bytes_omitted: usize,
    pub(crate) retained_output: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TerminalResultMetadata {
    pub(crate) result_ref: String,
    pub(crate) process_id: i32,
    pub(crate) item_id: String,
    pub(crate) command: String,
    pub(crate) cwd: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) failure_message: Option<String>,
    pub(crate) duration_ms: u64,
    pub(crate) output_bytes_total: usize,
    pub(crate) output_bytes_retained: usize,
    pub(crate) output_bytes_omitted: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalResultState {
    Available,
    Evicted,
    Unavailable,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TerminalResultRead {
    pub(crate) state: TerminalResultState,
    pub(crate) result_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<TerminalResultMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<usize>,
}

#[derive(Debug, Error)]
pub(crate) enum TerminalResultReadError {
    #[error("offset {offset} exceeds retained terminal result size {retained_bytes}")]
    OffsetOutOfRange {
        offset: usize,
        retained_bytes: usize,
    },
    #[error("offset {offset} is not a valid UTF-8 boundary")]
    OffsetNotCharBoundary { offset: usize },
}

struct StoredTerminalResult {
    metadata: TerminalResultMetadata,
    state: StoredTerminalResultState,
    last_access_sequence: u64,
}

enum StoredTerminalResultState {
    Available(String),
    Evicted,
}

pub(super) struct TerminalResultStore {
    entries: HashMap<String, StoredTerminalResult>,
    next_sequence: u64,
    max_available_results: usize,
    max_tombstones: usize,
}

impl Default for TerminalResultStore {
    fn default() -> Self {
        Self::with_limits(32, 64)
    }
}

impl TerminalResultStore {
    fn with_limits(max_available_results: usize, max_tombstones: usize) -> Self {
        Self {
            entries: HashMap::new(),
            next_sequence: 0,
            max_available_results,
            max_tombstones,
        }
    }

    fn retain(&mut self, input: TerminalResultInput) -> TerminalResultMetadata {
        let sequence = self.advance_sequence();
        let result_ref = format!("terminal-result:{}:{sequence}", input.process_id);
        let metadata = TerminalResultMetadata {
            result_ref: result_ref.clone(),
            process_id: input.process_id,
            item_id: input.item_id,
            command: input.command,
            cwd: input.cwd,
            exit_code: input.exit_code,
            failure_message: input.failure_message,
            duration_ms: input.duration_ms,
            output_bytes_total: input.output_bytes_total,
            output_bytes_retained: input.output_bytes_retained,
            output_bytes_omitted: input.output_bytes_omitted,
        };
        self.entries.insert(
            result_ref,
            StoredTerminalResult {
                metadata: metadata.clone(),
                state: StoredTerminalResultState::Available(input.retained_output),
                last_access_sequence: sequence,
            },
        );
        self.enforce_capacity();
        metadata
    }

    fn read(
        &mut self,
        result_ref: &str,
        offset: usize,
        max_bytes: usize,
    ) -> Result<TerminalResultRead, TerminalResultReadError> {
        let sequence = self.advance_sequence();
        let Some(entry) = self.entries.get_mut(result_ref) else {
            return Ok(TerminalResultRead {
                state: TerminalResultState::Unavailable,
                result_ref: result_ref.to_string(),
                metadata: None,
                output_offset: None,
                output: None,
                next_offset: None,
            });
        };
        entry.last_access_sequence = sequence;

        let state = match &entry.state {
            StoredTerminalResultState::Evicted => TerminalResultState::Evicted,
            StoredTerminalResultState::Available(output) => {
                if offset > output.len() {
                    return Err(TerminalResultReadError::OffsetOutOfRange {
                        offset,
                        retained_bytes: output.len(),
                    });
                }
                if !output.is_char_boundary(offset) {
                    return Err(TerminalResultReadError::OffsetNotCharBoundary { offset });
                }
                let mut end = offset.saturating_add(max_bytes).min(output.len());
                while end > offset && !output.is_char_boundary(end) {
                    end -= 1;
                }
                if end == offset
                    && let Some(ch) = output[offset..].chars().next()
                {
                    end += ch.len_utf8();
                }
                let next_offset = (end < output.len()).then_some(end);
                return Ok(TerminalResultRead {
                    state: TerminalResultState::Available,
                    result_ref: result_ref.to_string(),
                    metadata: Some(entry.metadata.clone()),
                    output_offset: Some(offset),
                    output: Some(output[offset..end].to_string()),
                    next_offset,
                });
            }
        };

        Ok(TerminalResultRead {
            state,
            result_ref: result_ref.to_string(),
            metadata: Some(entry.metadata.clone()),
            output_offset: None,
            output: None,
            next_offset: None,
        })
    }

    fn enforce_capacity(&mut self) {
        while self.available_count() > self.max_available_results {
            let candidate = self
                .entries
                .iter()
                .filter(|(_, entry)| matches!(entry.state, StoredTerminalResultState::Available(_)))
                .min_by_key(|(_, entry)| entry.last_access_sequence)
                .map(|(result_ref, _)| result_ref.clone());
            let Some(candidate) = candidate else {
                break;
            };
            if let Some(entry) = self.entries.get_mut(&candidate) {
                entry.state = StoredTerminalResultState::Evicted;
            }
        }
        self.trim_tombstones();
    }

    fn trim_tombstones(&mut self) {
        while self.tombstone_count() > self.max_tombstones {
            let candidate = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    !matches!(entry.state, StoredTerminalResultState::Available(_))
                })
                .min_by_key(|(_, entry)| entry.last_access_sequence)
                .map(|(result_ref, _)| result_ref.clone());
            let Some(candidate) = candidate else {
                break;
            };
            self.entries.remove(&candidate);
        }
    }

    fn available_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry.state, StoredTerminalResultState::Available(_)))
            .count()
    }

    fn tombstone_count(&self) -> usize {
        self.entries.len().saturating_sub(self.available_count())
    }

    fn advance_sequence(&mut self) -> u64 {
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_sequence
    }
}

impl UnifiedExecProcessManager {
    pub(crate) async fn retain_terminal_result(
        &self,
        input: TerminalResultInput,
    ) -> TerminalResultMetadata {
        self.terminal_result_store.lock().await.retain(input)
    }

    pub(crate) async fn read_terminal_result(
        &self,
        result_ref: &str,
        offset: usize,
        max_bytes: usize,
    ) -> Result<TerminalResultRead, TerminalResultReadError> {
        self.terminal_result_store
            .lock()
            .await
            .read(result_ref, offset, max_bytes)
    }
}

#[cfg(test)]
#[path = "terminal_result_store_tests.rs"]
mod tests;
