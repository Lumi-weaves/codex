<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>
</br>
If you want Codex in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>codex app</code> or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## About Lumi Codex

**Lumi Codex** is an open-source downstream created and maintained by
[Lumi](https://github.com/Lumi-weaves), with its original workflow ideas and
product direction developed together with Fletcher Tian. It follows published
OpenAI Codex releases rather than an arbitrary upstream `main` snapshot, preserves
upstream defaults, and keeps each downstream patch narrow enough to remove when
upstream provides equivalent behavior.

[`main`](https://github.com/Lumi-weaves/codex/tree/main) is the canonical Lumi
Codex product line. Its current upstream base is the published OpenAI tag
[`rust-v0.147.0`](https://github.com/openai/codex/releases/tag/rust-v0.147.0).
Future upstream upgrades start from immutable stable tags, are integrated on a
temporary sync branch, and reach `main` only after the downstream patch set and
release path have been validated together. Maintenance branches exist only for
older release lines that are still receiving fixes. This product line carries
the following downstream work:

### Cross-provider MultiAgentV2 delivery

Commit [`59b3699`](https://github.com/Lumi-weaves/codex/commit/59b369924db09005aae42f540c3314f9c59bfac4)
adds an opt-in compatibility namespace for Responses-compatible providers that
cannot decode OpenAI's encrypted collaboration payload. Commits
[`82ecd6e`](https://github.com/Lumi-weaves/codex/commit/82ecd6e18b) and
[`4d4c73c`](https://github.com/Lumi-weaves/codex/commit/4d4c73c445)
extend that route to direct messages and follow-up tasks, including follow-ups
sent after a worker has completed. The compatibility namespace requests
plaintext tool arguments and delivers them as ordinary model input for
non-OpenAI child providers, while the upstream `collaboration` namespace
remains unchanged.

```toml
[features.multi_agent_v2]
tool_namespace = "lumi_collaboration"
```

### Separate control and model credentials

Commit [`b3c43bd`](https://github.com/Lumi-weaves/codex/commit/b3c43bd7d8e9836dd8b0ae2ff491f1308de2a8f7)
allows Codex control-plane services and model-usage traffic to use different
credentials on one machine. The independent model credential supports the
existing API-key and managed ChatGPT OAuth flows, including refresh and 401
recovery, and never silently falls back to the control credential.

```shell
codex login --scope model
# Or: printenv OPENAI_API_KEY | codex login --scope model --with-api-key
```

```toml
model_auth_source = "model"
```

The default remains `model_auth_source = "control"`, so an unconfigured build
behaves like upstream.

### Public-delivery canary

Lumi builds identify themselves with a `-lumi.N` version and refuse every
official OpenAI update action, background update check, doctor update probe,
and announcement feed. Canary releases are installed independently as
`lumi-codex` through the canonical Codex precompiled-package installer flow,
fork-aware for Lumi: they download only from the `Lumi-weaves/codex` GitHub
Releases, install into a Lumi-owned root
(`${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex`), verify the checksum
manifest against the GitHub release-metadata digest and the package archive
against the manifest, and never modify `CODEX_HOME`, an existing `codex`
binary, a shell profile, or PATH.

After the `rust-v0.147.0-lumi.4` canary assets have been published, its
version-pinned one-command install is:

```shell
curl -fsSL https://github.com/Lumi-weaves/codex/releases/download/rust-v0.147.0-lumi.4/install.sh | \
  sh -s -- --release 0.147.0-lumi.4
```

```powershell
$env:LUMI_RELEASE = '0.147.0-lumi.4'
irm https://github.com/Lumi-weaves/codex/releases/download/rust-v0.147.0-lumi.4/install.ps1 | iex
```

The commands are intentionally version-pinned because GitHub's `latest`
endpoint does not select prereleases. Until that tag exists, build from source
instead; do not treat them as live release URLs.

Lumi publishes no x86_64 (Intel) macOS prebuilt; on an Intel Mac the Unix
installer fails early with that message, and you should build from source
instead (the ARM package is never used as a fallback).

```shell
lumi-codex
```

`lumi-codex` is a tiny launcher that execs the verified
`<root>/current/bin/codex`, so the packaged resources and the code-mode host
stay adjacent to the real binary. The Windows installer
(`scripts/install/install.ps1`) mirrors the fork repo, tag, root, and version
behavior and consumes the x86_64 or arm64 Windows package published with every
canary; an incomplete release fails closed. The earlier Lumi
canary manager actions (doctor, activate, rollback, uninstall) were removed
with the manager; see the
[installer documentation](./scripts/install/LUMI_INSTALL.md) for the full
flow, layout, and safety model, and
[distribution design](./docs/lumi-distribution.md) for the adopted upstream
contract, fork boundary, release graph, and first-tag acceptance gate.
The TUI can also cooperate with an explicitly user-started official app-server
from the same upstream `MAJOR.MINOR.PATCH` release; version mismatch or missing
identity fails before thread/session traffic. This is not Desktop discovery,
replacement, or lifecycle ownership. See
[remote app-server compatibility](./codex-rs/app-server/README.md#remote-client-compatibility).

The OpenAI install commands below install upstream Codex and do not include the
Lumi patches; they are served from OpenAI's own hosts. The Lumi fork installer
in this repository (`scripts/install/install.sh`, published as `install.sh`) downloads only from the `Lumi-weaves/codex` GitHub Releases
and never uses the OpenAI mirror. See
[Installing & building](./docs/install.md) to build the fork.

### Async terminal completion wake

Commit [`2b752e4`](https://github.com/Lumi-weaves/codex/commit/2b752e4a6db7861b76d55ad1d3f44b6e44b3d8d4)
adds an opt-in wake for background unified-exec terminal work. When a
terminal process finishes without a synchronous observation of its exit, the
session enqueues a bounded, model-visible completion fragment (process
identity, exit/failure status, duration, output-size metadata, and a small
head/tail excerpt of the transcript) and wakes a model turn to consume it.
`unified_exec_completion_wake` is a session-scoped runtime flag. On the Lumi
product line it is **enabled by default**: awaited background terminals keep
the task active, and their completion is wake-if-idle / queue-if-busy without
requiring a launch-time override. This product default matters for Desktop and
Remote-SSH reconnects, whose replacement app-server otherwise starts from the
ordinary persisted configuration rather than the previous process's ad-hoc
CLI flags. It can still be explicitly disabled for diagnosis or legacy
behavior:

```toml
[features]
unified_exec_completion_wake = false
```

The wake policy is wake-if-idle / queue-if-busy: an idle session is woken
immediately (including in Plan mode, like trigger-turn mailbox work), a busy
regular turn admits the completion at its next safe inference boundary, and a
turn already past its visible-answer boundary leaves the item queued to wake a
fresh turn after the old turn clears. Interrupts never lose queued completions
because abort cleanup only clears turn-local pending input.

Background terminal ownership is scoped to the lifetime of the app-server
runtime. A Desktop reconnect that replaces the remote app-server terminates
that runtime's terminals; the task transcript and ordinary thread state
survive, but live process handles and their future completions do not migrate
to the replacement process.

---

## Quickstart

### Installing and running Codex CLI

Run the following on Mac or Linux to install Codex CLI:

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

Run the following on Windows to install Codex CLI:

```shell
powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"
```

The standalone installers download from `https://releases.openai.com/codex` by default and fall back to GitHub Releases if a metadata or asset download is unavailable. To force GitHub Releases, set `CODEX_INSTALLER_USE_RELEASES_OPENAI_COM` to `false` (`0` and `no` are also accepted):

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_INSTALLER_USE_RELEASES_OPENAI_COM=false sh
```

```powershell
$env:CODEX_INSTALLER_USE_RELEASES_OPENAI_COM='false'; irm https://chatgpt.com/codex/install.ps1 | iex
```

Codex CLI can also be installed via the following package managers:

```shell
# Install using npm
npm install -g @openai/codex
```

```shell
# Install using Homebrew
brew install --cask codex
```

Then simply run `codex` to get started.

<details>
<summary>You can also go to the <a href="https://github.com/openai/codex/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

Each archive contains a single entry with the platform baked into the name (e.g., `codex-x86_64-unknown-linux-musl`), so you likely want to rename it to `codex` after extracting it.

</details>

### Using Codex with your ChatGPT plan

Run `codex` and select **Sign in with ChatGPT**. We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Codex with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Codex Documentation**](https://developers.openai.com/codex)
- [**Contributing**](./docs/contributing.md)
- [**Installing & building**](./docs/install.md)
- [**Open source fund**](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
