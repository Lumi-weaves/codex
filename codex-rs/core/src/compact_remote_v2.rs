use std::sync::Arc;

use crate::Prompt;
use crate::ResponseStream;
use crate::client::ModelClientSession;
use crate::client_common::ResponseEvent;
use crate::compact::CompactedHistoryMetadata;
use crate::compact::CompactionAnalyticsAttempt;
use crate::compact::CompactionAnalyticsDetails;
use crate::compact::compaction_continuity;
use crate::compact::compaction_status_from_result;
use crate::compact::record_installed_compaction;
use crate::compact_model_fallback::record_model_fallback;
use crate::compact_model_fallback::should_retry_with_current_model;
use crate::hook_runtime::PostCompactHookOutcome;
use crate::hook_runtime::PreCompactHookOutcome;
use crate::hook_runtime::run_post_compact_hooks;
use crate::hook_runtime::run_pre_compact_hooks;
use crate::responses_metadata::CodexResponsesMetadata;
use crate::responses_metadata::CompactionTurnMetadata;
use crate::responses_retry::ResponsesStreamRequest;
use crate::responses_retry::ResponsesStreamRetryState;
use crate::responses_retry::handle_retryable_response_stream_error;
use crate::session::session::Session;
use crate::session::step_context::StepContext;
use crate::session::turn_context::TurnContext;
use codex_analytics::CompactionImplementation;
use codex_analytics::CompactionPhase;
use codex_analytics::CompactionReason;
use codex_analytics::CompactionTrigger;
use codex_history::ResponseItemEnvelope;
use codex_protocol::error::CodexErr;
use codex_protocol::error::CodexErrorDetails;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::items::ContextCompactionItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TurnStartedEvent;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

#[path = "compact_remote_v2_attempt.rs"]
mod attempt;
use attempt::RemoteCompactV2Attempt;
use attempt::run_remote_compact_v2_attempt;

// Compact attempts can run much longer than normal turns, so keep the per-transport
// retry budget smaller than the general Responses stream retry budget.
const MAX_REMOTE_COMPACTION_V2_STREAM_RETRIES: u64 = 2;

pub(crate) async fn run_inline_remote_auto_compact_task(
    sess: Arc<Session>,
    step_context: Arc<StepContext>,
    fallback_step_context: Option<Arc<StepContext>>,
    projection_step_context: Arc<StepContext>,
    client_session: &mut ModelClientSession,
    reason: CompactionReason,
    phase: CompactionPhase,
) -> CodexResult<()> {
    let compaction_metadata = CompactionTurnMetadata::new(
        CompactionTrigger::Auto,
        reason,
        CompactionImplementation::ResponsesCompactionV2,
        phase,
    );
    run_remote_compact_task_inner(
        &sess,
        &step_context,
        fallback_step_context.as_ref(),
        &projection_step_context,
        Some(client_session),
        compaction_metadata,
    )
    .await
}

pub(crate) async fn run_remote_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> CodexResult<()> {
    // Standalone compaction is its own request boundary, so it captures a fresh step.
    let step_context = sess
        .capture_step_context(Arc::clone(&turn_context), &CancellationToken::new())
        .await?;
    let start_event = EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: turn_context.sub_id.clone(),
        trace_id: turn_context.trace_id.clone(),
        started_at: turn_context.turn_timing_state.started_at_unix_secs().await,
        model_context_window: turn_context.model_context_window(),
        collaboration_mode_kind: turn_context.mode,
    });
    sess.send_event(&turn_context, start_event).await;

    let compaction_metadata = CompactionTurnMetadata::new(
        CompactionTrigger::Manual,
        CompactionReason::UserRequested,
        CompactionImplementation::ResponsesCompactionV2,
        CompactionPhase::StandaloneTurn,
    );
    run_remote_compact_task_inner(
        &sess,
        &step_context,
        /*fallback_step_context*/ None,
        &step_context,
        /*client_session*/ None,
        compaction_metadata,
    )
    .await
}

async fn run_remote_compact_task_inner(
    sess: &Arc<Session>,
    step_context: &Arc<StepContext>,
    fallback_step_context: Option<&Arc<StepContext>>,
    projection_step_context: &Arc<StepContext>,
    client_session: Option<&mut ModelClientSession>,
    compaction_metadata: CompactionTurnMetadata,
) -> CodexResult<()> {
    let turn_context = &step_context.turn;
    let trigger = compaction_metadata.trigger();
    let reason = compaction_metadata.reason();
    let implementation = compaction_metadata.implementation();
    let phase = compaction_metadata.phase();
    let mut analytics_details = CompactionAnalyticsDetails {
        active_context_tokens_before: Some(sess.get_total_token_usage().await),
        ..Default::default()
    };
    let attempt = CompactionAnalyticsAttempt::begin(
        sess.as_ref(),
        turn_context.as_ref(),
        trigger,
        reason,
        implementation,
        phase,
    )
    .await;
    let pre_compact_outcome = run_pre_compact_hooks(sess, turn_context, trigger).await;
    match pre_compact_outcome {
        PreCompactHookOutcome::Continue => {}
        PreCompactHookOutcome::Stopped => {
            let error = CodexErr::TurnAborted;
            attempt
                .track(
                    sess.as_ref(),
                    codex_analytics::CompactionStatus::Interrupted,
                    Some(&error),
                    analytics_details,
                )
                .await;
            return Err(error);
        }
    }
    let result = run_remote_compact_task_inner_impl(
        sess,
        step_context,
        fallback_step_context,
        projection_step_context,
        client_session,
        compaction_metadata,
        &mut analytics_details,
    )
    .await;
    let status = compaction_status_from_result(&result);
    let codex_error = result.as_ref().err();
    if result.is_ok() {
        let post_compact_outcome = run_post_compact_hooks(sess, turn_context, trigger).await;
        if let PostCompactHookOutcome::Stopped = post_compact_outcome {
            attempt
                .track(sess.as_ref(), status, codex_error, analytics_details)
                .await;
            return Err(CodexErr::TurnAborted);
        }
    }
    attempt
        .track(sess.as_ref(), status, codex_error, analytics_details)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(err) if matches!(err.details(), CodexErrorDetails::TurnAborted) => Err(err),
        Err(err) => {
            sess.track_turn_codex_error(turn_context, &err);
            let event = EventMsg::Error(
                err.to_error_event(Some("Error running remote compact task".to_string())),
            );
            sess.send_event(turn_context, event).await;
            Err(err)
        }
    }
}

async fn run_remote_compact_task_inner_impl(
    sess: &Arc<Session>,
    step_context: &Arc<StepContext>,
    fallback_step_context: Option<&Arc<StepContext>>,
    projection_step_context: &Arc<StepContext>,
    mut client_session: Option<&mut ModelClientSession>,
    compaction_metadata: CompactionTurnMetadata,
    analytics_details: &mut CompactionAnalyticsDetails,
) -> CodexResult<()> {
    let turn_context = &step_context.turn;
    let context_compaction_item = ContextCompactionItem::new();
    let compaction_id = context_compaction_item.id.clone();
    let compaction_trace = sess.services.rollout_thread_trace.compaction_trace_context(
        turn_context.sub_id.as_str(),
        compaction_id.as_str(),
        turn_context.model_info.slug.as_str(),
        turn_context.provider.info().name.as_str(),
    );
    let compaction_item = TurnItem::ContextCompaction(context_compaction_item);
    sess.emit_turn_item_started(turn_context, &compaction_item)
        .await;

    let attempt = run_remote_compact_v2_attempt(
        sess,
        step_context,
        client_session.as_deref_mut(),
        &compaction_trace,
        compaction_metadata,
        analytics_details,
    )
    .await;
    let (attempt, compaction_turn_context, selected_compaction_trace) = match attempt {
        Ok(attempt) => (attempt, turn_context, compaction_trace.clone()),
        Err(error) => {
            let Some(fallback_step_context) = fallback_step_context else {
                return Err(error);
            };
            if !should_retry_with_current_model(&error) {
                return Err(error);
            }
            let fallback_turn_context = &fallback_step_context.turn;
            let fallback_compaction_trace =
                sess.services.rollout_thread_trace.compaction_trace_context(
                    fallback_turn_context.sub_id.as_str(),
                    compaction_id.as_str(),
                    fallback_turn_context.model_info.slug.as_str(),
                    fallback_turn_context.provider.info().name.as_str(),
                );
            let fallback_result = run_remote_compact_v2_attempt(
                sess,
                fallback_step_context,
                client_session,
                &fallback_compaction_trace,
                compaction_metadata,
                analytics_details,
            )
            .await;
            record_model_fallback(
                &sess.services.session_telemetry,
                turn_context.model_info.slug.as_str(),
                fallback_turn_context.model_info.slug.as_str(),
                compaction_metadata.reason(),
                compaction_metadata.implementation(),
                fallback_result.as_ref().err(),
            );
            match fallback_result {
                Ok(attempt) => (attempt, fallback_turn_context, fallback_compaction_trace),
                Err(_) => return Err(error),
            }
        }
    };
    let RemoteCompactV2Attempt {
        trace_input_history,
        compaction_output,
        token_usage,
        owned_client_session: _owned_client_session,
    } = attempt;
    if let Some(token_usage) = token_usage {
        sess.record_rollout_budget_usage(&token_usage)?;
        analytics_details.active_context_tokens_before = Some(token_usage.input_tokens);
        analytics_details.compaction_summary_tokens = Some(token_usage.output_tokens);
        analytics_details.cached_input_tokens = Some(token_usage.cached_input_tokens);
        analytics_details.cache_write_input_tokens = Some(token_usage.cache_write_input_tokens);
    }
    analytics_details.retained_image_count = Some(0);
    let projection_turn_context = &projection_step_context.turn;
    let world_state_baseline = Arc::new(
        sess.build_world_state_for_step(projection_step_context)
            .await?,
    );
    let mut new_history = vec![ResponseItemEnvelope::new(compaction_output)];
    new_history.extend(
        sess.build_initial_context_with_world_state(
            projection_turn_context.as_ref(),
            world_state_baseline.as_ref(),
        )
        .await
        .into_iter()
        .map(ResponseItemEnvelope::new),
    );
    let (new_window_number, new_window_ids) = sess.advance_auto_compact_window().await;
    let reference_context_item = Some(projection_turn_context.to_turn_context_item());
    let continuity = compaction_continuity(
        compaction_id,
        compaction_metadata,
        compaction_turn_context.as_ref(),
        true,
        true,
    );
    let installed = sess
        .replace_compacted_history(
            new_history,
            reference_context_item,
            Some(world_state_baseline),
            CompactedHistoryMetadata {
                message: String::new(),
                window_number: new_window_number,
                window_ids: new_window_ids,
                continuity: Some(continuity),
            },
        )
        .await;
    record_installed_compaction(
        &selected_compaction_trace,
        trace_input_history.as_deref(),
        &installed,
    );
    sess.recompute_token_usage(projection_turn_context).await;

    sess.emit_turn_item_completed(compaction_turn_context, compaction_item)
        .await;
    Ok(())
}

struct RemoteCompactionV2Output {
    compaction_output: ResponseItem,
    response_id: String,
    token_usage: Option<TokenUsage>,
}

async fn run_remote_compaction_request_v2(
    sess: &Session,
    turn_context: &TurnContext,
    client_session: &mut ModelClientSession,
    prompt: &Prompt,
    responses_metadata: &CodexResponsesMetadata,
    compaction_trace: &codex_rollout_trace::CompactionTraceContext,
) -> CodexResult<RemoteCompactionV2Output> {
    let max_retries = turn_context
        .provider
        .info()
        .stream_max_retries()
        .min(MAX_REMOTE_COMPACTION_V2_STREAM_RETRIES);
    let mut retry_state = ResponsesStreamRetryState::default();
    loop {
        let result = match client_session
            .stream_compaction(
                crate::PromptInvocationKind::RemoteCompaction,
                prompt,
                &turn_context.model_info,
                &turn_context.session_telemetry,
                turn_context.reasoning_effort.clone(),
                turn_context.reasoning_summary,
                turn_context.config.service_tier.clone(),
                responses_metadata,
                compaction_trace,
            )
            .await
        {
            Ok(stream) => collect_compaction_output(stream).await,
            Err(err) => Err(err),
        };

        match result {
            Ok(compaction_output) => return Ok(compaction_output),
            Err(err) if !err.is_retryable() => return Err(err),
            Err(err) => {
                handle_retryable_response_stream_error(
                    &mut retry_state,
                    max_retries,
                    err,
                    client_session,
                    sess,
                    turn_context,
                    ResponsesStreamRequest::RemoteCompactionV2,
                )
                .await?;
            }
        }
    }
}

async fn collect_compaction_output(
    mut stream: ResponseStream,
) -> CodexResult<RemoteCompactionV2Output> {
    let mut output_item_count = 0usize;
    let mut compaction_count = 0usize;
    let mut compaction_output = None;
    let mut saw_completed = false;
    let mut completed_response_id = None;
    let mut completed_token_usage = None;
    while let Some(event) = stream.next().await {
        match event? {
            ResponseEvent::OutputItemDone(item) => {
                output_item_count += 1;
                if let ResponseItem::Compaction { .. } = item {
                    compaction_count += 1;
                    if compaction_output.is_none() {
                        compaction_output = Some(item);
                    }
                }
            }
            ResponseEvent::Completed {
                response_id,
                token_usage,
                ..
            } => {
                saw_completed = true;
                completed_response_id = Some(response_id);
                completed_token_usage = token_usage;
                break;
            }
            _ => {}
        }
    }

    if !saw_completed {
        return Err(CodexErr::Stream(
            "remote compaction v2 stream closed before response.completed".to_string(),
        ));
    }

    if compaction_count != 1 {
        return Err(CodexErr::Fatal(format!(
            "remote compaction v2 expected exactly one compaction output item, got {compaction_count} from {output_item_count} output items"
        )));
    }

    let Some(compaction_output) = compaction_output else {
        unreachable!("compaction output must exist when count is exactly one");
    };
    let Some(response_id) = completed_response_id else {
        unreachable!("response id must exist after response.completed");
    };
    Ok(RemoteCompactionV2Output {
        compaction_output,
        response_id,
        token_usage: completed_token_usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::MessagePhase;
    use pretty_assertions::assert_eq;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn message(role: &str, text: &str, phase: Option<MessagePhase>) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: role.to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    fn response_stream(events: Vec<CodexResult<ResponseEvent>>) -> ResponseStream {
        let (tx_event, rx_event) = mpsc::channel(events.len().max(1));
        for event in events {
            tx_event
                .try_send(event)
                .expect("response stream test channel should have capacity");
        }
        drop(tx_event);
        ResponseStream {
            rx_event,
            consumer_dropped: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn collect_compaction_output_accepts_additional_output_items() {
        let compaction = ResponseItem::Compaction {
            id: None,
            encrypted_content: "encrypted".to_string(),
            internal_chat_message_metadata_passthrough: None,
        };
        let stream = response_stream(vec![
            Ok(ResponseEvent::OutputItemDone(message(
                "assistant",
                "IGNORED_COMPACT_REPLY",
                Some(MessagePhase::FinalAnswer),
            ))),
            Ok(ResponseEvent::OutputItemDone(compaction.clone())),
            Ok(ResponseEvent::Completed {
                response_id: "resp-compact".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: 123_456,
                    cached_input_tokens: 7_890,
                    cache_write_input_tokens: 0,
                    output_tokens: 42,
                    reasoning_output_tokens: 5,
                    total_tokens: 123_498,
                    codex_rollout_budget_units: None,
                }),
                end_turn: Some(true),
            }),
        ]);

        let output = collect_compaction_output(stream)
            .await
            .expect("compaction should be collected");

        assert_eq!(output.compaction_output, compaction);
        assert_eq!(output.response_id, "resp-compact");
        assert_eq!(
            output.token_usage,
            Some(TokenUsage {
                input_tokens: 123_456,
                cached_input_tokens: 7_890,
                cache_write_input_tokens: 0,
                output_tokens: 42,
                reasoning_output_tokens: 5,
                total_tokens: 123_498,
                codex_rollout_budget_units: None,
            })
        );
    }
}
