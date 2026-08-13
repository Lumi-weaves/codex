use super::ContextualUserFragment;
use crate::cockpit_operating_contract::COCKPIT_OPERATING_CONTRACT_CLOSE_TAG;
use crate::cockpit_operating_contract::COCKPIT_OPERATING_CONTRACT_OPEN_TAG;
use crate::cockpit_operating_contract::CockpitContractRole;
use crate::cockpit_operating_contract::contract_body;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CockpitOperatingContract {
    role: CockpitContractRole,
}

impl CockpitOperatingContract {
    pub(crate) fn new(role: CockpitContractRole) -> Self {
        Self { role }
    }
}

impl ContextualUserFragment for CockpitOperatingContract {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn requires_separate_message(&self) -> bool {
        true
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (
            COCKPIT_OPERATING_CONTRACT_OPEN_TAG,
            COCKPIT_OPERATING_CONTRACT_CLOSE_TAG,
        )
    }

    fn body(&self) -> String {
        contract_body(self.role)
    }
}
