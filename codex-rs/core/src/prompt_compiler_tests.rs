use std::sync::Arc;

use codex_protocol::agent::AgentDefinitionRef;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::BaseInstructionsProvenance;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::PromptCompilationTarget;
use super::PromptCompiler;
use super::PromptCompilerState;
use crate::client_common::Prompt;

#[test]
fn compile_prompt_preserves_the_resolved_step_state() {
    let input = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "compile this turn".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }];
    let tools: Arc<[ToolSpec]> = Arc::from([ToolSpec::Function(ResponsesApiTool {
        name: "sample_tool".to_string(),
        description: "A model-visible test tool.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::default(),
        output_schema: None,
    })]);
    let base_instructions = BaseInstructions {
        text: "resolved base instructions".to_string(),
        provenance: Some(BaseInstructionsProvenance::Agent {
            agent: AgentDefinitionRef {
                id: "codex".to_string(),
                revision: 1,
            },
        }),
    };
    let output_schema = Some(json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
    }));
    for target in [
        PromptCompilationTarget::Turn,
        PromptCompilationTarget::StartupPrewarm,
    ] {
        let compiler = PromptCompiler {
            state: PromptCompilerState {
                target,
                tools: Arc::clone(&tools),
                parallel_tool_calls: true,
                base_instructions: base_instructions.clone(),
                output_schema: output_schema.clone(),
                output_schema_strict: false,
            },
        };
        let expected = Prompt {
            input: input.clone(),
            tools: Arc::clone(&tools),
            parallel_tool_calls: true,
            base_instructions: base_instructions.clone(),
            output_schema: output_schema.clone(),
            output_schema_strict: false,
        };

        assert_eq!(compiler.compile_prompt(input.clone()), expected);
    }
}

#[test]
fn compaction_targets_preserve_their_distinct_request_shapes() {
    let input = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "history to compact".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }];
    let tools: Arc<[ToolSpec]> = Arc::from([ToolSpec::Function(ResponsesApiTool {
        name: "sample_tool".to_string(),
        description: "A model-visible test tool.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::default(),
        output_schema: None,
    })]);
    let base_instructions = BaseInstructions {
        text: "compaction base instructions".to_string(),
        provenance: None,
    };

    let local = PromptCompiler::for_local_compaction(base_instructions.clone())
        .compile_prompt(input.clone());
    assert_eq!(
        local,
        Prompt {
            input: input.clone(),
            tools: Arc::default(),
            parallel_tool_calls: false,
            base_instructions: base_instructions.clone(),
            output_schema: None,
            output_schema_strict: true,
        }
    );

    let remote = PromptCompiler {
        state: PromptCompilerState {
            target: PromptCompilationTarget::RemoteCompaction,
            tools: Arc::clone(&tools),
            parallel_tool_calls: true,
            base_instructions: base_instructions.clone(),
            output_schema: None,
            output_schema_strict: true,
        },
    }
    .compile_prompt(input.clone());
    assert_eq!(
        remote,
        Prompt {
            input: input.clone(),
            tools: Arc::clone(&tools),
            parallel_tool_calls: true,
            base_instructions: base_instructions.clone(),
            output_schema: None,
            output_schema_strict: true,
        }
    );

    let remote_v2 = PromptCompiler {
        state: PromptCompilerState {
            target: PromptCompilationTarget::RemoteCompactionV2,
            tools: Arc::clone(&tools),
            parallel_tool_calls: true,
            base_instructions: base_instructions.clone(),
            output_schema: None,
            output_schema_strict: true,
        },
    }
    .compile_prompt(input.clone());
    assert_eq!(
        remote_v2,
        Prompt {
            input: vec![input[0].clone(), ResponseItem::CompactionTrigger {}],
            tools,
            parallel_tool_calls: true,
            base_instructions,
            output_schema: None,
            output_schema_strict: true,
        }
    );
}
