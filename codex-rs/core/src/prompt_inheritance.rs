use codex_history::InitialHistory;
use codex_protocol::protocol::SessionSource;
use serde::Serialize;

pub const PROMPT_INHERITANCE_SCHEMA_VERSION: u32 = 1;
pub const PROMPT_COMPILER_REVISION: &str = "responses_request_lowering_v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptLifecycleShape {
    RootFresh,
    FullHistoryFork,
    LastNTurnFork,
    FreshRoleWorker,
    FollowUpParkedShadow,
    ResumeReconnect,
    ModelSwitch,
    CompactionNewWindow,
}

impl PromptLifecycleShape {
    fn persisted_name(self) -> &'static str {
        match self {
            Self::RootFresh => "root_fresh",
            Self::FullHistoryFork => "full_history_fork",
            Self::LastNTurnFork => "last_n_turn_fork",
            Self::FreshRoleWorker => "fresh_role_worker",
            Self::FollowUpParkedShadow => "follow_up_parked_shadow",
            Self::ResumeReconnect => "resume_reconnect",
            Self::ModelSwitch => "model_switch",
            Self::CompactionNewWindow => "compaction_new_window",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRevisionPolicy {
    PinCurrent,
    PinParent,
    PinPersisted,
    PreserveSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptContributionPolicy {
    BuildFresh,
    InheritOrderedHistory,
    InheritThenReplaceRoleLocal,
    RebuildFromCurrentState,
    PreserveSession,
    ReplaceCompactedHistory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptWorldStatePolicy {
    EstablishFresh,
    InheritReference,
    Rebuild,
    PreserveReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCapabilityPolicy {
    BindFromRuntimeSelection,
    RevalidateRuntimeSelection,
    PreserveExplicitRuntimeBinding,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInheritanceContract {
    pub lifecycle: PromptLifecycleShape,
    pub revision_policy: PromptRevisionPolicy,
    pub base_instructions: PromptContributionPolicy,
    pub conversation_history: PromptContributionPolicy,
    pub role_local_instructions: PromptContributionPolicy,
    pub world_state: PromptWorldStatePolicy,
    pub capability_binding: PromptCapabilityPolicy,
    pub context_inheritance_grants_authority: bool,
    pub stale_when: &'static [&'static str],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInheritanceMatrix {
    pub schema_version: u32,
    pub compiler_revision: &'static str,
    pub contracts: Vec<PromptInheritanceContract>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptContextSeed {
    pub(crate) lifecycle: PromptLifecycleShape,
    pub(crate) compiler_revision: String,
    pub(crate) prior_origin: Option<String>,
}

impl PromptContextSeed {
    pub(crate) fn fork(
        lifecycle: PromptLifecycleShape,
        compiler_revision: impl Into<String>,
        prior_origin: Option<String>,
    ) -> Self {
        debug_assert!(matches!(
            lifecycle,
            PromptLifecycleShape::FullHistoryFork | PromptLifecycleShape::LastNTurnFork
        ));
        Self {
            lifecycle,
            compiler_revision: compiler_revision.into(),
            prior_origin,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptRuntimeContext {
    lifecycle: PromptLifecycleShape,
    compiler_revision: String,
    revision_policy: PromptRevisionPolicy,
    prior_origin: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInheritanceProvenance {
    pub schema_version: u32,
    pub lifecycle_origin: PromptLifecycleShape,
    pub prior_origin: Option<String>,
    pub compiler_revision: String,
    pub revision_policy: PromptRevisionPolicy,
    pub base_instructions: PromptContributionPolicy,
    pub conversation_history: PromptContributionPolicy,
    pub role_local_instructions: PromptContributionPolicy,
    pub world_state: PromptWorldStatePolicy,
    pub capability_binding: PromptCapabilityPolicy,
    pub context_inheritance_grants_authority: bool,
}

impl PromptRuntimeContext {
    #[cfg(test)]
    pub(crate) fn root_for_tests() -> Self {
        Self::resolve(&InitialHistory::New, &SessionSource::Exec, None)
    }

    pub(crate) fn resolve(
        initial_history: &InitialHistory,
        session_source: &SessionSource,
        seed: Option<PromptContextSeed>,
    ) -> Self {
        if let Some(seed) = seed {
            return Self {
                lifecycle: seed.lifecycle,
                compiler_revision: seed.compiler_revision,
                revision_policy: PromptRevisionPolicy::PinParent,
                prior_origin: seed.prior_origin,
            };
        }

        let persisted_revision = persisted_prompt_compiler_revision(initial_history);
        let persisted_origin = persisted_prompt_context_origin(initial_history);
        match initial_history {
            InitialHistory::Resumed(_) => Self {
                lifecycle: PromptLifecycleShape::ResumeReconnect,
                compiler_revision: persisted_revision
                    .unwrap_or_else(|| PROMPT_COMPILER_REVISION.to_string()),
                revision_policy: PromptRevisionPolicy::PinPersisted,
                prior_origin: persisted_origin,
            },
            InitialHistory::Forked(_) => Self {
                lifecycle: PromptLifecycleShape::FullHistoryFork,
                compiler_revision: persisted_revision
                    .unwrap_or_else(|| PROMPT_COMPILER_REVISION.to_string()),
                revision_policy: PromptRevisionPolicy::PinParent,
                prior_origin: persisted_origin,
            },
            InitialHistory::New | InitialHistory::Cleared if session_source.is_non_root_agent() => {
                Self {
                    lifecycle: PromptLifecycleShape::FreshRoleWorker,
                    compiler_revision: PROMPT_COMPILER_REVISION.to_string(),
                    revision_policy: PromptRevisionPolicy::PinCurrent,
                    prior_origin: None,
                }
            }
            InitialHistory::New | InitialHistory::Cleared => Self {
                lifecycle: PromptLifecycleShape::RootFresh,
                compiler_revision: PROMPT_COMPILER_REVISION.to_string(),
                revision_policy: PromptRevisionPolicy::PinCurrent,
                prior_origin: None,
            },
        }
    }

    pub(crate) fn compiler_revision(&self) -> &str {
        &self.compiler_revision
    }

    pub(crate) fn persisted_origin(&self) -> String {
        self.lifecycle.persisted_name().to_string()
    }

    pub(crate) fn seed_for_fork(&self, lifecycle: PromptLifecycleShape) -> PromptContextSeed {
        PromptContextSeed::fork(
            lifecycle,
            self.compiler_revision.clone(),
            Some(self.persisted_origin()),
        )
    }

    pub(crate) fn provenance(&self) -> PromptInheritanceProvenance {
        let contract = contract_for(self.lifecycle);
        PromptInheritanceProvenance {
            schema_version: PROMPT_INHERITANCE_SCHEMA_VERSION,
            lifecycle_origin: self.lifecycle,
            prior_origin: self.prior_origin.clone(),
            compiler_revision: self.compiler_revision.clone(),
            revision_policy: self.revision_policy,
            base_instructions: contract.base_instructions,
            conversation_history: contract.conversation_history,
            role_local_instructions: contract.role_local_instructions,
            world_state: contract.world_state,
            capability_binding: contract.capability_binding,
            context_inheritance_grants_authority: false,
        }
    }
}

pub fn prompt_inheritance_matrix() -> PromptInheritanceMatrix {
    PromptInheritanceMatrix {
        schema_version: PROMPT_INHERITANCE_SCHEMA_VERSION,
        compiler_revision: PROMPT_COMPILER_REVISION,
        contracts: ALL_LIFECYCLES.into_iter().map(contract_for).collect(),
    }
}

const ALL_LIFECYCLES: [PromptLifecycleShape; 8] = [
    PromptLifecycleShape::RootFresh,
    PromptLifecycleShape::FullHistoryFork,
    PromptLifecycleShape::LastNTurnFork,
    PromptLifecycleShape::FreshRoleWorker,
    PromptLifecycleShape::FollowUpParkedShadow,
    PromptLifecycleShape::ResumeReconnect,
    PromptLifecycleShape::ModelSwitch,
    PromptLifecycleShape::CompactionNewWindow,
];

fn contract_for(lifecycle: PromptLifecycleShape) -> PromptInheritanceContract {
    use PromptCapabilityPolicy as Capability;
    use PromptContributionPolicy as Contribution;
    use PromptLifecycleShape as Lifecycle;
    use PromptRevisionPolicy as Revision;
    use PromptWorldStatePolicy as WorldState;

    let (revision_policy, base, history, role, world_state, capability, stale_when) =
        match lifecycle {
            Lifecycle::RootFresh => (
                Revision::PinCurrent,
                Contribution::BuildFresh,
                Contribution::BuildFresh,
                Contribution::BuildFresh,
                WorldState::EstablishFresh,
                Capability::BindFromRuntimeSelection,
                &[
                    "compiler_revision_changes",
                    "runtime_capability_selection_changes",
                ][..],
            ),
            Lifecycle::FullHistoryFork => (
                Revision::PinParent,
                Contribution::InheritOrderedHistory,
                Contribution::InheritOrderedHistory,
                Contribution::InheritThenReplaceRoleLocal,
                WorldState::InheritReference,
                Capability::RevalidateRuntimeSelection,
                &[
                    "parent_context_advances_after_snapshot",
                    "role_contract_changes",
                ][..],
            ),
            Lifecycle::LastNTurnFork => (
                Revision::PinParent,
                Contribution::InheritOrderedHistory,
                Contribution::InheritOrderedHistory,
                Contribution::InheritThenReplaceRoleLocal,
                WorldState::Rebuild,
                Capability::RevalidateRuntimeSelection,
                &[
                    "parent_context_advances_after_snapshot",
                    "truncated_turns_contain_required_state",
                ][..],
            ),
            Lifecycle::FreshRoleWorker => (
                Revision::PinCurrent,
                Contribution::BuildFresh,
                Contribution::BuildFresh,
                Contribution::BuildFresh,
                WorldState::EstablishFresh,
                Capability::BindFromRuntimeSelection,
                &[
                    "role_contract_changes",
                    "runtime_capability_selection_changes",
                ][..],
            ),
            Lifecycle::FollowUpParkedShadow => (
                Revision::PreserveSession,
                Contribution::PreserveSession,
                Contribution::PreserveSession,
                Contribution::PreserveSession,
                WorldState::PreserveReference,
                Capability::PreserveExplicitRuntimeBinding,
                &[
                    "parked_shadow_context_is_superseded",
                    "role_contract_changes",
                ][..],
            ),
            Lifecycle::ResumeReconnect => (
                Revision::PinPersisted,
                Contribution::InheritOrderedHistory,
                Contribution::InheritOrderedHistory,
                Contribution::InheritOrderedHistory,
                WorldState::Rebuild,
                Capability::RevalidateRuntimeSelection,
                &[
                    "persisted_revision_is_no_longer_supported",
                    "runtime_authority_changes",
                ][..],
            ),
            Lifecycle::ModelSwitch => (
                Revision::PreserveSession,
                Contribution::PreserveSession,
                Contribution::PreserveSession,
                Contribution::PreserveSession,
                WorldState::Rebuild,
                Capability::PreserveExplicitRuntimeBinding,
                &["new_model_requires_incompatible_base_contract"][..],
            ),
            Lifecycle::CompactionNewWindow => (
                Revision::PreserveSession,
                Contribution::PreserveSession,
                Contribution::ReplaceCompactedHistory,
                Contribution::PreserveSession,
                WorldState::Rebuild,
                Capability::PreserveExplicitRuntimeBinding,
                &[
                    "compaction_omits_required_context",
                    "world_state_baseline_is_invalid",
                ][..],
            ),
        };
    PromptInheritanceContract {
        lifecycle,
        revision_policy,
        base_instructions: base,
        conversation_history: history,
        role_local_instructions: role,
        world_state,
        capability_binding: capability,
        context_inheritance_grants_authority: false,
        stale_when,
    }
}

fn persisted_prompt_compiler_revision(initial_history: &InitialHistory) -> Option<String> {
    initial_history
        .get_rollout_items()
        .iter()
        .find_map(|item| match item {
            codex_history::RolloutItem::SessionMeta(meta) => {
                meta.meta.prompt_compiler_revision.clone()
            }
            _ => None,
        })
}

fn persisted_prompt_context_origin(initial_history: &InitialHistory) -> Option<String> {
    initial_history
        .get_rollout_items()
        .iter()
        .find_map(|item| match item {
            codex_history::RolloutItem::SessionMeta(meta) => {
                meta.meta.prompt_context_origin.clone()
            }
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use codex_history::ResumedHistory;
    use codex_history::RolloutItem;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::SessionMetaLine;
    use codex_protocol::protocol::SubAgentSource;

    #[test]
    fn matrix_is_exhaustive_unique_and_never_grants_authority() {
        let matrix = prompt_inheritance_matrix();
        assert_eq!(matrix.schema_version, PROMPT_INHERITANCE_SCHEMA_VERSION);
        assert_eq!(matrix.contracts.len(), ALL_LIFECYCLES.len());
        for (index, contract) in matrix.contracts.iter().enumerate() {
            assert!(!contract.context_inheritance_grants_authority);
            assert!(
                !matrix.contracts[..index]
                    .iter()
                    .any(|other| other.lifecycle == contract.lifecycle)
            );
        }
    }

    #[test]
    fn full_and_bounded_forks_replace_role_contract_without_sharing_authority() {
        for lifecycle in [
            PromptLifecycleShape::FullHistoryFork,
            PromptLifecycleShape::LastNTurnFork,
        ] {
            let contract = contract_for(lifecycle);
            assert_eq!(contract.revision_policy, PromptRevisionPolicy::PinParent);
            assert_eq!(
                contract.role_local_instructions,
                PromptContributionPolicy::InheritThenReplaceRoleLocal
            );
            assert_eq!(
                contract.capability_binding,
                PromptCapabilityPolicy::RevalidateRuntimeSelection
            );
            assert!(!contract.context_inheritance_grants_authority);
        }
    }

    #[test]
    fn session_transitions_preserve_the_pinned_revision() {
        for lifecycle in [
            PromptLifecycleShape::FollowUpParkedShadow,
            PromptLifecycleShape::ModelSwitch,
            PromptLifecycleShape::CompactionNewWindow,
        ] {
            assert_eq!(
                contract_for(lifecycle).revision_policy,
                PromptRevisionPolicy::PreserveSession
            );
        }
    }

    #[test]
    fn resume_pins_the_revision_and_origin_from_persisted_session_metadata() {
        let thread_id = ThreadId::new();
        let meta = codex_protocol::protocol::SessionMeta {
            id: thread_id,
            prompt_compiler_revision: Some("compiler_parent_v7".to_string()),
            prompt_context_origin: Some("full_history_fork".to_string()),
            ..Default::default()
        };
        let history = InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history: Arc::new(vec![RolloutItem::SessionMeta(SessionMetaLine {
                meta,
                git: None,
            })]),
            rollout_path: None,
        });

        let context = PromptRuntimeContext::resolve(&history, &SessionSource::Exec, None);
        let provenance = context.provenance();
        assert_eq!(
            provenance.lifecycle_origin,
            PromptLifecycleShape::ResumeReconnect
        );
        assert_eq!(
            provenance.revision_policy,
            PromptRevisionPolicy::PinPersisted
        );
        assert_eq!(provenance.compiler_revision, "compiler_parent_v7");
        assert_eq!(
            provenance.prior_origin.as_deref(),
            Some("full_history_fork")
        );
    }

    #[test]
    fn bounded_fork_seed_survives_history_truncation_without_granting_authority() {
        let parent = PromptRuntimeContext::root_for_tests();
        let seed = parent.seed_for_fork(PromptLifecycleShape::LastNTurnFork);
        let child = PromptRuntimeContext::resolve(
            &InitialHistory::Forked(Vec::new()),
            &SessionSource::SubAgent(SubAgentSource::Other("shadow".to_string())),
            Some(seed),
        );
        let provenance = child.provenance();
        assert_eq!(
            provenance.lifecycle_origin,
            PromptLifecycleShape::LastNTurnFork
        );
        assert_eq!(provenance.revision_policy, PromptRevisionPolicy::PinParent);
        assert_eq!(provenance.compiler_revision, PROMPT_COMPILER_REVISION);
        assert!(!provenance.context_inheritance_grants_authority);
        assert_eq!(provenance.world_state, PromptWorldStatePolicy::Rebuild);
    }

    #[test]
    fn fresh_role_worker_builds_context_and_authority_from_independent_inputs() {
        let context = PromptRuntimeContext::resolve(
            &InitialHistory::New,
            &SessionSource::SubAgent(SubAgentSource::Other("worker".to_string())),
            None,
        );
        let provenance = context.provenance();
        assert_eq!(
            provenance.lifecycle_origin,
            PromptLifecycleShape::FreshRoleWorker
        );
        assert_eq!(
            provenance.capability_binding,
            PromptCapabilityPolicy::BindFromRuntimeSelection
        );
        assert!(!provenance.context_inheritance_grants_authority);
    }
}
