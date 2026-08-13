use std::collections::VecDeque;
use std::sync::Arc;

use super::TRAILING_OUTPUT_GRACE;
use super::spawn_exit_watcher;
use super::split_valid_utf8_prefix_with_max;
use super::start_streaming_output;
use crate::context::ContextualUserFragment;
use crate::session::tests::make_session_and_context_with_rx;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::head_tail_buffer::HeadTailBuffer;
use crate::unified_exec::process::NoopSpawnLifecycle;
use crate::unified_exec::process::UnifiedExecProcess;
use codex_features::Feature;
use codex_protocol::items::CommandExecutionStatus;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_sandboxing::SandboxType;
use pretty_assertions::assert_eq;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::timeout;

struct StreamingOutputHarness {
    process: Arc<UnifiedExecProcess>,
    stdout_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    exit_tx: tokio::sync::oneshot::Sender<i32>,
    transcript: Arc<tokio::sync::Mutex<HeadTailBuffer>>,
    context: UnifiedExecContext,
    rx_event: async_channel::Receiver<Event>,
}

async fn streaming_output_harness() -> anyhow::Result<StreamingOutputHarness> {
    let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    let (stdout_tx, stdout_rx) = tokio::sync::broadcast::channel::<Vec<u8>>(8);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<i32>();
    let spawned = codex_utils_pty::spawn_from_driver(codex_utils_pty::ProcessDriver {
        writer_tx,
        stdout_rx,
        stderr_rx: None,
        exit_rx,
        terminator: None,
        writer_handle: None,
        resizer: None,
        #[cfg(windows)]
        tty: false,
    });
    let process = Arc::new(
        UnifiedExecProcess::from_spawned(spawned, SandboxType::None, Box::new(NoopSpawnLifecycle))
            .await?,
    );
    let (session, turn, rx_event) = make_session_and_context_with_rx().await;
    let context = UnifiedExecContext::new(
        session,
        crate::session::step_context::StepContext::for_test(turn),
        "streaming-output-test".to_string(),
    );
    let transcript = Arc::new(tokio::sync::Mutex::new(HeadTailBuffer::default()));
    start_streaming_output(&process, &context, Arc::clone(&transcript));

    Ok(StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        context,
        rx_event,
    })
}

#[tokio::test]
async fn streaming_output_finishes_on_close_without_waiting_for_grace() -> anyhow::Result<()> {
    let StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        ..
    } = streaming_output_harness().await?;
    let output_drained = process.output_drained_notify();
    let drained = output_drained.notified();
    tokio::pin!(drained);

    tokio::time::pause();
    let exited_at = Instant::now();
    exit_tx.send(0).expect("send exit");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        stdout_tx
            .send(b"LATE-OUTPUT-MARKER".to_vec())
            .expect("send late output");
    });

    (&mut drained).await;
    let elapsed = Instant::now().saturating_duration_since(exited_at);
    tokio::time::resume();

    assert!(
        elapsed >= Duration::from_millis(50) && elapsed < TRAILING_OUTPUT_GRACE,
        "output close should finish before the grace fallback: {elapsed:?}"
    );
    assert_eq!(
        transcript.lock().await.to_bytes_with_omission_marker(),
        b"LATE-OUTPUT-MARKER"
    );

    Ok(())
}

#[tokio::test]
async fn streaming_output_keeps_grace_as_fallback_without_close() -> anyhow::Result<()> {
    let StreamingOutputHarness {
        process,
        stdout_tx: _stdout_tx,
        exit_tx,
        ..
    } = streaming_output_harness().await?;
    let output_drained = process.output_drained_notify();
    let drained = output_drained.notified();
    tokio::pin!(drained);

    tokio::time::pause();
    let exited_at = Instant::now();
    exit_tx.send(0).expect("send exit");
    (&mut drained).await;
    let elapsed = Instant::now().saturating_duration_since(exited_at);
    tokio::time::resume();

    assert!(
        elapsed >= TRAILING_OUTPUT_GRACE
            && elapsed <= TRAILING_OUTPUT_GRACE + Duration::from_millis(10),
        "missing output close should use the grace fallback: {elapsed:?}"
    );

    Ok(())
}

#[tokio::test]
async fn exit_watcher_waits_for_late_network_denial_before_classifying_end() -> anyhow::Result<()> {
    let StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        context,
        rx_event,
    } = streaming_output_harness().await?;

    tokio::time::pause();
    let process_for_late_denial = Arc::clone(&process);
    let (late_denial_armed_tx, late_denial_armed_rx) = tokio::sync::oneshot::channel();
    let network_denial_monitor = tokio::spawn(async move {
        let sleep = tokio::time::sleep(Duration::from_millis(10));
        tokio::pin!(sleep);
        late_denial_armed_tx.send(()).expect("arm late denial");
        sleep.await;
        process_for_late_denial.fail_and_terminate("LATE_DENIAL".to_string());
    });
    late_denial_armed_rx.await.expect("late denial armed");

    #[allow(deprecated)]
    let cwd = context.step_context.turn.cwd.clone().into();
    spawn_exit_watcher(
        Arc::clone(&process),
        Arc::clone(&context.session),
        Arc::clone(&context.step_context.turn),
        context.call_id,
        vec!["proof".to_string()],
        cwd,
        /*process_id*/ 123,
        /*plugin_attribution*/ None,
        transcript,
        Instant::now(),
        Some(network_denial_monitor),
        /*plugin_metrics_sidecar*/ None,
    );

    let exited_at = Instant::now();
    exit_tx.send(0).expect("send exit");
    drop(stdout_tx);

    let event = rx_event.recv().await.expect("command end event");
    let elapsed = Instant::now().saturating_duration_since(exited_at);
    tokio::time::resume();
    let EventMsg::ItemCompleted(completed) = event.msg else {
        panic!("expected ItemCompleted");
    };
    let TurnItem::CommandExecution(item) = completed.item else {
        panic!("expected CommandExecution");
    };
    assert_eq!(
        (
            item.status,
            item.exit_code,
            item.aggregated_output.as_deref()
        ),
        (
            CommandExecutionStatus::Failed,
            Some(-1),
            Some("LATE_DENIAL")
        )
    );
    assert!(
        elapsed >= Duration::from_millis(10) && elapsed < TRAILING_OUTPUT_GRACE,
        "completion should wait for denial without falling back to the output grace: {elapsed:?}"
    );

    Ok(())
}

#[test]
fn split_valid_utf8_prefix_respects_max_bytes_for_ascii() {
    let mut buf = VecDeque::from(b"hello word!".to_vec());

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 5).expect("expected prefix");
    assert_eq!(first, b"hello".to_vec());
    assert_eq!(buf, VecDeque::from(b" word!".to_vec()));

    let second =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 5).expect("expected prefix");
    assert_eq!(second, b" word".to_vec());
    assert_eq!(buf, VecDeque::from(b"!".to_vec()));
}

#[test]
fn split_valid_utf8_prefix_avoids_splitting_utf8_codepoints() {
    // "é" is 2 bytes in UTF-8. With a max of 3 bytes, we should only emit 1 char (2 bytes).
    let mut buf = VecDeque::from("ééé".as_bytes().to_vec());

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 3).expect("expected prefix");
    assert_eq!(std::str::from_utf8(&first).unwrap(), "é");
    assert_eq!(buf, VecDeque::from("éé".as_bytes().to_vec()));
}

#[test]
fn split_valid_utf8_prefix_makes_progress_on_invalid_utf8() {
    let mut buf = VecDeque::from(vec![0xff, b'a', b'b']);

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 2).expect("expected prefix");
    assert_eq!(first, vec![0xff]);
    assert_eq!(buf, VecDeque::from(b"ab".to_vec()));
}

#[test]
fn split_valid_utf8_prefix_consumes_all_valid_bytes_before_invalid_utf8() {
    let mut bytes = vec![b'a'; 4096];
    bytes.push(0xff);
    bytes.extend(vec![b'b'; 4096]);
    let mut buf = VecDeque::from(bytes);

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 8192).expect("expected prefix");
    assert_eq!(first, vec![b'a'; 4096]);

    let second =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 8192).expect("expected prefix");
    assert_eq!(second, vec![0xff]);

    let third =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 8192).expect("expected prefix");
    assert_eq!(third, vec![b'b'; 4096]);
    assert!(buf.is_empty());
}

#[test]
fn split_invalid_utf8_advances_without_shifting_remaining_bytes() {
    let mut buf = VecDeque::from(vec![0xff; 1024]);
    let initial = buf.as_slices().0.as_ptr();

    for offset in 0..1024 {
        assert_eq!(
            split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 128),
            Some(vec![0xff])
        );
        if let Some(first) = buf.as_slices().0.first() {
            assert_eq!(first, &0xff);
            assert_eq!(buf.as_slices().0.as_ptr(), initial.wrapping_add(offset + 1));
        }
    }

    assert!(buf.is_empty());
}

/// Build a session with the completion-wake feature enabled and a live
/// internal-event channel, plus a fake PTY harness. Returns the internal event
/// receiver so tests can observe exactly-once admission.
async fn watcher_test_session_with_internal_channel() -> anyhow::Result<(
    Arc<crate::session::session::Session>,
    async_channel::Sender<crate::session::SessionIngress>,
    async_channel::Receiver<crate::session::SessionIngress>,
    StreamingOutputHarness,
)> {
    let (mut session, _turn, _rx) = make_session_and_context_with_rx().await;
    let (ingress_tx, ingress_rx) = async_channel::bounded(64);
    Arc::get_mut(&mut session)
        .expect("session must be uniquely held")
        .enable_feature_for_tests(Feature::UnifiedExecCompletionWake);
    Arc::get_mut(&mut session)
        .expect("session must be uniquely held")
        .internal_session_event_tx = ingress_tx.downgrade();
    let harness = streaming_output_harness().await?;
    Ok((session, ingress_tx, ingress_rx, harness))
}

fn completion_fragment_text(ingress: &crate::session::SessionIngress) -> String {
    let crate::session::SessionIngress::Internal(
        crate::session::internal_event::InternalSessionEvent::UnifiedExecCompletion(completion),
    ) = ingress
    else {
        panic!("expected an internal completion event");
    };
    completion.render()
}

async fn expect_no_internal_completion(
    ingress_rx: &mut async_channel::Receiver<crate::session::SessionIngress>,
) {
    assert!(
        timeout(Duration::from_millis(300), ingress_rx.recv())
            .await
            .is_err(),
        "no automatic completion event expected"
    );
}

#[tokio::test]
async fn exit_watcher_sends_completion_event_once_for_background_exit() -> anyhow::Result<()> {
    let (session, _ingress_tx, mut internal_rx, harness) =
        watcher_test_session_with_internal_channel().await?;
    let StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        context,
        rx_event: _,
    } = harness;

    #[allow(deprecated)]
    let cwd = context.step_context.turn.cwd.clone().into();
    spawn_exit_watcher(
        Arc::clone(&process),
        Arc::clone(&session),
        Arc::clone(&context.step_context.turn),
        context.call_id,
        vec!["sleep".to_string(), "10".to_string()],
        cwd,
        /*process_id*/ 4242,
        /*plugin_attribution*/ None,
        transcript,
        Instant::now(),
        /*network_denial_monitor*/ None,
        /*plugin_metrics_sidecar*/ None,
    );

    exit_tx.send(0).expect("send exit");
    drop(stdout_tx);

    let event = timeout(Duration::from_secs(5), internal_rx.recv())
        .await
        .expect("completion event should be sent by the exit watcher")
        .expect("internal channel should stay open");
    let rendered = completion_fragment_text(&event);
    assert!(rendered.contains("4242"));
    assert!(rendered.contains("exit code 0"));
    assert!(rendered.contains("write_stdin"));
    assert!(rendered.len() <= 16 * 1024, "fragment must stay bounded");

    // Exactly-once: a second watcher claim must not produce a second event.
    expect_no_internal_completion(&mut internal_rx).await;
    Ok(())
}

#[tokio::test]
async fn exit_watcher_skips_completion_when_exit_observed_synchronously() -> anyhow::Result<()> {
    let (session, _ingress_tx, mut internal_rx, harness) =
        watcher_test_session_with_internal_channel().await?;
    // Simulate a write_stdin poll (or the initial exec_command) having already
    // returned the exit to the model: removal from the process store records
    // the completion as observed.
    harness.process.record_completion_observed();
    let StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        context,
        rx_event: _,
    } = harness;

    #[allow(deprecated)]
    let cwd = context.step_context.turn.cwd.clone().into();
    spawn_exit_watcher(
        Arc::clone(&process),
        Arc::clone(&session),
        Arc::clone(&context.step_context.turn),
        context.call_id,
        vec!["sleep".to_string(), "10".to_string()],
        cwd,
        /*process_id*/ 4243,
        /*plugin_attribution*/ None,
        transcript,
        Instant::now(),
        /*network_denial_monitor*/ None,
        /*plugin_metrics_sidecar*/ None,
    );

    exit_tx.send(0).expect("send exit");
    drop(stdout_tx);

    expect_no_internal_completion(&mut internal_rx).await;
    Ok(())
}

#[tokio::test]
async fn exit_watcher_waits_for_initial_exec_command_observation_lock() -> anyhow::Result<()> {
    let (session, _ingress_tx, mut internal_rx, harness) =
        watcher_test_session_with_internal_channel().await?;
    let StreamingOutputHarness {
        process,
        stdout_tx,
        exit_tx,
        transcript,
        context,
        rx_event: _,
    } = harness;

    // The initial exec_command holds the process interaction lock through its
    // synchronous observation; the watcher waits on that lock and can only
    // claim the completion after the lock is released.
    let initial_observation_lock = process.interaction_lock().lock_owned().await;

    #[allow(deprecated)]
    let cwd = context.step_context.turn.cwd.clone().into();
    spawn_exit_watcher(
        Arc::clone(&process),
        Arc::clone(&session),
        Arc::clone(&context.step_context.turn),
        context.call_id,
        vec!["sleep".to_string(), "10".to_string()],
        cwd,
        /*process_id*/ 4244,
        /*plugin_attribution*/ None,
        transcript,
        Instant::now(),
        /*network_denial_monitor*/ None,
        /*plugin_metrics_sidecar*/ None,
    );

    exit_tx.send(0).expect("send exit");
    drop(stdout_tx);
    // While the initial call is still in flight (holding the lock), the
    // watcher must not send a completion event.
    expect_no_internal_completion(&mut internal_rx).await;

    // The initial call returns Alive without observing the exit: releasing the
    // lock lets the watcher claim the background completion.
    drop(initial_observation_lock);
    let event = timeout(Duration::from_secs(5), internal_rx.recv())
        .await
        .expect("completion should be sent after the initial call settles")
        .expect("internal channel should stay open");
    assert!(completion_fragment_text(&event).contains("4244"));
    expect_no_internal_completion(&mut internal_rx).await;
    Ok(())
}

#[tokio::test]
async fn cleanup_marking_suppresses_completion_event() -> anyhow::Result<()> {
    let (_session, _ingress_tx, mut internal_rx, harness) =
        watcher_test_session_with_internal_channel().await?;
    // Session shutdown / CleanBackgroundTerminals marks entries observed before
    // terminating them; the watcher must not emit an automatic completion.
    harness.process.record_completion_observed();
    harness.process.terminate();
    expect_no_internal_completion(&mut internal_rx).await;
    Ok(())
}
