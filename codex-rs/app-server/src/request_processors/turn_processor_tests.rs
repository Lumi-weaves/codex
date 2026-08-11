use super::cross_provider_model_switch_error;
use super::requested_model_for_provider_validation;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use std::collections::HashMap;

#[test]
fn routed_models_require_a_new_task_when_the_provider_changes() {
    let routes = HashMap::from([(
        "alibaba-token-plan/qwen3.8-max".to_string(),
        "opencodex".to_string(),
    )]);

    assert!(
        cross_provider_model_switch_error(
            "alibaba-token-plan/qwen3.8-max",
            "gpt-5.6-sol",
            "openai",
            &routes,
        )
        .is_some()
    );
    assert!(
        cross_provider_model_switch_error(
            "gpt-5.6-sol",
            "alibaba-token-plan/qwen3.8-max",
            "opencodex",
            &routes,
        )
        .is_some()
    );
    assert_eq!(
        cross_provider_model_switch_error(
            "alibaba-token-plan/qwen3.8-max",
            "alibaba-token-plan/qwen3.8-max",
            "opencodex",
            &routes,
        ),
        None
    );
    assert_eq!(
        cross_provider_model_switch_error("gpt-5.6-sol", "gpt-5.6-terra", "openai", &routes),
        None
    );
}

#[test]
fn unrelated_explicit_providers_keep_their_existing_model_switch_behavior() {
    let routes = HashMap::from([(
        "alibaba-token-plan/qwen3.8-max".to_string(),
        "opencodex".to_string(),
    )]);

    assert_eq!(
        cross_provider_model_switch_error("custom-model", "other-model", "custom", &routes),
        None
    );
}

#[test]
fn collaboration_mode_model_takes_precedence_for_provider_validation() {
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Plan,
        settings: Settings {
            model: "alibaba-token-plan/qwen3.8-max".to_string(),
            reasoning_effort: None,
            developer_instructions: None,
        },
    };

    assert_eq!(
        requested_model_for_provider_validation(Some("gpt-5.6-sol"), Some(&collaboration_mode),),
        Some("alibaba-token-plan/qwen3.8-max")
    );
}
