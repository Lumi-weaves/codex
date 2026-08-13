use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// A stable reference to one immutable Agent Definition revision.
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionRef {
    pub id: String,
    pub revision: u32,
}

/// The explicit boundary that selected an Agent for a thread.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentSelectionOrigin {
    Cli,
    Config,
    Resume,
    Fork,
}

/// The immutable Agent identity pinned to a live or persisted thread.
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentSelection {
    pub agent: AgentDefinitionRef,
    pub origin: AgentSelectionOrigin,
}

#[cfg(test)]
mod tests {
    use crate::protocol::SessionMeta;

    #[test]
    fn old_session_metadata_without_agent_selection_remains_legacy() {
        let mut value = serde_json::to_value(SessionMeta::default()).expect("serialize metadata");
        value
            .as_object_mut()
            .expect("metadata object")
            .remove("agent_selection");

        let metadata: SessionMeta = serde_json::from_value(value).expect("deserialize metadata");

        assert_eq!(metadata.agent_selection, None);
    }
}
