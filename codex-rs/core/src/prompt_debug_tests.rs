use super::*;
use std::collections::HashMap;

fn request_with(
    input: Vec<ResponseItem>,
    client_metadata: Option<HashMap<String, String>>,
) -> ResponsesApiRequest {
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
        client_metadata,
    }
}

fn message(role: &str, text: String) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText { text }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn cockpit_receipt_ignores_contract_markers_outside_developer_input_messages() {
    let rendered = crate::cockpit_operating_contract::rendered_contract(CockpitContractRole::Root);
    let request = request_with(
        vec![message("user", rendered.clone())],
        Some(HashMap::from([("diagnostic".to_string(), rendered)])),
    );

    let receipt = CockpitContractReceipt::inspect(None, &request).expect("excluded receipt");
    assert_eq!(receipt.effective_copy_count, 0);
}

#[test]
fn cockpit_receipt_counts_the_exact_standalone_developer_fragment() {
    let rendered = crate::cockpit_operating_contract::rendered_contract(CockpitContractRole::Root);
    let request = request_with(vec![message("developer", rendered)], None);

    let receipt = CockpitContractReceipt::inspect(Some(CockpitContractRole::Root), &request)
        .expect("included receipt");
    assert_eq!(receipt.effective_copy_count, 1);
}
