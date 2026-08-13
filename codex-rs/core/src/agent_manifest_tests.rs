use std::collections::HashMap;
use std::collections::HashSet;

use pretty_assertions::assert_eq;

use super::AgentCapabilityRef;
use super::AgentDefinitionRef;
use super::AgentLaunchControl;
use super::AgentPlayRef;
use super::CODEX_AGENT_ID;
use super::CODEX_AGENT_REVISION;
use super::CODEX_SOL_MODEL_TARGET;
use super::CODEX_SOL_PRESET_ID;
use super::CODEX_SOL_PRESET_REVISION;
use super::agent_catalog_manifest;
use super::validate_agent_catalog_manifest;
use super::validate_manifest;
use crate::PromptContributionKind;
use crate::PromptInvocationKind;
use crate::prompt_resource_manifest;

#[test]
fn catalog_is_deterministic_and_declares_the_initial_agent_and_preset() {
    let first = agent_catalog_manifest().expect("static Agent catalog should validate");
    let second = agent_catalog_manifest().expect("static Agent catalog should validate");
    assert_eq!(first, second);

    assert_eq!(first.agent_definitions.len(), 1);
    let agent = &first.agent_definitions[0];
    assert_eq!((agent.id.as_str(), agent.revision), (CODEX_AGENT_ID, 1));
    assert_eq!(
        agent.prompt_resource_refs,
        vec![PromptContributionKind::CodexAgentBaseInstructions]
    );
    assert_eq!(agent.capability_refs.len(), 1);
    assert!(agent.play_refs.is_empty());

    assert_eq!(first.launch_presets.len(), 1);
    let preset = &first.launch_presets[0];
    assert_eq!(
        (preset.id.as_str(), preset.revision),
        (CODEX_SOL_PRESET_ID, CODEX_SOL_PRESET_REVISION)
    );
    assert_eq!(
        preset.agent,
        AgentDefinitionRef {
            id: CODEX_AGENT_ID.to_string(),
            revision: CODEX_AGENT_REVISION,
        }
    );
    assert_eq!(preset.default_model_target, CODEX_SOL_MODEL_TARGET);
    assert_eq!(
        preset.user_adjustable_controls,
        vec![
            AgentLaunchControl::ReasoningEffort,
            AgentLaunchControl::ServiceTier,
        ]
    );
}

#[test]
fn codex_behavior_resource_exists_once_with_agent_owned_legacy_provenance() {
    let resources = prompt_resource_manifest().expect("prompt resources");
    let matches = resources
        .resources
        .iter()
        .filter(|resource| resource.id == PromptContributionKind::CodexAgentBaseInstructions)
        .collect::<Vec<_>>();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].owner, "Agent Definition codex@1");
    assert!(matches[0].provenance.contains("legacy model-catalog"));
    assert!(
        matches[0]
            .source_navigation
            .modules
            .iter()
            .any(|module| module == "codex-rs/models-manager/models.json")
    );
    assert!(PromptInvocationKind::ALL.into_iter().all(|invocation| {
        !invocation
            .contributions()
            .contains(&PromptContributionKind::CodexAgentBaseInstructions)
    }));
}

#[test]
fn validation_rejects_duplicate_and_dangling_definition_references() {
    let mut empty = agent_catalog_manifest().expect("valid catalog");
    empty.agent_definitions.clear();
    assert!(validate_agent_catalog_manifest(&empty).is_err());

    let mut duplicate = agent_catalog_manifest().expect("valid catalog");
    duplicate
        .agent_definitions
        .push(duplicate.agent_definitions[0].clone());
    assert!(validate_agent_catalog_manifest(&duplicate).is_err());

    let dangling_resource = agent_catalog_manifest().expect("valid catalog");
    let mut resources = prompt_resource_manifest().expect("prompt resources");
    resources
        .resources
        .retain(|resource| resource.id != PromptContributionKind::CodexAgentBaseInstructions);
    let capabilities = HashMap::from([(
        (
            crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_ID.to_string(),
            crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_REVISION,
        ),
        PromptContributionKind::CockpitOperatingContract,
    )]);
    let models = HashSet::from([CODEX_SOL_MODEL_TARGET.to_string()]);
    assert!(
        validate_manifest(
            &dangling_resource,
            &resources,
            &capabilities,
            &HashSet::new(),
            &models,
        )
        .is_err()
    );

    let dangling_capability_resource = agent_catalog_manifest().expect("valid catalog");
    let mut resources = prompt_resource_manifest().expect("prompt resources");
    resources
        .resources
        .retain(|resource| resource.id != PromptContributionKind::CockpitOperatingContract);
    assert!(
        validate_manifest(
            &dangling_capability_resource,
            &resources,
            &capabilities,
            &HashSet::new(),
            &models,
        )
        .is_err()
    );

    let mut dangling_capability = agent_catalog_manifest().expect("valid catalog");
    dangling_capability.agent_definitions[0].capability_refs = vec![AgentCapabilityRef {
        id: "missing_capability".to_string(),
        revision: 1,
    }];
    assert!(validate_agent_catalog_manifest(&dangling_capability).is_err());

    let mut dangling_play = agent_catalog_manifest().expect("valid catalog");
    dangling_play.agent_definitions[0].play_refs = vec![AgentPlayRef {
        id: "missing_play".to_string(),
        revision: 1,
    }];
    assert!(validate_agent_catalog_manifest(&dangling_play).is_err());
}

#[test]
fn validation_rejects_agent_edges_and_unresolved_presets() {
    let own_ref = AgentDefinitionRef {
        id: CODEX_AGENT_ID.to_string(),
        revision: CODEX_AGENT_REVISION,
    };
    let mut self_dependency = agent_catalog_manifest().expect("valid catalog");
    self_dependency.agent_definitions[0].dependencies = vec![own_ref.clone()];
    assert!(validate_agent_catalog_manifest(&self_dependency).is_err());

    let mut overlap = agent_catalog_manifest().expect("valid catalog");
    overlap.agent_definitions[0].dependencies = vec![own_ref.clone()];
    overlap.agent_definitions[0].conflicts = vec![own_ref];
    assert!(validate_agent_catalog_manifest(&overlap).is_err());

    let mut unresolved_agent = agent_catalog_manifest().expect("valid catalog");
    unresolved_agent.launch_presets[0].agent.id = "missing".to_string();
    assert!(validate_agent_catalog_manifest(&unresolved_agent).is_err());

    let mut unresolved_model = agent_catalog_manifest().expect("valid catalog");
    unresolved_model.launch_presets[0].default_model_target = "missing-model".to_string();
    assert!(validate_agent_catalog_manifest(&unresolved_model).is_err());
}

#[test]
fn validation_rejects_unsafe_navigation_and_duplicate_controls() {
    let mut unsafe_navigation = agent_catalog_manifest().expect("valid catalog");
    unsafe_navigation.agent_definitions[0]
        .source_navigation
        .modules[0] = "codex-rs/core/../../secret".to_string();
    assert!(validate_agent_catalog_manifest(&unsafe_navigation).is_err());

    let mut duplicate_controls = agent_catalog_manifest().expect("valid catalog");
    duplicate_controls.launch_presets[0]
        .user_adjustable_controls
        .push(AgentLaunchControl::ReasoningEffort);
    assert!(validate_agent_catalog_manifest(&duplicate_controls).is_err());
}
