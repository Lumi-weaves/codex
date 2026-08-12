use std::sync::Arc;

use anyhow::Result;
use codex_core::PromptReceiptView;
use codex_core::build_prompt_input;
use codex_core::build_prompt_request_receipt;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_home::CodexHomeUserInstructionsProvider;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::strip_metadata;
use core_test_support::responses::strip_response_item_id;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

const TEST_INSTRUCTIONS: &str = "Global test instructions";

#[tokio::test]
async fn build_prompt_input_includes_context_and_user_message() -> Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(codex_home.path().join("AGENTS.md"), TEST_INSTRUCTIONS)?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            codex_self_exe: Some(std::env::current_exe()?),
            ..ConfigOverrides::default()
        })
        .build()
        .await?;
    let user_instructions_provider = Arc::new(CodexHomeUserInstructionsProvider::new(
        config.codex_home.clone(),
    ));
    let input = build_prompt_input(
        config,
        vec![UserInput::Text {
            text: "hello from debug prompt".to_string(),
            text_elements: Vec::new(),
        }],
        /*state_db*/ None,
        Arc::new(ExtensionRegistryBuilder::new().build()),
        user_instructions_provider,
    )
    .await?;

    let expected_user_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "hello from debug prompt".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    assert_eq!(
        input
            .last()
            .cloned()
            .map(strip_metadata)
            .map(strip_response_item_id),
        Some(expected_user_message)
    );
    assert!(input.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };

        content.iter().any(|content_item| {
            let (ContentItem::InputText { text } | ContentItem::OutputText { text }) = content_item
            else {
                return false;
            };
            text.contains(TEST_INSTRUCTIONS)
        })
    }));
    Ok(())
}

#[tokio::test]
async fn build_prompt_request_receipt_includes_effective_request() -> Result<()> {
    const BASE_INSTRUCTIONS: &str = "Base effective instructions";

    for use_responses_lite in [false, true] {
        let codex_home = TempDir::new()?;
        let cwd = TempDir::new()?;
        std::fs::write(codex_home.path().join("AGENTS.md"), TEST_INSTRUCTIONS)?;
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .harness_overrides(ConfigOverrides {
                cwd: Some(cwd.path().to_path_buf()),
                codex_self_exe: Some(std::env::current_exe()?),
                ..ConfigOverrides::default()
            })
            .build()
            .await?;
        let model = "gpt-5.4";
        let mut model_catalog = codex_models_manager::bundled_models_response()?;
        model_catalog
            .models
            .iter_mut()
            .find(|model_info| model_info.slug == model)
            .expect("selected model should exist in bundled catalog")
            .use_responses_lite = use_responses_lite;
        config.model_catalog = Some(model_catalog);
        config.model = Some(model.to_string());
        config.base_instructions = Some(BASE_INSTRUCTIONS.to_string());
        let user_instructions_provider = Arc::new(CodexHomeUserInstructionsProvider::new(
            config.codex_home.clone(),
        ));
        let receipt = build_prompt_request_receipt(
            config,
            vec![UserInput::Text {
                text: "hello from effective request".to_string(),
                text_elements: Vec::new(),
            }],
            /*state_db*/ None,
            Arc::new(ExtensionRegistryBuilder::new().build()),
            user_instructions_provider,
        )
        .await?;

        let metadata_json = serde_json::to_value(receipt.render(PromptReceiptView::MetadataOnly))?;
        assert_eq!(metadata_json["schemaVersion"], 2);
        assert_eq!(
            metadata_json["compilerRevision"],
            "responses_request_lowering_v1"
        );
        assert_eq!(metadata_json["invocationKind"], "turn");
        assert_eq!(metadata_json["requestForm"], "logical_full");
        assert_eq!(metadata_json["provider"]["wireApi"], "responses");
        assert_eq!(metadata_json["redaction"]["view"], "metadata_only");
        assert_eq!(metadata_json["redaction"]["contentIncluded"], false);
        assert!(metadata_json.get("request").is_none());
        assert!(!metadata_json.to_string().contains(TEST_INSTRUCTIONS));
        assert!(
            !metadata_json
                .to_string()
                .contains("hello from effective request")
        );
        assert_eq!(metadata_json["provenance"]["censusSchemaVersion"], 1);
        assert!(
            metadata_json["provenance"]["contributionRefs"]
                .as_array()
                .is_some_and(|refs| refs.iter().any(|value| value == "base_instructions"))
        );
        assert_eq!(
            metadata_json["summary"]["canonicalRequestSha256"]
                .as_str()
                .map(str::len),
            Some(64)
        );
        assert!(
            metadata_json["summary"]["estimatedModelVisibleTokens"]
                .as_u64()
                .is_some_and(|tokens| tokens > 0)
        );

        let full_json = serde_json::to_value(receipt.render(PromptReceiptView::FullLocal))?;
        assert_eq!(full_json["redaction"]["view"], "full_local");
        assert_eq!(full_json["redaction"]["contentIncluded"], true);
        assert_eq!(
            metadata_json["summary"]["canonicalRequestSha256"],
            full_json["summary"]["canonicalRequestSha256"]
        );
        let request = &full_json["request"];
        assert!(request["input"].as_array().is_some_and(|input| {
            input.iter().any(|item| {
                item["content"].as_array().is_some_and(|content| {
                    content
                        .iter()
                        .any(|part| part["text"] == "hello from effective request")
                })
            })
        }));
        assert!(request.to_string().contains(TEST_INSTRUCTIONS));

        if use_responses_lite {
            assert_eq!(full_json["provider"]["lowering"], "responses_lite");
            assert!(request.get("instructions").is_none());
            assert!(request.get("tools").is_none());
            let input = request["input"]
                .as_array()
                .expect("request input should be an array");
            assert_eq!(input[0]["type"], "additional_tools");
            assert_eq!(input[0]["role"], "developer");
            assert_eq!(input[1]["role"], "developer");
            assert_eq!(input[1]["content"][0]["text"], BASE_INSTRUCTIONS);
        } else {
            assert_eq!(full_json["provider"]["lowering"], "responses");
            assert_eq!(request["instructions"], BASE_INSTRUCTIONS);
            assert!(request.get("tools").is_some());
        }
    }
    Ok(())
}
