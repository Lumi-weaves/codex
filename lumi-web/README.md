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

Without a backend, the development server uses a deterministic, privacy-safe
Agent Operations fixture. To exercise the narrow BFF contract instead:

```sh
LUMI_WEB_BFF_ORIGIN=http://127.0.0.1:PORT \
  pnpm --filter @lumi/codex-web dev
```

Vite keeps the browser same-origin and proxies `/api` to that BFF. The browser
never connects to app-server directly and this package does not expose generic
JSON-RPC.

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
