use std::sync::Arc;

use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use pretty_assertions::assert_eq;
use serde_json::json;

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
        provenance: None,
    };
    let output_schema = Some(json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
    }));
    let compiler = PromptCompiler {
        state: PromptCompilerState {
            tools: Arc::clone(&tools),
            parallel_tool_calls: true,
            base_instructions: base_instructions.clone(),
            output_schema: output_schema.clone(),
            output_schema_strict: false,
        },
    };

    let actual = compiler.compile_prompt(input.clone());
    let expected = Prompt {
        input,
        tools,
        parallel_tool_calls: true,
        base_instructions,
        output_schema,
        output_schema_strict: false,
    };

    assert_eq!(actual, expected);
}
