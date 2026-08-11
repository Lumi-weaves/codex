//! Integration coverage for unified-exec async completion wake: a background
//! terminal process that finishes without a synchronous observation wakes a
//! model turn carrying a bounded completion fragment.

use anyhow::Result;
use codex_features::Feature;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
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
use core_test_support::wait_for_event_with_timeout;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use tokio::time::Duration;
use wiremock::ResponseTemplate;

const COMPLETION_MARKER: &str = "<unified_exec_completion>";
const OUTPUT_AVAILABLE_MARKER: &str = "<unified_exec_output_available>";
const EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

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
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: prompt.into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: codex_protocol::protocol::ThreadSettingsOverrides {
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
        })
        .await?;
    Ok(())
}

fn request_contains_completion_marker(body: &Value) -> bool {
    request_contains_marker(body, COMPLETION_MARKER)
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

async fn wait_for_turn_complete(test: &TestCodex) {
    wait_for_event_with_timeout(
        &test.codex,
        |event| matches!(event, EventMsg::TurnComplete(_)),
        EVENT_WAIT_TIMEOUT,
    )
    .await;
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
async fn background_completion_wakes_idle_turn_by_default() -> Result<()> {
    // TODO(anp): Remove after unified-exec fixtures use target-native commands.
    skip_if_target_windows!(Ok(()), "uses a POSIX-only command fixture");
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));

    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(unified_exec_default_config);
    let test = builder.build_with_auto_env(&server).await?;

    let call_id = "uexec-async-completion-idle";
    let args = json!({
        "cmd": "sleep 1.5; printf 'IDLE-DONE'",
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
            ev_assistant_message("msg-1", "I will wait for the background process."),
            ev_completed("resp-2"),
        ]),
        // Request 3 is served to the automatic turn woken by the completion.
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-2", "The background process finished."),
            ev_completed("resp-3"),
        ]),
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the long command and wait").await?;

    let process_id = wait_for_exec_command_begin(&test, call_id)
        .await
        .expect("begin event should carry a process id for a long-lived session");
    wait_for_turn_complete(&test).await;
    assert_eq!(
        test.codex.agent_status().await,
        AgentStatus::Running,
        "visible final text ends the turn, but the task stays non-final while its terminal is awaited"
    );
    wait_for_exec_command_end(&test, call_id).await;
    // The completion wakes a fresh regular turn; its sampling is the 3rd
    // model request and carries the bounded completion fragment.
    wait_for_turn_complete(&test).await;
    assert!(matches!(
        test.codex.agent_status().await,
        AgentStatus::Completed(_)
    ));

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3, "exactly one auto-woken turn");
    let second = requests[1].body_json();
    let third = requests[2].body_json();
    assert!(
        !request_contains_completion_marker(&second),
        "the first turn must not observe the completion"
    );
    assert!(
        request_contains_completion_marker(&third),
        "the auto-woken turn must sample the completion fragment"
    );
    let third_text = serde_json::to_string(&third)?;
    assert!(
        third_text.contains(&format!("process_id\\\":{process_id}")),
        "completion fragment should carry the stable process id; expected process_id {process_id}"
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
        // turn is busy: the completion is admitted to the shared session
        // queue, survives the turn's visible-answer boundary, and wakes a
        // fresh turn only after the old turn clears.
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

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_exec_command_end(&test, call_id).await;
    // The busy turn finishes without sampling the completion...
    wait_for_turn_complete(&test).await;
    // ...and the queued completion wakes exactly one fresh turn.
    wait_for_turn_complete(&test).await;

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        3,
        "the completion is sampled serially by exactly one fresh turn"
    );
    let second = requests[1].body_json();
    let third = requests[2].body_json();
    assert!(
        !request_contains_completion_marker(&second),
        "the in-flight sampling request cannot contain the late completion"
    );
    assert!(
        request_contains_completion_marker(&third),
        "the fresh turn's first sampling request must carry the completion fragment"
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
    wait_for_turn_complete(&test).await;
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
    wait_for_turn_complete(&test).await;
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
async fn clean_background_terminals_does_not_wake_a_completion_turn() -> Result<()> {
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
    ];
    let response_mock = mount_sse_sequence(&server, responses).await;
    submit_turn(&test, "start the long command").await?;

    wait_for_exec_command_begin(&test, call_id).await;
    wait_for_turn_complete(&test).await;
    // Explicitly clean up all background terminals (feature enabled): the
    // cleanup must mark processes observed so no automatic completion wakes.
    test.codex
        .submit(Op::CleanBackgroundTerminals)
        .await
        .expect("clean background terminals submission");
    // The watcher still emits the terminal end item event for the terminated
    // process (existing behavior), but no completion fragment and no turn.
    wait_for_exec_command_end(&test, call_id).await;
    assert!(matches!(
        test.codex.agent_status().await,
        AgentStatus::Completed(_)
    ));
    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = response_mock.requests();
    assert_eq!(
        requests.len(),
        2,
        "cleanup must not wake an automatic completion turn"
    );
    assert!(
        !request_contains_completion_marker(&requests[1].body_json()),
        "no completion fragment after cleanup"
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
