# Upstream 0.148 watch note

Status recorded on 2026-08-11. This is an integration checkpoint, not a
commitment to adopt an alpha release.

## Current upstream state

- The latest stable OpenAI Codex release is still `rust-v0.147.0`.
- The newest published 0.148 tag is the prerelease
  [`rust-v0.148.0-alpha.6`](https://github.com/openai/codex/releases/tag/rust-v0.148.0-alpha.6),
  peeled to `bccbfd143de6e35a20bafe735fb5187f3b1930ea`.
- The alpha release bodies contain only the release title, not a curated
  changelog. The useful change inventory therefore comes from the upstream
  commits and referenced pull requests.
- At inspection time, upstream `main` was already 40 commits beyond alpha.6,
  including follow-up work in gRPC Code Mode, thread persistence, packaged
  runtime discovery, and unified-exec telemetry.

The 0.148 alpha line includes several worthwhile changes: a gRPC/TCP Code Mode
host, a major plugin/skills-loader consolidation, durable user-message queue
dispatch, asynchronous hooks, MCP event subscriptions, richer runtime
diagnostics, full-history agent roles, base-instruction provenance, session
archive/resume/export improvements, and line-ending preservation for
`apply_patch`.

## Merge probe

An isolated merge probe compared Lumi `main` at
`2efe4eff70fe2492d85f41f7d13f128c1ea8e137` with
`rust-v0.148.0-alpha.6`.

- merge base: `92b83e226df59dc5ec43a49259d7716821e20c85`;
- divergence: 47 Lumi commits and 104 upstream commits;
- exact changed-path overlap: 59 files;
- explicit conflicts: three text files (`Cargo.toml`, `Cargo.lock`, and
  `core/src/tasks/mod.rs`) plus two generated app-server schema archives;
- after a provisional resolution, `cargo check` passed for `codex-core`,
  `codex-app-server`, and `codex-cli`;
- all eight `unified_exec_async_completion` integration tests passed;
- compiling the broader core test surface then exposed ten downstream porting
  gaps around the new durable-queue `start_task` argument and the move from
  `UnifiedExecContext.turn` to `step_context.turn`.

The probe worktree was removed after observation. Lumi `main` was not changed.

## Why alpha.6 should not advance Lumi `main`

The visible conflicts are tractable, but the semantic boundary is not yet a
routine merge:

- awaited-terminal finality and cleanup must compose deliberately with
  upstream `ThreadIdleCause` and durable queued-user-message dispatch;
- completion/TTY-attention inputs must remain FIFO, exactly once, and must not
  trigger thread idle or unload prematurely;
- app-server schemas must be regenerated while retaining Lumi's
  `WaitingOnBackgroundTerminal` status and all new upstream protocol fields;
- role-local worker base instructions must compose with upstream base-
  instruction provenance and full-history roles;
- Lumi's separate control-plane and model-inference authentication managers
  must survive the new account, telemetry, routing, and identity paths;
- adopting an alpha as `0.148.0-lumi.N` would hide its prerelease provenance,
  while preserving `alpha.6` in the downstream version would require a wider
  release-tooling contract change.

Accordingly, `main` remains on `0.147.0-lumi.4`. A published stable
`rust-v0.148.0` with real release notes is the next integration trigger.

## Stable-release recheck

When `rust-v0.148.0` exists, repeat the port on a disposable sync worktree and
require at least:

1. regenerated Cargo lockfile and app-server schemas with a clean second
   generation;
2. awaited-terminal, durable-queue, TTY-attention, interrupt, cleanup, and
   exactly-once completion tests;
3. app-server waiting-status, resume, diagnostics, and unload tests;
4. custom-provider worker-base, follow-up routing, model-catalog, and dual-auth
   tests;
5. canonical package, installer, Code Mode host, shadow workflow, and one
   Desktop end-to-end completion-wake smoke before promotion to `main`.
