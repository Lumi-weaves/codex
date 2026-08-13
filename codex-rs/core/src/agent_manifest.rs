use std::collections::HashMap;
use std::collections::HashSet;

use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use serde::Serialize;

use crate::PromptContributionKind;
use crate::PromptResourceManifest;
use crate::PromptResourceSourceNavigation;
use crate::prompt_capability_manifest;
use crate::prompt_resource_manifest;

pub const AGENT_CATALOG_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CODEX_AGENT_ID: &str = "codex";
pub const CODEX_AGENT_REVISION: u32 = 1;
pub const CODEX_SOL_PRESET_ID: &str = "codex-5.6-sol";
pub const CODEX_SOL_PRESET_REVISION: u32 = 1;
pub const CODEX_SOL_MODEL_TARGET: &str = "gpt-5.6-sol";

/// A stable reference to one immutable Agent Definition revision.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionRef {
    pub id: String,
    pub revision: u32,
}

/// A stable reference to one prompt capability revision.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityRef {
    pub id: String,
    pub revision: u32,
}

/// A stable reference to a future cross-tool or cross-Agent play registry.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlayRef {
    pub id: String,
    pub revision: u32,
}

/// The common execution surface a compatible Model Target must provide.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionAbi {
    pub id: String,
    pub revision: u32,
    pub requirements: Vec<String>,
}

/// One versioned, model-neutral Agent program declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub revision: u32,
    pub display_name: String,
    pub owner: String,
    pub provenance: String,
    pub prompt_resource_refs: Vec<PromptContributionKind>,
    pub capability_refs: Vec<AgentCapabilityRef>,
    pub play_refs: Vec<AgentPlayRef>,
    pub public_views: Vec<String>,
    pub execution_abi: AgentExecutionAbi,
    pub dependencies: Vec<AgentDefinitionRef>,
    pub conflicts: Vec<AgentDefinitionRef>,
    pub source_navigation: PromptResourceSourceNavigation,
}

/// Inference controls an App-facing launch recipe leaves adjustable by the user.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLaunchControl {
    ReasoningEffort,
    ServiceTier,
}

/// A one-dimensional frontend recipe over the native Agent and Model Target axes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLaunchPreset {
    pub id: String,
    pub revision: u32,
    pub display_name: String,
    pub owner: String,
    pub provenance: String,
    pub agent: AgentDefinitionRef,
    pub default_model_target: String,
    pub user_adjustable_controls: Vec<AgentLaunchControl>,
    pub source_navigation: PromptResourceSourceNavigation,
}

/// The static declaration plane. It does not claim that any entry was selected for a turn.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCatalogManifest {
    pub schema_version: u32,
    pub agent_definitions: Vec<AgentDefinition>,
    pub launch_presets: Vec<AgentLaunchPreset>,
}

/// Build and validate the deterministic version-one Agent catalog.
pub fn agent_catalog_manifest() -> CodexResult<AgentCatalogManifest> {
    let manifest = AgentCatalogManifest {
        schema_version: AGENT_CATALOG_MANIFEST_SCHEMA_VERSION,
        agent_definitions: vec![codex_agent_definition()],
        launch_presets: vec![codex_sol_launch_preset()],
    };
    validate_agent_catalog_manifest(&manifest)?;
    Ok(manifest)
}

/// Validate a caller-provided catalog against the current flat dependency registries.
pub fn validate_agent_catalog_manifest(manifest: &AgentCatalogManifest) -> CodexResult<()> {
    let resources = prompt_resource_manifest()?;
    let capabilities = prompt_capability_manifest();
    let model_targets = codex_models_manager::bundled_models_response()
        .map_err(|error| invalid_manifest(format!("bundled model catalog is invalid: {error}")))?
        .models
        .into_iter()
        .map(|model| model.slug)
        .collect::<HashSet<_>>();
    let capability_refs = capabilities
        .capabilities
        .into_iter()
        .map(|capability| {
            (
                (capability.id.to_string(), capability.revision),
                capability.prompt_resource_id,
            )
        })
        .collect::<HashMap<_, _>>();

    validate_manifest(
        manifest,
        &resources,
        &capability_refs,
        &HashSet::new(),
        &model_targets,
    )
}

fn validate_manifest(
    manifest: &AgentCatalogManifest,
    resources: &PromptResourceManifest,
    capability_refs: &HashMap<(String, u32), PromptContributionKind>,
    play_refs: &HashSet<(String, u32)>,
    model_targets: &HashSet<String>,
) -> CodexResult<()> {
    if manifest.schema_version != AGENT_CATALOG_MANIFEST_SCHEMA_VERSION {
        return Err(invalid_manifest(format!(
            "unsupported schema version {}",
            manifest.schema_version
        )));
    }
    if manifest.agent_definitions.is_empty() || manifest.launch_presets.is_empty() {
        return Err(invalid_manifest(
            "catalog must declare at least one Agent Definition and launch preset".to_string(),
        ));
    }

    let mut agent_refs = HashSet::new();
    for agent in &manifest.agent_definitions {
        let agent_ref = (agent.id.clone(), agent.revision);
        if agent.id.trim().is_empty()
            || agent.revision == 0
            || !agent_refs.insert(agent_ref.clone())
        {
            return Err(invalid_manifest(format!(
                "invalid or duplicate Agent Definition {}@{}",
                agent.id, agent.revision
            )));
        }
        require_text(&agent.display_name, "Agent display name")?;
        require_text(&agent.owner, "Agent owner")?;
        require_text(&agent.provenance, "Agent provenance")?;
        validate_navigation(&agent.source_navigation, &agent.id)?;
        validate_abi(&agent.execution_abi, &agent.id)?;
        if agent.prompt_resource_refs.is_empty() {
            return Err(invalid_manifest(format!(
                "Agent {}@{} has no prompt resources",
                agent.id, agent.revision
            )));
        }
        unique_values(&agent.prompt_resource_refs, &agent.id, "prompt resource")?;
        unique_values(&agent.capability_refs, &agent.id, "capability")?;
        unique_values(&agent.play_refs, &agent.id, "play")?;
        unique_values(&agent.public_views, &agent.id, "public view")?;
        agent
            .public_views
            .iter()
            .try_for_each(|view| require_text(view, "public view"))?;

        for resource_ref in &agent.prompt_resource_refs {
            if !resources
                .resources
                .iter()
                .any(|resource| resource.id == *resource_ref)
            {
                return Err(invalid_manifest(format!(
                    "Agent {}@{} has dangling prompt resource {}",
                    agent.id,
                    agent.revision,
                    resource_ref.as_str()
                )));
            }
        }
        for capability_ref in &agent.capability_refs {
            let Some(capability_resource) =
                capability_refs.get(&(capability_ref.id.clone(), capability_ref.revision))
            else {
                return Err(invalid_manifest(format!(
                    "Agent {}@{} has dangling capability {}@{}",
                    agent.id, agent.revision, capability_ref.id, capability_ref.revision
                )));
            };
            if !resources
                .resources
                .iter()
                .any(|resource| resource.id == *capability_resource)
            {
                return Err(invalid_manifest(format!(
                    "capability {}@{} has dangling prompt resource {}",
                    capability_ref.id,
                    capability_ref.revision,
                    capability_resource.as_str()
                )));
            }
        }
        for play_ref in &agent.play_refs {
            if !play_refs.contains(&(play_ref.id.clone(), play_ref.revision)) {
                return Err(invalid_manifest(format!(
                    "Agent {}@{} has dangling play {}@{}",
                    agent.id, agent.revision, play_ref.id, play_ref.revision
                )));
            }
        }
    }

    for agent in &manifest.agent_definitions {
        let own_ref = AgentDefinitionRef {
            id: agent.id.clone(),
            revision: agent.revision,
        };
        unique_values(&agent.dependencies, &agent.id, "dependency")?;
        unique_values(&agent.conflicts, &agent.id, "conflict")?;
        for reference in agent.dependencies.iter().chain(&agent.conflicts) {
            if !agent_refs.contains(&(reference.id.clone(), reference.revision)) {
                return Err(invalid_manifest(format!(
                    "Agent {}@{} references unknown Agent {}@{}",
                    agent.id, agent.revision, reference.id, reference.revision
                )));
            }
            if *reference == own_ref {
                return Err(invalid_manifest(format!(
                    "Agent {}@{} references itself",
                    agent.id, agent.revision
                )));
            }
        }
        if agent
            .dependencies
            .iter()
            .any(|dependency| agent.conflicts.contains(dependency))
        {
            return Err(invalid_manifest(format!(
                "Agent {}@{} has an edge in both dependencies and conflicts",
                agent.id, agent.revision
            )));
        }
    }

    let mut preset_refs = HashSet::new();
    for preset in &manifest.launch_presets {
        if preset.id.trim().is_empty()
            || preset.revision == 0
            || !preset_refs.insert((preset.id.clone(), preset.revision))
        {
            return Err(invalid_manifest(format!(
                "invalid or duplicate launch preset {}@{}",
                preset.id, preset.revision
            )));
        }
        require_text(&preset.display_name, "launch preset display name")?;
        require_text(&preset.owner, "launch preset owner")?;
        require_text(&preset.provenance, "launch preset provenance")?;
        validate_navigation(&preset.source_navigation, &preset.id)?;
        if preset.user_adjustable_controls.is_empty() {
            return Err(invalid_manifest(format!(
                "launch preset {}@{} has no user-adjustable controls",
                preset.id, preset.revision
            )));
        }
        unique_values(
            &preset.user_adjustable_controls,
            &preset.id,
            "user-adjustable control",
        )?;
        if !agent_refs.contains(&(preset.agent.id.clone(), preset.agent.revision)) {
            return Err(invalid_manifest(format!(
                "launch preset {}@{} has unresolved Agent {}@{}",
                preset.id, preset.revision, preset.agent.id, preset.agent.revision
            )));
        }
        if !model_targets.contains(&preset.default_model_target) {
            return Err(invalid_manifest(format!(
                "launch preset {}@{} has unresolved Model Target {}",
                preset.id, preset.revision, preset.default_model_target
            )));
        }
    }

    Ok(())
}

fn codex_agent_definition() -> AgentDefinition {
    AgentDefinition {
        id: CODEX_AGENT_ID.to_string(),
        revision: CODEX_AGENT_REVISION,
        display_name: "Codex".to_string(),
        owner: "RichCodex Agent catalog".to_string(),
        provenance: "explicit declaration over the legacy default Codex behavior template; declaration-only in schema version 1".to_string(),
        prompt_resource_refs: vec![PromptContributionKind::CodexAgentBaseInstructions],
        capability_refs: vec![AgentCapabilityRef {
            id: crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_ID.to_string(),
            revision: crate::multi_agent_v2_capability::MULTI_AGENT_V2_CAPABILITY_REVISION,
        }],
        play_refs: Vec::new(),
        public_views: vec![
            "agent_state".to_string(),
            "residency".to_string(),
            "native_status".to_string(),
        ],
        execution_abi: AgentExecutionAbi {
            id: "richcodex_text_agent".to_string(),
            revision: 1,
            requirements: vec![
                "text_input".to_string(),
                "tool_calling".to_string(),
                "responses_or_responses_lite".to_string(),
            ],
        },
        dependencies: Vec::new(),
        conflicts: Vec::new(),
        source_navigation: PromptResourceSourceNavigation {
            modules: vec![
                "codex-rs/core/src/agent_manifest.rs".to_string(),
                "codex-rs/core/src/prompt_resource_definitions.rs".to_string(),
                "codex-rs/models-manager/models.json".to_string(),
            ],
            symbols: vec![
                "codex_agent_definition".to_string(),
                "CodexAgentBaseInstructions".to_string(),
                "gpt-5.6-sol".to_string(),
            ],
            keywords: vec![
                "Agent Definition".to_string(),
                "legacy model catalog template".to_string(),
                "model-neutral behavior".to_string(),
            ],
            tests: vec!["codex-rs/core/src/agent_manifest_tests.rs".to_string()],
        },
    }
}

fn codex_sol_launch_preset() -> AgentLaunchPreset {
    AgentLaunchPreset {
        id: CODEX_SOL_PRESET_ID.to_string(),
        revision: CODEX_SOL_PRESET_REVISION,
        display_name: "Codex 5.6 Sol".to_string(),
        owner: "RichCodex Agent catalog".to_string(),
        provenance: "Codex App compatibility recipe; not an upstream model identity".to_string(),
        agent: AgentDefinitionRef {
            id: CODEX_AGENT_ID.to_string(),
            revision: CODEX_AGENT_REVISION,
        },
        default_model_target: CODEX_SOL_MODEL_TARGET.to_string(),
        user_adjustable_controls: vec![
            AgentLaunchControl::ReasoningEffort,
            AgentLaunchControl::ServiceTier,
        ],
        source_navigation: PromptResourceSourceNavigation {
            modules: vec![
                "codex-rs/core/src/agent_manifest.rs".to_string(),
                "codex-rs/models-manager/models.json".to_string(),
                "codex-rs/app-server/src/model_list_catalog.rs".to_string(),
            ],
            symbols: vec![
                "codex_sol_launch_preset".to_string(),
                "gpt-5.6-sol".to_string(),
                "ModelListCatalog".to_string(),
            ],
            keywords: vec![
                "Agent Launch Preset".to_string(),
                "frontend compatibility facade".to_string(),
                "resolver before model lookup".to_string(),
            ],
            tests: vec!["codex-rs/core/src/agent_manifest_tests.rs".to_string()],
        },
    }
}

fn validate_abi(abi: &AgentExecutionAbi, owner: &str) -> CodexResult<()> {
    require_text(&abi.id, "execution ABI id")?;
    if abi.revision == 0 || abi.requirements.is_empty() {
        return Err(invalid_manifest(format!(
            "Agent {owner} has incomplete execution ABI"
        )));
    }
    unique_values(&abi.requirements, owner, "execution ABI requirement")?;
    abi.requirements
        .iter()
        .try_for_each(|requirement| require_text(requirement, "execution ABI requirement"))
}

fn validate_navigation(
    navigation: &PromptResourceSourceNavigation,
    owner: &str,
) -> CodexResult<()> {
    if navigation.modules.is_empty()
        || navigation.symbols.is_empty()
        || navigation.keywords.is_empty()
        || navigation.tests.is_empty()
    {
        return Err(invalid_manifest(format!(
            "{owner} is missing source navigation"
        )));
    }
    if navigation
        .modules
        .iter()
        .chain(&navigation.tests)
        .any(|path| {
            path.trim().is_empty()
                || path.starts_with('/')
                || path.starts_with("../")
                || path.split('/').any(|segment| segment == "..")
        })
        || navigation
            .symbols
            .iter()
            .chain(&navigation.keywords)
            .any(|value| value.trim().is_empty())
    {
        return Err(invalid_manifest(format!(
            "{owner} has unsafe or incomplete source navigation"
        )));
    }
    Ok(())
}

fn require_text(value: &str, label: &str) -> CodexResult<()> {
    if value.trim().is_empty() {
        Err(invalid_manifest(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn unique_values<T: Eq + std::hash::Hash>(
    values: &[T],
    owner: &str,
    kind: &str,
) -> CodexResult<()> {
    let mut seen = HashSet::new();
    if values.iter().any(|value| !seen.insert(value)) {
        Err(invalid_manifest(format!(
            "{owner} has a duplicate {kind} reference"
        )))
    } else {
        Ok(())
    }
}

fn invalid_manifest(message: String) -> CodexErr {
    CodexErr::InvalidRequest(format!("invalid Agent catalog manifest: {message}"))
}

#[cfg(test)]
#[path = "agent_manifest_tests.rs"]
mod tests;
