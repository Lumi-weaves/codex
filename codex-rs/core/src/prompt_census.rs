use std::fmt;

use codex_protocol::protocol::InternalSessionSource;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use serde::Serialize;

use crate::guardian::is_guardian_reviewer_source;

const PROMPT_CENSUS_SCHEMA_VERSION: u32 = 1;

/// Stable identity for every client-owned model invocation family known to the prompt plane.
///
/// Keep [`Self::ALL`] and [`invocation_definition`] exhaustive. Sampling call sites carry this
/// value explicitly so a model invocation cannot silently disappear behind the generic Responses
/// transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptInvocationKind {
    Turn,
    StartupPrewarm,
    Review,
    Guardian,
    LocalCompaction,
    RemoteCompaction,
    Realtime,
    MemoryExtraction,
    MemoryTraceSummarization,
    MemoryConsolidation,
}

impl PromptInvocationKind {
    pub const ALL: [Self; 10] = [
        Self::Turn,
        Self::StartupPrewarm,
        Self::Review,
        Self::Guardian,
        Self::LocalCompaction,
        Self::RemoteCompaction,
        Self::Realtime,
        Self::MemoryExtraction,
        Self::MemoryTraceSummarization,
        Self::MemoryConsolidation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::StartupPrewarm => "startup_prewarm",
            Self::Review => "review",
            Self::Guardian => "guardian",
            Self::LocalCompaction => "local_compaction",
            Self::RemoteCompaction => "remote_compaction",
            Self::Realtime => "realtime",
            Self::MemoryExtraction => "memory_extraction",
            Self::MemoryTraceSummarization => "memory_trace_summarization",
            Self::MemoryConsolidation => "memory_consolidation",
        }
    }

    /// Classifies calls that use the ordinary session turn runner.
    pub(crate) fn for_session_turn(session_source: &SessionSource) -> Self {
        if is_guardian_reviewer_source(session_source) {
            return Self::Guardian;
        }

        match session_source {
            SessionSource::SubAgent(SubAgentSource::Review) => Self::Review,
            SessionSource::SubAgent(SubAgentSource::Compact) => Self::LocalCompaction,
            SessionSource::Internal(InternalSessionSource::MemoryConsolidation)
            | SessionSource::SubAgent(SubAgentSource::MemoryConsolidation) => {
                Self::MemoryConsolidation
            }
            _ => Self::Turn,
        }
    }
}

impl fmt::Display for PromptInvocationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptContributionKind {
    BaseInstructions,
    WorldStateDeveloperContext,
    WorldStateContextualUserContext,
    ConversationHistory,
    InvocationInput,
    ToolSpecifications,
    OutputSchema,
    RealtimeSessionInstructions,
    RealtimeConversationInput,
    RawMemoryTraces,
    ProviderLowering,
    ProviderProcessing,
}

impl PromptContributionKind {
    pub const ALL: [Self; 12] = [
        Self::BaseInstructions,
        Self::WorldStateDeveloperContext,
        Self::WorldStateContextualUserContext,
        Self::ConversationHistory,
        Self::InvocationInput,
        Self::ToolSpecifications,
        Self::OutputSchema,
        Self::RealtimeSessionInstructions,
        Self::RealtimeConversationInput,
        Self::RawMemoryTraces,
        Self::ProviderLowering,
        Self::ProviderProcessing,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaseInstructions => "base_instructions",
            Self::WorldStateDeveloperContext => "world_state_developer_context",
            Self::WorldStateContextualUserContext => "world_state_contextual_user_context",
            Self::ConversationHistory => "conversation_history",
            Self::InvocationInput => "invocation_input",
            Self::ToolSpecifications => "tool_specifications",
            Self::OutputSchema => "output_schema",
            Self::RealtimeSessionInstructions => "realtime_session_instructions",
            Self::RealtimeConversationInput => "realtime_conversation_input",
            Self::RawMemoryTraces => "raw_memory_traces",
            Self::ProviderLowering => "provider_lowering",
            Self::ProviderProcessing => "provider_processing",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CensusCompleteness {
    Static,
    RuntimeEnumerable,
    Incomplete,
    ProviderOwnedUnknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptContextCensus {
    pub schema_version: u32,
    pub scope: CensusScope,
    pub invocations: Vec<PromptInvocationDefinition>,
    pub contributions: Vec<PromptContributionDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CensusScope {
    pub boundary: &'static str,
    pub registry: &'static str,
    pub provider_processing: CensusCompleteness,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInvocationDefinition {
    pub id: PromptInvocationKind,
    pub owner: &'static str,
    pub request_routes: &'static [&'static str],
    pub runtime_discriminator: &'static str,
    pub base_instructions_source: &'static str,
    pub input_assembly: &'static str,
    pub tool_source: &'static str,
    pub output_control_source: &'static str,
    pub lifecycle: &'static str,
    pub contributions: &'static [PromptContributionKind],
    pub completeness: CensusCompleteness,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptContributionDefinition {
    pub id: PromptContributionKind,
    pub owner: &'static str,
    pub placement: &'static str,
    pub provenance: &'static str,
    pub availability: &'static str,
    pub hard_bound: &'static str,
    pub governance: &'static str,
    pub inheritance: &'static str,
    pub sensitivity: &'static str,
    pub completeness: CensusCompleteness,
}

const TURN_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::WorldStateDeveloperContext,
    PromptContributionKind::WorldStateContextualUserContext,
    PromptContributionKind::ConversationHistory,
    PromptContributionKind::InvocationInput,
    PromptContributionKind::ToolSpecifications,
    PromptContributionKind::OutputSchema,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const PREWARM_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::ToolSpecifications,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const COMPACTION_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::ConversationHistory,
    PromptContributionKind::InvocationInput,
    PromptContributionKind::ToolSpecifications,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const REALTIME_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::RealtimeSessionInstructions,
    PromptContributionKind::RealtimeConversationInput,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const MEMORY_TRACE_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::RawMemoryTraces,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const MEMORY_EXTRACTION_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::InvocationInput,
    PromptContributionKind::OutputSchema,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

const MEMORY_CONSOLIDATION_CONTRIBUTIONS: &[PromptContributionKind] = &[
    PromptContributionKind::BaseInstructions,
    PromptContributionKind::WorldStateDeveloperContext,
    PromptContributionKind::WorldStateContextualUserContext,
    PromptContributionKind::ConversationHistory,
    PromptContributionKind::InvocationInput,
    PromptContributionKind::ToolSpecifications,
    PromptContributionKind::ProviderLowering,
    PromptContributionKind::ProviderProcessing,
];

/// Return a deterministic, versioned census generated from the same typed invocation identities
/// used by the model client. It intentionally reports missing provenance rather than reconstructing
/// it from model-visible text.
pub fn prompt_context_census() -> PromptContextCensus {
    PromptContextCensus {
        schema_version: PROMPT_CENSUS_SCHEMA_VERSION,
        scope: CensusScope {
            boundary: "client_emitted_model_visible_context",
            registry: "typed_static_registry",
            provider_processing: CensusCompleteness::ProviderOwnedUnknown,
        },
        invocations: PromptInvocationKind::ALL
            .into_iter()
            .map(invocation_definition)
            .collect(),
        contributions: PromptContributionKind::ALL
            .into_iter()
            .map(contribution_definition)
            .collect(),
    }
}

fn invocation_definition(id: PromptInvocationKind) -> PromptInvocationDefinition {
    match id {
        PromptInvocationKind::Turn => PromptInvocationDefinition {
            id,
            owner: "core::session::turn",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "ordinary session turn; root and non-special subagent sessions",
            base_instructions_source: "config override, then inherited rollout base, then rendered model template",
            input_assembly: "ordered initial world state, conversation history, current user input, and tool results",
            tool_source: "turn ToolRouter model-visible specifications",
            output_control_source: "turn final_output_json_schema and strictness policy",
            lifecycle: "root or shadow session; inherited history is selected before prompt assembly",
            contributions: TURN_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
        PromptInvocationKind::StartupPrewarm => PromptInvocationDefinition {
            id,
            owner: "core::session_startup_prewarm",
            request_routes: &["responses_websocket"],
            runtime_discriminator: "startup prewarm feature and WebSocket transport eligibility",
            base_instructions_source: "resolved session base instructions",
            input_assembly: "empty input; the warmup establishes reusable request state",
            tool_source: "startup turn ToolRouter model-visible specifications",
            output_control_source: "none",
            lifecycle: "one opportunistic prewarm before the first submitted turn",
            contributions: PREWARM_CONTRIBUTIONS,
            completeness: CensusCompleteness::Static,
        },
        PromptInvocationKind::Review => PromptInvocationDefinition {
            id,
            owner: "core::tasks::review",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "SessionSource::SubAgent(SubAgentSource::Review)",
            base_instructions_source: "versioned REVIEW_PROMPT override",
            input_assembly: "review request plus the review subagent's selected inherited context",
            tool_source: "review session ToolRouter model-visible specifications",
            output_control_source: "review task JSON output schema",
            lifecycle: "dedicated review subagent session",
            contributions: TURN_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
        PromptInvocationKind::Guardian => PromptInvocationDefinition {
            id,
            owner: "core::guardian",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "guardian-labelled subagent SessionSource",
            base_instructions_source: "rendered Guardian policy prompt override",
            input_assembly: "bounded parent transcript, reviewed action, policy context, and follow-ups",
            tool_source: "Guardian session ToolRouter; normally policy-restricted",
            output_control_source: "Guardian assessment schema with non-strict compatibility lowering",
            lifecycle: "new or reused dedicated Guardian review session",
            contributions: TURN_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
        PromptInvocationKind::LocalCompaction => PromptInvocationDefinition {
            id,
            owner: "core::compact",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "local compaction task stream call",
            base_instructions_source: "current session base instructions",
            input_assembly: "trimmed conversation history followed by the versioned summarization request",
            tool_source: "none",
            output_control_source: "none",
            lifecycle: "in-session compaction task with retry-local client session",
            contributions: COMPACTION_CONTRIBUTIONS,
            completeness: CensusCompleteness::Static,
        },
        PromptInvocationKind::RemoteCompaction => PromptInvocationDefinition {
            id,
            owner: "core::compact_remote",
            request_routes: &["responses_compact", "responses_v2_compaction_trigger"],
            runtime_discriminator: "remote compaction attempt selected by feature and provider capability",
            base_instructions_source: "current session base instructions",
            input_assembly: "trimmed conversation history; v2 appends a compaction trigger item",
            tool_source: "current turn ToolRouter model-visible specifications",
            output_control_source: "none",
            lifecycle: "in-session unary compact endpoint or streamed v2 compaction attempt",
            contributions: COMPACTION_CONTRIBUTIONS,
            completeness: CensusCompleteness::Static,
        },
        PromptInvocationKind::Realtime => PromptInvocationDefinition {
            id,
            owner: "core::realtime_conversation",
            request_routes: &["realtime_webrtc", "realtime_websocket"],
            runtime_discriminator: "RealtimeConversationManager session start",
            base_instructions_source: "config override, then request override, then versioned BACKEND_PROMPT",
            input_assembly: "live text/audio events and client-managed handoff items",
            tool_source: "realtime session configuration and handoff contract",
            output_control_source: "realtime event parser and session configuration",
            lifecycle: "long-lived realtime call with a sideband or direct WebSocket",
            contributions: REALTIME_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
        PromptInvocationKind::MemoryExtraction => PromptInvocationDefinition {
            id,
            owner: "memories::write phase1",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "stage-one memory job for one selected rollout",
            base_instructions_source: "versioned stage-one memory extraction prompt",
            input_assembly: "filtered rollout items plus rollout path and workspace context",
            tool_source: "none",
            output_control_source: "strict stage-one memory JSON schema",
            lifecycle: "detached unary-like streamed request per selected rollout",
            contributions: MEMORY_EXTRACTION_CONTRIBUTIONS,
            completeness: CensusCompleteness::Static,
        },
        PromptInvocationKind::MemoryTraceSummarization => PromptInvocationDefinition {
            id,
            owner: "core::client memory trace summarization",
            request_routes: &["memories_trace_summarize"],
            runtime_discriminator: "stage-one memory summarization client method",
            base_instructions_source: "provider endpoint contract; no client prompt field",
            input_assembly: "bounded raw rollout-memory records",
            tool_source: "none",
            output_control_source: "typed memory summarize endpoint response",
            lifecycle: "unary stage-one memory job",
            contributions: MEMORY_TRACE_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
        PromptInvocationKind::MemoryConsolidation => PromptInvocationDefinition {
            id,
            owner: "memories::write phase2 agent",
            request_routes: &["responses_http", "responses_websocket"],
            runtime_discriminator: "InternalSessionSource::MemoryConsolidation",
            base_instructions_source: "memory consolidation agent config and model instructions",
            input_assembly: "versioned consolidation task plus synced memory workspace and session context",
            tool_source: "consolidation agent ToolRouter model-visible specifications",
            output_control_source: "none; artifacts are validated after the agent turn",
            lifecycle: "dedicated internal agent session with workspace resources",
            contributions: MEMORY_CONSOLIDATION_CONTRIBUTIONS,
            completeness: CensusCompleteness::Incomplete,
        },
    }
}

fn contribution_definition(id: PromptContributionKind) -> PromptContributionDefinition {
    match id {
        PromptContributionKind::BaseInstructions => PromptContributionDefinition {
            id,
            owner: "session configuration and model catalog",
            placement: "Responses instructions, or a leading developer item under Responses Lite",
            provenance: "BaseInstructions provenance is retained in core but not yet emitted by request receipts",
            availability: "effective value is runtime-enumerable in prompt-receipt",
            hard_bound: "no independent uniform bound; participates in the model context window",
            governance: "versioned model default or operator-authored override",
            inheritance: "full forks may preserve it; fresh role layers may replace it",
            sensitivity: "may contain private operator instructions",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::WorldStateDeveloperContext => PromptContributionDefinition {
            id,
            owner: "session::world_state and registered context contributors",
            placement: "ordered developer messages before contextual user messages and live turn input",
            provenance: "built-in fragment types plus extension contributors; item-level provenance is not preserved after assembly",
            availability: "effective items are runtime-enumerable; contributor identity is incomplete",
            hard_bound: "fragment-specific bounds only; no uniform aggregate bound",
            governance: "mixed versioned contract, harness runtime facts, and extension-authored context",
            inheritance: "recomputed from child session configuration and inherited state",
            sensitivity: "may contain paths, policies, environment facts, and extension content",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::WorldStateContextualUserContext => PromptContributionDefinition {
            id,
            owner: "session::world_state contextual user fragments",
            placement: "ordered contextual user messages after developer context and before live user input",
            provenance: "built-in fragment types; item-level provenance is not preserved after assembly",
            availability: "effective items are runtime-enumerable; contributor identity is incomplete",
            hard_bound: "fragment-specific bounds only; no uniform aggregate bound",
            governance: "mixed harness runtime facts and versioned context policy",
            inheritance: "recomputed for each session and turn",
            sensitivity: "may contain runtime state or prior task context",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::ConversationHistory => PromptContributionDefinition {
            id,
            owner: "context_manager::history",
            placement: "after initial context according to recorded item order",
            provenance: "rollout items retain roles and item kinds; contribution lineage is only partially explicit",
            availability: "runtime-enumerable in the effective request",
            hard_bound: "model context window plus truncation and compaction policy",
            governance: "user, assistant, tool, and runtime-authored transcript",
            inheritance: "full, last-N, fresh, or referenced history according to fork policy",
            sensitivity: "may contain user content, tool output, secrets, and encrypted provider items",
            completeness: CensusCompleteness::RuntimeEnumerable,
        },
        PromptContributionKind::InvocationInput => PromptContributionDefinition {
            id,
            owner: "logical invocation owner",
            placement: "invocation-specific items appended to or assembled with history",
            provenance: "call-site-owned prompt, trigger, transcript, or current user input",
            availability: "runtime-enumerable in the effective request",
            hard_bound: "invocation-specific; Guardian and compaction apply explicit trimming",
            governance: "user-authored, versioned task prompt, or harness runtime fact",
            inheritance: "invocation-specific and normally not inherited as a base contract",
            sensitivity: "may contain user content and selected runtime evidence",
            completeness: CensusCompleteness::RuntimeEnumerable,
        },
        PromptContributionKind::ToolSpecifications => PromptContributionDefinition {
            id,
            owner: "ToolRouter and dynamic tool registry",
            placement: "Responses tools field, or leading AdditionalTools item under Responses Lite",
            provenance: "effective schemas are visible; per-tool registration provenance is not yet emitted",
            availability: "effective tool schemas are runtime-enumerable in prompt-receipt",
            hard_bound: "no locally enforced uniform aggregate bound",
            governance: "versioned built-ins, MCP/app/plugin tools, and runtime dynamic tools",
            inheritance: "rebuilt for each step from session capabilities and runtime bindings",
            sensitivity: "schemas may expose internal names, descriptions, and argument structure",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::OutputSchema => PromptContributionDefinition {
            id,
            owner: "turn context and invocation policy",
            placement: "Responses text.format JSON schema control",
            provenance: "caller-supplied final output schema plus invocation strictness policy",
            availability: "runtime-enumerable in prompt-receipt",
            hard_bound: "no independent local uniform bound",
            governance: "caller-authored or versioned invocation contract",
            inheritance: "turn-scoped",
            sensitivity: "normally structural; descriptions may contain private contract text",
            completeness: CensusCompleteness::RuntimeEnumerable,
        },
        PromptContributionKind::RealtimeSessionInstructions => PromptContributionDefinition {
            id,
            owner: "realtime_prompt and realtime session configuration",
            placement: "realtime session instructions",
            provenance: "config override, request override, or versioned backend prompt",
            availability: "effective session config exists at runtime but has no prompt-plane receipt",
            hard_bound: "provider contract; no client-wide bound recorded",
            governance: "operator-authored override or versioned default",
            inheritance: "realtime-call scoped",
            sensitivity: "may contain private operator instructions and user identity hints",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::RealtimeConversationInput => PromptContributionDefinition {
            id,
            owner: "realtime_conversation input channels",
            placement: "ordered live audio, text, and handoff events",
            provenance: "channel and event kinds are known at runtime",
            availability: "event-stream enumerable; no consolidated request receipt",
            hard_bound: "bounded local queues; provider session history bound is not client-visible",
            governance: "user input and harness handoff runtime facts",
            inheritance: "realtime-call scoped",
            sensitivity: "may contain voice, transcript, user content, and model handoffs",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::RawMemoryTraces => PromptContributionDefinition {
            id,
            owner: "memories stage-one selection",
            placement: "typed raw_memories endpoint payload",
            provenance: "selected rollout and memory records",
            availability: "runtime payload is enumerable; no prompt-plane receipt or redaction summary",
            hard_bound: "memory selection policy",
            governance: "harness-selected historical runtime facts",
            inheritance: "memory job scoped",
            sensitivity: "may contain prior user, workspace, and rollout content",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::ProviderLowering => PromptContributionDefinition {
            id,
            owner: "ModelClient request builder",
            placement: "normal Responses fields or Responses Lite developer input lowering",
            provenance: "provider configuration and model capability flags",
            availability: "logical full request is runtime-enumerable in prompt-receipt for ordinary turns",
            hard_bound: "transport-specific normalization and compatibility rules",
            governance: "versioned client implementation plus provider configuration",
            inheritance: "resolved per model request",
            sensitivity: "may rearrange sensitive contributions without changing their sensitivity",
            completeness: CensusCompleteness::Incomplete,
        },
        PromptContributionKind::ProviderProcessing => PromptContributionDefinition {
            id,
            owner: "model provider",
            placement: "after the client-emitted request boundary",
            provenance: "not observable from the Codex client",
            availability: "not runtime-enumerable by the client",
            hard_bound: "provider-owned",
            governance: "provider-owned unknown",
            inheritance: "provider-owned unknown",
            sensitivity: "inherits all request sensitivity",
            completeness: CensusCompleteness::ProviderOwnedUnknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use codex_protocol::protocol::InternalSessionSource;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::SubAgentSource;

    use super::CensusCompleteness;
    use super::PromptContributionKind;
    use super::PromptInvocationKind;
    use super::prompt_context_census;

    #[test]
    fn census_registers_every_known_invocation_and_contribution_once() {
        let census = prompt_context_census();
        assert_eq!(census.schema_version, 1);
        assert_eq!(census.invocations.len(), PromptInvocationKind::ALL.len());
        assert_eq!(
            census.contributions.len(),
            PromptContributionKind::ALL.len()
        );

        let invocation_ids = census
            .invocations
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(invocation_ids.len(), PromptInvocationKind::ALL.len());

        let contribution_ids = census
            .contributions
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(contribution_ids.len(), PromptContributionKind::ALL.len());
        assert!(census.invocations.iter().all(|entry| {
            !entry.contributions.is_empty()
                && entry
                    .contributions
                    .iter()
                    .all(|kind| contribution_ids.contains(kind.as_str()))
        }));
        assert!(
            census
                .contributions
                .iter()
                .any(|entry| { entry.completeness == CensusCompleteness::ProviderOwnedUnknown })
        );
        assert!(
            census
                .contributions
                .iter()
                .any(|entry| { entry.completeness == CensusCompleteness::Incomplete })
        );
    }

    #[test]
    fn session_turn_invocations_are_classified_before_transport() {
        assert_eq!(
            PromptInvocationKind::for_session_turn(&SessionSource::Cli),
            PromptInvocationKind::Turn
        );
        assert_eq!(
            PromptInvocationKind::for_session_turn(&SessionSource::SubAgent(
                SubAgentSource::Review
            )),
            PromptInvocationKind::Review
        );
        assert_eq!(
            PromptInvocationKind::for_session_turn(&SessionSource::SubAgent(
                SubAgentSource::Compact
            )),
            PromptInvocationKind::LocalCompaction
        );
        assert_eq!(
            PromptInvocationKind::for_session_turn(&SessionSource::SubAgent(
                SubAgentSource::Other("guardian".to_string())
            )),
            PromptInvocationKind::Guardian
        );
        assert_eq!(
            PromptInvocationKind::for_session_turn(&SessionSource::Internal(
                InternalSessionSource::MemoryConsolidation
            )),
            PromptInvocationKind::MemoryConsolidation
        );
    }
}
