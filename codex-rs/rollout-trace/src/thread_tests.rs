use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use tempfile::TempDir;

use super::*;
use crate::AgentResultTracePayload;
use crate::CompactionCheckpointTracePayload;
use crate::CompactionContinuityTracePayload;
use crate::ExecutionStatus;
use crate::RawTraceEventPayload;
use crate::RolloutStatus;
use crate::replay_bundle;

#[test]
fn create_in_root_writes_replayable_lifecycle_events() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let thread_id = ThreadId::new();
    let thread_trace = ThreadTraceContext::start_root_in_root_for_test(
        temp.path(),
        ThreadStartedTraceMetadata {
            thread_id: thread_id.to_string(),
            agent_path: "/root".to_string(),
            task_name: None,
            nickname: None,
            agent_role: None,
            session_source: SessionSource::Exec,
            cwd: PathBuf::from("/workspace"),
            rollout_path: Some(PathBuf::from("/tmp/rollout.jsonl")),
            model: "gpt-test".to_string(),
            provider_name: "test-provider".to_string(),
            approval_policy: "never".to_string(),
            sandbox_policy: format!("{:?}", SandboxPolicy::DangerFullAccess),
        },
    )?;

    thread_trace.record_ended(RolloutStatus::Completed);

    let bundle_dir = single_bundle_dir(temp.path())?;
    let replayed = replay_bundle(&bundle_dir)?;

    assert_eq!(replayed.status, RolloutStatus::Completed);
    assert_eq!(replayed.root_thread_id, thread_id.to_string());
    assert_eq!(replayed.threads[&thread_id.to_string()].agent_path, "/root");
    assert_eq!(replayed.raw_payloads.len(), 1);

    Ok(())
}

#[test]
fn spawned_thread_start_appends_to_root_bundle() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let root_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let root_trace = ThreadTraceContext::start_root_in_root_for_test(
        temp.path(),
        minimal_metadata(root_thread_id),
    )?;

    let child_trace = root_trace.start_child_thread_trace_or_disabled(ThreadStartedTraceMetadata {
        thread_id: child_thread_id.to_string(),
        agent_path: "/root/repo_file_counter".to_string(),
        task_name: Some("repo_file_counter".to_string()),
        nickname: Some("Kepler".to_string()),
        agent_role: Some("worker".to_string()),
        session_source: SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: root_thread_id,
            depth: 1,
            agent_path: Some(
                AgentPath::try_from("/root/repo_file_counter").map_err(anyhow::Error::msg)?,
            ),
            agent_nickname: Some("Kepler".to_string()),
            agent_role: Some("worker".to_string()),
        }),
        cwd: PathBuf::from("/workspace"),
        rollout_path: Some(PathBuf::from("/tmp/child-rollout.jsonl")),
        model: "gpt-test".to_string(),
        provider_name: "test-provider".to_string(),
        approval_policy: "never".to_string(),
        sandbox_policy: format!("{:?}", SandboxPolicy::DangerFullAccess),
    });
    child_trace.record_ended(RolloutStatus::Completed);
    let bundle_dir = single_bundle_dir(temp.path())?;
    let replayed = replay_bundle(&bundle_dir)?;

    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    assert_eq!(replayed.threads.len(), 2);
    assert_eq!(
        replayed.threads[&child_thread_id.to_string()].agent_path,
        "/root/repo_file_counter"
    );
    assert_eq!(replayed.status, RolloutStatus::Running);
    assert_eq!(
        replayed.threads[&child_thread_id.to_string()]
            .execution
            .status,
        ExecutionStatus::Completed
    );
    assert_eq!(replayed.raw_payloads.len(), 2);

    Ok(())
}

#[test]
fn disabled_thread_context_accepts_trace_calls_without_writing() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let thread_trace = ThreadTraceContext::disabled();

    thread_trace.record_ended(RolloutStatus::Completed);
    thread_trace.record_protocol_event(&EventMsg::ShutdownComplete);
    thread_trace.record_codex_turn_event("turn-1", &EventMsg::ShutdownComplete);
    thread_trace.record_tool_call_event("turn-1", &EventMsg::ShutdownComplete);
    thread_trace.record_agent_result_interaction(
        "turn-1",
        ThreadId::new(),
        &AgentResultTracePayload {
            child_agent_path: "/root/child",
            message: "done",
            status: &AgentStatus::Completed(Some("done".to_string())),
        },
    );

    let inference_trace =
        thread_trace.inference_trace_context("turn-1", "gpt-test", "test-provider");
    let inference_attempt = inference_trace.start_attempt();
    inference_attempt.record_started(&serde_json::json!({ "kind": "inference" }));
    let token_usage: Option<codex_protocol::protocol::TokenUsage> = None;
    inference_attempt.record_completed("response-1", Some("req-1"), &token_usage, &[]);
    inference_attempt.record_failed("inference failed", /*upstream_request_id*/ None, &[]);

    let compaction_trace = thread_trace.compaction_trace_context(
        "turn-1",
        "compaction-1",
        "gpt-test",
        "test-provider",
    );
    assert!(!compaction_trace.is_enabled());
    let compaction_attempt =
        compaction_trace.start_attempt(&serde_json::json!({ "kind": "compaction" }));
    compaction_attempt.record_completed(&[]);
    compaction_attempt.record_failed("compaction failed");
    compaction_trace.record_installed(&CompactionCheckpointTracePayload {
        input_history: &[],
        replacement_history: &[],
        continuity: CompactionContinuityTracePayload {
            trigger: "manual",
            reason: "user_requested",
            implementation: "responses_compaction_v2",
            phase: "standalone_turn",
            model: "gpt-test",
            provider: "test-provider",
            reference_context_installed: false,
            world_state_baseline_installed: false,
        },
        window_number: Some(1),
        first_window_id: Some("first-window"),
        previous_window_id: Some("previous-window"),
        window_id: Some("current-window"),
    });

    let built_dispatch_invocation = Cell::new(false);
    let dispatch_trace = thread_trace.start_tool_dispatch_trace(|| {
        built_dispatch_invocation.set(true);
        None
    });
    assert!(!built_dispatch_invocation.get());
    assert!(!dispatch_trace.is_enabled());

    assert_eq!(fs::read_dir(temp.path())?.count(), 0);

    Ok(())
}

#[test]
fn compaction_contexts_share_identity_across_models() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let thread_id = ThreadId::new();
    let thread_trace =
        ThreadTraceContext::start_root_in_root_for_test(temp.path(), minimal_metadata(thread_id))?;
    thread_trace.record_codex_turn_started("turn-1");

    for model in ["gpt-previous", "gpt-selected"] {
        let compaction_trace =
            thread_trace.compaction_trace_context("turn-1", "compaction-1", model, "test-provider");
        assert!(compaction_trace.is_enabled());
        compaction_trace
            .start_attempt(&serde_json::json!({ "model": model }))
            .record_failed("test failure");
    }

    let replayed = replay_bundle(&single_bundle_dir(temp.path())?)?;
    let mut attempts = replayed
        .compaction_requests
        .values()
        .map(|attempt| (attempt.model.clone(), attempt.compaction_id.clone()))
        .collect::<Vec<_>>();
    attempts.sort();
    assert_eq!(
        attempts,
        vec![
            ("gpt-previous".to_string(), "compaction-1".to_string()),
            ("gpt-selected".to_string(), "compaction-1".to_string()),
        ]
    );

    Ok(())
}

#[test]
fn local_compaction_trace_links_request_install_and_continuation() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let thread_id = ThreadId::new();
    let thread_id_string = thread_id.to_string();
    let thread_trace =
        ThreadTraceContext::start_root_in_root_for_test(temp.path(), minimal_metadata(thread_id))?;
    thread_trace.record_codex_turn_started("compact-turn");

    let user = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "history to compact".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    let summary = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "compacted summary".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    let compaction_trace = thread_trace.compaction_trace_context(
        "compact-turn",
        "compaction-1",
        "gpt-test",
        "test-provider",
    );
    compaction_trace
        .start_attempt(&serde_json::json!({
            "model": "gpt-test",
            "input": [&user],
            "instructions": "compact locally"
        }))
        .record_stream_completed("local-response-1", &[]);
    compaction_trace.record_installed(&CompactionCheckpointTracePayload {
        input_history: std::slice::from_ref(&user),
        replacement_history: std::slice::from_ref(&summary),
        continuity: CompactionContinuityTracePayload {
            trigger: "manual",
            reason: "user_requested",
            implementation: "responses",
            phase: "standalone_turn",
            model: "gpt-test",
            provider: "test-provider",
            reference_context_installed: false,
            world_state_baseline_installed: false,
        },
        window_number: Some(1),
        first_window_id: Some("first-window"),
        previous_window_id: Some("previous-window"),
        window_id: Some("current-window"),
    });

    thread_trace.record_codex_turn_started("continuation-turn");
    thread_trace
        .inference_trace_context("continuation-turn", "gpt-test", "test-provider")
        .start_attempt()
        .record_started(&serde_json::json!({
            "model": "gpt-test",
            "input": [&summary]
        }));

    let rollout = replay_bundle(&single_bundle_dir(temp.path())?)?;
    let compaction = &rollout.compactions["compaction-1"];
    let request_id = compaction
        .request_ids
        .first()
        .expect("local compaction request");
    let request = &rollout.compaction_requests[request_id];
    let continuation_id = compaction
        .continuation_inference_call_id
        .as_ref()
        .expect("continuation inference");
    let continuation = &rollout.inference_calls[continuation_id];
    assert_eq!(
        (
            request.compaction_id.as_str(),
            request.execution.status.clone(),
            request.response_id.as_deref(),
            compaction.implementation.as_deref(),
            compaction.thread_id.as_str(),
            continuation.request_item_ids.as_slice(),
        ),
        (
            "compaction-1",
            ExecutionStatus::Completed,
            Some("local-response-1"),
            Some("responses"),
            thread_id_string.as_str(),
            compaction.replacement_item_ids.as_slice(),
        )
    );

    Ok(())
}

#[test]
fn protocol_wrapper_records_selected_events_as_raw_payloads() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let thread_id = ThreadId::new();
    let thread_trace =
        ThreadTraceContext::start_root_in_root_for_test(temp.path(), minimal_metadata(thread_id))?;

    thread_trace.record_protocol_event(&EventMsg::ShutdownComplete);

    let event_log = fs::read_to_string(single_bundle_dir(temp.path())?.join("trace.jsonl"))?;
    let protocol_event_seen = event_log.lines().any(|line| {
        let event: crate::RawTraceEvent = serde_json::from_str(line).expect("raw trace event");
        matches!(
            event.payload,
            RawTraceEventPayload::ProtocolEventObserved {
                event_type,
                ..
            } if event_type == "shutdown_complete"
        )
    });

    assert!(protocol_event_seen);
    Ok(())
}

fn minimal_metadata(thread_id: ThreadId) -> ThreadStartedTraceMetadata {
    ThreadStartedTraceMetadata {
        thread_id: thread_id.to_string(),
        agent_path: "/root".to_string(),
        task_name: None,
        nickname: None,
        agent_role: None,
        session_source: SessionSource::Exec,
        cwd: PathBuf::from("/workspace"),
        rollout_path: None,
        model: "gpt-test".to_string(),
        provider_name: "test-provider".to_string(),
        approval_policy: "never".to_string(),
        sandbox_policy: "danger-full-access".to_string(),
    }
}

fn single_bundle_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let mut entries = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    assert_eq!(entries.len(), 1);
    Ok(entries.remove(0))
}
