use super::CensusCompleteness;
use super::PromptContributionKind;
use super::PromptResourceClassification;
use super::PromptResourceId;
use super::PromptResourceKind;
use super::PromptResourceSourceNavigation;

#[derive(Clone, Copy)]
pub(crate) struct StaticSourceNavigation {
    pub(crate) modules: &'static [&'static str],
    pub(crate) symbols: &'static [&'static str],
    pub(crate) keywords: &'static [&'static str],
    pub(crate) tests: &'static [&'static str],
}

impl StaticSourceNavigation {
    pub(crate) fn to_owned(self) -> PromptResourceSourceNavigation {
        PromptResourceSourceNavigation {
            modules: self
                .modules
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            symbols: self
                .symbols
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            keywords: self
                .keywords
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            tests: self
                .tests
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }
}

pub(crate) struct StaticPromptResource {
    pub(crate) id: PromptContributionKind,
    pub(crate) kind: PromptResourceKind,
    pub(crate) classification: PromptResourceClassification,
    pub(crate) owner: &'static str,
    pub(crate) placement: &'static str,
    pub(crate) provenance: &'static str,
    pub(crate) availability: &'static str,
    pub(crate) hard_bound: &'static str,
    pub(crate) governance: &'static str,
    pub(crate) inheritance: &'static str,
    pub(crate) sensitivity: &'static str,
    pub(crate) completeness: CensusCompleteness,
    pub(crate) dependencies: &'static [PromptResourceId],
    pub(crate) conflicts: &'static [PromptResourceId],
    pub(crate) source_navigation: StaticSourceNavigation,
}

const NO_RESOURCE_REFS: &[PromptResourceId] = &[];
const MANIFEST_TESTS: &[&str] = &["codex-rs/core/src/prompt_resources_tests.rs"];

fn navigation_for(id: PromptContributionKind) -> StaticSourceNavigation {
    match id {
        PromptContributionKind::BaseInstructions => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/prompt_census.rs",
                "codex-rs/core/src/prompt_debug.rs",
            ],
            symbols: &[
                "PromptCompiler",
                "BaseInstructions",
                "build_prompt_request_receipt",
            ],
            keywords: &["base instructions", "model catalog", "prompt receipt"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::CodexAgentBaseInstructions => StaticSourceNavigation {
            modules: &[
                "codex-rs/models-manager/models.json",
                "codex-rs/core/src/agent_manifest.rs",
                "codex-rs/core/src/prompt_resource_definitions.rs",
            ],
            symbols: &[
                "gpt-5.6-sol",
                "codex_agent_definition",
                "CodexAgentBaseInstructions",
            ],
            keywords: &[
                "legacy model catalog template",
                "default Codex Agent",
                "behavioral base instructions",
            ],
            tests: &[
                "codex-rs/core/src/agent_manifest_tests.rs",
                "codex-rs/core/src/prompt_resources_tests.rs",
            ],
        },
        PromptContributionKind::WorldStateDeveloperContext => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/session/world_state.rs",
                "codex-rs/core/src/context/world_state",
            ],
            symbols: &["WorldState", "ContextualUserFragment", "developer"],
            keywords: &["world state", "developer context", "fragment"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::CockpitOperatingContract => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/cockpit_operating_contract.rs",
                "codex-rs/core/src/context",
            ],
            symbols: &[
                "cockpit_operating_contract_manifest",
                "CockpitOperatingContract",
                "CockpitContractRole",
            ],
            keywords: &["root", "shadow", "resource audit", "event wake"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::WorldStateContextualUserContext => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/session/world_state.rs",
                "codex-rs/core/src/context",
            ],
            symbols: &["ContextualUserFragment", "world_state", "contextual user"],
            keywords: &["contextual user", "world state", "runtime facts"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::ConversationHistory => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/session/rollout_reconstruction.rs",
                "codex-rs/core/src/context_manager",
            ],
            symbols: &["InitialHistory", "ContextManager", "conversation history"],
            keywords: &["history", "fork", "compaction", "resume"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::InvocationInput => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/session/turn.rs",
                "codex-rs/core/src/client_common.rs",
            ],
            symbols: &["build_prompt", "ResponseItem", "user input"],
            keywords: &["invocation input", "turn", "current user"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::ToolSpecifications => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/tools/router.rs",
                "codex-rs/core/src/prompt_debug.rs",
            ],
            symbols: &["ToolRouter", "model_visible_specs", "PromptRequestReceipt"],
            keywords: &["tools", "schemas", "model-visible", "registry"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::OutputSchema => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/prompt_compiler.rs",
                "codex-rs/core/src/session/turn.rs",
            ],
            symbols: &[
                "output_schema",
                "final_output_json_schema",
                "PromptCompiler",
            ],
            keywords: &["output schema", "strict", "JSON schema"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::RealtimeSessionInstructions => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/realtime_prompt.rs",
                "codex-rs/core/src/realtime_conversation.rs",
            ],
            symbols: &[
                "BACKEND_PROMPT",
                "RealtimeConversationManager",
                "session instructions",
            ],
            keywords: &["realtime", "session instructions", "backend prompt"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::RealtimeConversationInput => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/realtime_conversation.rs",
                "codex-rs/core/src/realtime_context.rs",
            ],
            symbols: &[
                "RealtimeConversationManager",
                "RealtimeConversationEvent",
                "audio",
            ],
            keywords: &["realtime", "audio", "text", "handoff"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::RawMemoryTraces => StaticSourceNavigation {
            modules: &[
                "codex-rs/memories/write/src/prompts.rs",
                "codex-rs/memories/write/src/phase1.rs",
            ],
            symbols: &["raw_memories", "MemoryExtraction", "stage_one"],
            keywords: &["memory traces", "rollout", "selection"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::ProviderLowering => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/client.rs",
                "codex-rs/core/src/prompt_compiler.rs",
            ],
            symbols: &["ModelClient", "PromptCompiler", "ResponsesApiRequest"],
            keywords: &["provider lowering", "Responses", "Responses Lite"],
            tests: MANIFEST_TESTS,
        },
        PromptContributionKind::ProviderProcessing => StaticSourceNavigation {
            modules: &[
                "codex-rs/core/src/client.rs",
                "codex-rs/core/src/prompt_census.rs",
            ],
            symbols: &["ModelClient", "ProviderOwnedUnknown", "provider processing"],
            keywords: &["provider", "post-client", "unknown"],
            tests: MANIFEST_TESTS,
        },
    }
}

pub(crate) fn resource_definition(id: PromptContributionKind) -> StaticPromptResource {
    let (kind, classification) = match id {
        PromptContributionKind::BaseInstructions
        | PromptContributionKind::CodexAgentBaseInstructions => (
            PromptResourceKind::BaseInstructions,
            PromptResourceClassification::Static,
        ),
        PromptContributionKind::WorldStateDeveloperContext
        | PromptContributionKind::WorldStateContextualUserContext => (
            PromptResourceKind::WorldState,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::CockpitOperatingContract => (
            PromptResourceKind::OperatingContract,
            PromptResourceClassification::Static,
        ),
        PromptContributionKind::ConversationHistory => (
            PromptResourceKind::ConversationHistory,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::InvocationInput => (
            PromptResourceKind::InvocationInput,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::ToolSpecifications => (
            PromptResourceKind::ToolSpecifications,
            PromptResourceClassification::Aggregate,
        ),
        PromptContributionKind::OutputSchema => (
            PromptResourceKind::OutputSchema,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::RealtimeSessionInstructions
        | PromptContributionKind::RealtimeConversationInput => (
            PromptResourceKind::Realtime,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::RawMemoryTraces => (
            PromptResourceKind::Memory,
            PromptResourceClassification::Runtime,
        ),
        PromptContributionKind::ProviderLowering => (
            PromptResourceKind::ProviderLowering,
            PromptResourceClassification::Aggregate,
        ),
        PromptContributionKind::ProviderProcessing => (
            PromptResourceKind::ProviderProcessing,
            PromptResourceClassification::ProviderOwned,
        ),
    };

    let (
        owner,
        placement,
        provenance,
        availability,
        hard_bound,
        governance,
        inheritance,
        sensitivity,
        completeness,
    ) = match id {
        PromptContributionKind::BaseInstructions => (
            "session configuration and model catalog",
            "Responses instructions, or a leading developer item under Responses Lite",
            "BaseInstructions provenance is retained in core but not yet emitted by request receipts",
            "effective value is runtime-enumerable in prompt-receipt",
            "no independent uniform bound; participates in the model context window",
            "versioned model default or operator-authored override",
            "full forks may preserve it; fresh role layers may replace it",
            "may contain private operator instructions",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::CodexAgentBaseInstructions => (
            "Agent Definition codex@1",
            "Agent-owned base-instructions resource; declaration-only until explicit Agent compilation is enabled",
            "references the gpt-5.6-sol legacy model-catalog template during the strangler migration",
            "statically enumerable in prompt-resources and agents; not selected on a request path in this slice",
            "no independent uniform bound; participates in the model context window once compiled",
            "versioned Agent resource; model identity must not select it",
            "pinned by Agent Definition revision; resume and fork rules begin with explicit Agent selection",
            "may contain private operator instructions when replaced or extended",
            CensusCompleteness::Static,
        ),
        PromptContributionKind::WorldStateDeveloperContext => (
            "session::world_state and registered context contributors",
            "ordered developer messages before contextual user messages and live turn input",
            "built-in fragment types plus extension contributors; item-level provenance is not preserved after assembly",
            "effective items are runtime-enumerable; contributor identity is incomplete",
            "fragment-specific bounds only; no uniform aggregate bound",
            "mixed versioned contract, harness runtime facts, and extension-authored context",
            "recomputed from child session configuration and inherited state",
            "may contain paths, policies, environment facts, and extension content",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::CockpitOperatingContract => (
            "Lumi Prompt / Context Plane",
            "one marked standalone developer message before generic multi-agent hints",
            "stable contract id, revision, role, hash, bounds, and governance are emitted by prompt-receipt",
            "runtime-enumerable with exact effective copy count in prompt-receipt; static manifest is available through debug prompt-contract",
            "4096 UTF-8 bytes",
            "versioned built-in contract with prompt-plane conformance review",
            "root fresh receives root; full, bounded, and fresh role shadows receive exactly one shadow copy; non-owning internal agents receive zero",
            "static public operating semantics; contains no runtime state or user content",
            CensusCompleteness::RuntimeEnumerable,
        ),
        PromptContributionKind::WorldStateContextualUserContext => (
            "session::world_state contextual user fragments",
            "ordered contextual user messages after developer context and before live user input",
            "built-in fragment types; item-level provenance is not preserved after assembly",
            "effective items are runtime-enumerable; contributor identity is incomplete",
            "fragment-specific bounds only; no uniform aggregate bound",
            "mixed harness runtime facts and versioned context policy",
            "recomputed for each session and turn",
            "may contain runtime state or prior task context",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::ConversationHistory => (
            "context_manager::history",
            "after initial context according to recorded item order",
            "rollout items retain roles and item kinds; contribution lineage is only partially explicit",
            "runtime-enumerable in the effective request",
            "model context window plus truncation and compaction policy",
            "user, assistant, tool, and runtime-authored transcript",
            "full, last-N, fresh, or referenced history according to fork policy",
            "may contain user content, tool output, secrets, and encrypted provider items",
            CensusCompleteness::RuntimeEnumerable,
        ),
        PromptContributionKind::InvocationInput => (
            "logical invocation owner",
            "invocation-specific items appended to or assembled with history",
            "call-site-owned prompt, trigger, transcript, or current user input",
            "runtime-enumerable in the effective request",
            "invocation-specific; Guardian and compaction apply explicit trimming",
            "user-authored, versioned task prompt, or harness runtime fact",
            "invocation-specific and normally not inherited as a base contract",
            "may contain user content and selected runtime evidence",
            CensusCompleteness::RuntimeEnumerable,
        ),
        PromptContributionKind::ToolSpecifications => (
            "ToolRouter and dynamic tool registry",
            "Responses tools field, or leading AdditionalTools item under Responses Lite",
            "effective schemas are visible; per-tool registration provenance is not yet emitted",
            "effective tool schemas are runtime-enumerable in prompt-receipt",
            "no locally enforced uniform aggregate bound",
            "versioned built-ins, MCP/app/plugin tools, and runtime dynamic tools",
            "rebuilt for each step from session capabilities and runtime bindings",
            "schemas may expose internal names, descriptions, and argument structure",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::OutputSchema => (
            "turn context and invocation policy",
            "Responses text.format JSON schema control",
            "caller-supplied final output schema plus invocation strictness policy",
            "runtime-enumerable in prompt-receipt",
            "no independent local uniform bound",
            "caller-authored or versioned invocation contract",
            "turn-scoped",
            "normally structural; descriptions may contain private contract text",
            CensusCompleteness::RuntimeEnumerable,
        ),
        PromptContributionKind::RealtimeSessionInstructions => (
            "realtime_prompt and realtime session configuration",
            "realtime session instructions",
            "config override, request override, or versioned backend prompt",
            "effective session config exists at runtime but has no prompt-plane receipt",
            "provider contract; no client-wide bound recorded",
            "operator-authored override or versioned default",
            "realtime-call scoped",
            "may contain private operator instructions and user identity hints",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::RealtimeConversationInput => (
            "realtime_conversation input channels",
            "ordered live audio, text, and handoff events",
            "channel and event kinds are known at runtime",
            "event-stream enumerable; no consolidated request receipt",
            "bounded local queues; provider session history bound is not client-visible",
            "user input and harness handoff runtime facts",
            "realtime-call scoped",
            "may contain voice, transcript, user content, and model handoffs",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::RawMemoryTraces => (
            "memories stage-one selection",
            "typed raw_memories endpoint payload",
            "selected rollout and memory records",
            "runtime payload is enumerable; no prompt-plane receipt or redaction summary",
            "memory selection policy",
            "harness-selected historical runtime facts",
            "memory job scoped",
            "may contain prior user, workspace, and rollout content",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::ProviderLowering => (
            "ModelClient request builder",
            "normal Responses fields or Responses Lite developer input lowering",
            "provider configuration and model capability flags",
            "logical full request is runtime-enumerable in prompt-receipt for ordinary turns",
            "transport-specific normalization and compatibility rules",
            "versioned client implementation plus provider configuration",
            "resolved per model request",
            "may rearrange sensitive contributions without changing their sensitivity",
            CensusCompleteness::Incomplete,
        ),
        PromptContributionKind::ProviderProcessing => (
            "model provider",
            "after the client-emitted request boundary",
            "not observable from the Codex client",
            "not runtime-enumerable by the client",
            "provider-owned",
            "provider-owned unknown",
            "provider-owned unknown",
            "inherits all request sensitivity",
            CensusCompleteness::ProviderOwnedUnknown,
        ),
    };

    StaticPromptResource {
        id,
        kind,
        classification,
        owner,
        placement,
        provenance,
        availability,
        hard_bound,
        governance,
        inheritance,
        sensitivity,
        completeness,
        dependencies: NO_RESOURCE_REFS,
        conflicts: NO_RESOURCE_REFS,
        source_navigation: navigation_for(id),
    }
}
