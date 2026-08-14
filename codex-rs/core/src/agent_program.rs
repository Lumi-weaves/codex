use std::sync::LazyLock;

use codex_protocol::agent::AgentDefinitionRef;
use codex_protocol::agent::AgentSelection;
use codex_protocol::config_types::Personality;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::BaseInstructionsProvenance;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;

use crate::PromptInvocationKind;
use crate::agent_manifest::AgentDefinition;
use crate::agent_manifest::CODEX_SOL_MODEL_TARGET;
use crate::agent_manifest::agent_catalog_manifest;
use crate::prompt_hash::sha256_hex;

const CODEX_AGENT_BASE_TEMPLATE_SHA256: &str =
    "9cda3267b624589d2d9a4253945ee63ee83bf22be606250fe7e2af46a5b1b7d4";

/// One resolved, immutable Agent program revision ready for prompt compilation.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedAgentProgram {
    pub(crate) definition: AgentDefinition,
    pub(crate) base_instructions: BaseInstructions,
}

/// Resolve a selected Agent independently of the session's Model Target.
pub(crate) fn resolve_agent_program(
    selection: &AgentSelection,
    personality: Option<Personality>,
) -> CodexResult<ResolvedAgentProgram> {
    let manifest = agent_catalog_manifest()?;
    let definition = manifest
        .agent_definitions
        .into_iter()
        .find(|definition| {
            definition.id == selection.agent.id && definition.revision == selection.agent.revision
        })
        .ok_or_else(|| {
            CodexErr::Fatal(format!(
                "selected Agent {}@{} is unavailable in this RichCodex build",
                selection.agent.id, selection.agent.revision
            ))
        })?;
    if definition.prompt_resource_refs
        != [crate::PromptContributionKind::CodexAgentBaseInstructions]
    {
        return Err(CodexErr::Fatal(format!(
            "selected Agent {}@{} has an unsupported base resource graph",
            selection.agent.id, selection.agent.revision
        )));
    }

    let text = resolve_agent_base_instructions(selection, personality)?;
    Ok(ResolvedAgentProgram {
        definition,
        base_instructions: BaseInstructions {
            text,
            provenance: Some(BaseInstructionsProvenance::Agent {
                agent: selection.agent.clone(),
            }),
        },
    })
}

pub(crate) fn resolve_agent_base_instructions(
    selection: &AgentSelection,
    personality: Option<Personality>,
) -> CodexResult<String> {
    ensure_codex_agent(selection)?;
    let instructions = codex_prompt_source()?.get_model_instructions(personality);
    Ok(if personality == Some(Personality::None) {
        codex_models_manager::model_info::strip_personality_section(instructions)
    } else {
        instructions
    })
}

pub(crate) fn resolve_agent_personality_message(
    selection: &AgentSelection,
    personality: Personality,
) -> CodexResult<Option<String>> {
    ensure_codex_agent(selection)?;
    Ok(codex_prompt_source()?
        .model_messages
        .as_ref()
        .and_then(|messages| messages.get_personality_message(Some(personality)))
        .filter(|message| !message.is_empty()))
}

fn ensure_codex_agent(selection: &AgentSelection) -> CodexResult<()> {
    is_codex_agent(&selection.agent)
        .then_some(())
        .ok_or_else(|| {
            CodexErr::Fatal(format!(
                "selected Agent {}@{} has no codex@1 prompt resource resolver",
                selection.agent.id, selection.agent.revision
            ))
        })
}

fn codex_prompt_source() -> CodexResult<&'static ModelInfo> {
    static SOURCE: LazyLock<Result<ModelInfo, String>> = LazyLock::new(|| {
        let source = codex_models_manager::bundled_models_response()
            .map_err(|error| format!("failed to load the pinned codex@1 prompt resource: {error}"))?
            .models
            .into_iter()
            .find(|model| model.slug == CODEX_SOL_MODEL_TARGET)
            .ok_or_else(|| {
                format!("codex@1 prompt source `{CODEX_SOL_MODEL_TARGET}` is unavailable")
            })?;
        let template = source
            .model_messages
            .as_ref()
            .and_then(|messages| messages.instructions_template.as_deref())
            .ok_or_else(|| "codex@1 prompt source has no instructions template".to_string())?;
        let actual_sha256 = sha256_hex(template.as_bytes());
        if actual_sha256 != CODEX_AGENT_BASE_TEMPLATE_SHA256 {
            return Err(format!(
                "codex@1 prompt resource changed without an Agent revision bump: expected {CODEX_AGENT_BASE_TEMPLATE_SHA256}, found {actual_sha256}"
            ));
        }
        Ok(source)
    });
    SOURCE
        .as_ref()
        .map_err(|error| CodexErr::Fatal(error.clone()))
}

pub(crate) fn is_codex_agent(agent: &AgentDefinitionRef) -> bool {
    agent.id == crate::agent_manifest::CODEX_AGENT_ID
        && agent.revision == crate::agent_manifest::CODEX_AGENT_REVISION
}

pub(crate) fn applies_to_session_source(session_source: &SessionSource) -> bool {
    matches!(
        PromptInvocationKind::for_session_turn(session_source),
        PromptInvocationKind::Turn | PromptInvocationKind::LocalCompaction
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::InternalSessionSource;
    use codex_protocol::protocol::SubAgentSource;

    #[test]
    fn explicit_agent_program_applies_only_to_ordinary_and_compaction_sessions() {
        let ordinary_shadow = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: ThreadId::new(),
            depth: 1,
            agent_path: None,
            agent_nickname: None,
            agent_role: Some("worker".to_string()),
        });
        let included = [
            SessionSource::Cli,
            ordinary_shadow,
            SessionSource::SubAgent(SubAgentSource::Compact),
        ];
        let excluded = [
            SessionSource::SubAgent(SubAgentSource::Review),
            SessionSource::SubAgent(SubAgentSource::Other("guardian".to_string())),
            SessionSource::Internal(InternalSessionSource::MemoryConsolidation),
            SessionSource::SubAgent(SubAgentSource::MemoryConsolidation),
        ];

        for source in included {
            assert!(
                applies_to_session_source(&source),
                "expected Agent program for {source:?}"
            );
        }
        for source in excluded {
            assert!(
                !applies_to_session_source(&source),
                "specialized session must retain its own prompt owner: {source:?}"
            );
        }
    }
}
