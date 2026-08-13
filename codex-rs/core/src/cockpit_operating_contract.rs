use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

pub const COCKPIT_OPERATING_CONTRACT_ID: &str = "lumi_cockpit_operating_contract";
pub const COCKPIT_OPERATING_CONTRACT_REVISION: u32 = 1;
pub const COCKPIT_OPERATING_CONTRACT_OPEN_TAG: &str = "<lumi_cockpit_operating_contract>";
pub const COCKPIT_OPERATING_CONTRACT_CLOSE_TAG: &str = "</lumi_cockpit_operating_contract>";

const OWNER: &str = "Lumi Prompt / Context Plane";
const GOVERNANCE: &str =
    "versioned built-in contract; changes require prompt-plane conformance review";
const MAX_UTF8_BYTES: usize = 4_096;

fn assert_contract_body_is_bounded(body: &str) {
    let rendered_len = COCKPIT_OPERATING_CONTRACT_OPEN_TAG.len()
        + body.len()
        + COCKPIT_OPERATING_CONTRACT_CLOSE_TAG.len();
    assert!(
        rendered_len <= MAX_UTF8_BYTES,
        "cockpit operating contract exceeds its {MAX_UTF8_BYTES}-byte hard bound"
    );
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CockpitContractRole {
    Root,
    Shadow,
}

impl CockpitContractRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Shadow => "shadow",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CockpitIngressKind {
    PrivilegedFletcherMessage,
    CompactEventWake,
    ConsolidatedResourceAuditWake,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CockpitIngressContract {
    pub kind: CockpitIngressKind,
    pub payload: &'static str,
    pub admission: &'static str,
    pub required_interpretation: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CockpitOperatingContractDescriptor {
    pub id: &'static str,
    pub revision: u32,
    pub role: CockpitContractRole,
    pub owner: &'static str,
    pub governance: &'static str,
    pub max_utf8_bytes: usize,
    pub sha256: String,
    pub ingress: Vec<CockpitIngressContract>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CockpitOperatingContractManifest {
    pub schema_version: u32,
    pub id: &'static str,
    pub revision: u32,
    pub owner: &'static str,
    pub governance: &'static str,
    pub max_utf8_bytes: usize,
    pub eligible_session_sources: &'static [&'static str],
    pub excluded_session_sources: &'static [&'static str],
    pub contracts: Vec<CockpitOperatingContractDocument>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CockpitOperatingContractDocument {
    pub descriptor: CockpitOperatingContractDescriptor,
    pub rendered_contract: String,
}

pub(crate) fn role_for_session_source(source: &SessionSource) -> Option<CockpitContractRole> {
    match source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }) => {
            Some(CockpitContractRole::Shadow)
        }
        SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => Some(CockpitContractRole::Root),
        SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
    }
}

pub(crate) fn ingress_contracts() -> Vec<CockpitIngressContract> {
    vec![
        CockpitIngressContract {
            kind: CockpitIngressKind::PrivilegedFletcherMessage,
            payload: "the user message, not a red dot and not an implicit resource dashboard",
            admission: "admit first at the next model-generation safe boundary",
            required_interpretation: "receive Fletcher immediately; then continue holding active work unless he changes it",
        },
        CockpitIngressContract {
            kind: CockpitIngressKind::CompactEventWake,
            payload: "only compact typed red dots carrying stable handle identity",
            admission: "serialize at the next model-generation safe boundary",
            required_interpretation: "open one or more typed views deliberately; delivery alone is not acceptance or resolution, and the wake does not reset the independent resource-audit cadence",
        },
        CockpitIngressContract {
            kind: CockpitIngressKind::ConsolidatedResourceAuditWake,
            payload: "one consolidated view of currently held active resources",
            admission: "periodic while resources are held; cadence is adjustable",
            required_interpretation: "detect leaks or forgotten handles; never treat the audit as a routine progress feed",
        },
    ]
}

pub(crate) fn contract_body(role: CockpitContractRole) -> String {
    let lifecycle = match role {
        CockpitContractRole::Root => {
            "You own every Shadow Lumi lifecycle: follow up, park, refresh, refork when its knowledge fork is stale, retire, and close with a receipt. Shadows own only their internal resources unless you explicitly grant supervisory ownership."
        }
        CockpitContractRole::Shadow => {
            "Root Lumi owns your lifecycle. Own resources internal to your assignment and return concise receipts; do not spawn or manage peers or children unless root explicitly grants supervisory ownership."
        }
    };

    let body = format!(
        r#"
id: {COCKPIT_OPERATING_CONTRACT_ID}
revision: {COCKPIT_OPERATING_CONTRACT_REVISION}
role: {}
owner: {OWNER}
governance: {GOVERNANCE}

Operate the cockpit as a typed event-and-handle system, not as a dashboard transcript.
- Fletcher messages are privileged ingress: admit them first at the next safe model-generation boundary. They carry no implicit status panel.
- Event wakes carry only compact typed red dots with stable handle identity, never source payloads or the resource-audit panel. Open any needed typed views deliberately; progressive disclosure is the default.
- Resource-audit wakes carry one consolidated view of active resources while resources are held. Their adjustable cadence is a leak/forgetfulness safety rope, never a progress feed. Events may wake earlier without the audit panel and do not reset its independent cadence.
- Serialize all ingress at model-generation safe boundaries. Handle state is explicit: delivered != opened != accepted != resolved.
- Every handle exposes its available typed views and actions. Preserve its identity through follow-up, parking, refresh, retirement, and closure receipts.
- {lifecycle}
- With no active resources, work may end unless an explicit wake-capable message subscription remains. Other subscriptions are active handles and therefore keep the owning task live while armed.
"#,
        role.as_str()
    );
    // This is the production render path used by `ContextualUserFragment::body`, so additions
    // cannot bypass the public descriptor's hard prompt bound.
    assert_contract_body_is_bounded(&body);
    body
}

pub(crate) fn rendered_contract(role: CockpitContractRole) -> String {
    format!(
        "{COCKPIT_OPERATING_CONTRACT_OPEN_TAG}{}{COCKPIT_OPERATING_CONTRACT_CLOSE_TAG}",
        contract_body(role)
    )
}

pub(crate) fn descriptor(role: CockpitContractRole) -> CockpitOperatingContractDescriptor {
    let rendered = rendered_contract(role);
    CockpitOperatingContractDescriptor {
        id: COCKPIT_OPERATING_CONTRACT_ID,
        revision: COCKPIT_OPERATING_CONTRACT_REVISION,
        role,
        owner: OWNER,
        governance: GOVERNANCE,
        max_utf8_bytes: MAX_UTF8_BYTES,
        sha256: format!("{:x}", Sha256::digest(rendered.as_bytes())),
        ingress: ingress_contracts(),
    }
}

pub fn cockpit_operating_contract_manifest() -> CockpitOperatingContractManifest {
    CockpitOperatingContractManifest {
        schema_version: 1,
        id: COCKPIT_OPERATING_CONTRACT_ID,
        revision: COCKPIT_OPERATING_CONTRACT_REVISION,
        owner: OWNER,
        governance: GOVERNANCE,
        max_utf8_bytes: MAX_UTF8_BYTES,
        eligible_session_sources: &[
            "cli",
            "vscode",
            "exec",
            "mcp",
            "custom",
            "unknown",
            "sub_agent.thread_spawn",
        ],
        excluded_session_sources: &[
            "internal.*",
            "sub_agent.review",
            "sub_agent.compact",
            "sub_agent.memory_consolidation",
            "sub_agent.other",
        ],
        contracts: [CockpitContractRole::Root, CockpitContractRole::Shadow]
            .into_iter()
            .map(|role| CockpitOperatingContractDocument {
                descriptor: descriptor(role),
                rendered_contract: rendered_contract(role),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_distinguishes_event_and_audit_wakes_without_harness_knowledge() {
        let manifest = cockpit_operating_contract_manifest();
        let event = manifest.contracts[0]
            .descriptor
            .ingress
            .iter()
            .find(|entry| entry.kind == CockpitIngressKind::CompactEventWake)
            .expect("event ingress");
        let audit = manifest.contracts[0]
            .descriptor
            .ingress
            .iter()
            .find(|entry| entry.kind == CockpitIngressKind::ConsolidatedResourceAuditWake)
            .expect("audit ingress");

        assert!(event.payload.contains("red dots"));
        assert!(event.required_interpretation.contains("typed views"));
        assert!(event.required_interpretation.contains("does not reset"));
        assert!(audit.payload.contains("consolidated view"));
        assert!(audit.required_interpretation.contains("never treat"));
        assert_ne!(event.required_interpretation, audit.required_interpretation);
    }

    #[test]
    fn contracts_are_bounded_versioned_and_role_specific() {
        for role in [CockpitContractRole::Root, CockpitContractRole::Shadow] {
            let rendered = rendered_contract(role);
            let descriptor = descriptor(role);
            assert!(rendered.len() <= descriptor.max_utf8_bytes);
            assert!(rendered.contains("delivered != opened != accepted != resolved"));
            assert!(rendered.contains(&format!("role: {}", role.as_str())));
        }
        assert_ne!(
            rendered_contract(CockpitContractRole::Root),
            rendered_contract(CockpitContractRole::Shadow)
        );
    }
}
