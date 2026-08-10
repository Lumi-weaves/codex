# Lumi Codex distribution

Lumi Codex follows OpenAI Codex's canonical precompiled-package contract with a
small fork-owned publication and installation layer. This document records the
durable boundary: what is adopted from upstream, what intentionally differs,
and what must be proven before a canary is promoted.

## Current maturity

The delivery workflow is implemented on `codex/lumi-public-delivery`, but no
Lumi release tag has been published yet. The first tag run is therefore a build
and packaging canary, not a stable release.

All current artifacts are intentionally **unsigned canaries**. The workflow
does not claim macOS signing or notarization, Windows Authenticode signing,
Linux signatures, provenance attestation, or a stable update channel.

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

The six release targets match upstream Codex:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
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
2. build the six-target matrix and create each canonical archive;
3. fan all builds into one release job;
4. validate archive member safety, exact package metadata, required resources,
   target architecture, and embedded version;
5. generate and verify `codex-package_SHA256SUMS`;
6. stage exactly nine assets: six packages, the checksum manifest,
   `install.sh`, and `install.ps1`;
7. create an immutable GitHub prerelease and refuse to overwrite an existing
   release or its assets.

GitHub's `/releases/latest` excludes prereleases. Canary installation must pin
an exact version with `--release`, `-Release`, or `LUMI_RELEASE`; a no-argument
installer is not a canary channel.

## First-tag acceptance gate

Before documenting a Lumi release URL as live or repinning a managed machine,
the first real tag run must prove:

- all six hosted-runner builds complete;
- the release contains exactly the intended nine assets;
- GitHub API metadata exposes a `sha256:` digest for every asset;
- a pinned macOS/Linux install and a pinned Windows install complete from the
  published assets;
- `lumi-codex --version`, Code Mode host discovery, and packaged resources work
  from the real installed layout;
- an existing official Codex binary and `CODEX_HOME` remain unchanged.

Signing, notarization, stable/latest metadata, additional package managers, and
fleet cutover are later decisions. They are not implied by a successful
unsigned canary.

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
- [Installer behavior and safety model](../scripts/install/LUMI_INSTALL.md)
- [Unix installer](../scripts/install/install.sh)
- [Windows installer](../scripts/install/install.ps1)
- [Canonical package builder](../scripts/build_codex_package.py)
