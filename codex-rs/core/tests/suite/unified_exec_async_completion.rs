//! Integration coverage for unified-exec async completion wake: a background
//! terminal process that finishes without a synchronous observation wakes a
//! model turn carrying a bounded completion fragment.

use anyhow::Result;
use codex_features::Feature;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_message_item_added;
use core_test_support::responses::ev_output_text_delta;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_response_sequence;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::skip_if_sandbox;
use core_test_support::skip_if_target_windows;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::wait_for_event_match;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use tokio::time::Duration;
use tokio::time::timeout;
use wiremock::ResponseTemplate;

const COMPLETION_MARKER: &str = "<unified_exec_completion>";
const NO_FINISH_MARKER: &str = "no-finish";
const OUTPUT_AVAILABLE_MARKER: &str = "<unified_exec_output_available>";

fn completion_feature_config(config: &mut codex_core::config::Config) {
    config.use_experimental_unified_exec_tool = true;
    config
        .features
        .enable(Feature::UnifiedExec)
        .expect("test config should allow feature update");
    config
        .features
        .enable(Feature::UnifiedExecCompletionWake)
        .expect("test config should allow feature update");
}

fn unified_exec_default_config(config: &mut codex_core::config::Config) {
    config.use_experimental_unified_exec_tool = true;
    config
        .features
        .enable(Feature::UnifiedExec)
        .expect("test config should allow feature update");
}

async fn submit_turn(test: &TestCodex, prompt: &str) -> Result<()> {
    let session_model = test.session_configured.model.clone();
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());
    test.codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![UserInput::Text {
                text: prompt.into(),
                text_elements: Vec::new(),
            }])
            .with_thread_settings(
                codex_protocol::protocol::ThreadSettingsOverrides {
                    approval_policy: Some(AskForApproval::Never),
                    sandbox_policy: Some(sandbox_policy),
                    permission_profile,
                    collaboration_mode: Some(codex_protocol::config_types::CollaborationMode {
                        mode: codex_protocol::config_types::ModeKind::Default,
                        settings: codex_protocol::config_types::Settings {
                            model: session_model,
                            reasoning_effort: None,
                            developer_instructions: None,
                        },
                    }),
                    ..Default::default()
                },
            ),
        )
        .await?;
    Ok(())
}

fn request_contains_completion_marker(body: &Value) -> bool {
    request_contains_marker(body, COMPLETION_MARKER)
}

fn no_finish_message_count(body: &Value) -> usize {
    body.get("input")
        .and_then(Value::as_array)
        .map(|input| {
            input
                .iter()
                .filter(|item| {
                    item.get("role").and_then(Value::as_str) == Some("developer")
                        && item
                            .get("content")
                            .and_then(Value::as_array)
                            .is_some_and(|content| {
                                content.as_slice()
                                    == [json!({
                                        "text": NO_FINISH_MARKER,
                                        "type": "input_text",
                                    })]
                            })
                })
                .count()
        })
        .unwrap_or_default()
}

fn request_contains_assistant_text(body: &Value, text: &str) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|input| {
            input.iter().any(|item| {
                item.get("role").and_then(Value::as_str) == Some("assistant")
                    && item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| {
                            content
                                .iter()
                                .any(|part| part.get("text").and_then(Value::as_str) == Some(text))
                        })
            })
        })
}

fn request_contains_combined_assistant_text(body: &Value, text: &str) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|input| {
            input.iter().any(|item| {
                if item.get("role").and_then(Value::as_str) != Some("assistant") {
                    return false;
                }
                let combined = item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>();
                combined == text
            })
        })
}

fn request_contains_marker(body: &Value, marker: &str) -> bool {
    let Some(input) = body.get("input").and_then(Value::as_array) else {
        return false;
    };
    let mut text = String::new();
    for item in input {
        collect_text(item, &mut text);
    }
    text.contains(marker)
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => {
            for item in items {
                collect_text(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_text(item, out);
            }
        }
        _ => {}
    }
}

async fn wait_for_exec_command_begin(test: &TestCodex, call_id: &str) -> Option<String> {
    wait_for_event_match(&test.codex, |event| match event {
        EventMsg::ExecCommandBegin(begin) if begin.call_id == call_id => {
            Some(begin.process_id.clone())
        }
        _ => None,
    })
    .await
}

async fn wait_for_exec_command_end(test: &TestCodex, call_id: &str) {
    wait_for_event_match(&test.codex, |event| match event {
        EventMsg::ExecCommandEnd(end) if end.call_id == call_id => Some(()),
        _ => None,
    })
    .await;
}

async fn wait_for_turn_complete(test: &TestCodex) -> TurnCompleteEvent {
    wait_for_event_match(&test.codex, |event| match event {
        EventMsg::TurnComplete(event) => Some(event.clone()),
        _ => None,
    })
    .await
}

async fn wait_for_raw_response_completed(test: &TestCodex, response_id: &str) {
    wait_for_event_match(&test.codex, |event| match event {
        EventMsg::RawResponseCompleted(event) if event.response_id == response_id => Some(()),
        _ => None,
    })
    .await;
}

async fn wait_for_merged_commentary(
    test: &TestCodex,
    response_id: &str,
    expected_item_id: &str,
    expected_message: &str,
) {
    let mut completed_count = 0;
    loop {
        let event = test
            .codex
            .next_event()
            .await
            .expect("event stream should remain open");
        match event.msg {
            EventMsg::AgentMessageContentDelta(_) => {
                panic!("constrained assistant text must be held until its phase is final")
            }
            EventMsg::ItemCompleted(event) => {
                if let TurnItem::AgentMessage(item) = event.item {
                    let combined = item
                        .content
                        .iter()
                        .map(|content| match content {
                            AgentMessageContent::Text { text } => text.as_str(),
                        })
                        .collect::<String>();
                    assert_eq!(item.id, expected_item_id);
                    assert_eq!(item.phase, Some(MessagePhase::Commentary));
                    assert_eq!(combined, expected_message);
                    completed_count += 1;
                }
            }
            EventMsg::RawResponseCompleted(event) if event.response_id == response_id => {
                assert_eq!(
                    completed_count, 1,
                    "deliver exactly one merged commentary item"
                );
                return;
            }
            _ => {}
        }
    }
}

async fn wait_for_response_before_exec_end(test: &TestCodex, response_id: &str, call_id: &str) {
    loop {
        let event = test
            .codex
            .next_event()
            .await
            .expect("event stream should remain open");
        match event.msg {
            EventMsg::RawResponseCompleted(event) if event.response_id == response_id => return,
            EventMsg::ExecCommandEnd(event) if event.call_id == call_id => {
                panic!("terminal completed before user input was handled")
            }
            _ => {}
        }
    }
}

async fn assert_no_turn_complete_for(test: &TestCodex, duration: Duration) {
    let turn_complete = timeout(duration, async {
        loop {
            let event = test
                .codex
                .next_event()
                .await
                .expect("event stream should remain open");
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                return;
            }
        }
    })
    .await;
    assert!(
        turn_complete.is_err(),
        "turn must remain open while awaited work is live"
    );
}

fn ev_assistant_message_with_phase(id: &str, text: &str, phase: &str) -> Value {
    let mut event = ev_assistant_message(id, text);
    event["item"]["phase"] = Value::String(phase.to_string());
    event
}

fn ev_assistant_message_added_with_phase(id: &str, phase: &str) -> Value {
    let mut event = ev_message_item_added(id, "");
    event["item"]["phase"] = Value::String(phase.to_string());
    event
}

fn delayed_sse(delay: Duration, events: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(sse(events))
        .set_delay(delay)
}

fn sse_template(events: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(sse(events))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commentary_only_response_waits_for_awaited_terminal_and_resumes_same_turn() -> Result<()> {
    // TODO(anp): Remove after unified-exec fixtures use target-native commands.
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(unified_exec_default_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-idle";
    let args = json!({
        "cmd": "sleep 3; printf 'IDLE-DONE'",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message_with_phase(
                "msg-1",
                "I will wait for the background process.",
                "commentary",
            ),
            ev_completed("resp-2"),
        ]),
        // Request 3 resumes the same logical turn after completion.
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message_with_phase(
                "msg-2",
                "The background process finished.",
                "final_answer",
            ),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the long command and wait").await?;

    let turn_id = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::TurnStarted(event) => Some(event.turn_id.clone()),
        _ => None,
    })
    .await;
    let process_id = wait_for_exec_command_begin(&test, call_id)
        .await
        .expect("begin event should carry a process id for a long-lived session");
    wait_for_raw_response_completed(&test, "resp-2").await;
    assert_eq!(test.codex.awaited_background_terminal_count().await, 1);
    assert_eq!(
        test.codex.agent_status().await,
        AgentStatus::Running,
        "the active turn remains running while its terminal is awaited"
    );
    assert_no_turn_complete_for(&test, Duration::from_millis(500)).await;

    wait_for_exec_command_end(&test, call_id).await;
    let completed = wait_for_turn_complete(&test).await;
    assert_eq!(completed.turn_id, turn_id);
    assert_eq!(
        completed.last_agent_message.as_deref(),
        Some("The background process finished.")
    );
    assert!(matches!(
        test.codex.agent_status().await,
        AgentStatus::Completed(_)
    ));

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3, "exactly one resumed model poll");
    let second = requests[1].body_json();
    let third = requests[2].body_json();
    assert!(
        !request_contains_completion_marker(&second),
        "the first turn must not observe the completion"
    );
    assert!(
        request_contains_completion_marker(&third),
        "the resumed turn must sample the completion fragment"
    );
    assert!(
        request_contains_assistant_text(&third, "I will wait for the background process."),
        "explicit commentary must remain in history while no-finish is active"
    );
    let third_text = serde_json::to_string(&third)?;
    assert!(
        third_text.contains(&format!("process_id\\\":{process_id}")),
        "completion fragment should carry the stable process id; expected process_id {process_id}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_input_wakes_awaited_turn_before_terminal_completion() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-user-wakes-awaited-turn";
    let args = json!({
        "cmd": "sleep 4; printf 'USER-WAKE-DONE'",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-user-wake-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-user-wake-1"),
        ]),
        sse(vec![
            ev_response_created("resp-user-wake-2"),
            ev_assistant_message_with_phase(
                "msg-user-wake-1",
                "I am waiting for the terminal.",
                "commentary",
            ),
            ev_completed("resp-user-wake-2"),
        ]),
        sse(vec![
            ev_response_created("resp-user-wake-3"),
            ev_assistant_message_with_phase(
                "msg-user-wake-2",
                "I saw the user message while the terminal was still running.",
                "commentary",
            ),
            ev_completed("resp-user-wake-3"),
        ]),
        sse(vec![
            ev_response_created("resp-user-wake-4"),
            ev_assistant_message_with_phase(
                "msg-user-wake-3",
                "The terminal completion arrived afterward.",
                "final_answer",
            ),
            ev_completed("resp-user-wake-4"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the command and remain responsive").await?;

    let turn_id = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::TurnStarted(event) => Some(event.turn_id.clone()),
        _ => None,
    })
    .await;
    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_raw_response_completed(&test, "resp-user-wake-2").await;
    assert_eq!(test.codex.awaited_background_terminal_count().await, 1);

    submit_turn(&test, "USER_MESSAGE_DURING_AWAITED_TERMINAL").await?;
    wait_for_response_before_exec_end(&test, "resp-user-wake-3", call_id).await;
    assert_eq!(
        test.codex.awaited_background_terminal_count().await,
        1,
        "user input must be handled without resolving the terminal"
    );

    wait_for_exec_command_end(&test, call_id).await;
    let completed = wait_for_turn_complete(&test).await;
    assert_eq!(completed.turn_id, turn_id);
    assert_eq!(
        completed.last_agent_message.as_deref(),
        Some("The terminal completion arrived afterward.")
    );

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 4, "one user wake and one completion wake");
    let user_wake_request = requests[2].body_json();
    let completion_request = requests[3].body_json();
    assert!(
        serde_json::to_string(&user_wake_request)?.contains("USER_MESSAGE_DURING_AWAITED_TERMINAL"),
        "the wake before completion must carry the user's message"
    );
    assert!(
        !request_contains_completion_marker(&user_wake_request),
        "the user wake must happen before terminal completion"
    );
    assert_eq!(
        no_finish_message_count(&user_wake_request),
        1,
        "the model must be told not to finish while the terminal remains live"
    );
    assert!(
        request_contains_completion_marker(&completion_request),
        "the later poll must still consume the terminal completion"
    );
    assert_eq!(
        no_finish_message_count(&completion_request),
        0,
        "the constraint must disappear after the terminal closes"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_answer_does_not_complete_turn_while_terminal_is_awaited() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-final-conflict";
    let args = json!({
        "cmd": "sleep 3; printf 'FINAL-CONFLICT-DONE'",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message_with_phase(
                "msg-commentary",
                "I am still waiting for the resource.",
                "commentary",
            ),
            // The completed phase is authoritative even if the added item was mislabeled.
            ev_assistant_message_added_with_phase("msg-1", "commentary"),
            ev_output_text_delta("This final is premature."),
            ev_assistant_message_with_phase("msg-1", "This final is premature.", "final_answer"),
            ev_completed("resp-2"),
        ]),
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message_with_phase(
                "msg-2",
                "The awaited work is now handled.",
                "final_answer",
            ),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the command and do not finish early").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_merged_commentary(
        &test,
        "resp-2",
        "msg-commentary",
        "I am still waiting for the resource.\n\nThis final is premature.",
    )
    .await;
    assert_no_turn_complete_for(&test, Duration::from_millis(500)).await;
    wait_for_exec_command_end(&test, call_id).await;
    let completed = wait_for_turn_complete(&test).await;

    assert_eq!(
        completed.last_agent_message.as_deref(),
        Some("The awaited work is now handled.")
    );
    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        no_finish_message_count(&requests[1].body_json()),
        1,
        "the premature final-answer request must carry the active-resource constraint"
    );
    assert_eq!(
        no_finish_message_count(&requests[2].body_json()),
        0,
        "the completion request must not retain the transient constraint"
    );
    assert!(
        request_contains_combined_assistant_text(
            &requests[2].body_json(),
            "I am still waiting for the resource.\n\nThis final is premature."
        ),
        "useful commentary and final-answer text must survive as one commentary history item"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_completion_sampled_serially_while_turn_busy() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-busy";
    let args = json!({
        "cmd": "sleep 1; printf 'BUSY-DONE'",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse_template(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        // Delay the second sampling request so the process exits while the
        // turn is busy. The completion is admitted to the shared session
        // queue and consumed by the next poll of the same turn.
        delayed_sse(
            Duration::from_secs(4),
            vec![
                ev_response_created("resp-2"),
                ev_assistant_message("msg-1", "still waiting"),
                ev_completed("resp-2"),
            ],
        ),
        sse_template(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-2", "I see the completion now."),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_response_sequence(&server, responses).await;
    submit_turn(&test, "start the long command and keep polling").await?;

    let turn_id = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::TurnStarted(event) => Some(event.turn_id.clone()),
        _ => None,
    })
    .await;
    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_exec_command_end(&test, call_id).await;
    let completed = wait_for_turn_complete(&test).await;
    assert_eq!(completed.turn_id, turn_id);

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        3,
        "the completion is sampled serially by exactly one resumed poll"
    );
    let second = requests[1].body_json();
    let third = requests[2].body_json();
    assert!(
        !request_contains_completion_marker(&second),
        "the in-flight sampling request cannot contain the late completion"
    );
    assert!(
        request_contains_completion_marker(&third),
        "the resumed poll must carry the completion fragment"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_output_wakes_idle_turn_for_stdin() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX shell read fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let start_call_id = "uexec-async-attention-start";
    let write_call_id = "uexec-async-attention-write";
    let start_args = json!({
        "cmd": "sleep 1; printf 'Continue? [y/N] '; read answer; printf 'ANSWER=%s' \"$answer\"",
        "yield_time_ms": 250,
        "tty": true,
    });
    let write_args = json!({
        "chars": "y\n",
        "session_id": 1000,
        "yield_time_ms": 2000,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(
                start_call_id,
                "exec_command",
                &serde_json::to_string(&start_args)?,
            ),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message("msg-1", "I am waiting for the interactive terminal."),
            ev_completed("resp-2"),
        ]),
        sse(vec![
            ev_response_created("resp-3"),
            ev_function_call(
                write_call_id,
                "write_stdin",
                &serde_json::to_string(&write_args)?,
            ),
            ev_completed("resp-3"),
        ]),
        sse(vec![
            ev_response_created("resp-4"),
            ev_assistant_message("msg-2", "The interactive terminal accepted the answer."),
            ev_completed("resp-4"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the interactive command and answer its prompt").await?;

    wait_for_exec_command_begin(&test, start_call_id).await;
    wait_for_exec_command_end(&test, start_call_id).await;
    wait_for_turn_complete(&test).await;

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        4,
        "one attention wake and no duplicate completion wake"
    );
    assert!(
        request_contains_marker(&requests[2].body_json(), OUTPUT_AVAILABLE_MARKER),
        "the idle continuation should receive the interactive-output attention fragment"
    );
    assert!(
        !request_contains_completion_marker(&requests[3].body_json()),
        "write_stdin observed the exit synchronously, so no completion wake is needed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_exit_during_attention_debounce_emits_completion_only() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-attention-exit-race";
    let args = json!({
        "cmd": "sleep 1; printf 'LAST-OUTPUT'; sleep 0.05",
        "yield_time_ms": 250,
        "tty": true,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message("msg-1", "I am waiting."),
            ev_completed("resp-2"),
        ]),
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-2", "The terminal completed."),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "run the interactive command").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_exec_command_end(&test, call_id).await;
    wait_for_turn_complete(&test).await;

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        3,
        "exit should supersede the pending attention wake"
    );
    assert!(request_contains_completion_marker(&requests[2].body_json()));
    assert!(!request_contains_marker(
        &requests[2].body_json(),
        OUTPUT_AVAILABLE_MARKER
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_background_terminals_allows_final_after_downgraded_finish() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-cleanup";
    let args = json!({
        "cmd": "sleep 30",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message("msg-1", "done"),
            ev_completed("resp-2"),
        ]),
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message_with_phase(
                "msg-2",
                "The cleaned-up work is now closed.",
                "final_answer",
            ),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the long command").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_merged_commentary(&test, "resp-2", "msg-1", "done").await;
    assert_no_turn_complete_for(&test, Duration::from_millis(500)).await;
    // Explicitly clean up all background terminals (feature enabled). The
    // downgraded finish leaves an obligation to sample an accepted final after closure.
    test.codex
        .submit(Op::CleanBackgroundTerminals)
        .await
        .expect("clean background terminals submission");
    // The watcher still emits the terminal end item event for the terminated
    // process (existing behavior), then the original turn completes.
    wait_for_exec_command_end(&test, call_id).await;
    let completed = wait_for_turn_complete(&test).await;
    assert_eq!(
        completed.last_agent_message.as_deref(),
        Some("The cleaned-up work is now closed.")
    );
    assert!(matches!(
        test.codex.agent_status().await,
        AgentStatus::Completed(_)
    ));
    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3, "cleanup permits one accepted final poll");
    assert!(
        !request_contains_completion_marker(&requests[1].body_json()),
        "no completion fragment after cleanup"
    );
    assert_eq!(no_finish_message_count(&requests[1].body_json()), 1);
    assert_eq!(no_finish_message_count(&requests[2].body_json()), 0);
    assert!(
        request_contains_assistant_text(&requests[2].body_json(), "done"),
        "the downgraded untagged answer must survive as commentary history"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_exit_does_not_start_an_extra_turn() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-sync";
    // The process exits inside the initial yield window, so exec_command
    // returns the final exit synchronously and no watcher is spawned.
    let args = json!({
        "cmd": "sleep 0.2; printf 'SYNC-DONE'",
        "yield_time_ms": 1000,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message("msg-1", "The command finished synchronously."),
            ev_completed("resp-2"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "run the short command").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_exec_command_end(&test, call_id).await;
    wait_for_turn_complete(&test).await;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2, "no extra turn for a synchronous exit");
    assert_eq!(
        no_finish_message_count(&requests[1].body_json()),
        0,
        "a synchronously closed terminal must not prohibit finishing"
    );
    assert!(
        !request_contains_completion_marker(&requests[1].body_json()),
        "no completion fragment when the exit was returned synchronously"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_stdin_observing_exit_does_not_start_an_extra_turn() -> Result<()> {
    skip_if_target_windows!(Ok(()), "asserts Unix SIGINT and exit-code semantics");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(completion_feature_config);
    let test = builder.build_with_auto_env(&server).await?;

    let start_call_id = "uexec-async-completion-poll-start";
    let poll_call_id = "uexec-async-completion-poll";
    let start_args = json!({
        "cmd": "sleep 30",
        "yield_time_ms": 250,
        "tty": false,
    });
    let poll_args = json!({
        "chars": "\u{3}",
        "session_id": 1000,
        "yield_time_ms": 2000,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(
                start_call_id,
                "exec_command",
                &serde_json::to_string(&start_args)?,
            ),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_function_call(
                poll_call_id,
                "write_stdin",
                &serde_json::to_string(&poll_args)?,
            ),
            ev_completed("resp-2"),
        ]),
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-1", "The poll observed the exit."),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the command and interrupt it").await?;

    wait_for_exec_command_begin(&test, start_call_id).await;
    wait_for_exec_command_end(&test, start_call_id).await;
    wait_for_turn_complete(&test).await;

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        3,
        "a poll that returns the exit synchronously must not wake another turn"
    );
    assert!(
        !request_contains_completion_marker(&requests[2].body_json()),
        "no completion fragment after a synchronous poll observed the exit"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_opt_out_preserves_existing_behavior() -> Result<()> {
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config.use_experimental_unified_exec_tool = true;
        config
            .features
            .enable(Feature::UnifiedExec)
            .expect("test config should allow feature update");
        config
            .features
            .disable(Feature::UnifiedExecCompletionWake)
            .expect("test config should allow feature update");
    });
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-disabled";
    let args = json!({
        "cmd": "sleep 1.5; printf 'DISABLED-DONE'",
        "yield_time_ms": 250,
    });
    let responses = vec![
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "exec_command", &serde_json::to_string(&args)?),
            ev_completed("resp-1"),
        ]),
        sse(vec![
            ev_response_created("resp-2"),
            ev_assistant_message("msg-1", "done"),
            ev_completed("resp-2"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the long command").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_turn_complete(&test).await;
    assert!(matches!(
        test.codex.agent_status().await,
        AgentStatus::Completed(_)
    ));
    // The background exit still emits its ExecCommandEnd item event...
    wait_for_exec_command_end(&test, call_id).await;
    // ...but no automatic turn is woken.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        2,
        "no automatic turn when the feature is off"
    );
    assert!(
        !request_contains_completion_marker(&requests[1].body_json()),
        "no completion fragment when the feature is off"
    );
    Ok(())
}
