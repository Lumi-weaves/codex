# Lumi Codex canonical installer (fork)

`scripts/install/install.sh` (published as `install.sh`) and
`scripts/install/install.ps1` install Lumi Codex package releases from the
**Lumi-weaves/codex** GitHub Releases. They are the OpenAI canonical
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
curl -fsSL https://github.com/Lumi-weaves/codex/releases/download/rust-v0.147.0-lumi.1/install.sh | \
  sh -s -- --release 0.147.0-lumi.1

# From a checkout:
sh scripts/install/install.sh --release 0.147.0-lumi.1

lumi-codex   # runs the installed Lumi Codex CLI
```

## Quick start (Windows)

```powershell
# Version-pinned canary bootstrap (published release asset):
$env:LUMI_RELEASE = '0.147.0-lumi.1'
irm https://github.com/Lumi-weaves/codex/releases/download/rust-v0.147.0-lumi.1/install.ps1 | iex

# From a checkout:
powershell -ExecutionPolicy ByPass -File scripts\install\install.ps1 -Release 0.147.0-lumi.1

lumi-codex   # runs the installed Lumi Codex CLI (lumi-codex.cmd)
```

Each canary publishes the two canonical Windows packages alongside the four
Unix packages. The PowerShell installer selects the native x86_64 or arm64
asset, verifies the same checksum manifest, and fails closed if the release is
incomplete; it never falls back to a legacy package.

## Versions and targets

- `--release` / `LUMI_RELEASE` accepts `latest`, `x.y.z-lumi.N`, or
  `rust-vx.y.z-lumi.N` (the `rust-v` / `v` prefixes are normalized away).
  GitHub's `latest` endpoint excludes prereleases, so canary installs pin the
  exact `x.y.z-lumi.N` version.
- `--target` / `LUMI_TARGET` overrides platform detection and must be one of
  the four Unix targets (`x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`) or one of the two Windows targets
  (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`). Unknown or unsafe
  targets are rejected before any network request.

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `LUMI_RELEASE` | `latest` | Version to install (overridden by `--release`). |
| `LUMI_TARGET` | detected | Package target (overridden by `--target`). |
| `LUMI_ROOT` | `${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex` on Unix; `%LOCALAPPDATA%\lumi-codex` on Windows | Lumi-owned install root; must be absolute; a symlinked/junction root is rejected. |
| `LUMI_INSTALL_DIR` | `$HOME/.local/bin` on Unix; `%LOCALAPPDATA%\Programs\Lumi\Codex\bin` on Windows | Directory for the visible `lumi-codex` launcher. |

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
