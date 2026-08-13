use codex_api::ResponsesApiRequest;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::BaseInstructionsProvenance;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use serde::Serialize;

use crate::agent_program::ResolvedAgentProgram;
use crate::prompt_census::PromptContributionKind;
use crate::prompt_hash::canonical_json_bytes;
use crate::prompt_hash::sha256_hex;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentProgramReceipt {
    hash_algorithm: &'static str,
    sha256: String,
    resources: Vec<AgentProgramResourceReceipt>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentProgramResourceReceipt {
    id: PromptContributionKind,
    status: &'static str,
    logical_placement: &'static str,
    lowered_placement: &'static str,
    content_sha256: String,
    content_bytes: usize,
    expected_copy_count: usize,
    effective_copy_count: usize,
}

impl AgentProgramReceipt {
    pub(crate) fn inspect(
        program: &ResolvedAgentProgram,
        request: &ResponsesApiRequest,
    ) -> CodexResult<Self> {
        if !matches!(
            program.base_instructions.provenance.as_ref(),
            Some(BaseInstructionsProvenance::Agent { agent })
                if agent.id == program.definition.id && agent.revision == program.definition.revision
        ) {
            return Err(CodexErr::Fatal(
                "resolved Agent base instructions lost their Agent provenance".to_string(),
            ));
        }

        let text = &program.base_instructions.text;
        let instructions_copies = usize::from(request.instructions == *text);
        let developer_copies = request
            .input
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    ResponseItem::Message { role, content, .. }
                        if role == "developer"
                            && matches!(content.as_slice(), [ContentItem::InputText { text: actual }] if actual == text)
                )
            })
            .count();
        let effective_copy_count = instructions_copies + developer_copies;
        let lowered_placement = match (instructions_copies, developer_copies) {
            (1, 0) => "responses_instructions",
            (0, 1) => "responses_lite_developer_input",
            _ => {
                return Err(CodexErr::Fatal(format!(
                    "Agent base-instructions conformance failed: expected one effective copy, found {effective_copy_count}"
                )));
            }
        };
        let resource_id = PromptContributionKind::CodexAgentBaseInstructions;
        let content_sha256 = sha256_hex(text.as_bytes());
        let program_bytes = canonical_json_bytes(&serde_json::json!({
            "schemaVersion": 1,
            "agent": { "id": program.definition.id, "revision": program.definition.revision },
            "promptResources": [{
                "id": resource_id,
                "logicalPlacement": "base_instructions",
                "contentSha256": content_sha256,
            }],
            "capabilityRefs": program.definition.capability_refs,
            "playRefs": program.definition.play_refs,
            "publicViews": program.definition.public_views,
            "executionAbi": program.definition.execution_abi,
            "dependencies": program.definition.dependencies,
            "conflicts": program.definition.conflicts,
        }))?;

        Ok(Self {
            hash_algorithm: "sha256_canonical_json",
            sha256: sha256_hex(&program_bytes),
            resources: vec![AgentProgramResourceReceipt {
                id: resource_id,
                status: "included",
                logical_placement: "base_instructions",
                lowered_placement,
                content_sha256,
                content_bytes: text.len(),
                expected_copy_count: 1,
                effective_copy_count,
            }],
        })
    }
}
