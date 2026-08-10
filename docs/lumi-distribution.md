# Lumi Codex distribution

Lumi Codex follows OpenAI Codex's canonical precompiled-package contract with a
small fork-owned publication and installation layer. This document records the
durable boundary: what is adopted from upstream, what intentionally differs,
and what must be proven before a canary is promoted.

## Current maturity

The delivery workflow and active Lumi product line live on `main`.
`rust-v0.147.0-lumi.1` passed its tag gate but stopped before the release
mutation point: paid macOS larger runners could not start, and the hosted x64
Linux runner could not link the ARM musl package with its host `musl-gcc`.
It published no release assets. The next successful tag is therefore still a
build and packaging canary, not a stable release.

`rust-v0.147.0-lumi.2` proved the standard native ARM runner route: both Linux
targets and Windows x86_64 completed, while the remaining cold macOS builds
and Windows ARM packaging reached the workflow's original 90-minute timeout.
That run also stopped before the release mutation point and published no
release assets. Later canaries retain the standard runners with a wider build
watchdog.

`rust-v0.147.0-lumi.3` became the first canary with published release assets
under the widened watchdog, and its version-pinned install commands are the
current documented baseline in the README. Later canaries follow the same
standard-runner route and keep the release mutation point unchanged.

`rust-v0.147.0-lumi.4` was published before Lumi retired the x86_64 (Intel)
macOS target from the release contract; that historical prerelease may still
contain the retired `codex-package-x86_64-apple-darwin.tar.gz` asset. It is
not rewritten or deleted.

All current artifacts are intentionally **unsigned canaries**. The workflow
does not claim macOS signing or notarization, Windows Authenticode signing,
Linux signatures, provenance attestation, or a stable update channel.

## Product line and upstream tracking

`main` is the canonical Lumi Codex line: default checkouts, ongoing downstream
work, documentation, and release tags all converge there. Its history is not
periodically rebased onto OpenAI's moving `main` branch.

The `upstream` remote tracks `openai/codex`. Published upstream tags such as
`rust-v0.147.0` are the immutable integration bases. To adopt a later stable
release, create a temporary sync branch from Lumi `main`, merge the selected
upstream tag, reconcile and validate the downstream patch set, then
fast-forward the accepted result back to `main`. Alpha tags and arbitrary
upstream `main` snapshots do not silently advance the product line.

Lumi release tags such as `rust-v0.147.0-lumi.4` remain immutable publication
snapshots. A `lumi/release-X.Y.Z` branch is kept only when that older line is
still supported for backports; official upstream tags already provide the
historical source baselines and do not need duplicate tracking branches.

## Adopted upstream contract

The fork reuses the upstream release machinery rather than maintaining a
parallel package format:

- exact release tags of the form `rust-vX.Y.Z-lumi.N`, with the tag version
  required to equal the workspace Cargo version;
- `.github/scripts/build-codex-package-archive.sh` and
  `scripts/build_codex_package.py` as the canonical package producer;
- one primary `codex-package-<target>.tar.gz` for each release target;
- `codex-package_SHA256SUMS`, covering every canonical package archive;
- GitHub release-asset SHA-256 digests as the manifest trust anchor;
- a complete package layout containing the real entrypoint, Code Mode host,
  `rg`, platform resources, and the package metadata file;
- staged immutable release directories, an installation lock, a `current`
  pointer, and exact binary-version verification.

The five Lumi release targets (upstream Codex additionally publishes
x86_64-apple-darwin; Lumi does not publish Intel macOS prebuilts, so Intel
Mac users build from source):

- `aarch64-apple-darwin`
- `aarch64-unknown-linux-musl`
- `x86_64-unknown-linux-musl`
- `aarch64-pc-windows-msvc`
- `x86_64-pc-windows-msvc`

The workflow uses GitHub-hosted runners only. macOS and Windows ARM packages
are cross-built where appropriate and validated statically for their target
architecture; the release job does not pretend to execute a foreign binary.

## Fork-owned differences

The differences are deliberately narrow:

- release discovery and asset download are restricted to
  `Lumi-weaves/codex` GitHub Releases;
- versions carry the `-lumi.N` suffix and package metadata identifies the Lumi
  distribution;
- Lumi builds refuse official OpenAI update and announcement channels;
- installation is side-by-side under a Lumi-owned root and exposes only
  `lumi-codex` (or `lumi-codex.cmd`);
- installers do not modify `CODEX_HOME`, an existing `codex`, shell profiles,
  PATH, official package-manager state, authentication, or configuration;
- Unix extraction adds a member/type preflight that rejects absolute paths,
  traversal, symlinks, hardlinks, and special archive entries;
- releases are GitHub-only: no OpenAI R2 namespace, npm scope, Homebrew,
  WinGet, DotSlash channel, website hook, or private signing environment is
  copied into the fork.

The earlier Lumi canary manager was removed. It duplicated the canonical
installer with receipts, journals, activation, doctor, rollback, and uninstall
actions. Version switching now means rerunning the installer for an exact
release; removal means deleting the Lumi-owned root and visible launcher. This
keeps the public maintenance surface close to upstream while retaining the
fork's isolation and inexpensive archive hardening.

## Release graph and assets

A matching tag push runs `.github/workflows/lumi-release.yml`:

1. validate tag shape and equality with `codex-rs/Cargo.toml`;
2. build the five-target matrix and create each canonical archive;
3. fan all builds into one release job;
4. validate archive member safety, exact package metadata, required resources,
   target architecture, and embedded version;
5. generate and verify `codex-package_SHA256SUMS`;
6. stage exactly eight assets: five packages, the checksum manifest,
   `install.sh`, and `install.ps1`;
7. create an immutable GitHub prerelease and refuse to overwrite an existing
   release or its assets.

GitHub's `/releases/latest` excludes prereleases. Canary installation must pin
an exact version with `--release`, `-Release`, or `LUMI_RELEASE`; a no-argument
installer is not a canary channel.

## First-tag acceptance gate

Before documenting a Lumi release URL as live or repinning a managed machine,
the first real tag run must prove:

- all five hosted-runner builds complete;
- the release contains exactly the intended eight assets;
- GitHub API metadata exposes a `sha256:` digest for every asset;
- a pinned macOS/Linux install and a pinned Windows install complete from the
  published assets;
- `lumi-codex --version`, Code Mode host discovery, and packaged resources work
  from the real installed layout;
- an existing official Codex binary and `CODEX_HOME` remain unchanged.

Signing, notarization, stable/latest metadata, additional package managers, and
fleet cutover are later decisions. They are not implied by a successful
unsigned canary.

## Manual shadow workflow and just-in-time runner activation

The shadow workflow `.github/workflows/lumi-release-shadow-worker.yml` is
manual-only and never triggered by automation in this repository. A human
controller on an external trusted machine inspects an authorized
`workflow_dispatch` run and provisions a one-shot self-hosted runner through
`scripts/release/lumi_shadow_dispatch_jit.py` (the JIT dispatcher). Nothing
in this repository stores or derives credentials; activation requires a
short-lived GitHub token with the narrowest API authority (read repository
actions state, create JIT runner configurations), granted by the operator at
activation time and never committed. Permissions and real activation are
deliberately deferred to tomorrow's separate activation step.

Activation contract:

- the dispatcher hardcodes repository `Lumi-weaves/codex` and workflow path
  `.github/workflows/lumi-release-shadow-worker.yml`; the workflow static
  test locks the exact job names and per-run label formulas on both sides;
- the token is read only from the fixed environment variable
  `LUMI_GITHUB_TOKEN`; it is never accepted as an argument, written to a
  file, echoed, or included in any error output, and it is stripped (with
  `GH_TOKEN`, `GITHUB_TOKEN`, and any TOKEN/AUTHORIZATION-bearing variable)
  from the runner child process environment;
- before the single POST, the dispatcher fail-closed verifies: the exact
  run id/attempt, `workflow_dispatch` event, `main` head branch, exact
  40-hex `head_sha`, the workflow id resolving to the exact shadow workflow
  path, `refs/heads/main` matching the run commit, the run still live, the
  gate job `Resolve source to exact commit` completed/success, and the
  chosen job (`Build and validate shadow packages (aarch64)` or
  `Build and validate shadow package (x86_64)`) queued, unassigned, and
  carrying exactly the run's deterministic label
  `lumi-shadow-arm64-<run-id>-<attempt>` or
  `lumi-shadow-x86_64-<run-id>-<attempt>` (only the documented GitHub-added
  read-only label `self-hosted` is tolerated in the job's label list);
- the attempt-specific jobs are re-read immediately before the single
  non-retried `generate-jitconfig` POST, which requests the deterministic
  runner name, the explicit runner group, exactly the expected label, and
  work folder `_work`;
- the returned runner must match the requested name, be idle, and carry the
  expected label as its only custom label (GitHub-added read-only labels
  are allowed); the `encoded_jit_config` must be nonempty, at most 65536
  bytes (the shared conservative 64KiB hard cap both host controllers
  enforce, well under the Linux execve single-string limit), canonical
  base64. It is then streamed exactly once (plus one newline) to the
  runner command's stdin. It is never decoded, logged, stored, or echoed,
  and the dispatcher never polls, backgrounds, registers a runner, or
  makes any further API call.

The dispatcher and `LUMI_GITHUB_TOKEN` stay only on the trusted local
control host (Omen for the x86_64 target, the control host's SSH session
for the arm64 target). Export the token from tomorrow's credential/session
setup first, never inline; the runner child receives only the encoded
config on stdin, and SSH stdin carries only that encoded config. Examples
use placeholders only; never put the token in shell history, dotfiles, or
the repository.

Local Omen control host, x86_64 target (the mydotfiles Omen controller
`omen-codex-build-worker` runs the one-shot JIT runner; it stays attached
until the runner exits):

    export LUMI_GITHUB_TOKEN   # from tomorrow's credential/session setup
    python3 scripts/release/lumi_shadow_dispatch_jit.py \
      --run-id <RUN_ID> --run-attempt <ATTEMPT> --target x86_64 \
      --runner-group-id <RUNNER_GROUP_ID> -- \
      /path/to/mydotfiles/system/omen-codex-build-worker/bin/omen-codex-build-worker \
      runner-run-jit

Trusted local control host, arm64 target over SSH. The dispatcher runs on
the control host; `ssh` is the runner child and its stdin carries only the
encoded config. The remote helper path is deployed tomorrow (placeholder
beneath); the helper is the mydotfiles Mac controller
`macmini-codex-build-worker`, never actions-runner/run.sh. The SSH session
stays attached until the runner exits:

    export LUMI_GITHUB_TOKEN   # from tomorrow's credential/session setup
    python3 scripts/release/lumi_shadow_dispatch_jit.py \
      --run-id <RUN_ID> --run-attempt <ATTEMPT> --target arm64 \
      --runner-group-id <RUNNER_GROUP_ID> -- \
      ssh -T lumi-builder@macmini \
      /Users/lumi-builder/.local/bin/macmini-codex-build-worker runner-run-jit

The controller session stays attached until the one-shot runner exits; the
dispatcher propagates the runner's exit status. The encoded config is
ephemeral by design: it exists only in the dispatcher's memory and the
runner's stdin.

## Privacy and fleet boundary

The public distribution contains no private prompts, relationship skills,
credentials, provider keys, SSH topology, machine inventory, or personalized
Codex configuration. Those remain outside this repository.

`CubeLander/mydotfiles` may pin, cache, transport, configure, and verify a
published package for Fletcher's machines. It does not define the public
package format or release authority. A managed machine should move from a
private source build to a public package only after the corresponding public
artifact passes the first-tag gate above.

## Relevant source

- [Release workflow](../.github/workflows/lumi-release.yml)
- [Shadow workflow](../.github/workflows/lumi-release-shadow-worker.yml)
- [JIT dispatcher](../scripts/release/lumi_shadow_dispatch_jit.py)
- [Installer behavior and safety model](../scripts/install/LUMI_INSTALL.md)
- [Unix installer](../scripts/install/install.sh)
- [Windows installer](../scripts/install/install.ps1)
- [Canonical package builder](../scripts/build_codex_package.py)
