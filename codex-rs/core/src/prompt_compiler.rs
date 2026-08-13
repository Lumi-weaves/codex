use std::sync::Arc;

use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ResponseItem;
use codex_tools::ToolSpec;
use serde_json::Value;
use tracing::instrument;

use crate::client_common::Prompt;
use crate::guardian::is_guardian_reviewer_source;
use crate::session::step_context::StepContext;

/// Facade for compiling a model request from one resolved sampling-step snapshot.
///
/// The compiler owns only derived request state. Runtime and session truth remain owned by the
/// captured [`StepContext`] and the session that supplied `base_instructions`.
pub(crate) struct PromptCompiler {
    state: PromptCompilerState,
}

/// Immutable inputs that apply to every request attempt in one sampling step.
struct PromptCompilerState {
    tools: Arc<[ToolSpec]>,
    parallel_tool_calls: bool,
    base_instructions: BaseInstructions,
    output_schema: Option<Value>,
    output_schema_strict: bool,
}

impl PromptCompiler {
    /// Captures the model-visible request state resolved for `step_context`.
    pub(crate) fn for_step(
        step_context: &StepContext,
        base_instructions: BaseInstructions,
    ) -> Self {
        let turn = step_context.turn.as_ref();
        Self {
            state: PromptCompilerState {
                tools: step_context.tool_router.model_visible_specs(),
                parallel_tool_calls: turn.model_info.supports_parallel_tool_calls,
                base_instructions,
                output_schema: turn.final_output_json_schema.clone(),
                output_schema_strict: !is_guardian_reviewer_source(&turn.session_source),
            },
        }
    }

    /// Compiles one logical prompt while preserving the captured step state across retries.
    #[instrument(name = "build_prompt", level = "trace", skip_all)]
    pub(crate) fn compile_prompt(&self, input: Vec<ResponseItem>) -> Prompt {
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
