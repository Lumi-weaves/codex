use std::sync::Arc;

use codex_api::ResponsesApiRequest;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::ExecServerRuntimePaths;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::UserInstructionsProvider;
use codex_login::AuthManager;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use codex_utils_string::approx_tokens_from_byte_count;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use tokio_util::sync::CancellationToken;

use crate::TurnContext;
use crate::client_common::Prompt;
use crate::cockpit_operating_contract::CockpitContractRole;
use crate::cockpit_operating_contract::CockpitOperatingContractDescriptor;
use crate::cockpit_operating_contract::descriptor as cockpit_contract_descriptor;
use crate::config::Config;
use crate::context::CockpitOperatingContract;
use crate::context::ContextualUserFragment;
use crate::prompt_census::PROMPT_CENSUS_SCHEMA_VERSION;
use crate::prompt_census::PromptContributionKind;
use crate::prompt_census::PromptInvocationKind;
use crate::prompt_inheritance::PromptInheritanceProvenance;
use crate::resolve_installation_id;
use crate::responses_metadata::CodexResponsesRequestKind;
use crate::session::session::Session;
use crate::session::turn::build_prompt;
use crate::state_db_bridge::StateDbHandle;
use crate::thread_manager::StartThreadOptions;
use crate::thread_manager::ThreadManager;
use crate::thread_manager::thread_store_from_config;

const PROMPT_RECEIPT_SCHEMA_VERSION: u32 = 4;

/// The client-owned logical request produced for one local prompt diagnostic.
///
/// This is deliberately a receipt of the effective request rather than a second prompt model. The
/// nested request is built by the same lowering path used for inference.
#[derive(Debug)]
pub struct PromptRequestReceipt {
    schema_version: u32,
    compiler_revision: String,
    invocation_kind: PromptInvocationKind,
    request_form: PromptRequestForm,
    provider: PromptRequestProvider,
    provenance: PromptReceiptProvenance,
    context_inheritance: PromptInheritanceProvenance,
    cockpit_contract: CockpitContractReceipt,
    summary: PromptReceiptSummary,
    bounds: PromptReceiptBounds,
    request: ResponsesApiRequest,
}

/// Controls whether sensitive model-visible content is included in a rendered receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptReceiptView {
    /// Safe default: stable hashes, sizes, estimates, and provenance without prompt content.
    MetadataOnly,
    /// Explicit local diagnostic view containing the complete client-owned logical request.
    FullLocal,
}

/// A serialization view over one underlying receipt. Redacted and full output never rebuild the
/// request and therefore cannot drift from each other.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedPromptRequestReceipt<'a> {
    schema_version: u32,
    compiler_revision: &'a str,
    invocation_kind: PromptInvocationKind,
    request_form: &'a PromptRequestForm,
    provider: &'a PromptRequestProvider,
    provenance: &'a PromptReceiptProvenance,
    context_inheritance: &'a PromptInheritanceProvenance,
    cockpit_contract: &'a CockpitContractReceipt,
    summary: &'a PromptReceiptSummary,
    bounds: &'a PromptReceiptBounds,
    redaction: PromptReceiptRedaction,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<&'a ResponsesApiRequest>,
}

impl PromptRequestReceipt {
    pub(crate) fn from_lowered_request(
        invocation_kind: PromptInvocationKind,
        provider_id: String,
        provider_info: &codex_model_provider_info::ModelProviderInfo,
        use_responses_lite: bool,
        request: ResponsesApiRequest,
        context_inheritance: PromptInheritanceProvenance,
        cockpit_contract_role: Option<CockpitContractRole>,
    ) -> CodexResult<Self> {
        let lowering = if use_responses_lite {
            PromptRequestLowering::ResponsesLite
        } else {
            PromptRequestLowering::Responses
        };
        let client_normalization = if provider_info.is_openai() {
            PromptClientNormalization::OpenAi
        } else {
            PromptClientNormalization::NonOpenAiSanitized
        };
        let summary = build_receipt_summary(&request)?;
        let cockpit_contract = CockpitContractReceipt::inspect(cockpit_contract_role, &request)?;

        Ok(Self {
            schema_version: PROMPT_RECEIPT_SCHEMA_VERSION,
            compiler_revision: context_inheritance.compiler_revision.clone(),
            invocation_kind,
            request_form: PromptRequestForm::LogicalFull,
            provider: PromptRequestProvider {
                id: provider_id,
                name: provider_info.name.clone(),
                wire_api: provider_info.wire_api.to_string(),
                lowering,
                client_normalization,
            },
            provenance: PromptReceiptProvenance {
                census_schema_version: PROMPT_CENSUS_SCHEMA_VERSION,
                invocation_ref: invocation_kind,
                contribution_refs: invocation_kind.contributions(),
                provider_processing: "provider_owned_unknown",
            },
            context_inheritance,
            cockpit_contract,
            summary,
            bounds: PromptReceiptBounds {
                receipt_content_truncated: false,
                request_is_post_client_lowering: true,
                upstream_prompt_bounds_already_applied: true,
                provider_owned_processing_observable: false,
            },
            request,
        })
    }

    pub fn render(&self, view: PromptReceiptView) -> RenderedPromptRequestReceipt<'_> {
        RenderedPromptRequestReceipt {
            schema_version: self.schema_version,
            compiler_revision: &self.compiler_revision,
            invocation_kind: self.invocation_kind,
            request_form: &self.request_form,
            provider: &self.provider,
            provenance: &self.provenance,
            context_inheritance: &self.context_inheritance,
            cockpit_contract: &self.cockpit_contract,
            summary: &self.summary,
            bounds: &self.bounds,
            redaction: PromptReceiptRedaction {
                view,
                content_included: view == PromptReceiptView::FullLocal,
                full_local_requires_explicit_opt_in: true,
                persisted_by_debug_command: false,
                transmitted_by_debug_command: false,
            },
            request: (view == PromptReceiptView::FullLocal).then_some(&self.request),
        }
    }

    /// The exact client-owned logical request used to compute this receipt.
    #[doc(hidden)]
    pub fn request(&self) -> &ResponsesApiRequest {
        &self.request
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CockpitContractReceiptStatus {
    Included,
    Excluded,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CockpitContractReceipt {
    status: CockpitContractReceiptStatus,
    expected_copy_count: usize,
    effective_copy_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptor: Option<CockpitOperatingContractDescriptor>,
}

impl CockpitContractReceipt {
    fn inspect(
        expected_role: Option<CockpitContractRole>,
        request: &ResponsesApiRequest,
    ) -> CodexResult<Self> {
        let effective_contracts = cockpit_contract_texts(request);
        let effective_copy_count = effective_contracts.len();
        let expected_copy_count = usize::from(expected_role.is_some());
        if effective_copy_count != expected_copy_count {
            return Err(CodexErr::Fatal(format!(
                "cockpit operating contract conformance failed: expected {expected_copy_count} effective copies, found {effective_copy_count}"
            )));
        }
        if let Some(expected_role) = expected_role {
            let expected_contract =
                crate::cockpit_operating_contract::rendered_contract(expected_role);
            let effective_role_copy_count = effective_contracts
                .iter()
                .filter(|contract| **contract == expected_contract.as_str())
                .count();
            if effective_role_copy_count != 1 {
                return Err(CodexErr::Fatal(format!(
                    "cockpit operating contract role conformance failed: expected one {} contract, found {effective_role_copy_count}",
                    expected_role.as_str()
                )));
            }
        }

        Ok(Self {
            status: if expected_role.is_some() {
                CockpitContractReceiptStatus::Included
            } else {
                CockpitContractReceiptStatus::Excluded
            },
            expected_copy_count,
            effective_copy_count,
            descriptor: expected_role.map(cockpit_contract_descriptor),
        })
    }
}

fn cockpit_contract_texts(request: &ResponsesApiRequest) -> Vec<&str> {
    request
        .input
        .iter()
        .filter_map(|item| {
            let ResponseItem::Message { role, content, .. } = item else {
                return None;
            };
            let [ContentItem::InputText { text }] = content.as_slice() else {
                return None;
            };
            (role == "developer" && CockpitOperatingContract::matches_text(text))
                .then_some(text.as_str())
        })
        .collect()
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
    pub client_normalization: PromptClientNormalization,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRequestLowering {
    Responses,
    ResponsesLite,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptClientNormalization {
    OpenAi,
    NonOpenAiSanitized,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceiptProvenance {
    census_schema_version: u32,
    invocation_ref: PromptInvocationKind,
    contribution_refs: &'static [PromptContributionKind],
    provider_processing: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceiptSummary {
    hash_algorithm: &'static str,
    canonical_request_sha256: String,
    canonical_request_bytes: usize,
    estimated_model_visible_tokens: u64,
    estimate_method: &'static str,
    regions: Vec<PromptReceiptRegion>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceiptRegion {
    id: &'static str,
    contribution_refs: &'static [PromptContributionKind],
    sha256: String,
    canonical_bytes: usize,
    estimated_tokens: u64,
    sensitive: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceiptBounds {
    receipt_content_truncated: bool,
    request_is_post_client_lowering: bool,
    upstream_prompt_bounds_already_applied: bool,
    provider_owned_processing_observable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptReceiptRedaction {
    view: PromptReceiptView,
    content_included: bool,
    full_local_requires_explicit_opt_in: bool,
    persisted_by_debug_command: bool,
    transmitted_by_debug_command: bool,
}

const INPUT_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::WorldStateDeveloperContext,
    PromptContributionKind::CockpitOperatingContract,
    PromptContributionKind::WorldStateContextualUserContext,
    PromptContributionKind::ConversationHistory,
    PromptContributionKind::InvocationInput,
    PromptContributionKind::ToolSpecifications,
    PromptContributionKind::ProviderLowering,
];

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
    PromptRequestReceipt::from_lowered_request(
        PromptInvocationKind::Turn,
        turn_context.config.model_provider_id.clone(),
        turn_context.provider.info(),
        turn_context.model_info.use_responses_lite,
        request,
        sess.prompt_context.provenance(),
        (turn_context.multi_agent_version == codex_protocol::protocol::MultiAgentVersion::V2)
            .then(|| {
                crate::cockpit_operating_contract::role_for_session_source(
                    &turn_context.session_source,
                )
            })
            .flatten(),
    )
}

fn build_receipt_summary(request: &ResponsesApiRequest) -> CodexResult<PromptReceiptSummary> {
    let request_bytes = canonical_json_bytes(request)?;
    let regions = vec![
        receipt_region(
            "base_instructions",
            &[
                PromptContributionKind::BaseInstructions,
                PromptContributionKind::ProviderLowering,
            ],
            &request.instructions,
            /*sensitive*/ true,
        )?,
        receipt_region(
            "ordered_input",
            INPUT_CONTRIBUTIONS,
            &request.input,
            /*sensitive*/ true,
        )?,
        receipt_region(
            "tool_specifications",
            &[
                PromptContributionKind::ToolSpecifications,
                PromptContributionKind::ProviderLowering,
            ],
            &request.tools,
            /*sensitive*/ true,
        )?,
        receipt_region(
            "output_control",
            &[
                PromptContributionKind::OutputSchema,
                PromptContributionKind::ProviderLowering,
            ],
            &request.text,
            /*sensitive*/ true,
        )?,
        receipt_region(
            "request_shape",
            &[PromptContributionKind::ProviderLowering],
            &serde_json::json!({
                "model": request.model,
                "tool_choice": request.tool_choice,
                "parallel_tool_calls": request.parallel_tool_calls,
                "reasoning": request.reasoning,
                "store": request.store,
                "stream": request.stream,
                "stream_options": request.stream_options,
                "include": request.include,
                "service_tier": request.service_tier,
            }),
            /*sensitive*/ false,
        )?,
    ];
    let estimated_model_visible_tokens = regions.iter().map(|region| region.estimated_tokens).sum();

    Ok(PromptReceiptSummary {
        hash_algorithm: "sha256_canonical_json",
        canonical_request_sha256: sha256_hex(&request_bytes),
        canonical_request_bytes: request_bytes.len(),
        estimated_model_visible_tokens,
        estimate_method: "canonical_region_bytes_div_4_ceiling",
        regions,
    })
}

fn receipt_region<T: Serialize>(
    id: &'static str,
    contribution_refs: &'static [PromptContributionKind],
    value: &T,
    sensitive: bool,
) -> CodexResult<PromptReceiptRegion> {
    let bytes = canonical_json_bytes(value)?;
    Ok(PromptReceiptRegion {
        id,
        contribution_refs,
        sha256: sha256_hex(&bytes),
        canonical_bytes: bytes.len(),
        estimated_tokens: approx_tokens_from_byte_count(bytes.len()),
        sensitive,
    })
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> CodexResult<Vec<u8>> {
    let value = serde_json::to_value(value)
        .map_err(|err| CodexErr::Fatal(format!("failed to serialize prompt receipt: {err}")))?;
    serde_json::to_vec(&canonicalize_json(value))
        .map_err(|err| CodexErr::Fatal(format!("failed to encode prompt receipt: {err}")))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize_json(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "prompt_debug_tests.rs"]
mod tests;
