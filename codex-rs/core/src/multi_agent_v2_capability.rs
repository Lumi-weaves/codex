use codex_protocol::protocol::MultiAgentVersion;
use serde::Serialize;

use crate::cockpit_operating_contract::CockpitContractRole;
use crate::prompt_census::PromptContributionKind;
use crate::session::turn_context::TurnContext;

pub const MULTI_AGENT_V2_CAPABILITY_ID: &str = "multi_agent_v2_cockpit_lifecycle";
pub const MULTI_AGENT_V2_CAPABILITY_REVISION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilityManifest {
    pub schema_version: u32,
    pub capabilities: Vec<MultiAgentV2CapabilityManifest>,
}

pub fn prompt_capability_manifest() -> PromptCapabilityManifest {
    PromptCapabilityManifest {
        schema_version: 1,
        capabilities: vec![multi_agent_v2_capability_manifest()],
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MultiAgentV2Action {
    SpawnAgent,
    SendMessage,
    FollowupTask,
    WaitAgent,
    InterruptAgent,
    CloseAgent,
    ListAgents,
}

impl MultiAgentV2Action {
    pub(crate) const ALL: [Self; 7] = [
        Self::SpawnAgent,
        Self::SendMessage,
        Self::FollowupTask,
        Self::WaitAgent,
        Self::InterruptAgent,
        Self::CloseAgent,
        Self::ListAgents,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SpawnAgent => "spawn_agent",
            Self::SendMessage => "send_message",
            Self::FollowupTask => "followup_task",
            Self::WaitAgent => "wait_agent",
            Self::InterruptAgent => "interrupt_agent",
            Self::CloseAgent => "close_agent",
            Self::ListAgents => "list_agents",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentV2CapabilityManifest {
    pub schema_version: u32,
    pub id: &'static str,
    pub revision: u32,
    pub prompt_resource_id: PromptContributionKind,
    pub eligible_session_sources: &'static [&'static str],
    pub excluded_session_sources: &'static [&'static str],
    pub tool_eligibility: &'static str,
    pub actions: Vec<MultiAgentV2ActionDescriptor>,
    pub public_views: &'static [&'static str],
    pub lifecycle: MultiAgentV2LifecycleDescriptor,
    pub source_navigation: MultiAgentV2SourceNavigation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentV2ActionDescriptor {
    pub id: &'static str,
    pub optional_gate: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentV2LifecycleDescriptor {
    pub parent_facing_states: &'static [&'static str],
    pub closure_action: &'static str,
    pub stopped_is_closed: bool,
    pub successful_close_receipt_is_closure: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentV2SourceNavigation {
    pub modules: &'static [&'static str],
    pub symbols: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub tests: &'static [&'static str],
}

pub fn multi_agent_v2_capability_manifest() -> MultiAgentV2CapabilityManifest {
    let cockpit_contract = crate::cockpit_operating_contract::cockpit_operating_contract_manifest();
    MultiAgentV2CapabilityManifest {
        schema_version: 1,
        id: MULTI_AGENT_V2_CAPABILITY_ID,
        revision: MULTI_AGENT_V2_CAPABILITY_REVISION,
        prompt_resource_id: PromptContributionKind::CockpitOperatingContract,
        eligible_session_sources: cockpit_contract.eligible_session_sources,
        excluded_session_sources: cockpit_contract.excluded_session_sources,
        tool_eligibility: "root session, or a canonical-path shadow whose model declares multi_agent_v2 support",
        actions: MultiAgentV2Action::ALL
            .iter()
            .map(|action| MultiAgentV2ActionDescriptor {
                id: action.as_str(),
                optional_gate: (*action == MultiAgentV2Action::WaitAgent)
                    .then_some("wait_agent_enabled"),
            })
            .collect(),
        public_views: &["agent_state", "residency", "native_status"],
        lifecycle: MultiAgentV2LifecycleDescriptor {
            parent_facing_states: &["running", "stopped"],
            closure_action: "close_agent",
            stopped_is_closed: false,
            successful_close_receipt_is_closure: true,
        },
        source_navigation: MultiAgentV2SourceNavigation {
            modules: &[
                "codex-rs/core/src/cockpit_operating_contract.rs",
                "codex-rs/core/src/session/multi_agents.rs",
                "codex-rs/core/src/tools/spec_plan.rs",
                "codex-rs/core/src/tools/handlers/multi_agents_spec.rs",
                "codex-rs/core/src/tools/handlers/multi_agents_v2",
            ],
            symbols: &[
                "CockpitOperatingContractManifest",
                "multi_agent_v2_projection",
                "add_collaboration_tools",
                "SpawnAgentHandlerV2",
                "CloseAgentHandlerV2",
                "ListAgentsHandlerV2",
            ],
            keywords: &[
                "MultiAgentVersion::V2",
                "CockpitContractRole",
                "wait_agent_enabled",
                "tool_namespace",
                "running",
                "stopped",
                "close_agent",
            ],
            tests: &[
                "core/src/tools/spec_plan_tests.rs",
                "core/src/tools/handlers/multi_agents_spec_tests.rs",
                "core/src/tools/handlers/multi_agents_tests.rs",
                "core/src/agent/control_tests.rs",
            ],
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MultiAgentV2ProjectionOmission {
    CapabilityDisabled,
    PromptRoleIneligible,
    ShadowModelUnsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MultiAgentV2CapabilityProjection {
    pub(crate) enabled: bool,
    pub(crate) prompt_role: Option<CockpitContractRole>,
    pub(crate) prompt_omission: Option<MultiAgentV2ProjectionOmission>,
    pub(crate) tools_included: bool,
    pub(crate) tools_omission: Option<MultiAgentV2ProjectionOmission>,
    pub(crate) tool_namespace: Option<String>,
    pub(crate) plaintext_messages: bool,
    pub(crate) actions: Vec<MultiAgentV2Action>,
}

pub(crate) fn multi_agent_v2_projection(
    turn_context: &TurnContext,
) -> MultiAgentV2CapabilityProjection {
    if turn_context.multi_agent_version != MultiAgentVersion::V2 {
        return MultiAgentV2CapabilityProjection {
            enabled: false,
            prompt_role: None,
            prompt_omission: Some(MultiAgentV2ProjectionOmission::CapabilityDisabled),
            tools_included: false,
            tools_omission: Some(MultiAgentV2ProjectionOmission::CapabilityDisabled),
            tool_namespace: None,
            plaintext_messages: false,
            actions: Vec::new(),
        };
    }

    let prompt_role =
        crate::cockpit_operating_contract::role_for_session_source(&turn_context.session_source);
    let tools_included = turn_context.session_source.get_agent_path().is_none()
        || turn_context.model_info.multi_agent_version == Some(MultiAgentVersion::V2);
    let tool_namespace = turn_context
        .provider
        .capabilities()
        .namespace_tools
        .then(|| turn_context.config.multi_agent_v2.tool_namespace.clone())
        .flatten();
    let plaintext_messages = tool_namespace.as_deref()
        == Some(crate::tools::handlers::multi_agents_spec::PLAINTEXT_MULTI_AGENT_V2_NAMESPACE);
    let actions = if tools_included {
        MultiAgentV2Action::ALL
            .iter()
            .copied()
            .filter(|action| {
                *action != MultiAgentV2Action::WaitAgent
                    || turn_context.config.multi_agent_v2.wait_agent_enabled
            })
            .collect()
    } else {
        Vec::new()
    };

    MultiAgentV2CapabilityProjection {
        enabled: true,
        prompt_role,
        prompt_omission: prompt_role
            .is_none()
            .then_some(MultiAgentV2ProjectionOmission::PromptRoleIneligible),
        tools_included,
        tools_omission: (!tools_included)
            .then_some(MultiAgentV2ProjectionOmission::ShadowModelUnsupported),
        tool_namespace,
        plaintext_messages,
        actions,
    }
}

#[cfg(test)]
#[path = "multi_agent_v2_capability_tests.rs"]
mod tests;
