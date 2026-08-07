<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>
</br>
If you want Codex in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>codex app</code> or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## About the Lumi fork

This repository is also home to a lightweight downstream maintained by
[CubeLander](https://github.com/CubeLander) and
[Lumi](https://github.com/Lumi-weaves). It follows published OpenAI Codex
releases rather than an arbitrary `main` snapshot, preserves upstream defaults,
and keeps each downstream patch narrow enough to remove when upstream provides
equivalent behavior.

The current patch line is
[`lumi/release-0.147.0-alpha.1.2`](https://github.com/Lumi-weaves/codex/tree/lumi/release-0.147.0-alpha.1.2),
based on upstream tag
[`rust-v0.147.0-alpha.1.2`](https://github.com/openai/codex/releases/tag/rust-v0.147.0-alpha.1.2).
It currently carries two fixes:

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
behaves like upstream. This branch is currently distributed as source; the
OpenAI install commands below install upstream Codex and do not include these
patches. See [Installing & building](./docs/install.md) to build the fork.

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
