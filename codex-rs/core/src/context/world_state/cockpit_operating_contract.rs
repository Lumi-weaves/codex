use super::PreviousSectionState;
use super::WorldStateHash;
use super::WorldStateSection;
use crate::cockpit_operating_contract::CockpitContractRole;
use crate::context::CockpitOperatingContract;
use crate::context::ContextualUserFragment;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CockpitOperatingContractState {
    role: CockpitContractRole,
}

impl CockpitOperatingContractState {
    pub(crate) fn new(role: CockpitContractRole) -> Self {
        Self { role }
    }
}

impl WorldStateSection for CockpitOperatingContractState {
    const ID: &'static str = "cockpit_operating_contract";
    type Snapshot = WorldStateHash;

    fn snapshot(&self) -> Self::Snapshot {
        WorldStateHash::from_fragment(&CockpitOperatingContract::new(self.role))
    }

    fn matches_legacy_fragment(role: &str, text: &str) -> bool {
        role == "developer" && CockpitOperatingContract::matches_text(text)
    }

    fn has_retained_fragment_matcher() -> bool {
        true
    }

    fn matches_retained_fragment(role: &str, text: &str) -> bool {
        Self::matches_legacy_fragment(role, text)
    }

    fn render_diff(
        &self,
        previous: PreviousSectionState<'_, Self::Snapshot>,
    ) -> Option<Box<dyn ContextualUserFragment>> {
        if matches!(previous, PreviousSectionState::Known(previous) if previous == &self.snapshot())
            || matches!(previous, PreviousSectionState::Unknown)
        {
            return None;
        }
        Some(Box::new(CockpitOperatingContract::new(self.role)))
    }
}
