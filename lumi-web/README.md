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

Agent Operations currently opens a sanitized real hw1 workflow as a native
Perfetto TrackEvent trace. The page is deliberately a trace selector and
privacy boundary rather than a home-grown renderer: Perfetto owns timeline
navigation, flows, counters, and SQL analysis. Prompt text, model output,
commands, terminal output, and filesystem paths are absent from the export.

The captured dogfood fixture is produced in two explicit stages:

```sh
# Run the historical adapter on the host that owns the state DB and rollouts.
python3 lumi-web/scripts/export-runtime-trace.py STATE_DB ROOT_THREAD_ID \
  > lumi-web/src/fixtures/hw1-runtime-trace.json

# Convert the closed JSON DTO to native Perfetto protobuf.
uv run --isolated --with perfetto==0.57.2 python \
  lumi-web/scripts/runtime_trace_to_perfetto.py \
  lumi-web/src/fixtures/hw1-runtime-trace.json \
  lumi-web/src/fixtures/hw1-runtime-trace.pftrace
```

The historical adapter infers generation and consume boundaries from rollout
recorder order. It exists to make real dogfood possible; live runtime tracing
must eventually supply authoritative boundaries and sequence IDs.

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
