use std::sync::Arc;

use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ResponseItem;
use codex_tools::ToolSpec;
use serde_json::Value;
use tracing::instrument;

use crate::client_common::Prompt;
use crate::guardian::is_guardian_reviewer_source;
use crate::session::step_context::StepContext;

/// Named request shapes owned by the prompt compiler facade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromptCompilationTarget {
    Turn,
    StartupPrewarm,
    LocalCompaction,
    RemoteCompaction,
    RemoteCompactionV2,
}

/// Facade for compiling a model request from one resolved invocation snapshot.
///
/// The compiler owns only derived request state. Runtime and session truth remain owned by the
/// captured [`StepContext`] and the session that supplied `base_instructions`.
pub(crate) struct PromptCompiler {
    state: PromptCompilerState,
}

/// Immutable inputs that apply to each prompt compiled by this facade instance.
struct PromptCompilerState {
    target: PromptCompilationTarget,
    tools: Arc<[ToolSpec]>,
    parallel_tool_calls: bool,
    base_instructions: BaseInstructions,
    output_schema: Option<Value>,
    output_schema_strict: bool,
}

impl PromptCompiler {
    /// Captures a regular turn's model-visible request state.
    pub(crate) fn for_turn(
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        Self::for_step_target(
            PromptCompilationTarget::Turn,
            step_context,
            base_instructions,
        )
    }

    /// Captures the semantically transparent startup prewarm request state.
    pub(crate) fn for_startup_prewarm(
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        Self::for_step_target(
            PromptCompilationTarget::StartupPrewarm,
            step_context,
            base_instructions,
        )
    }

    /// Captures a local summarization compaction request with no advertised tools.
    pub(crate) fn for_local_compaction(base_instructions: BaseInstructions) -> Self {
        Self {
            state: PromptCompilerState {
                target: PromptCompilationTarget::LocalCompaction,
                tools: Arc::default(),
                parallel_tool_calls: false,
                base_instructions,
                output_schema: None,
                output_schema_strict: true,
            },
        }
    }

    /// Captures a legacy remote compaction request from the exact step tool snapshot.
    pub(crate) fn for_remote_compaction(
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        Self::for_step_target(
            PromptCompilationTarget::RemoteCompaction,
            step_context,
            base_instructions,
        )
    }

    /// Captures a remote-v2 compaction request and its required trigger item.
    pub(crate) fn for_remote_compaction_v2(
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        Self::for_step_target(
            PromptCompilationTarget::RemoteCompactionV2,
            step_context,
            base_instructions,
        )
    }

    fn for_step_target(
        target: PromptCompilationTarget,
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        let turn = step_context.turn.as_ref();
        let (output_schema, output_schema_strict) = match target {
            PromptCompilationTarget::Turn | PromptCompilationTarget::StartupPrewarm => (
                turn.final_output_json_schema.clone(),
                !is_guardian_reviewer_source(&turn.session_source),
            ),
            PromptCompilationTarget::RemoteCompaction
            | PromptCompilationTarget::RemoteCompactionV2 => (None, true),
            PromptCompilationTarget::LocalCompaction => {
                unreachable!("local compaction does not capture a step context")
            }
        };
        Self {
            state: PromptCompilerState {
                target,
                tools: step_context.tool_router.model_visible_specs(),
                parallel_tool_calls: turn.model_info.supports_parallel_tool_calls,
                base_instructions,
                output_schema,
                output_schema_strict,
            },
        }
    }

    /// Compiles one logical prompt while preserving the captured step state across retries.
    #[instrument(name = "build_prompt", level = "trace", skip_all)]
    pub(crate) fn compile_prompt(&self, mut input: Vec<ResponseItem>) -> Prompt {
        if self.state.target == PromptCompilationTarget::RemoteCompactionV2 {
            input.push(ResponseItem::CompactionTrigger {});
        }
        Prompt {
            input,
            tools: Arc::clone(&self.state.tools),
            parallel_tool_calls: self.state.parallel_tool_calls,
            base_instructions: self.state.base_instructions.clone(),
            output_schema: self.state.output_schema.clone(),
            output_schema_strict: self.state.output_schema_strict,
        }
    }
}

#[cfg(test)]
#[path = "prompt_compiler_tests.rs"]
mod tests;
