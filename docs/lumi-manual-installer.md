# Manual multi-platform installer kits

Lumi Codex has a deliberately opt-in delivery path for trying the current
product `main` without creating a release. It is meant for Fletcher's own
machines and other trusted testers: build only when requested, download one
short-lived artifact, then make any takeover choice in the local terminal.

## Product contract

The manual workflow is `.github/workflows/lumi-manual-installer.yml`.

- It has only a `workflow_dispatch` trigger. Pushes, pull requests, schedules,
  and release tags do not start it.
- The low-bandwidth default builds only Linux x86-64. Linux ARM64, Apple
  Silicon macOS, or all three supported Lumi targets can be selected.
- It accepts only the current `main` checkout and never publishes a GitHub
  Release or mutates repository contents.
- Every package passes the same member, metadata, architecture, static-link,
  embedded-version, resource, and native `--version` checks as the shadow
  release path before the kit is assembled.
- Installer artifacts expire after three days. The already-compressed kit is
  uploaded without another expensive compression pass.
- Windows and Intel macOS remain outside the Lumi prebuilt product line.

Each selected target produces the Actions artifact
`lumi-codex-installer-<target>`. GitHub's browser download is a zip; `gh run
download` extracts the same artifact as a directory. From the extracted kit,
run:

```sh
sh install-lumi.sh
```

The kit contains the canonical Codex package archive, its SHA-256 manifest,
the normal side-by-side installer, and a small takeover helper. Installation
does not need another network request.

## Installation and takeover are separate decisions

The package is always verified and installed side-by-side first. The ordinary
`lumi-codex` launcher remains available even when every takeover question is
declined.

In an interactive terminal the kit then offers:

1. **CLI takeover** — atomically make `~/.local/bin/codex` and its Code Mode
   host point at the current verified Lumi package.
2. **Codex Desktop backend takeover** on macOS — configure the supported
   `CODEX_CLI_PATH` override to that stable `codex` path.

Neither choice modifies `CODEX_HOME`, auth, configuration, conversation
history, or shell profiles. The second choice does not replace, patch, sign,
or start Codex Desktop itself; Desktop continues to own the app-server child
and its lifecycle.

The macOS override cannot be implemented reliably by adding `export
CODEX_CLI_PATH=...` to `.zshrc`: a Finder-launched GUI application does not
inherit interactive-shell initialization. The helper instead updates the
current launchd GUI environment and installs the Lumi-owned LaunchAgent
`~/Library/LaunchAgents/io.lumi.codex-cli-path.plist` so the same override is
restored at login. Desktop must then be quit normally and reopened when active
work is safe.

If another user LaunchAgent also manages `CODEX_CLI_PATH`, takeover proceeds
only when it selects the same stable `codex` path (as Fletcher's mydotfiles
agent does). A competing value fails closed instead of creating a login-order
race.

Linux kits offer CLI takeover only. No supported Linux Desktop backend
discovery contract is assumed.

## Non-interactive use

Automation never receives an implicit yes. Available explicit modes are:

```sh
sh install-lumi.sh --side-by-side
sh install-lumi.sh --no-prompt --takeover-cli
sh install-lumi.sh --no-prompt --takeover-desktop  # macOS; ensures CLI takeover
```

After installation, the persistent helper lives under the Lumi install root:

```sh
~/.local/share/lumi-codex/tools/takeover.sh status
~/.local/share/lumi-codex/tools/takeover.sh rollback
```

The helper records only the paths and previous launchd override needed to
restore what it replaced. It fails closed on drift rather than deleting or
overwriting a path it no longer owns.

## Trust and remaining release boundary

Manual Actions artifacts are unsigned development candidates. The kit checks
its package against the included workflow-generated SHA-256 manifest, while
GitHub transports the enclosing Actions artifact. This is not a substitute
for the two-layer GitHub Release metadata trust anchor, signing, or macOS
notarization used by a future stable channel.

The tag-driven canary workflow and immutable GitHub Release assets remain the
publication path. A successful manual kit is evidence for a candidate, not a
release promotion.
