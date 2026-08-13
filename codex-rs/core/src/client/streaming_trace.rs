use std::fmt::Display;

use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use codex_rollout_trace::CompactionTraceAttempt;
use codex_rollout_trace::CompactionTraceContext;
use codex_rollout_trace::InferenceTraceAttempt;
use codex_rollout_trace::InferenceTraceContext;
use http::HeaderMap as ApiHeaderMap;

#[derive(Clone, Copy)]
pub(super) enum StreamingTraceContext<'a> {
    Inference(&'a InferenceTraceContext),
    Compaction(&'a CompactionTraceContext),
}

pub(super) enum StreamingTraceAttempt {
    Inference(InferenceTraceAttempt),
    Compaction(CompactionTraceAttempt),
}

impl StreamingTraceContext<'_> {
    pub(super) fn start_attempt(&self) -> StreamingTraceAttempt {
        match self {
            Self::Inference(context) => StreamingTraceAttempt::Inference(context.start_attempt()),
            Self::Compaction(context) => {
                StreamingTraceAttempt::Compaction(context.start_attempt_unrecorded())
            }
        }
    }
}

impl StreamingTraceAttempt {
    pub(super) fn disabled_inference() -> Self {
        Self::Inference(InferenceTraceAttempt::disabled())
    }

    pub(super) fn add_request_headers(&self, headers: &mut ApiHeaderMap) {
        if let Self::Inference(attempt) = self {
            attempt.add_request_headers(headers);
        }
    }

    pub(super) fn record_started(&self, request: &impl serde::Serialize) {
        match self {
            Self::Inference(attempt) => attempt.record_started(request),
            Self::Compaction(attempt) => attempt.record_started(request),
        }
    }

    pub(super) fn record_completed(
        &self,
        response_id: &str,
        upstream_request_id: Option<&str>,
        token_usage: &Option<TokenUsage>,
        output_items: &[ResponseItem],
    ) {
        match self {
            Self::Inference(attempt) => attempt.record_completed(
                response_id,
                upstream_request_id,
                token_usage,
                output_items,
            ),
            Self::Compaction(attempt) => {
                attempt.record_stream_completed(response_id, output_items);
            }
        }
    }

    pub(super) fn record_failed(
        &self,
        error: impl Display,
        upstream_request_id: Option<&str>,
        output_items: &[ResponseItem],
    ) {
        match self {
            Self::Inference(attempt) => {
                attempt.record_failed(error, upstream_request_id, output_items);
            }
            Self::Compaction(attempt) => attempt.record_failed(error),
        }
    }

    pub(super) fn record_cancelled(
        &self,
        reason: &str,
        upstream_request_id: Option<&str>,
        output_items: &[ResponseItem],
    ) {
        match self {
            Self::Inference(attempt) => {
                attempt.record_cancelled(reason, upstream_request_id, output_items);
            }
            Self::Compaction(attempt) => attempt.record_failed(reason),
        }
    }
}
