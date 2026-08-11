# Changelog

## Unreleased

### Changed

- Lumi Codex removes the upstream built-in `explorer` agent role, its embedded
  configuration, and its orchestration guidance. There is no compatibility
  alias or fallback: `agent_type = "explorer"` is now unknown unless a user
  intentionally defines a custom role with that name. Exploration is left to
  explicitly configured roles instead.

Upstream release notes can be found on the
[OpenAI Codex releases page](https://github.com/openai/codex/releases).
