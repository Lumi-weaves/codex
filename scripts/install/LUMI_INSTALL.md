# Lumi Codex canonical installer (fork)

For the publication graph, upstream adoption boundary, and first-tag gate, see
[the Lumi distribution design](../../docs/lumi-distribution.md).

`scripts/install/install.sh` (published as `install.sh`) installs Lumi Codex
package releases from the **Lumi-weaves/codex** GitHub Releases. It is the OpenAI canonical
precompiled-package installer flow made fork-aware: GitHub release metadata
(tag + per-asset SHA-256 digests), the `codex-package_SHA256SUMS` checksum
manifest, staged immutable version directories, an atomic `current` pointer,
an install lock, and exact binary-version verification.

Installs are **side-by-side**: the installer never writes to `CODEX_HOME`,
never touches a `codex` binary, a shell profile, or PATH, and never modifies
official or package-managed Codex state. It installs a visible `lumi-codex`
launcher that execs the real `<root>/current/bin/codex`, so the packaged
`codex-resources`, `codex-path`, and the `codex-code-mode-host` stay adjacent
to the real binary (required for code mode on macOS).

## Quick start (macOS / Linux)

```sh
# Version-pinned canary bootstrap (published release asset):
curl -fsSL https://github.com/Lumi-weaves/codex/releases/download/rust-v0.147.0-lumi.4/install.sh | \
  sh -s -- --release 0.147.0-lumi.4

# From a checkout:
sh scripts/install/install.sh --release 0.147.0-lumi.4

lumi-codex   # runs the installed Lumi Codex CLI
```

## Versions and targets

- `--release` / `LUMI_RELEASE` accepts `latest`, `x.y.z-lumi.N`, or
  `rust-vx.y.z-lumi.N` (the `rust-v` / `v` prefixes are normalized away).
  GitHub's `latest` endpoint excludes prereleases, so canary installs pin the
  exact `x.y.z-lumi.N` version.
- `--target` / `LUMI_TARGET` overrides platform detection and must be one of
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, or
  `aarch64-apple-darwin`. Unknown or unsafe targets are rejected before any
  network request.

Lumi does not publish x86_64 (Intel) macOS prebuilt binaries. On an Intel Mac
the Unix installer fails early with an unsupported-platform message instead of
downloading anything; build from source there. Intel Macs never fall back to
the ARM package or Rosetta.

Lumi also does not publish Windows packages. Windows source inherited from
upstream remains in the repository, but Windows is outside the supported Lumi
distribution contract.

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `LUMI_RELEASE` | `latest` | Version to install (overridden by `--release`). |
| `LUMI_TARGET` | detected | Package target (overridden by `--target`). |
| `LUMI_ROOT` | `${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex` | Lumi-owned install root; must be absolute and must not be a symlink. |
| `LUMI_INSTALL_DIR` | `$HOME/.local/bin` | Directory for the visible `lumi-codex` launcher. |

## Managed layout

```
<root>/
  current -> releases/<version>-<target>   # atomic pointer switch
  releases/<version>-<target>/             # immutable verified package
  install.lock | install.lock.d            # flock/lockf or mkdir lock
```

The complete canonical package layout is preserved under each release
directory (`codex-package.json`, `bin/codex`, `bin/codex-code-mode-host`,
`codex-path/rg`, `codex-resources/...`, plus the installer-created top-level
`codex` link).

## Verification and safety model

- Two-layer SHA-256 verification: the checksum manifest is verified against
  the GitHub release-metadata digest, and the package archive is verified
  against the manifest digest. Downloads that fail one layer are re-verified
  against the GitHub release-metadata digest (the canary trust anchor; Lumi
  releases are not artifact-signed yet) before the install fails closed.
- Staging, lock, and an atomic `current` switch; failure aborts without
  switching.
- Archive preflight rejects absolute members, `..` traversal, symlinks,
  hardlinks, and special entries before extraction; required package files
  must be regular (non-symlink) executables.
- Version verification: the installed binary must report exactly the resolved
  version.
- Fail-closed conflicts: an existing `current` that is not a symlink into our
  `releases` directory, a non-directory at the release path, a foreign file
  or reparse point at the launcher path, a symlinked/junction root, or a
  symlinked install directory all abort without mutation.
- No rollback/doctor/activate/uninstall machinery: the manager actions,
  receipts, journals, shims, and profile blocks from the earlier Lumi canary
  manager were removed. To switch versions, run the installer again for the
  desired release; to remove, delete the Lumi root and the `lumi-codex`
  launcher.
