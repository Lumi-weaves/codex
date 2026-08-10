# Lumi Codex standalone installer/manager (Unix canary)

`lumi-install.sh` installs and manages the official Codex CLI package releases
published on the **Lumi-weaves/codex** GitHub Releases, as an independent
installation beside any package-managed official Codex. It never touches
`CODEX_HOME` or official `codex` binaries and never uses the upstream
`install.sh`. It works both from a local file and through
`curl ... | sh` bootstraps.

## Requirements

- Linux (x86_64 is the canary target; `aarch64-unknown-linux-musl` is allowed
  when the release package is complete)
- POSIX `sh`, plus the tools the upstream installer already needs:
  `curl` or `wget`, `tar`, `mktemp`, `awk`, `sed`, `grep`, `fold`, `find`,
  `readlink`, `sha256sum`/`shasum`/`openssl`; `flock` is used when present
  (mkdir-lock fallback otherwise). `mktemp -d` must succeed; there is no
  fallback temporary directory.

## Quick start

```sh
sh lumi-install.sh manage install        # resolve latest, install side-by-side
lumi-codex                               # runs the installed Codex CLI
lumi-codex manage doctor                 # verify the managed install
```

Default installs are **side-by-side**: no shell profile is modified and
`codex` still resolves to your official install. A visible `lumi-codex`
launcher is installed at `${LUMI_INSTALL_DIR:-$HOME/.local/bin}/lumi-codex`;
if that directory is not on `PATH`, the installer reports the exact path.

## Commands

Run the script directly (`sh lumi-install.sh <action>`) or through the
`lumi-codex` launcher (`lumi-codex manage <action>`).

| Command | What it does |
| --- | --- |
| `install [--release V] [--target T] [--activate]` | Download and verify the release, stage it, switch `current` atomically, keep `previous`, install the verified manager copy and visible launcher, and (with `--activate`) add the owned shim PATH block. Default action. |
| `doctor` | Verify confinement, packages, receipts, journal, current/previous consistency, manager/launcher/shim/profile ownership, actual `--version`, and report the official `codex` path outside the shim (without executing it). Reconciles a valid pending journal. |
| `list` | Show current, previous, installed releases, launcher, and activation state. |
| `rollback` | Switch back to `previous` (fails closed when there is none). |
| `activate` / `deactivate` | Add/remove exactly the owned Lumi Codex shim PATH block so `codex` is shadowed/restored; drift and symlinked profiles fail closed. |
| `uninstall` | Remove only receipt-proven owned paths (including the visible launcher) and the owned PATH block; refuses on unknown symlinks, drifted profiles, a pending journal, or a missing receipt. |

## Environment

- `LUMI_RELEASE` — version to install (default `latest`; accepts Lumi SemVer
  tags such as `rust-v0.147.0-lumi.1`)
- `LUMI_TARGET` / `--target` — package target from the Linux canary allowlist
  (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`); traversal or
  unknown targets are rejected before any URL/path use
- `LUMI_INSTALL_DIR` — where the visible `lumi-codex` launcher is installed
  (default `$HOME/.local/bin`; must be absolute and safe)
- `LUMI_ROOT` — override the managed root (must be absolute; an existing
  symlink root is rejected)
- `LUMI_PROFILE` — override the shell profile to activate
- `LUMI_DEV_MANAGER_SELF` — explicit developer/test mode: use this local file
  as the manager source (still `sh -n` checked) instead of the release asset
- Root default: `${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex`

## Managed layout

```
<root>/
  current -> releases/<version>-<target>   # atomic symlink switch
  previous -> releases/...                 # previous release (rollback target)
  releases/<version>-<target>/             # immutable verified package
  receipts/*.receipt                       # strict schema/magic receipts
  shim/codex, shim/lumi-codex              # owned symlinks, only on PATH when activated
  manager/lumi-install.sh                  # verified manager copy
  tmp/pending.journal                      # prepared-operation journal (transient)
```

## Safety model

- Fork-only downloads from `Lumi-weaves/codex` GitHub Releases; canonical
  `codex-package-*.tar.gz` archives verified against
  `codex-package_SHA256SUMS` **and** the release-metadata digest, which is the
  canary trust anchor: releases are not artifact-signed yet, so the GitHub
  release digest is the anchor and is documented as such.
- The manager copy is always the verified `lumi-install.sh` release asset
  (digest-checked and `sh -n` checked before install); `$0` is never copied,
  so `curl ... | sh` works.
- Archive hardening: absolute/`..` members, symlinks, hardlinks, and special
  entries are rejected before extraction; extraction does not preserve owner
  or permissions; `codex-package.json` must declare `distribution=lumi`,
  `variant=codex`, `entrypoint=bin/codex`, the exact version, and the target;
  required executables must be regular non-symlink files before the installer
  creates its own top-level `codex` link.
- Lock + staging + validation + atomic `current` switch; failure aborts
  without switching (no non-atomic fallback).
- A prepared-operation journal is written before switching `current`, so a
  SIGKILL immediately after the switch cannot lose the old release as the
  rollback target; `install`, `rollback`, and `doctor` deterministically
  reconcile a valid journal, and a tampered journal fails closed.
- Activation adds one uniquely marked, exactly owned PATH block for the shim
  directory; official package-managed binaries are never overwritten.
- Uninstall deletes only receipt-proven owned paths (and the owned launcher
  symlink), never `CODEX_HOME` or official Codex, and never unlinks the live
  lock while holding it.

## Unsupported edges (canary)

- macOS and Windows are not supported yet (Linux only).
- No public GitHub workflow, release publishing, or artifact signing; no
  Desktop replacement; no Rust identity management.
- A crash between the `current` switch and receipt finalization is
  journal-reconciled by the next `install`/`doctor`/`rollback`; a crash before
  the journal write leaves the previous state fully intact.
