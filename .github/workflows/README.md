# Workflow Strategy

Lumi Codex owns a small CI and delivery contract rather than running OpenAI's
private-runner matrix. The rationale, supported targets, and rollout invariants
are recorded in [LUMI_CI_CONTRACT.md](LUMI_CI_CONTRACT.md).

## Required change checks

`blocking-ci.yml` is the only automatic entrypoint for pull requests and pushes
to `main`. Its terminal `CI required` job is the eventual branch-protection
surface. It calls:

- `lumi-ci.yml`, which classifies changed paths and runs repository policy,
  Cargo-native Linux x86_64, SDK, and distribution checks only when relevant;
- `v8-canary.yml`, whose metadata job skips compilation for unrelated changes
  and whose relevant-change matrix contains only Linux x86_64 release and
  ptrcomp-sandbox variants.

A documentation-only change runs policy and metadata checks but no compiler.
Workflow changes exercise every fork-owned CI surface.

## Diagnostics

The inherited OpenAI workflows remain available as source and, where they have
a manual trigger, as diagnostics. They are not part of Lumi's automatic or
required contract. In particular, Lumi does not depend on OpenAI's
`codex-runners` groups, BuildBuddy topology, cross-platform Cargo nextest
shards, Intel macOS, or Windows CI.

## Delivery

- `lumi-release.yml` publishes unsigned, checksum-validated packages for Apple
  Silicon macOS and x86_64/arm64 Linux musl from exact Lumi tags.
- `lumi-release-shadow-worker.yml` is an exact-source, non-publishing manual
  build on our one-shot JIT infrastructure.
- Windows and Intel macOS remain in inherited source where useful for upstream
  merges, but they are not Lumi CI or release targets.
