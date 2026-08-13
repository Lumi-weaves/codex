use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use serde::Serialize;

use crate::cockpit_operating_contract::CockpitOperatingContractDocument;
use crate::cockpit_operating_contract::cockpit_operating_contract_manifest;
use crate::prompt_census::CensusCompleteness;
use crate::prompt_census::PromptContributionDefinition;
use crate::prompt_census::PromptContributionKind;
use crate::prompt_census::PromptInvocationKind;

#[path = "prompt_resource_definitions.rs"]
mod prompt_resource_definitions;
use prompt_resource_definitions::resource_definition;

/// Schema version for the static prompt-resource registry.
pub const PROMPT_RESOURCE_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// The broad source family that owns a prompt resource.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptResourceKind {
    BaseInstructions,
    WorldState,
    OperatingContract,
    ConversationHistory,
    InvocationInput,
    ToolSpecifications,
    OutputSchema,
    Realtime,
    Memory,
    ProviderLowering,
    ProviderProcessing,
}

/// Whether a resource is static, runtime-derived, an aggregate, or provider-owned.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptResourceClassification {
    Static,
    Runtime,
    Aggregate,
    ProviderOwned,
}

/// Stable typed identity used by dependency and conflict edges.
pub type PromptResourceId = PromptContributionKind;

/// Repository navigation anchors for one prompt resource.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResourceSourceNavigation {
    pub modules: Vec<String>,
    pub symbols: Vec<String>,
    pub keywords: Vec<String>,
    pub tests: Vec<String>,
}

/// One flat prompt resource descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResource {
    pub id: PromptResourceId,
    pub kind: PromptResourceKind,
    pub classification: PromptResourceClassification,
    pub owner: String,
    pub placement: String,
    pub provenance: String,
    pub availability: String,
    pub hard_bound: String,
    pub governance: String,
    pub inheritance: String,
    pub sensitivity: String,
    pub completeness: CensusCompleteness,
    pub applicable_invocations: Vec<PromptInvocationKind>,
    pub dependencies: Vec<PromptResourceId>,
    pub conflicts: Vec<PromptResourceId>,
    pub source_navigation: PromptResourceSourceNavigation,
    pub rendered_documents: Vec<CockpitOperatingContractDocument>,
}

/// A validated collection of all statically registered prompt resources.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResourceManifest {
    pub schema_version: u32,
    pub resources: Vec<PromptResource>,
}

impl PromptResourceManifest {
    /// Validate graph references and source navigation without inspecting the filesystem.
    pub fn validate(&self) -> CodexResult<()> {
        if self.schema_version != PROMPT_RESOURCE_MANIFEST_SCHEMA_VERSION {
            return Err(invalid_manifest(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }

        for (index, resource) in self.resources.iter().enumerate() {
            if self.resources[..index]
                .iter()
                .any(|prior| prior.id == resource.id)
            {
                return Err(invalid_manifest(format!(
                    "duplicate resource id {}",
                    resource.id.as_str()
                )));
            }
        }

        for resource in &self.resources {
            for dependency in &resource.dependencies {
                if !self
                    .resources
                    .iter()
                    .any(|candidate| candidate.id == *dependency)
                {
                    return Err(invalid_manifest(format!(
                        "resource {} has dangling dependency {}",
                        resource.id.as_str(),
                        dependency.as_str()
                    )));
                }
                if *dependency == resource.id {
                    return Err(invalid_manifest(format!(
                        "resource {} depends on itself",
                        resource.id.as_str()
                    )));
                }
            }

            for conflict in &resource.conflicts {
                if !self
                    .resources
                    .iter()
                    .any(|candidate| candidate.id == *conflict)
                {
                    return Err(invalid_manifest(format!(
                        "resource {} has dangling conflict {}",
                        resource.id.as_str(),
                        conflict.as_str()
                    )));
                }
                if *conflict == resource.id {
                    return Err(invalid_manifest(format!(
                        "resource {} conflicts with itself",
                        resource.id.as_str()
                    )));
                }
            }

            if resource
                .dependencies
                .iter()
                .any(|dependency| resource.conflicts.contains(dependency))
            {
                return Err(invalid_manifest(format!(
                    "resource {} has an edge in both dependencies and conflicts",
                    resource.id.as_str()
                )));
            }

            let navigation = &resource.source_navigation;
            if navigation.modules.is_empty()
                || navigation.symbols.is_empty()
                || navigation.keywords.is_empty()
                || navigation.tests.is_empty()
            {
                return Err(invalid_manifest(format!(
                    "resource {} is missing source navigation",
                    resource.id.as_str()
                )));
            }
            if navigation.modules.iter().any(|module| {
                module.is_empty() || module.starts_with('/') || module.starts_with("../")
            }) || navigation.symbols.iter().any(String::is_empty)
                || navigation.keywords.iter().any(String::is_empty)
                || navigation
                    .tests
                    .iter()
                    .any(|test| test.is_empty() || test.starts_with('/') || test.starts_with("../"))
            {
                return Err(invalid_manifest(format!(
                    "resource {} has incomplete source navigation",
                    resource.id.as_str()
                )));
            }
        }

        Ok(())
    }
}

/// Validate a caller-provided prompt-resource manifest.
pub fn validate_prompt_resource_manifest(manifest: &PromptResourceManifest) -> CodexResult<()> {
    manifest.validate()
}

/// Build the deterministic version-one prompt-resource manifest.
pub fn prompt_resource_manifest() -> CodexResult<PromptResourceManifest> {
    let cockpit_documents = cockpit_operating_contract_manifest().contracts;
    let manifest = PromptResourceManifest {
        schema_version: PROMPT_RESOURCE_MANIFEST_SCHEMA_VERSION,
        resources: PromptContributionKind::ALL
            .into_iter()
            .map(|id| resource_entry(id, &cockpit_documents))
            .collect(),
    };
    manifest.validate()?;
    Ok(manifest)
}

fn resource_entry(
    id: PromptContributionKind,
    cockpit_documents: &[CockpitOperatingContractDocument],
) -> PromptResource {
    let definition = resource_definition(id);
    PromptResource {
        id: definition.id,
        kind: definition.kind,
        classification: definition.classification,
        owner: definition.owner.to_string(),
        placement: definition.placement.to_string(),
        provenance: definition.provenance.to_string(),
        availability: definition.availability.to_string(),
        hard_bound: definition.hard_bound.to_string(),
        governance: definition.governance.to_string(),
        inheritance: definition.inheritance.to_string(),
        sensitivity: definition.sensitivity.to_string(),
        completeness: definition.completeness,
        applicable_invocations: PromptInvocationKind::ALL
            .into_iter()
            .filter(|invocation| invocation.contributions().contains(&id))
            .collect(),
        dependencies: definition.dependencies.to_vec(),
        conflicts: definition.conflicts.to_vec(),
        source_navigation: definition.source_navigation.to_owned(),
        rendered_documents: if id == PromptContributionKind::CockpitOperatingContract {
            cockpit_documents.to_vec()
        } else {
            Vec::new()
        },
    }
}

pub(crate) fn prompt_contribution_definition(
    id: PromptContributionKind,
) -> PromptContributionDefinition {
    let definition = resource_definition(id);
    PromptContributionDefinition {
        id: definition.id,
        owner: definition.owner,
        placement: definition.placement,
        provenance: definition.provenance,
        availability: definition.availability,
        hard_bound: definition.hard_bound,
        governance: definition.governance,
        inheritance: definition.inheritance,
        sensitivity: definition.sensitivity,
        completeness: definition.completeness,
    }
}

fn invalid_manifest(message: String) -> CodexErr {
    CodexErr::InvalidRequest(format!("invalid prompt resource manifest: {message}"))
}

#[cfg(test)]
#[path = "prompt_resources_tests.rs"]
mod tests;
