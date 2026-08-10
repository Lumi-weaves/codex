# Lumi Codex standalone installer/manager (Unix canary)

`lumi-install.sh` installs and manages the official Codex CLI package releases
published on the **Lumi-weaves/codex** GitHub Releases, as an independent
installation beside any package-managed official Codex. It never touches
`CODEX_HOME` or official `codex` binaries and never uses the upstream
`install.sh`.

## Requirements

- Linux (x86_64 is the canary target; `--target` selects other known targets)
- POSIX `sh`, plus the tools the upstream installer already needs:
  `curl` or `wget`, `tar`, `mktemp`, `awk`, `sed`, `grep`, `fold`, `find`,
  `readlink`, `sha256sum`/`shasum`/`openssl`; `flock` is used when present
  (mkdir-lock fallback otherwise)

## Quick start

```sh
sh lumi-install.sh manage install          # resolve latest, install, activate
lumi-codex                                 # runs the installed Codex CLI
lumi-codex manage doctor                   # verify the managed install
```

## Commands

Run the script directly (`sh lumi-install.sh <action>`) or through the
`lumi-codex` launcher (`lumi-codex manage <action>`).

| Command | What it does |
| --- | --- |
| `install [--release V] [--target T] [--no-activate]` | Download and verify the release, stage it, switch `current` atomically, keep `previous`, refresh the manager copy and shim, and (unless `--no-activate`) add the owned PATH block. Default action. |
| `doctor` | Verify confinement, package completeness, receipts, current/previous consistency, launcher/shim/profile ownership, actual `--version`, and report the official `codex` path outside the shim (without executing it). |
| `list` | Show current, previous, installed releases, and activation state. |
| `rollback` | Switch back to `previous` (fails closed when there is none). |
| `activate` / `deactivate` | Add/remove exactly the owned Lumi Codex PATH block; drift fails closed. |
| `uninstall` | Remove only receipt-proven owned paths and the owned PATH block; refuses on unknown symlinks or a missing receipt. |

## Environment

- `LUMI_RELEASE` — version to install (default `latest`)
- `LUMI_TARGET` / `--target` — package target (default `x86_64-unknown-linux-musl`)
- `LUMI_ROOT` — override the managed root (must be absolute)
- `LUMI_PROFILE` — override the shell profile to activate
- Root default: `${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex`

## Managed layout

```
<root>/
  current -> releases/<version>-<target>   # atomic symlink switch
  previous -> releases/...                 # previous release
  releases/<version>-<target>/             # immutable verified package
  receipts/*.receipt                       # strict schema/magic receipts
  shim/codex, shim/lumi-codex              # owned symlinks on PATH when activated
  manager/lumi-install.sh                  # owned manager copy
```

## Safety model

- Fork-only downloads from `Lumi-weaves/codex` GitHub Releases; canonical
  `codex-package-*.tar.gz` archives verified against
  `codex-package_SHA256SUMS` and the release metadata digest.
- Lock + staging + complete-package validation + atomic `current` switch;
  failure aborts without switching (no non-atomic fallback).
- Activation adds one uniquely marked, exactly owned PATH block for the shim
  directory; official package-managed binaries are never overwritten.
- Uninstall deletes only receipt-proven owned paths; `CODEX_HOME` and official
  Codex are never touched.

## Unsupported edges (canary)

- macOS and Windows are not supported yet (Linux only).
- No public GitHub workflow, release publishing, or artifact signing; no
  Desktop replacement; no Rust identity management.
- A crash between the `current` switch and receipt write is detectable by
  `doctor`; re-running `install` repairs it.
