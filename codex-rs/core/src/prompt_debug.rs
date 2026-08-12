use std::sync::Arc;

use codex_api::ResponsesApiRequest;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::ExecServerRuntimePaths;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::UserInstructionsProvider;
use codex_login::AuthManager;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::TurnContext;
use crate::client_common::Prompt;
use crate::config::Config;
use crate::prompt_census::PromptInvocationKind;
use crate::resolve_installation_id;
use crate::responses_metadata::CodexResponsesRequestKind;
use crate::session::session::Session;
use crate::session::turn::build_prompt;
use crate::state_db_bridge::StateDbHandle;
use crate::thread_manager::StartThreadOptions;
use crate::thread_manager::ThreadManager;
use crate::thread_manager::thread_store_from_config;

const PROMPT_RECEIPT_SCHEMA_VERSION: u32 = 1;

/// The client-owned logical request produced for one local prompt diagnostic.
///
/// This is deliberately a receipt of the effective request rather than a second prompt model. The
/// nested request is built by the same lowering path used for inference.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequestReceipt {
    pub schema_version: u32,
    pub invocation_kind: PromptInvocationKind,
    pub request_form: PromptRequestForm,
    pub provider: PromptRequestProvider,
    pub request: ResponsesApiRequest,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRequestForm {
    /// The complete logical request before an optional WebSocket incremental transport delta.
    LogicalFull,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequestProvider {
    pub id: String,
    pub name: String,
    pub wire_api: String,
    pub lowering: PromptRequestLowering,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRequestLowering {
    Responses,
    ResponsesLite,
}

/// Build the model-visible `input` list for a single debug turn.
#[doc(hidden)]
pub async fn build_prompt_input(
    mut config: Config,
    input: Vec<UserInput>,
    state_db: Option<StateDbHandle>,
    extensions: Arc<ExtensionRegistry<Config>>,
    user_instructions_provider: Arc<dyn UserInstructionsProvider>,
) -> CodexResult<Vec<ResponseItem>> {
    config.ephemeral = true;

    let auth_manager =
        AuthManager::shared_from_config(&config, /*enable_codex_api_key_env*/ false).await;
    let model_auth_manager =
        AuthManager::shared_for_model_from_config(&config, Arc::clone(&auth_manager)).await;

    let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
        config.codex_self_exe.clone(),
        config.codex_linux_sandbox_exe.clone(),
    )?;

    let thread_store = thread_store_from_config(&config, state_db.clone());
    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let thread_manager = ThreadManager::new_with_model_auth_manager(
        &config,
        Arc::clone(&auth_manager),
        Arc::clone(&model_auth_manager),
        crate::thread_manager::build_models_manager(&config, model_auth_manager),
        crate::CodexAppsToolsCache::default(),
        SessionSource::Exec,
        Arc::new(
            EnvironmentManager::from_codex_home(
                config.codex_home.clone(),
                Some(local_runtime_paths),
                config.http_client_factory(),
            )
            .await
            .map_err(|err| CodexErr::Fatal(err.to_string()))?,
        ),
        extensions,
        user_instructions_provider,
        /*analytics_events_client*/ None,
        thread_store,
        crate::local_agent_graph_store_from_state_db(state_db.as_ref()),
        installation_id,
        /*attestation_provider*/ None,
        /*external_time_provider*/ None,
    );
    let thread = thread_manager
        .start_thread(StartThreadOptions::new(config))
        .await?;

    let output = build_prompt_input_from_session(&thread.thread.session, input).await;
    let shutdown = thread.thread.shutdown_and_wait().await;
    let _removed = thread_manager.remove_thread(&thread.thread_id).await;

    shutdown?;
    output
}

/// Build the effective client-owned logical Responses request for a single debug turn.
#[doc(hidden)]
pub async fn build_prompt_request_receipt(
    mut config: Config,
    input: Vec<UserInput>,
    state_db: Option<StateDbHandle>,
    extensions: Arc<ExtensionRegistry<Config>>,
    user_instructions_provider: Arc<dyn UserInstructionsProvider>,
) -> CodexResult<PromptRequestReceipt> {
    config.ephemeral = true;

    let auth_manager =
        AuthManager::shared_from_config(&config, /*enable_codex_api_key_env*/ false).await;
    let model_auth_manager =
        AuthManager::shared_for_model_from_config(&config, Arc::clone(&auth_manager)).await;

    let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
        config.codex_self_exe.clone(),
        config.codex_linux_sandbox_exe.clone(),
    )?;

    let thread_store = thread_store_from_config(&config, state_db.clone());
    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let thread_manager = ThreadManager::new_with_model_auth_manager(
        &config,
        Arc::clone(&auth_manager),
        Arc::clone(&model_auth_manager),
        crate::thread_manager::build_models_manager(&config, model_auth_manager),
        crate::CodexAppsToolsCache::default(),
        SessionSource::Exec,
        Arc::new(
            EnvironmentManager::from_codex_home(
                config.codex_home.clone(),
                Some(local_runtime_paths),
                config.http_client_factory(),
            )
            .await
            .map_err(|err| CodexErr::Fatal(err.to_string()))?,
        ),
        extensions,
        user_instructions_provider,
        /*analytics_events_client*/ None,
        thread_store,
        crate::local_agent_graph_store_from_state_db(state_db.as_ref()),
        installation_id,
        /*attestation_provider*/ None,
        /*external_time_provider*/ None,
    );
    let thread = thread_manager
        .start_thread(StartThreadOptions::new(config))
        .await?;

    let output = build_prompt_request_receipt_from_session(&thread.thread.session, input).await;
    let shutdown = thread.thread.shutdown_and_wait().await;
    let _removed = thread_manager.remove_thread(&thread.thread_id).await;

    shutdown?;
    output
}

pub(crate) async fn build_prompt_input_from_session(
    sess: &Arc<Session>,
    input: Vec<UserInput>,
) -> CodexResult<Vec<ResponseItem>> {
    let (_, prompt) = build_prompt_from_session(sess, input).await?;
    Ok(prompt.input)
}

async fn build_prompt_from_session(
    sess: &Arc<Session>,
    input: Vec<UserInput>,
) -> CodexResult<(Arc<TurnContext>, Prompt)> {
    let turn_context = sess.new_default_turn().await;
    // Prompt debugging builds a standalone request without entering run_turn.
    let step_context = sess
        .capture_step_context(Arc::clone(&turn_context), &CancellationToken::new())
        .await?;
    sess.record_context_updates_and_set_reference_context_item(step_context.as_ref())
        .await?;

    if !input.is_empty() {
        let response_item = sess.response_item_from_user_input(input);
        sess.record_conversation_items(turn_context.as_ref(), std::slice::from_ref(&response_item))
            .await;
    }

    let prompt_input = sess
        .clone_history()
        .await
        .for_prompt(&turn_context.model_info.input_modalities);
    let base_instructions = sess.get_base_instructions().await;
    let prompt = build_prompt(
        prompt_input,
        step_context.tool_router.as_ref(),
        turn_context.as_ref(),
        base_instructions,
    );
    Ok((turn_context, prompt))
}

pub(crate) async fn build_prompt_request_receipt_from_session(
    sess: &Arc<Session>,
    input: Vec<UserInput>,
) -> CodexResult<PromptRequestReceipt> {
    let (turn_context, prompt) = build_prompt_from_session(sess, input).await?;
    let window_id = sess.current_window_id().await;
    let responses_metadata = turn_context.turn_metadata_state.to_responses_metadata(
        sess.installation_id.clone(),
        window_id,
        CodexResponsesRequestKind::Turn,
    );
    let request = turn_context
        .model_client
        .build_responses_request_for_debug(
            &prompt,
            &turn_context.model_info,
            turn_context.reasoning_effort.clone(),
            turn_context.reasoning_summary,
            turn_context.config.service_tier.clone(),
            &responses_metadata,
        )
        .await?;
    let provider_info = turn_context.provider.info();
    let lowering = if turn_context.model_info.use_responses_lite {
        PromptRequestLowering::ResponsesLite
    } else {
        PromptRequestLowering::Responses
    };

    Ok(PromptRequestReceipt {
        schema_version: PROMPT_RECEIPT_SCHEMA_VERSION,
        invocation_kind: PromptInvocationKind::Turn,
        request_form: PromptRequestForm::LogicalFull,
        provider: PromptRequestProvider {
            id: turn_context.config.model_provider_id.clone(),
            name: provider_info.name.clone(),
            wire_api: provider_info.wire_api.to_string(),
            lowering,
        },
        request,
    })
}
