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

## Prototype replay decisions

The stashed async-poll/provider prototype has now been replayed on the clean
alpha.7 baseline. The replay keeps a yielded terminal inside the same logical
turn: a model poll may produce commentary or no message and park; queued user
input, terminal output needing stdin, or terminal completion resumes that turn.
A final message is an exclusive handoff and does not complete a turn while the
turn still owns awaited work.

Provider selection is deliberately route-first. A model-specific entry in
`model_provider_routes` is more specific than the global `model_provider`,
which remains the fallback for unrouted models. This ordering is required for
same-task switching and also prevents the effective provider copied during
resume or fork from pinning all later models to that provider. Focused core,
app-server settings-update, resume, and fork tests lock the boundary.

The prototype exporter is profile-driven (`just export-prototype`) and builds
the CLI and Code Mode host into one isolated package before atomically replacing
the previous runnable candidate.

## Remaining promotion gates

Before this integration can advance Lumi `main`:

1. complete the replayed async-poll, provider resume/fork, and prompt-contract
   regression suite;
2. rerun dual-auth, model-catalog, app-server waiting-status, packaging,
   installer, and Code Mode host gates;
3. run the Lumi CI contract and one Desktop completion-wake smoke;
4. review the final diff and publish only after the alpha provenance and
   residual upstream risk are explicit.

The original prototype remains recoverable in stash
`c69e323daa41a3f5c08b817daa8f56f4eefbdb14` until its rebased form is accepted.

## Alpha.12 selective recheck

Status recorded on 2026-08-13 after fetching `rust-v0.148.0-alpha.12`.

Lumi `main` now contains the deliberate alpha.7 integration plus newer
cockpit, prompt-receipt, and compaction-continuity work. A disposable alpha.12
merge probe produced 22 conflicts concentrated in RichCodex-owned seams:
turn-input admission and routing, delegated-turn lineage, compaction/history,
authentication selection, awaited-terminal completion, and app-server status.
The 96-commit alpha.7-to-alpha.12 delta therefore does **not** advance the
RichCodex base release or downstream version as a unit.

RichCodex treats tagged upstream releases as a source inventory rather than a
replacement harness. Substrate fixes may be ported independently when they do
not redefine cockpit ownership, attention ordering, lifecycle, or continuity.
Changes to those semantics are omitted by default and must be reimplemented or
explicitly reconciled against the central cockpit invariants before adoption;
reverting them after a broad merge is a fallback, not the normal update route.

The accepted alpha.12-derived substrate batch is limited to:

- exec-server startup retry;
- fail-closed Linux unreadable-glob handling;
- MCP elicitation cleanup and local custom-CA propagation;
- disabled Azure Responses storage;
- Windows sandbox/proxy correctness and nested-repository support;
- remote `apply_patch` sandbox hardening;
- network credential-broker hardening with bounded fallback ports; and
- protection against inline-visualization writes through the sandbox.

In particular, the upstream queued-user-message rewrite, unified turn-input
submission, root-turn tracking, interrupted-turn recovery, response-envelope
history migration, rollout/thread identity migration, and approval-pipeline
unification remain outside this batch. They overlap the RichCodex cockpit
kernel and require their own invariant-led decision rather than tag-driven
adoption.
