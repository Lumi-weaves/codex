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
#     and downloads remain checksum-verified;
#   * the hosted gate packages the small workflow-commit helper surface once,
#     so self-hosted jobs never clone the large repository a second time;
#   * the JIT dispatcher (lumi_shadow_dispatch_jit.py) hardcodes the exact
#     workflow job names, per-run label formulas, and workflow path, so the
#     external controller can never route to a renamed job or label.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workflow="${here}/../../.github/workflows/lumi-release-shadow-worker.yml"
build_script="${here}/lumi_shadow_build_linux.sh"
x86_build_script="${here}/lumi_shadow_build_linux_x86.sh"
swap_script="${here}/lumi_shadow_swap.sh"
fetch_helper="${here}/lumi_shadow_fetch_dotslash.py"
install_rust="${here}/lumi_shadow_install_rust.sh"
curl_wrapper="${here}/lumi_shadow_curl.sh"
config="${here}/lumi_shadow_actionlint.yaml"
dispatcher="${here}/lumi_shadow_dispatch_jit.py"

for file in "${workflow}" "${build_script}" "${x86_build_script}" "${swap_script}" \
  "${fetch_helper}" "${install_rust}" "${curl_wrapper}" "${config}" "${dispatcher}"; do
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

# 12. The hosted gate stays the single authorization/source gate: exactly
#     one hosted job, main-only, and it must not depend on anything. It ships
#     the trusted workflow-commit helpers once as a short-retention artifact.
[[ "$(grep -c 'runs-on: ubuntu-24.04' "${workflow}")" -eq 1 ]] \
  || { echo "FAIL: expected exactly one hosted (ubuntu-24.04) job" >&2; exit 1; }
grep -q 'Require dispatch from current main' "${workflow}" \
  || { echo "FAIL: main-only dispatch check missing" >&2; exit 1; }
grep -q 'Resolve source to exact commit SHA' "${workflow}" \
  || { echo "FAIL: source gate step missing" >&2; exit 1; }
grep -q 'name: lumi-shadow-worker-tools-\${{ github.run_id }}-\${{ github.run_attempt }}' "${workflow}" \
  || { echo "FAIL: attempt-scoped helper artifact missing" >&2; exit 1; }
grep -q 'retention-days: 1' "${workflow}" \
  || { echo "FAIL: helper artifact must have one-day retention" >&2; exit 1; }
[[ "$(grep -c 'actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "${workflow}")" -eq 2 ]] \
  || { echo "FAIL: both self-hosted jobs must download the trusted helper artifact" >&2; exit 1; }
[[ "$(grep -c 'Checkout shadow worker helpers' "${workflow}")" -eq 0 ]] \
  || { echo "FAIL: redundant workflow-commit helper checkout remains" >&2; exit 1; }

# The dedicated Mac account's rustup shim directory must enter GITHUB_PATH
# before dtolnay/rust-toolchain; the live runner proved that finding rustup via
# Homebrew alone does not make the later `rustc` action substep resolvable.
rust_path_line="$(grep -n 'Expose rustup shims to composite actions' "${workflow}" | head -n1 | cut -d: -f1)"
rust_action_line="$(grep -n 'Install Rust toolchain' "${workflow}" | head -n1 | cut -d: -f1)"
[[ -n "${rust_path_line}" && -n "${rust_action_line}" && "${rust_path_line}" -lt "${rust_action_line}" ]] \
  || { echo "FAIL: rustup shim PATH step must precede toolchain action" >&2; exit 1; }
grep -Fq '${HOME}/.cargo/bin/rustup' "${workflow}" \
  || { echo "FAIL: rustup shim preflight missing" >&2; exit 1; }
grep -Fq 'echo "${HOME}/.cargo/bin" >> "${GITHUB_PATH}"' "${workflow}" \
  || { echo "FAIL: rustup shim directory is not exported through GITHUB_PATH" >&2; exit 1; }

# 13. x86 sibling job block: needs the gate, dynamic per-run label, finite
#     timeout, contents:read, one exact gated checkout and one hosted-gate
#     helper artifact download, bounded curl wrapper first, and one shadow-only
#     artifact upload.
x86_block="$(sed -n '/^  build-shadow-x86:/,/^  [a-z]/p' "${workflow}")"
[[ -n "${x86_block}" ]] || { echo "FAIL: build-shadow-x86 job missing" >&2; exit 1; }
grep -q 'needs: gate' <<<"${x86_block}" \
  || { echo "FAIL: x86 job does not need the gate" >&2; exit 1; }
grep -q 'runs-on: lumi-shadow-x86_64-\${{ github.run_id }}-\${{ github.run_attempt }}' <<<"${x86_block}" \
  || { echo "FAIL: x86 per-run label missing" >&2; exit 1; }
grep -q 'timeout-minutes:' <<<"${x86_block}" \
  || { echo "FAIL: x86 job has no finite timeout" >&2; exit 1; }
grep -q 'contents: read' <<<"${x86_block}" \
  || { echo "FAIL: x86 job lacks contents: read" >&2; exit 1; }
grep -q 'ref: \${{ needs.gate.outputs.sha }}' <<<"${x86_block}" \
  || { echo "FAIL: x86 job lacks the exact gated checkout" >&2; exit 1; }
grep -q 'persist-credentials: false' <<<"${x86_block}" \
  || { echo "FAIL: x86 checkout does not disable credential persistence" >&2; exit 1; }
[[ "$(grep -c 'actions/checkout@' <<<"${x86_block}")" -eq 1 ]] \
  || { echo "FAIL: x86 job must checkout the large repository exactly once" >&2; exit 1; }
grep -q 'actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' <<<"${x86_block}" \
  || { echo "FAIL: x86 job lacks the trusted helper artifact download" >&2; exit 1; }
grep -q 'shadow-tools' <<<"${x86_block}" \
  || { echo "FAIL: x86 job does not stage shadow-tools" >&2; exit 1; }
grep -q 'lumi_shadow_curl.sh' <<<"${x86_block}" \
  || { echo "FAIL: x86 job does not install the bounded curl wrapper" >&2; exit 1; }
grep -q 'lumi_shadow_build_linux_x86.sh' <<<"${x86_block}" \
  || { echo "FAIL: x86 job does not run the x86 build helper" >&2; exit 1; }
grep -q 'name: lumi-shadow-codex-package-x86_64-unknown-linux-musl' <<<"${x86_block}" \
  || { echo "FAIL: x86 shadow artifact name missing" >&2; exit 1; }
grep -q 'actions/upload-artifact' <<<"${x86_block}" \
  || { echo "FAIL: x86 job has no upload step" >&2; exit 1; }
[[ "$(grep -c 'name: lumi-shadow-codex-package-' <<<"${x86_block}")" -eq 1 ]] \
  || { echo "FAIL: x86 job must upload exactly one shadow artifact" >&2; exit 1; }

# 14. No fixed reusable x86 label: any lumi-shadow-x86_64 runs-on value
#     must contain a ${{ expression }} (no static label remains).
if grep -E 'runs-on: lumi-shadow-x86_64-[^$]+$' "${workflow}"; then
  echo "FAIL: fixed reusable x86 self-hosted label found" >&2
  exit 1
fi

# 15. The x86 job must not carry the ARM guest proxy, VM, swap, or rust
#     installer contracts, and the ARM job's proxy-present preflight must
#     remain intact (the inverse no-proxy boundary lives in the x86 helper).
if grep -q '192.168.5.2' <<<"${x86_block}"; then
  echo "FAIL: x86 job references the ARM guest proxy" >&2
  exit 1
fi
grep -q 'runner proxy env is unset' "${workflow}" \
  || { echo "FAIL: ARM proxy-present preflight changed" >&2; exit 1; }

# 16. No canonical artifact names, caches, signing, publication, or
#     elevated tokens anywhere in the shadow workflow.
if grep -q 'name: codex-package-' "${workflow}"; then
  echo "FAIL: canonical artifact name used by shadow workflow" >&2
  exit 1
fi
if grep -q 'actions/cache' "${workflow}"; then
  echo "FAIL: shadow workflow uses actions/cache" >&2
  exit 1
fi
if grep -q 'id-token' "${workflow}"; then
  echo "FAIL: shadow workflow requests id-token" >&2
  exit 1
fi
if grep -q 'contents: write' "${workflow}"; then
  echo "FAIL: shadow workflow requests contents: write" >&2
  exit 1
fi
if grep -Eq 'gh release|softprops|cosign|sigstore' "${workflow}"; then
  echo "FAIL: shadow workflow contains a publication/signing step" >&2
  exit 1
fi

# 17. x86 helper contract: no-proxy preflight boundary, no runtime apt with
#     a trusted no-op sudo shim, per-run cargo state, canonical reuse, and
#     bounded + checksum-verified network downloads.
for token in \
  'lumi_shadow_x86_preflight' \
  'proxy variable' \
  'Omen requires no proxy env' \
  'DOCKER_HOST' \
  'docker.sock' \
  'target-libdir' \
  'cmake' \
  'ninja-build' \
  'lumi_shadow_x86_write_sudo_shim' \
  'install-musl-build-tools.sh' \
  'lumi_shadow_curl.sh' \
  'lumi_shadow_fetch_dotslash.py' \
  'lumi_shadow_validate_package.py' \
  'build-codex-package-archive.sh' \
  'rusty_v8_bazel.py' \
  'CODEX_BWRAP_SHA256' \
  'CARGO_BUILD_JOBS=24' \
  'CARGO_HOME' \
  'CARGO_TARGET_DIR' \
  'sha256sum -c' \
  '--retry 5' \
  '--connect-timeout 20' \
  '--max-time 300' \
  'https://github.com' \
  'https://index.crates.io' \
  'https://static.crates.io'; do
  grep -Fq -- "${token}" "${x86_build_script}" \
    || { echo "FAIL: x86 helper missing ${token}" >&2; exit 1; }
done
for token in '192.168.5.2' 'limactl' 'lumi_shadow_ensure_swap' \
  'lumi_shadow_install_rust'; do
  grep -Fq -- "${token}" "${x86_build_script}" \
    && { echo "FAIL: x86 helper must not reference ${token}" >&2; exit 1; }
done
# The trusted no-op sudo shim fails closed on the exact canonical apt argv:
# the exact ordered package list must be encoded in the shim (the mock test
# proves acceptance of exactly that argv and rejection of subsets,
# duplicates, reordering, proxy -o pairs, and extras).
grep -Fq 'expected_packages=(ca-certificates curl musl-tools pkg-config libcap-dev g++ clang libc++-dev libc++abi-dev lld xz-utils)' \
  "${x86_build_script}" \
  || { echo "FAIL: x86 shim lacks the exact canonical package list" >&2; exit 1; }

# 18. Real x86 smoke evidence is encoded in comments/tests only (never a
#     mutable artifact dependency).
grep -q '63c8477512eedd1fa625d8545139435d9773c2fae8f897123dcb643aa4dd7a76' "${workflow}" \
  || { echo "FAIL: x86 smoke digest missing from workflow docs" >&2; exit 1; }
grep -q '35f9bb0540b9f7819a2ec6f88df516773973099d' "${x86_build_script}" \
  || { echo "FAIL: x86 smoke source missing from helper docs" >&2; exit 1; }

# 19. JIT dispatcher coupling: the external controller depends on the exact
#     gate/job names and per-run label formulas. Lock both the workflow and
#     the dispatcher constants so activation can never target a renamed job
#     or a changed label formula.
for job_name in \
  'Resolve source to exact commit' \
  'Build and validate shadow packages (aarch64)' \
  'Build and validate shadow package (x86_64)'; do
  # Only job-level name: lines count (step names like '... commit SHA'
  # contain the gate name as a substring and must not match).
  count="$(grep -E '^[[:space:]]*name:' "${workflow}" \
    | sed 's/^[[:space:]]*//' | grep -Fxc "name: ${job_name}")"
  [[ "${count}" -eq 1 ]] \
    || { echo "FAIL: job name '${job_name}' must appear exactly once" >&2; exit 1; }
  grep -Fq "${job_name}" "${dispatcher}" \
    || { echo "FAIL: dispatcher lacks job name '${job_name}'" >&2; exit 1; }
done
for formula in \
  'lumi-shadow-arm64-${{ github.run_id }}-${{ github.run_attempt }}' \
  'lumi-shadow-x86_64-${{ github.run_id }}-${{ github.run_attempt }}'; do
  [[ "$(grep -Fc "runs-on: ${formula}" "${workflow}")" -eq 1 ]] \
    || { echo "FAIL: runs-on formula '${formula}' must appear exactly once" >&2; exit 1; }
done
grep -Fq 'lumi-shadow-arm64-' "${dispatcher}" \
  || { echo "FAIL: dispatcher lacks the arm64 label prefix" >&2; exit 1; }
grep -Fq 'lumi-shadow-x86_64-' "${dispatcher}" \
  || { echo "FAIL: dispatcher lacks the x86_64 label prefix" >&2; exit 1; }
grep -Fq '.github/workflows/lumi-release-shadow-worker.yml' "${dispatcher}" \
  || { echo "FAIL: dispatcher lacks the exact workflow path" >&2; exit 1; }

echo "workflow static contract test OK"
