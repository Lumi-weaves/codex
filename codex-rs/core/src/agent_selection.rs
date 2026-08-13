use codex_protocol::agent::AgentDefinitionRef;

use crate::agent_manifest::agent_catalog_manifest;

pub(crate) fn resolve_agent_selector(selector: &str) -> Result<AgentDefinitionRef, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("Agent selector cannot be empty".to_string());
    }

    let (id, requested_revision) = match selector.rsplit_once('@') {
        Some((id, revision)) => {
            if id.is_empty() || revision.is_empty() {
                return Err(format!("invalid Agent selector `{selector}`"));
            }
            let revision = revision
                .parse::<u32>()
                .map_err(|_| format!("invalid Agent revision in selector `{selector}`"))?;
            if revision == 0 {
                return Err(format!("invalid Agent revision in selector `{selector}`"));
            }
            (id, Some(revision))
        }
        None => (selector, None),
    };

    let manifest = agent_catalog_manifest().map_err(|error| error.to_string())?;
    manifest
        .agent_definitions
        .iter()
        .filter(|definition| definition.id == id)
        .filter(|definition| {
            requested_revision.is_none_or(|revision| definition.revision == revision)
        })
        .max_by_key(|definition| definition.revision)
        .map(|definition| AgentDefinitionRef {
            id: definition.id.clone(),
            revision: definition.revision,
        })
        .ok_or_else(|| format!("unknown Agent selector `{selector}`"))
}

#[cfg(test)]
#[path = "agent_selection_tests.rs"]
mod tests;
