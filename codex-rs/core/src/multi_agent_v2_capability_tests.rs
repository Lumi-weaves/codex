use pretty_assertions::assert_eq;

use super::MULTI_AGENT_V2_CAPABILITY_ID;
use super::multi_agent_v2_capability_manifest;

#[test]
fn manifest_links_the_cockpit_contract_to_every_lifecycle_action() {
    let manifest = multi_agent_v2_capability_manifest();
    assert_eq!(manifest.id, MULTI_AGENT_V2_CAPABILITY_ID);
    assert_eq!(
        manifest.prompt_resource_id,
        crate::prompt_census::PromptContributionKind::CockpitOperatingContract
    );
    assert_eq!(
        manifest
            .actions
            .iter()
            .map(|action| action.id)
            .collect::<Vec<_>>(),
        vec![
            "spawn_agent",
            "send_message",
            "followup_task",
            "wait_agent",
            "interrupt_agent",
            "close_agent",
            "list_agents",
        ]
    );
    assert_eq!(manifest.lifecycle.closure_action, "close_agent");
    assert!(!manifest.lifecycle.stopped_is_closed);
    assert!(manifest.lifecycle.successful_close_receipt_is_closure);
}

#[test]
fn manifest_navigation_is_flat_and_complete() {
    let navigation = multi_agent_v2_capability_manifest().source_navigation;
    assert!(!navigation.modules.is_empty());
    assert!(!navigation.symbols.is_empty());
    assert!(!navigation.keywords.is_empty());
    assert!(!navigation.tests.is_empty());
    assert!(
        navigation
            .modules
            .iter()
            .all(|path| path.starts_with("codex-rs/core/"))
    );
}
