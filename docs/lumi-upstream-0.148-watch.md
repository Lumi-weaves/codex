# Upstream 0.148 integration note

Status recorded on 2026-08-11. This is an isolated alpha integration baseline,
not yet a Lumi `main` promotion or release.

## Upstream state

- The latest published 0.148 tag inspected here is
  `rust-v0.148.0-alpha.7`, peeled to
  `fb954fd60f5d8182ced65fbe041466c4333a98a0`.
- The alpha release body has no curated changelog. The integration inventory
  therefore comes from the upstream commits and source diff.
- Relative to Lumi `origin/main` at `0a232c8f35`, Git reports 51 Lumi commits
  and 149 upstream commits from merge base
  `92b83e226df59dc5ec43a49259d7716821e20c85`.
- The large count partly reflects rewritten upstream history: older 0.147 tag
  commits are not ancestors of alpha.7 even though much of their content had
  already reached Lumi.

The useful 0.148 changes include the gRPC/TCP Code Mode host, consolidated
skills loading, durable queued user messages, asynchronous hooks, MCP event
subscriptions, runtime diagnostics, richer thread persistence and export,
base-instruction provenance, and line-ending-safe patch application.

## Alpha.7 integration receipt

The merge was built in the isolated worktree/branch
`codex/upstream-0.148-integration` from Lumi `0.147.0-lumi.5`. The downstream
version is deliberately `0.148.0-alpha.7-lumi.1` so prerelease provenance is
not hidden.

Resolved semantic boundaries:

- preserved Lumi's single serialized `SessionIngress` FIFO for external
  submissions, terminal completions, and TTY-attention events while adopting
  upstream's move-only `Submission`/`Op` dispatch;
- kept completion admission queue-before-resolution and session-scoped
  pending input, so an interrupt before inference sampling cannot erase a
  terminal completion;
- composed awaited-terminal finality with upstream `ThreadIdleCause` using a
  session-owned one-shot idle claim. Resolution, task finalization, and queued
  continuation paths may race, but only one can restore finality and emit the
  retained `Completed`, `Interrupted`, or `Failed` cause;
- retained separate control-plane and model-inference authentication managers;
- regenerated Cargo.lock for the Lumi version without refreshing external
  dependency selections;
- retained `ThreadActiveFlag::WaitingOnBackgroundTerminal` and regenerated
  stable and experimental app-server schema exports;
- repaired `just write-app-server-schema` to use upstream's Python generator
  after the Rust generator binary was removed.

Validation completed on the integration baseline:

- `cargo check --locked` for `codex-core`, `codex-app-server`, and `codex-cli`;
- clean second-generation stable and experimental app-server schemas;
- all eight `unified_exec_async_completion` integration tests;
- focused FIFO, completion-ingress, interrupt, cleanup, finality, retained idle
  cause, and schema fixture tests.

Rust core tests require `RUST_MIN_STACK=8388608`; without it, the large
completion-ingress future can overflow the default test-thread stack even
though the same test passes with the repository's intended stack size.

## Remaining promotion gates

Before this baseline can advance Lumi `main`:

1. reapply the stashed same-task provider-switch prototype against the merged
   provider/client runtime instead of restoring its old routing structure
   mechanically;
2. rerun dual-auth, model-catalog, app-server waiting-status, packaging,
   installer, and Code Mode host gates;
3. run the Lumi CI contract and one Desktop completion-wake smoke;
4. review the final diff and publish only after the alpha provenance and
   residual upstream risk are explicit.

The original prototype remains recoverable in stash
`c69e323daa41a3f5c08b817daa8f56f4eefbdb14` until its rebased form is accepted.
