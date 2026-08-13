use pretty_assertions::assert_eq;

use super::*;

#[test]
fn resolves_bare_and_revision_pinned_codex_agent() {
    let expected = AgentDefinitionRef {
        id: "codex".to_string(),
        revision: 1,
    };

    assert_eq!(resolve_agent_selector("codex"), Ok(expected.clone()));
    assert_eq!(resolve_agent_selector("codex@1"), Ok(expected));
}

#[test]
fn rejects_unknown_or_malformed_agent_selectors() {
    assert_eq!(
        resolve_agent_selector("missing"),
        Err("unknown Agent selector `missing`".to_string())
    );
    assert_eq!(
        resolve_agent_selector("codex@nope"),
        Err("invalid Agent revision in selector `codex@nope`".to_string())
    );
}
