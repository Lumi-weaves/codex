#!/usr/bin/env bash
# Build and validate the x86_64-unknown-linux-musl shadow package in the
# future Omen one-shot JIT container. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# Runs directly on the Omen fresh isolated container (no VM hop): Linux
# x86_64, Rust 1.95.0 with the x86_64-unknown-linux-musl target std
# preinstalled, Zig 0.14.0 preinstalled, ordinary Docker bridge egress, no
# proxy environment, no sudo privilege, and no Docker socket or host state.
# The exact gated commit is checked out by the workflow at $1; this script,
# the safe env loader, the bounded curl wrapper, the DotSlash fetch helper,
# and the validator travel from the workflow's own commit in
# shadow-tools/scripts/release.
#
# Canonical release steps are mirrored/reused exactly as in the ARM shadow
# helper, but without any runtime apt: preflight independently proves the
# apt packages and tools install-musl-build-tools.sh would install (plus the
# smoke/image build tools cmake and ninja) are already present, and that the
# preinstalled rustc resolves the musl target std. A trusted no-op `sudo`
# shim (first in PATH) then fails closed: it accepts only the exact canonical
# apt update/install argv, in exact order, with no subsets, duplicates,
# proxy -o pairs, reordering, or extras (the x86 path sets no APT_* args and
# preflight forbids proxies). This intentionally fails if the gated
# canonical script's apt contract drifts. Zig is preinstalled
# (version-verified), so no Zig download occurs; libcap is still built and
# checksum-verified by the canonical script, rusty_v8 and rg/zsh are
# checksum-verified via the bounded curl wrapper and
# lumi_shadow_fetch_dotslash.py, bwrap's digest is exported for embedding,
# and the canonical package wrapper plus native validation run unchanged.
#
# Real x86_64 smoke evidence (documented here only; never a mutable artifact
# dependency): exact source 35f9bb0540b9f7819a2ec6f88df516773973099d
# produced a 119,303,356-byte codex-package-x86_64-unknown-linux-musl.tar.gz
# with SHA256 63c8477512eedd1fa625d8545139435d9773c2fae8f897123dcb643aa4dd7a76;
# the exact eight-field manifest, x86_64 static-pie/no PT_INTERP, native
# version/feature/code-mode-host runs, and the exported bwrap digest all
# passed validation. Total wall time was ~101m, dominated by first-run
# provisioning and network; the Cargo product build was ~11m at
# CARGO_BUILD_JOBS=24 on 48 GiB. The international bridge was slow and
# TLS-flaky, so every network curl is finite-bounded with retries and any
# failure fails the run (isolation is never weakened); Cargo/git direct
# bridge reliability remains a real end-to-end risk.
#
# Usage: lumi_shadow_build_linux_x86.sh <gated-repo-root> [zsh-manifest-url]
#
# Test/mock seams (never set by the workflow): LUMI_SHADOW_UNAME,
# LUMI_SHADOW_RUSTC, LUMI_SHADOW_CARGO, LUMI_SHADOW_ZIG,
# LUMI_SHADOW_DPKG_QUERY, LUMI_SHADOW_REQUIRED_TOOLS,
# LUMI_SHADOW_REQUIRED_PACKAGES, LUMI_SHADOW_DOCKER_SOCKET,
# LUMI_SHADOW_PROBE_URLS, LUMI_SHADOW_REAL_CURL (bounded curl wrapper).
set -euo pipefail

TARGET="x86_64-unknown-linux-musl"
RUST_VERSION="${RUST_VERSION:-1.95.0}"
ZIG_VERSION="${ZIG_VERSION:-0.14.0}"

lumi_shadow_x86_fail() {
  echo "lumi_shadow_build_linux_x86: $*" >&2
  return 1
}

# ---------------------------------------------------------------------------
# Preflight: the Omen container contract. Every check is mock-testable; the
# workflow never sets the LUMI_SHADOW_* overrides.
# ---------------------------------------------------------------------------
lumi_shadow_x86_preflight() {
  # Each failed check explicitly `return 1`s (bash suppresses errexit for
  # commands in &&/|| lists, so a bare `|| lumi_shadow_x86_fail` would not
  # abort); the outer `|| return 1` propagates the failure to callers that
  # run this function inside a test condition context.
  (
    set -euo pipefail
    local uname_bin="${LUMI_SHADOW_UNAME:-uname}"
    local rustc_bin="${LUMI_SHADOW_RUSTC:-rustc}"
    local cargo_bin="${LUMI_SHADOW_CARGO:-cargo}"
    local zig_bin="${LUMI_SHADOW_ZIG:-zig}"
    local dpkg_query="${LUMI_SHADOW_DPKG_QUERY:-dpkg-query}"
    local docker_socket="${LUMI_SHADOW_DOCKER_SOCKET:-/var/run/docker.sock}"
    local tool version url target_libdir
    local -a required_tools probe_urls required_packages

    # 1. OS/arch: Linux x86_64 only.
    [[ "$("${uname_bin}" -s)" == "Linux" ]] \
      || { lumi_shadow_x86_fail "container OS is not Linux: $("${uname_bin}" -s)"; return 1; }
    [[ "$("${uname_bin}" -m)" == "x86_64" ]] \
      || { lumi_shadow_x86_fail "container arch is not x86_64: $("${uname_bin}" -m)"; return 1; }

    # 2. No proxy environment: the Omen container uses the ordinary Docker
    #    bridge egress directly. Any proxy variable fails the run.
    for tool in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY \
      all_proxy ALL_PROXY no_proxy NO_PROXY; do
      [[ -z "${!tool:-}" ]] \
        || { lumi_shadow_x86_fail "proxy variable ${tool} is set; Omen requires no proxy env"; return 1; }
    done

    # 3. No Docker socket or host state: fresh isolated container only.
    [[ -z "${DOCKER_HOST:-}" ]] \
      || { lumi_shadow_x86_fail "DOCKER_HOST is set; Omen has no host state"; return 1; }
    [[ ! -e "${docker_socket}" && ! -L "${docker_socket}" ]] \
      || { lumi_shadow_x86_fail "Docker socket found at ${docker_socket}; Omen has no host socket"; return 1; }

    # 4. Tool versions: pinned Rust + Zig preinstalled in the image.
    version="$("${rustc_bin}" -V)"
    [[ "${version}" == "rustc ${RUST_VERSION}"* ]] \
      || { lumi_shadow_x86_fail "rustc is not ${RUST_VERSION}: ${version}"; return 1; }
    version="$("${cargo_bin}" -V)"
    [[ "${version}" == "cargo ${RUST_VERSION}"* ]] \
      || { lumi_shadow_x86_fail "cargo is not ${RUST_VERSION}: ${version}"; return 1; }
    version="$("${zig_bin}" version)"
    [[ "${version}" == "${ZIG_VERSION}" ]] \
      || { lumi_shadow_x86_fail "zig is not ${ZIG_VERSION}: ${version}"; return 1; }

    # 5. Rust target std: the preinstalled toolchain must resolve the musl
    #    target libdir (rustc errors when the target std is missing).
    target_libdir="$("${rustc_bin}" --print target-libdir --target "${TARGET}" 2>/dev/null)" \
      || { lumi_shadow_x86_fail "rustc cannot resolve target-libdir for ${TARGET}; musl target std not installed"; return 1; }
    [[ -n "${target_libdir}" && -d "${target_libdir}" ]] \
      || { lumi_shadow_x86_fail "rustc target libdir does not exist: ${target_libdir}"; return 1; }

    # 6. Required tools and apt packages are already present (no runtime
    #    apt; the no-op sudo shim is trusted only after this proof). cmake
    #    and ninja are the smoke/image build tools of the Omen contract;
    #    Ubuntu package names are cmake and ninja-build.
    read -r -a required_tools <<<"${LUMI_SHADOW_REQUIRED_TOOLS:-git python3 curl tar xz make ar ranlib strip sha256sum zstd pkg-config clang g++ lld dpkg-query cmake ninja}"
    for tool in "${required_tools[@]}"; do
      command -v "${tool}" >/dev/null 2>&1 \
        || { lumi_shadow_x86_fail "required tool missing: ${tool}"; return 1; }
    done
    # The canonical musl linker: musl-tools ships musl-gcc (and the
    # arch-prefixed x86_64-linux-musl-gcc variant on Ubuntu).
    if ! command -v musl-gcc >/dev/null 2>&1 \
      && ! command -v x86_64-linux-musl-gcc >/dev/null 2>&1; then
      lumi_shadow_x86_fail "required musl linker missing: musl-gcc"
      return 1
    fi
    read -r -a required_packages <<<"${LUMI_SHADOW_REQUIRED_PACKAGES:-ca-certificates curl musl-tools pkg-config libcap-dev g++ clang libc++-dev libc++abi-dev lld xz-utils python3 zstd cmake ninja-build}"
    for tool in "${required_packages[@]}"; do
      [[ "$("${dpkg_query}" -W -f='${Status}' "${tool}" 2>/dev/null)" \
        == *"install ok installed"* ]] \
        || { lumi_shadow_x86_fail "required package not installed: ${tool}"; return 1; }
    done

    # 7. Direct bounded HTTPS reachability to every endpoint this build
    #    uses (no proxy, no host network). Any transport error fails the
    #    run; any HTTP status still proves TLS+egress, so non-2xx is
    #    accepted. The workflow-owned bounded curl wrapper is first in PATH
    #    by the time preflight runs.
    read -r -a probe_urls <<<"${LUMI_SHADOW_PROBE_URLS:-https://github.com https://codeload.github.com https://api.github.com https://objects.githubusercontent.com https://index.crates.io https://static.crates.io}"
    for url in "${probe_urls[@]}"; do
      curl -sS -o /dev/null -w '%{http_code}\n' --max-time 30 "${url}" >/dev/null \
        || { lumi_shadow_x86_fail "direct HTTPS probe failed: ${url}"; return 1; }
    done
    echo "Preflight ok: Linux x86_64, rust ${RUST_VERSION}, zig ${ZIG_VERSION}, no proxy/socket, direct HTTPS reachable"
  ) || return 1
}

# ---------------------------------------------------------------------------
# Trusted no-op sudo shim: the Omen container has no sudo privilege, and
# install-musl-build-tools.sh must run unmodified. Preflight independently
# proved every package/tool it would install is already present. The x86
# path sets no APT_* args and preflight forbids proxies, so the shim fails
# closed: it accepts only exactly `apt-get update` (no extra args) and
# exactly the current canonical `apt-get install -y` package list in exact
# order, and rejects subsets, duplicates, reordering, proxy -o pairs, and
# extras. This intentionally fails if the gated canonical script's apt
# contract drifts. It never runs apt and never mutates anything.
# ---------------------------------------------------------------------------
lumi_shadow_x86_write_sudo_shim() {
  local dir="${1:?shim directory required}"
  local shim="${dir}/sudo"
  mkdir -p "${dir}"
  cat > "${shim}" <<'SHIM_EOF'
#!/usr/bin/env bash
# Trusted no-op sudo shim, owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml (written by
# lumi_shadow_build_linux_x86.sh). The Omen container has no sudo
# privilege; preflight independently proved the packages/tools that
# install-musl-build-tools.sh would install are already present, so its
# exact apt argv is a safe no-op. Fail closed: only the exact canonical
# argv is accepted, in exact order, with no subsets, duplicates,
# reordering, proxy -o pairs, or extras. This intentionally fails if the
# gated canonical script's apt contract drifts.
set -euo pipefail

expected_packages=(ca-certificates curl musl-tools pkg-config libcap-dev g++ clang libc++-dev libc++abi-dev lld xz-utils)

reject() {
  echo "lumi_shadow_sudo: refusing command: $*" >&2
  exit 1
}

[[ "${1:-}" == "apt-get" ]] || reject "$@"
op="${2:-}"
[[ -n "${op}" ]] || reject "$@"
shift 2

case "${op}" in
  update)
    # Exactly `apt-get update` with no extra args.
    [[ $# -eq 0 ]] || reject "apt-get update" "$@"
    ;;
  install)
    # Exactly `apt-get install -y <canonical packages in exact order>`.
    [[ $# -eq $((${#expected_packages[@]} + 1)) ]] \
      || reject "apt-get install" "$@"
    [[ "${1}" == "-y" ]] || reject "apt-get install" "$@"
    shift
    i=0
    for pkg in "$@"; do
      [[ "${pkg}" == "${expected_packages[${i}]}" ]] \
        || reject "apt-get install" "$@"
      i=$((i + 1))
    done
    ;;
  *)
    reject "$@"
    ;;
esac
exit 0
SHIM_EOF
  chmod 0755 "${shim}"
}

# ---------------------------------------------------------------------------
# Full build: preflight -> canonical musl/Zig env -> toolchain proof ->
# rusty_v8 -> bwrap -> codex binaries -> DotSlash -> package -> validate.
# ---------------------------------------------------------------------------
lumi_shadow_x86_main() {
  local repo_root="${1:?gated repository root required}"
  local manifest_url="${2:-https://github.com/openai/codex/releases/download/codex-zsh-v0.1.0/codex-zsh}"
  local here work out runner_temp shadow_bin env_file version
  local -a zsh_args

  [[ -d "${repo_root}/codex-rs" ]] \
    || lumi_shadow_x86_fail "gated source missing codex-rs at ${repo_root}"
  [[ -f "${repo_root}/.github/scripts/install-musl-build-tools.sh" ]] \
    || lumi_shadow_x86_fail "gated source missing install-musl-build-tools.sh"

  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # shellcheck source=lumi_shadow_env.sh
  source "${here}/lumi_shadow_env.sh"

  # Per-run dirs under the runner's per-run temp; no shared writable caches.
  work="${RUNNER_TEMP:?RUNNER_TEMP must be set}/lumi-shadow-x86"
  out="${work}/out"
  runner_temp="${work}/runner-temp"
  mkdir -p "${work}" "${out}" "${runner_temp}"
  export RUNNER_TEMP="${runner_temp}"
  export GITHUB_WORKSPACE="${repo_root}"
  export CARGO_NET_GIT_FETCH_WITH_CLI="true"
  # Real smoke evidence: CARGO_BUILD_JOBS=24 on 48 GiB built the product in
  # ~11m. Cargo state stays per-run under the workspace checkout.
  export CARGO_BUILD_JOBS=24
  export CARGO_HOME="${repo_root}/.cargo-home"
  export CARGO_TARGET_DIR="${repo_root}/codex-rs/target"
  mkdir -p "${CARGO_HOME}/bin"
  : > "${CARGO_HOME}/config.toml"

  # Bounded curl wrapper first in PATH: canonical bare `curl` calls
  # (install-musl-build-tools.sh libcap download, package builder paths)
  # inherit finite retry/connect/max-time behavior unmodified.
  shadow_bin="${work}/shadow-bin"
  mkdir -p "${shadow_bin}"
  install -m 0755 "${here}/lumi_shadow_curl.sh" "${shadow_bin}/curl"
  export PATH="${shadow_bin}:${PATH}"
  "${shadow_bin}/curl" --version >/dev/null

  echo "::group::Preflight (Omen container contract)"
  lumi_shadow_x86_preflight
  echo "::endgroup::"

  # No-op sudo shim first in PATH (after the curl wrapper): the canonical
  # script's apt calls (invoked through `sudo`) resolve to this shim and
  # no-op; anything else fails. The container never gains sudo.
  lumi_shadow_x86_write_sudo_shim "${shadow_bin}"

  echo "::group::Canonical musl/Zig build environment"
  env_file="$(mktemp)"
  TARGET="${TARGET}" GITHUB_ENV="${env_file}" \
    bash "${repo_root}/.github/scripts/install-musl-build-tools.sh"
  lumi_shadow_load_env_file "${env_file}"
  echo "::endgroup::"

  # Prove the preinstalled toolchain compiles, links, and runs for the musl
  # target before the Codex build (linker env from install-musl-build-tools
  # is in scope). The hello artifacts stay out of the product target dir.
  echo "::group::Target toolchain proof"
  mkdir -p "${work}/hello/src"
  cat > "${work}/hello/Cargo.toml" <<'HELLO_EOF'
[package]
name = "shadow-hello"
version = "0.1.0"
edition = "2021"
HELLO_EOF
  cat > "${work}/hello/src/main.rs" <<'HELLO_EOF'
fn main() {
    println!("shadow hello");
}
HELLO_EOF
  (cd "${work}/hello" \
    && CARGO_TARGET_DIR="${work}/hello-target" \
       cargo build --target "${TARGET}" --release --quiet)
  hello_out="$("${work}/hello-target/${TARGET}/release/shadow-hello")"
  [[ "${hello_out}" == "shadow hello" ]] \
    || lumi_shadow_x86_fail "target hello run produced: ${hello_out}"
  echo "Target toolchain proven: rustc/cargo + ${TARGET} hello ran"
  echo "::endgroup::"

  echo "::group::rusty_v8 artifacts"
  # Mirror .github/actions/setup-rusty-v8 (composite actions cannot take
  # overrides here) with the identical commands, bounded + checksum-verified.
  version="$(python3 "${repo_root}/.github/scripts/rusty_v8_bazel.py" \
    resolved-v8-crate-version)"
  release_tag="rusty-v8-v${version}"
  base_url="https://github.com/openai/codex/releases/download/${release_tag}"
  profile="ptrcomp_sandbox_release"
  archive_name="librusty_v8_${profile}_${TARGET}.a.gz"
  binding_name="src_binding_${profile}_${TARGET}.rs"
  checksums_name="rusty_v8_${profile}_${TARGET}.sha256"
  binding_dir="${RUNNER_TEMP}/rusty_v8"
  mkdir -p "${binding_dir}"
  for f in "${archive_name}" "${binding_name}" "${checksums_name}"; do
    curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors \
      --connect-timeout 20 --max-time 300 "${base_url}/${f}" -o "${binding_dir}/${f}"
  done
  [[ "$(wc -l < "${binding_dir}/${checksums_name}")" -eq 2 ]] \
    || lumi_shadow_x86_fail "expected exactly two checksums for ${TARGET}"
  (cd "${binding_dir}" && tr -d '\r' < "${checksums_name}" | sha256sum -c -)
  export RUSTY_V8_ARCHIVE="${binding_dir}/${archive_name}"
  export RUSTY_V8_SRC_BINDING_PATH="${binding_dir}/${binding_name}"
  echo "::endgroup::"

  # Canonical musl build environment (mirrors the lumi-release workflow).
  export AWS_LC_SYS_NO_JITTER_ENTROPY=1
  export AWS_LC_SYS_NO_JITTER_ENTROPY_X86_64_UNKNOWN_LINUX_MUSL=1

  cd "${repo_root}/codex-rs"

  echo "::group::Build bwrap and export digest"
  cargo build --target "${TARGET}" --release --timings --bin bwrap
  bwrap_path="target/${TARGET}/release/bwrap"
  [[ -f "${bwrap_path}" ]] || lumi_shadow_x86_fail "bwrap binary not found"
  strip --strip-debug --strip-unneeded "${bwrap_path}"
  digest="$(sha256sum "${bwrap_path}" | awk '{print $1}')"
  export CODEX_BWRAP_SHA256="${digest}"
  echo "Built bwrap sha256:${digest}"
  echo "::endgroup::"

  echo "::group::Cargo build (canary binaries)"
  cargo build --target "${TARGET}" --release --timings \
    --bin codex --bin codex-code-mode-host
  for binary in codex codex-code-mode-host; do
    binary_path="target/${TARGET}/release/${binary}"
    [[ -f "${binary_path}" ]] || lumi_shadow_x86_fail "binary not found: ${binary_path}"
    strip --strip-debug --strip-unneeded "${binary_path}"
  done
  echo "::endgroup::"

  echo "::group::Fetch rg and zsh through DotSlash manifests"
  dotslash_env="${RUNNER_TEMP}/dotslash.env"
  python3 "${here}/lumi_shadow_fetch_dotslash.py" \
    --target "${TARGET}" \
    --tools-root "$(cd "${here}/../.." && pwd)" \
    --zsh-manifest-url "${manifest_url}" \
    --output-dir "${RUNNER_TEMP}/lumi-shadow-dotslash" \
    --output-file "${dotslash_env}"
  lumi_shadow_load_env_file "${dotslash_env}"
  echo "::endgroup::"

  echo "::group::Build Codex package archive"
  cd "${repo_root}"
  zsh_args=()
  if [[ -n "${LUMI_SHADOW_ZSH_BIN:-}" ]]; then
    zsh_args+=(--zsh-bin "${LUMI_SHADOW_ZSH_BIN}")
  fi
  bash .github/scripts/build-codex-package-archive.sh \
    --target "${TARGET}" \
    --bundle primary \
    --entrypoint-dir "codex-rs/target/${TARGET}/release" \
    --archive-dir "${out}" \
    --rg-bin "${LUMI_SHADOW_RG_BIN}" \
    "${zsh_args[@]}"
  echo "::endgroup::"

  echo "::group::Validate canonical package"
  version="$(grep -m1 '^version' codex-rs/Cargo.toml \
    | sed -E 's/version *= *"([^"]+)".*/\1/')"
  python3 "${here}/lumi_shadow_validate_package.py" \
    --archive "${out}/codex-package-${TARGET}.tar.gz" \
    --target "${TARGET}" \
    --expected-version "${version}" \
    --run
  echo "::endgroup::"

  echo "::group::Shadow package checksum"
  (cd "${out}" && sha256sum "codex-package-${TARGET}.tar.gz")
  echo "::endgroup::"
  echo "Shadow x86_64 Linux build complete: ${out}/codex-package-${TARGET}.tar.gz"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  lumi_shadow_x86_main "$@"
fi
