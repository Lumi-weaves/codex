# Lumi Codex CI Contract

This document records the CI and delivery contract owned by Lumi Codex. It is
deliberately smaller than the upstream OpenAI matrix: the fork borrows the
checks that protect product invariants without inheriting OpenAI's private
runner fleet, BuildBuddy topology, release channels, or cross-platform test
budget.

## Product boundary

Lumi Codex publishes unsigned canary packages for:

- Linux x86_64 and arm64 (`musl`);
- Apple Silicon macOS (`aarch64-apple-darwin`); and
- Windows x86_64 and arm64 (`msvc`).

Linux x86_64 is the primary development and CI platform. Apple Silicon, Linux
arm64, and Windows are distribution targets. Windows publication requires a
successful build and package validation, but not OpenAI's full Windows Bazel,
Cargo nextest, native-main, or argument-comment-lint matrices.

Lumi Codex does not publish or continuously validate Intel macOS. A maintainer
who needs `x86_64-apple-darwin` must build it independently.

## What the upstream workflows protect

The upstream layout has two fan-in workflows:

- `blocking-ci.yml` calls Bazel, blob-size, dependency-policy, spelling,
  repository-policy, fast Cargo, and SDK workflows. `CI required` is the one
  intended required check.
- `postmerge-ci.yml` calls the full cross-platform Cargo matrix and the
  path-gated V8 canary. Its terminal job keeps broad main-only signal outside
  PR latency.

That split is useful and is retained as a design principle. Its concrete
matrix is not our contract:

- `bazel.yml` assumes OpenAI macOS larger runners and the repository-specific
  `codex-runners` group for Windows. It runs Linux GNU and musl tests, four
  Windows cross-test shards, native Windows main tests, cross-platform clippy,
  and release-configuration builds.
- `rust-ci-full.yml` adds Cargo clippy and four-shard nextest archives across
  macOS, Linux, and Windows, including remote-executor tests and native Windows
  arm64 replay.
- `sdk.yml` assumes the `codex-runners` Linux group even though its Python and
  TypeScript SDK checks are otherwise portable.
- `repo-checks.yml` mixes portable policy/unit tests with an npm-staging probe
  pinned to an `openai/codex` workflow artifact. A fork token cannot be assumed
  to read that artifact, so the workflow is not reusable as a unit.
- `v8-canary.yml` correctly avoids expensive work for unrelated changes, but
  its relevant-change matrix includes Intel macOS and Windows source builds
  that are outside Lumi's continuous-test budget.

The upstream fast Rust change detector is narrow: on a non-PR invocation it
intentionally forces the fast Rust bundle. The other children of
`blocking-ci.yml`, including Bazel and SDK, have no caller-level path gate.
Consequently, even a documentation-only push to `main` starts the heavyweight
matrix. This is an upstream operational choice, not a requirement of the
product.

## Lumi tiers

### 1. Required change checks

Run for pull requests and pushes to `main`. A single terminal result is the
eventual branch-protection surface.

Always-cheap policy checks:

- changed-blob size policy;
- Codespell;
- Cargo manifest and repository-boundary policy scripts;
- workflow/static-script tests when their owning files change; and
- formatting for changed source surfaces.

For Rust, build-system, or CI changes, add Cargo-native Linux x86_64 signal:

- Cargo formatting, dependency-shear, and dependency-policy checks;
- benchmark smoke test;
- `cargo clippy --tests` for `x86_64-unknown-linux-gnu`;
- one unsharded nextest run on `x86_64-unknown-linux-gnu`; and
- a release-mode compile on Linux x86_64 before publication rather than an
  always-on cross-platform release matrix.

OpenAI's Bazel lane is valuable to OpenAI because it verifies their Bazel
graph, exercises remote BuildBuddy execution, and warms shared caches. In the
fork it falls back to cold local execution and the observed Linux jobs exhaust
their 30-minute budget. Bazel lock/policy scripts remain cheap checks, while a
full hosted Bazel lane is manual diagnostic evidence until it proves cheaper
or more discriminating than the Cargo-native gate.

For SDK changes, run Python and TypeScript build/lint/test on an ordinary
GitHub-hosted Linux runner. Do not require OpenAI's runner group.

A documentation-only change must not start a compiler. A workflow change must
exercise the workflow and policy surface rather than silently selecting the
documentation fast path.

### 2. Targeted main/manual checks

Do not duplicate the entire required suite merely because a commit reached
`main`.

- V8 or code-mode dependency changes require a targeted Linux x86_64 canary.
  Apple Silicon and Linux arm64 package behavior is checked by the shadow
  release flow before publishing. Windows V8 source matrices and Intel macOS
  canaries are excluded.
- Full Cargo nextest, remote-executor, or additional architecture probes are
  manual diagnostics beyond the single primary-platform test run until
  repeated failures demonstrate that a scheduled lane would pay for itself.
- A trusted self-hosted machine may accelerate a main-only or manually gated
  check. It must use an exact authorized commit and an ephemeral/JIT runner;
  untrusted pull-request code never receives a persistent self-hosted runner.

### 3. Release checks

`lumi-release.yml` remains tag-gated and is the publication authority. Every
retained target must build the canonical package, validate its exact contents,
and contribute to the checksum manifest before the GitHub prerelease is
created.

`lumi-release-shadow-worker.yml` is manual, exact-source, non-publishing
evidence for our own Apple Silicon, Linux arm64, and Linux x86_64 builders. A
shadow artifact is never silently promoted into a release.

Windows x86_64 and arm64 remain build-and-package release targets. They do not
create a standing Windows CI matrix. Intel macOS is absent from both CI and
new releases.

Official OpenAI release-preparation schedules, signing, npm/R2/DotSlash,
Winget, and website deployment are not Lumi delivery dependencies.

## Required invariants

- A skipped relevant job is not success. Fan-in jobs accept only explicit
  success for work selected by change detection.
- Every injected or generated file check leaves the worktree clean.
- Workflow permissions are explicit and least-privilege; reusable workflows
  cannot request permissions their caller did not grant.
- Fork-owned jobs use ordinary hosted runners unless an exact-source JIT gate
  explicitly authorizes our infrastructure.
- Release jobs publish only from an exact Lumi tag whose version matches the
  workspace.
- CI failure must identify whether the product, a policy check, or unavailable
  infrastructure failed. Missing OpenAI runner groups must never masquerade as
  product regressions.

## Rollout

1. Restore the inherited policy checks and make workflow files validate.
2. Introduce one fork-owned, path-gated required workflow on hosted Linux.
3. Remove OpenAI-internal and unsupported platform jobs from automatic Lumi
   entrypoints while leaving upstream implementations available as reference.
4. Run representative documentation, Rust, SDK, workflow, and Lumi-tag changes
   through the new routing.
5. Only after the terminal check is reliably green, protect `main` with that
   single required context.
6. Generalize the existing JIT controller for trusted main-only acceleration
   only if hosted Linux latency remains materially costly.

This contract should change when observed escapes justify more coverage, not
when upstream adds a matrix leg for infrastructure or distribution targets
that Lumi does not own.
