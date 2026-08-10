#!/usr/bin/env bash
# Static tests for the shadow workflow contract. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# Asserts the invariants from the real ARM smoke run:
#   * deterministic per-run self-hosted label, never a fixed reusable one;
#   * CARGO_BUILD_JOBS=4 (jobs=6 and =4 OOMed without the durable swap);
#   * the durable workflow-owned guest swap contract
#     ($HOME/.local/state/lumi-codex-arm-builder, 0600, mkswap/swapon,
#     never deletes anything);
#   * rg/zsh fetched through the canonical DotSlash manifests with explicit
#     --rg-bin/--zsh-bin overrides into the canonical package wrapper;
#   * apt uses exact Acquire::http::Proxy / Acquire::https::Proxy args;
#   * every network curl on the path is bounded (finite retry/connect/max-time)
#     and downloads remain checksum-verified.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workflow="${here}/../../.github/workflows/lumi-release-shadow-worker.yml"
build_script="${here}/lumi_shadow_build_linux.sh"
swap_script="${here}/lumi_shadow_swap.sh"
fetch_helper="${here}/lumi_shadow_fetch_dotslash.py"
install_rust="${here}/lumi_shadow_install_rust.sh"
curl_wrapper="${here}/lumi_shadow_curl.sh"
config="${here}/lumi_shadow_actionlint.yaml"

for file in "${workflow}" "${build_script}" "${swap_script}" \
  "${fetch_helper}" "${install_rust}" "${curl_wrapper}" "${config}"; do
  [[ -f "${file}" ]] || { echo "FAIL: missing file: ${file}" >&2; exit 1; }
done

# 1. The build job must run on the deterministic per-run label.
grep -q 'runs-on: lumi-shadow-arm64-\${{ github.run_id }}-\${{ github.run_attempt }}' "${workflow}" \
  || { echo "FAIL: per-run runs-on label missing" >&2; exit 1; }

# 2. No fixed reusable label: any runs-on value starting with the shadow
#    prefix must contain a ${{ expression }} (no static label remains).
if grep -E 'runs-on: lumi-shadow-arm64-[^$]+$' "${workflow}"; then
  echo "FAIL: fixed reusable self-hosted label found" >&2
  exit 1
fi

# 3. The actionlint config must not declare a fixed label.
if grep -q 'lumi-shadow-arm64-mac-mini' "${config}"; then
  echo "FAIL: actionlint config still declares a fixed label" >&2
  exit 1
fi

# 4. The workflow must not reference the legacy static label anywhere.
if grep -q 'lumi-shadow-arm64-mac-mini' "${workflow}"; then
  echo "FAIL: workflow still references the legacy static label" >&2
  exit 1
fi

# 5. Proven-safe parallelism: CARGO_BUILD_JOBS=4, never the OOM values.
grep -q 'CARGO_BUILD_JOBS=4' "${build_script}" \
  || { echo "FAIL: CARGO_BUILD_JOBS=4 missing" >&2; exit 1; }
if grep -Eq '^[[:space:]]*export[[:space:]]+CARGO_BUILD_JOBS=6([^0-9]|$)' "${build_script}"; then
  echo "FAIL: CARGO_BUILD_JOBS=6 still present" >&2
  exit 1
fi

# 6. Durable guest swap contract: the build invokes the helper, which owns
#    exactly one swapfile under the exact state root, validates ownership/
#    mode/size, activates idempotently, and never deletes anything.
grep -q 'lumi_shadow_ensure_swap' "${build_script}" \
  || { echo "FAIL: build script does not invoke the swap helper" >&2; exit 1; }
grep -Fq '${HOME}/.local/state/lumi-codex-arm-builder' "${swap_script}" \
  || { echo "FAIL: swap helper lacks the exact workflow state root" >&2; exit 1; }
for token in 'chmod 600' 'mkswap' 'swapon' 'fallocate' 'stat -c %s' 'stat -c %a' 'stat -c %u'; do
  grep -Fq "${token}" "${swap_script}" \
    || { echo "FAIL: swap helper missing ${token}" >&2; exit 1; }
done
if grep -Eq '(^|[[:space:]])rm([[:space:]]|$)' "${swap_script}"; then
  echo "FAIL: swap helper contains a delete operation" >&2
  exit 1
fi
if grep -q 'swapfile' "${workflow}"; then
  # The swapfile lives entirely inside the guest; the workflow must not
  # touch host-side swap state.
  echo "FAIL: workflow references guest swapfile state from the host" >&2
  exit 1
fi
# The real default for mkswap/swapon is passwordless `sudo -n` (the VM
# contract); tests disable/replace it via LUMI_SHADOW_SUDO only.
grep -Fq 'sudo_cmd=(sudo -n)' "${swap_script}" \
  || { echo "FAIL: swap helper real default is not 'sudo -n'" >&2; exit 1; }

# 7. Explicit rg/zsh overrides: the fetch helper resolves the canonical
#    DotSlash manifests with bounded curl and the package wrapper receives
#    --rg-bin/--zsh-bin on both the macOS host and in the guest.
grep -q -- '--rg-bin "${LUMI_SHADOW_RG_BIN}"' "${build_script}" \
  || { echo "FAIL: guest package wrapper lacks --rg-bin override" >&2; exit 1; }
grep -q -- '--zsh-bin "${LUMI_SHADOW_ZSH_BIN}"' "${build_script}" \
  || { echo "FAIL: guest package wrapper lacks --zsh-bin override" >&2; exit 1; }
grep -q -- '--rg-bin "${LUMI_SHADOW_RG_BIN}"' "${workflow}" \
  || { echo "FAIL: macOS package wrapper lacks --rg-bin override" >&2; exit 1; }
grep -q -- '--zsh-bin "${LUMI_SHADOW_ZSH_BIN}"' "${workflow}" \
  || { echo "FAIL: macOS package wrapper lacks --zsh-bin override" >&2; exit 1; }
grep -q 'lumi_shadow_fetch_dotslash.py' "${workflow}" \
  || { echo "FAIL: workflow does not run the DotSlash fetch helper" >&2; exit 1; }
grep -q 'scripts/codex_package' "${workflow}" \
  || { echo "FAIL: workflow does not stage the canonical DotSlash modules" >&2; exit 1; }
grep -q 'from codex_package.dotslash import' "${fetch_helper}" \
  || { echo "FAIL: fetch helper does not use canonical dotslash.py APIs" >&2; exit 1; }
grep -q 'verify_archive' "${fetch_helper}" \
  || { echo "FAIL: fetch helper does not verify size+SHA-256 canonically" >&2; exit 1; }

# 8. apt proxy args: initial apt commands and the canonical musl tools
#    script receive the exact Acquire proxy options (no smoke apt config).
grep -q 'Acquire::http::Proxy' "${build_script}" \
  || { echo "FAIL: build script lacks Acquire::http::Proxy" >&2; exit 1; }
grep -q 'Acquire::https::Proxy' "${build_script}" \
  || { echo "FAIL: build script lacks Acquire::https::Proxy" >&2; exit 1; }
grep -q 'APT_UPDATE_ARGS' "${build_script}" \
  || { echo "FAIL: build script does not set APT_UPDATE_ARGS" >&2; exit 1; }
grep -q 'APT_INSTALL_ARGS' "${build_script}" \
  || { echo "FAIL: build script does not set APT_INSTALL_ARGS" >&2; exit 1; }
grep -q -- '-o "Acquire::http::Proxy=${LUMI_SHADOW_APT_PROXY}"' "${build_script}" \
  || { echo "FAIL: initial apt-get lacks the exact http proxy arg" >&2; exit 1; }
grep -q -- '-o "Acquire::https::Proxy=${LUMI_SHADOW_APT_PROXY}"' "${build_script}" \
  || { echo "FAIL: initial apt-get lacks the exact https proxy arg" >&2; exit 1; }

# 9. Bounded retry behavior where meaningful: every network curl on this
#    path has finite retries, connect timeout, and max time.
for file in "${build_script}" "${install_rust}"; do
  grep -q -- '--retry 5' "${file}" \
    || { echo "FAIL: ${file} lacks bounded --retry" >&2; exit 1; }
  grep -q -- '--connect-timeout 20' "${file}" \
    || { echo "FAIL: ${file} lacks --connect-timeout" >&2; exit 1; }
  grep -q -- '--max-time 300' "${file}" \
    || { echo "FAIL: ${file} lacks --max-time" >&2; exit 1; }
done
grep -q '"--retry"' "${fetch_helper}" \
  || { echo "FAIL: fetch helper lacks --retry" >&2; exit 1; }
grep -q '"--connect-timeout"' "${fetch_helper}" \
  || { echo "FAIL: fetch helper lacks --connect-timeout" >&2; exit 1; }
grep -q '"--max-time"' "${fetch_helper}" \
  || { echo "FAIL: fetch helper lacks --max-time" >&2; exit 1; }

# 10. Bounded curl wrapper: bounded defaults, explicit real-curl resolution
#     without recursion, shipped in shadow-tools, and placed first in PATH
#     on the host (before preflight and the canonical setup-rusty-v8
#     composite action) and in the guest (before the canonical
#     install-musl-build-tools.sh invocation), so the bare curls in those
#     canonical files resolve through the wrapper in these contexts.
for token in '--retry 5' '--retry-delay 2' '--retry-all-errors' \
  '--connect-timeout 20' '--max-time 300' '/usr/bin/curl'; do
  grep -Fq -- "${token}" "${curl_wrapper}" \
    || { echo "FAIL: curl wrapper missing ${token}" >&2; exit 1; }
done
grep -Fq 'lumi_shadow_curl.sh' "${workflow}" \
  || { echo "FAIL: workflow does not ship the curl wrapper" >&2; exit 1; }
grep -Fq 'lumi_shadow_curl_wrapper=' "${build_script}" \
  || { echo "FAIL: guest build does not install the curl wrapper" >&2; exit 1; }

install_line="$(grep -n 'Install bounded curl wrapper first in PATH' "${workflow}" | head -n1 | cut -d: -f1)"
preflight_line="$(grep -n 'Preflight runner, proxy, and persistent Lima VM' "${workflow}" | head -n1 | cut -d: -f1)"
rusty_line="$(grep -n 'Configure rusty_v8 artifact overrides and verify checksums' "${workflow}" | head -n1 | cut -d: -f1)"
[[ -n "${install_line}" && -n "${preflight_line}" && -n "${rusty_line}" ]] \
  || { echo "FAIL: cannot locate host wrapper/preflight/rusty_v8 steps" >&2; exit 1; }
[[ "${install_line}" -lt "${preflight_line}" && "${preflight_line}" -lt "${rusty_line}" ]] \
  || { echo "FAIL: host curl wrapper is not first before preflight/setup-rusty-v8" >&2; exit 1; }
grep -Fq 'lumi-shadow-bin' "${workflow}" \
  || { echo "FAIL: host wrapper install does not use a RUNNER_TEMP bin dir" >&2; exit 1; }
grep -Fq 'GITHUB_PATH' "${workflow}" \
  || { echo "FAIL: host wrapper install does not prepend PATH" >&2; exit 1; }

guest_wrapper_line="$(grep -n 'lumi_shadow_curl_wrapper=' "${build_script}" | head -n1 | cut -d: -f1)"
installer_line="$(grep -n 'bash "${repo_root}/.github/scripts/install-musl-build-tools.sh"' "${build_script}" | head -n1 | cut -d: -f1)"
[[ -n "${guest_wrapper_line}" && -n "${installer_line}" && "${guest_wrapper_line}" -lt "${installer_line}" ]] \
  || { echo "FAIL: guest curl wrapper must precede install-musl-build-tools.sh" >&2; exit 1; }

# 11. The guest health probe runs before the wrapper is installed inside the
#     guest shell, so it carries explicit bounded retry flags; the host
#     probe resolves through the wrapper instead.
grep -q -- '--retry 5 --retry-delay 2 --retry-all-errors' "${workflow}" \
  || { echo "FAIL: guest health probe lacks explicit bounded retries" >&2; exit 1; }

echo "workflow static contract test OK"
