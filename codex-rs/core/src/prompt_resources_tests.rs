use std::collections::HashSet;

use pretty_assertions::assert_eq;

use crate::PromptResourceManifest;
use crate::cockpit_operating_contract::cockpit_operating_contract_manifest;
use crate::prompt_census::PromptContributionKind;
use crate::prompt_census::PromptInvocationKind;
use crate::prompt_census::prompt_context_census;
use crate::prompt_resource_manifest;

#[test]
fn manifest_has_full_unique_coverage_and_deterministic_order() {
    let first = prompt_resource_manifest().expect("static resource manifest should validate");
    let second = prompt_resource_manifest().expect("static resource manifest should validate");

    assert_eq!(first, second);
    assert_eq!(
        first
            .resources
            .iter()
            .map(|resource| resource.id)
            .collect::<Vec<_>>(),
        PromptContributionKind::ALL.to_vec()
    );
    let ids = first
        .resources
        .iter()
        .map(|resource| resource.id)
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), PromptContributionKind::ALL.len());
    assert_eq!(first.resources.len(), PromptContributionKind::ALL.len());

    for resource in &first.resources {
        let expected = PromptInvocationKind::ALL
            .into_iter()
            .filter(|invocation| invocation.contributions().contains(&resource.id))
            .collect::<Vec<_>>();
        assert_eq!(resource.applicable_invocations, expected);
    }
}

#[test]
fn census_semantics_are_projected_from_resource_descriptors() {
    let manifest = prompt_resource_manifest().expect("static resource manifest should validate");
    let census = prompt_context_census();

    for resource in &manifest.resources {
        let census_definition = census
            .contributions
            .iter()
            .find(|definition| definition.id == resource.id)
            .expect("every resource has a census definition");
        assert_eq!(
            (
                resource.id,
                resource.owner.as_str(),
                resource.placement.as_str(),
                resource.provenance.as_str(),
                resource.availability.as_str(),
                resource.hard_bound.as_str(),
                resource.governance.as_str(),
                resource.inheritance.as_str(),
                resource.sensitivity.as_str(),
                resource.completeness,
            ),
            (
                census_definition.id,
                census_definition.owner,
                census_definition.placement,
                census_definition.provenance,
                census_definition.availability,
                census_definition.hard_bound,
                census_definition.governance,
                census_definition.inheritance,
                census_definition.sensitivity,
                census_definition.completeness,
            )
        );
    }
}

#[test]
fn cockpit_resource_reuses_current_rendered_documents() {
    let manifest = prompt_resource_manifest().expect("static resource manifest should validate");
    let resource = manifest
        .resources
        .iter()
        .find(|resource| resource.id == PromptContributionKind::CockpitOperatingContract)
        .expect("cockpit resource");

    assert_eq!(
        resource.rendered_documents,
        cockpit_operating_contract_manifest().contracts
    );
    assert!(
        manifest
            .resources
            .iter()
            .filter(|candidate| candidate.id != PromptContributionKind::CockpitOperatingContract)
            .all(|candidate| candidate.rendered_documents.is_empty())
    );
}

#[test]
fn every_resource_has_source_navigation() {
    let manifest = prompt_resource_manifest().expect("static resource manifest should validate");

    assert!(manifest.resources.iter().all(|resource| {
        let navigation = &resource.source_navigation;
        !navigation.modules.is_empty()
            && !navigation.symbols.is_empty()
            && !navigation.keywords.is_empty()
            && !navigation.tests.is_empty()
            && navigation
                .modules
                .iter()
                .all(|module| !module.is_empty() && !module.starts_with('/'))
    }));
}

#[test]
fn validation_rejects_duplicate_ids() {
    let mut manifest = valid_manifest();
    manifest.resources.push(manifest.resources[0].clone());
    assert!(manifest.validate().is_err());
}

#[test]
fn validation_rejects_dangling_references() {
    let mut manifest = valid_manifest();
    manifest.resources[0].dependencies = vec![PromptContributionKind::ProviderProcessing];
    manifest
        .resources
        .retain(|resource| resource.id != PromptContributionKind::ProviderProcessing);
    assert!(manifest.validate().is_err());
}

#[test]
fn validation_rejects_self_references_and_dependency_conflict_overlap() {
    let mut self_dependency = valid_manifest();
    self_dependency.resources[0].dependencies = vec![self_dependency.resources[0].id];
    assert!(self_dependency.validate().is_err());

    let mut self_conflict = valid_manifest();
    self_conflict.resources[0].conflicts = vec![self_conflict.resources[0].id];
    assert!(self_conflict.validate().is_err());

    let mut overlap = valid_manifest();
    overlap.resources[0].dependencies = vec![PromptContributionKind::ProviderProcessing];
    overlap.resources[0].conflicts = vec![PromptContributionKind::ProviderProcessing];
    assert!(overlap.validate().is_err());
}

#[test]
fn validation_rejects_missing_source_navigation() {
    let mut manifest = valid_manifest();
    manifest.resources[0].source_navigation.modules.clear();
    assert!(manifest.validate().is_err());
}

fn valid_manifest() -> PromptResourceManifest {
    prompt_resource_manifest().expect("static resource manifest should validate")
}
