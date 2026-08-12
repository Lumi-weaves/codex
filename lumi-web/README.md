# Lumi Codex Web

Private, local Web management shell for Lumi Codex. Agent Operations is the
first read-only module; Prompt Studio, providers, profiles, releases, and hosts
are only navigation placeholders until they have real product contracts.

## Development

Use the repository's Node and pnpm versions, then run from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm --filter @lumi/codex-web dev
```

Agent Operations currently uses a deterministic, privacy-safe causal runtime
trace fixture. This branch deliberately does not define or proxy a BFF: the
prototype first tests whether generations, async spans, event queues, joins,
and pooled subagent lanes form a useful product contract. Prompt text, model
output, terminal output, and filesystem paths are absent from the fixture.

## Checks and build

```sh
pnpm --filter @lumi/codex-web run format
pnpm --filter @lumi/codex-web run lint
pnpm --filter @lumi/codex-web run typecheck
pnpm --filter @lumi/codex-web run test
pnpm --filter @lumi/codex-web run build
```

The production build is written to `lumi-web/dist/`. It is platform-neutral
and intentionally not embedded into a Rust compilation. A later release stage
will copy it into the native package for the local Rust BFF to serve.
