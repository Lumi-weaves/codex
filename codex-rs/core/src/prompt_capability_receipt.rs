use codex_api::ResponsesApiRequest;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use serde::Serialize;
use serde_json::Value;

use crate::cockpit_operating_contract::CockpitContractRole;
use crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_ID;
use crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_REVISION;
use crate::multi_agent_v2_capability::MultiAgentV2CapabilityProjection;
use crate::multi_agent_v2_capability::MultiAgentV2ProjectionOmission;
use crate::prompt_census::PromptContributionKind;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum MultiAgentV2CapabilityReceiptStatus {
    Included,
    Excluded,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MultiAgentV2CapabilityReceipt {
    id: &'static str,
    revision: u32,
    pub(crate) prompt_resource_id: PromptContributionKind,
    status: MultiAgentV2CapabilityReceiptStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<CockpitContractRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_omission: Option<MultiAgentV2ProjectionOmission>,
    tools_included: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools_omission: Option<MultiAgentV2ProjectionOmission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_namespace: Option<String>,
    pub(crate) expected_action_ids: Vec<String>,
    pub(crate) effective_action_ids: Vec<String>,
}

pub(crate) fn inspect_multi_agent_v2_capability(
    projection: Option<&MultiAgentV2CapabilityProjection>,
    request: &ResponsesApiRequest,
) -> CodexResult<MultiAgentV2CapabilityReceipt> {
    let manifest = crate::multi_agent_v2_capability::multi_agent_v2_capability_manifest();
    let Some(projection) = projection else {
        return Ok(MultiAgentV2CapabilityReceipt {
            id: MULTI_AGENT_V2_CAPABILITY_ID,
            revision: MULTI_AGENT_V2_CAPABILITY_REVISION,
            prompt_resource_id: manifest.prompt_resource_id,
            status: MultiAgentV2CapabilityReceiptStatus::Excluded,
            role: None,
            prompt_omission: Some(MultiAgentV2ProjectionOmission::CapabilityDisabled),
            tools_included: false,
            tools_omission: Some(MultiAgentV2ProjectionOmission::CapabilityDisabled),
            tool_namespace: None,
            expected_action_ids: Vec::new(),
            effective_action_ids: Vec::new(),
        });
    };

    let expected_action_ids = projection
        .actions
        .iter()
        .map(|action| effective_action_id(projection.tool_namespace.as_deref(), action.as_str()))
        .collect::<Vec<_>>();
    let all_effective_tools = request_tool_ids(request)?;
    let effective_action_ids = expected_action_ids
        .iter()
        .filter(|expected| all_effective_tools.iter().any(|actual| actual == *expected))
        .cloned()
        .collect::<Vec<_>>();
    let declared_effective_action_ids = manifest
        .actions
        .iter()
        .map(|action| effective_action_id(projection.tool_namespace.as_deref(), action.id))
        .filter(|action| all_effective_tools.iter().any(|actual| actual == action))
        .collect::<Vec<_>>();
    if effective_action_ids != expected_action_ids
        || declared_effective_action_ids != expected_action_ids
    {
        return Err(CodexErr::Fatal(format!(
            "multi-agent v2 capability conformance failed: expected actions {expected_action_ids:?}, found {declared_effective_action_ids:?}"
        )));
    }

    Ok(MultiAgentV2CapabilityReceipt {
        id: MULTI_AGENT_V2_CAPABILITY_ID,
        revision: MULTI_AGENT_V2_CAPABILITY_REVISION,
        prompt_resource_id: manifest.prompt_resource_id,
        status: if projection.enabled {
            MultiAgentV2CapabilityReceiptStatus::Included
        } else {
            MultiAgentV2CapabilityReceiptStatus::Excluded
        },
        role: projection.prompt_role,
        prompt_omission: projection.prompt_omission,
        tools_included: projection.tools_included,
        tools_omission: projection.tools_omission,
        tool_namespace: projection.tool_namespace.clone(),
        expected_action_ids,
        effective_action_ids,
    })
}

fn effective_action_id(namespace: Option<&str>, action: &str) -> String {
    namespace
        .map(|namespace| format!("{namespace}.{action}"))
        .unwrap_or_else(|| action.to_string())
}

fn request_tool_ids(request: &ResponsesApiRequest) -> CodexResult<Vec<String>> {
    let request = serde_json::to_value(request).map_err(|err| {
        CodexErr::Fatal(format!("failed to inspect effective request tools: {err}"))
    })?;
    let mut ids = Vec::new();
    if let Some(tools) = request.get("tools").and_then(Value::as_array) {
        collect_tool_ids(tools, None, &mut ids);
    }
    if let Some(input) = request.get("input").and_then(Value::as_array) {
        for item in input {
            if item.get("type").and_then(Value::as_str) == Some("additional_tools")
                && let Some(tools) = item.get("tools").and_then(Value::as_array)
            {
                collect_tool_ids(tools, None, &mut ids);
            }
        }
    }
    Ok(ids)
}

fn collect_tool_ids(tools: &[Value], namespace: Option<&str>, ids: &mut Vec<String>) {
    for tool in tools {
        let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
            continue;
        };
        match tool_type {
            "namespace" => {
                let Some(namespace) = tool.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(children) = tool.get("tools").and_then(Value::as_array) {
                    collect_tool_ids(children, Some(namespace), ids);
                }
            }
            "function" | "custom" => {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    ids.push(effective_action_id(namespace, name));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "prompt_capability_receipt_tests.rs"]
mod tests;
