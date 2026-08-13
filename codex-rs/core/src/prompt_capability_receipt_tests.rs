use codex_api::ResponsesApiRequest;
use codex_protocol::models::ResponseItem;

use super::inspect_multi_agent_v2_capability;
use crate::cockpit_operating_contract::CockpitContractRole;
use crate::multi_agent_v2_capability::MultiAgentV2Action;
use crate::multi_agent_v2_capability::MultiAgentV2CapabilityProjection;
use crate::multi_agent_v2_capability::MultiAgentV2ProjectionOmission;

fn request_with(input: Vec<ResponseItem>) -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: "test-model".to_string(),
        instructions: String::new(),
        input,
        tools: None,
        tool_choice: "auto".to_string(),
        parallel_tool_calls: false,
        reasoning: None,
        store: false,
        stream: true,
        stream_options: None,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    }
}

#[test]
fn receipt_links_namespaced_actions_to_the_contract_resource() {
    let actions = [
        "spawn_agent",
        "send_message",
        "followup_task",
        "wait_agent",
        "interrupt_agent",
        "close_agent",
        "list_agents",
    ];
    let request = request_with(vec![ResponseItem::AdditionalTools {
        id: None,
        role: "developer".to_string(),
        tools: vec![serde_json::json!({
            "type": "namespace",
            "name": "agents",
            "description": "test collaboration tools",
            "tools": actions
                .iter()
                .map(|name| serde_json::json!({"type": "function", "name": name}))
                .collect::<Vec<_>>(),
        })],
    }]);
    let projection = MultiAgentV2CapabilityProjection {
        enabled: true,
        prompt_role: Some(CockpitContractRole::Root),
        prompt_omission: None,
        tools_included: true,
        tools_omission: None,
        tool_namespace: Some("agents".to_string()),
        plaintext_messages: false,
        actions: MultiAgentV2Action::ALL.to_vec(),
    };

    let receipt =
        inspect_multi_agent_v2_capability(Some(&projection), &request).expect("capability receipt");
    assert_eq!(receipt.expected_action_ids, receipt.effective_action_ids);
    assert_eq!(receipt.role, Some(CockpitContractRole::Root));
    assert_eq!(
        receipt.prompt_resource_id,
        crate::prompt_census::PromptContributionKind::CockpitOperatingContract
    );
}

#[test]
fn receipt_rejects_a_missing_declared_action() {
    let request = request_with(vec![ResponseItem::AdditionalTools {
        id: None,
        role: "developer".to_string(),
        tools: vec![serde_json::json!({
            "type": "function",
            "name": "spawn_agent",
        })],
    }]);
    let projection = MultiAgentV2CapabilityProjection {
        enabled: true,
        prompt_role: None,
        prompt_omission: Some(MultiAgentV2ProjectionOmission::PromptRoleIneligible),
        tools_included: true,
        tools_omission: None,
        tool_namespace: None,
        plaintext_messages: false,
        actions: vec![
            MultiAgentV2Action::SpawnAgent,
            MultiAgentV2Action::CloseAgent,
        ],
    };

    let error = inspect_multi_agent_v2_capability(Some(&projection), &request)
        .expect_err("missing close_agent should fail conformance");
    assert!(error.to_string().contains("expected actions"));
    assert!(error.to_string().contains("close_agent"));
}
