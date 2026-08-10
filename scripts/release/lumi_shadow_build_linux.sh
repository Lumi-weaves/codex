#!/usr/bin/env bash
# Build and validate the aarch64-unknown-linux-musl shadow package inside the
# persistent Lima VM lumi-codex-arm-builder. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# Runs inside the guest (default user, passwordless sudo). The exact gated
# commit is extracted by the host at <base>/src (passed as $1); this script,
# the safe env loader, and the validator travel from the workflow's own commit
# in <base>/tools. Canonical release steps are mirrored/reused:
# install-musl-build-tools.sh, the setup-rusty-v8 commands, and
# build-codex-package-archive.sh. The VM has no mounts; only the fixed proxy
# variables arrive via `limactl shell --preserve-env` (LIMA_SHELLENV_BLOCK="*"),
# and only the package archive + SHA256SUMS return via `limactl copy`.
#
# Hardening from the real smoke run: the guest has 8 GiB RAM and no swap, and
# the aarch64 musl canary OOMed at both jobs=6 and jobs=4 until the
# workflow-owned 8 GiB swapfile (lumi_shadow_swap.sh under
# $HOME/.local/state/lumi-codex-arm-builder) was active; the proven-safe
# parallelism is CARGO_BUILD_JOBS=4. apt gets the exact Acquire proxy args
# (no reliance on host apt config), rg/zsh are fetched by
# lumi_shadow_fetch_dotslash.py with bounded curl retries instead of urllib,
# and every remaining network curl uses bounded retry/connect/max-time with
# checksum verification.
#
# Usage: lumi_shadow_build_linux.sh <guest-base> [zsh-manifest-url]
set -euo pipefail

base="${1:?guest base directory required (src/work/out live under it)}"
manifest_url="${2:-https://github.com/openai/codex/releases/download/codex-zsh-v0.1.0/codex-zsh}"
TARGET="aarch64-unknown-linux-musl"
RUST_VERSION="${RUST_VERSION:-1.95.0}"
ZIG_VERSION="${ZIG_VERSION:-0.14.0}"
# sha256 of https://ziglang.org/download/0.14.0/zig-linux-aarch64-0.14.0.tar.xz
# (official ziglang.org download index).
ZIG_SHA256="ab64e3ea277f6fc5f3d723dcd95d9ce1ab282c8ed0f431b4de880d30df891e4f"

repo_root="${base}/src"
work="${base}/work"
out="${base}/out"
runner_temp="${work}/runner-temp"
mkdir -p "${work}" "${out}" "${runner_temp}"

export GITHUB_WORKSPACE="${repo_root}"
export RUNNER_TEMP="${runner_temp}"
export CARGO_NET_GIT_FETCH_WITH_CLI="true"
# The VM has 6 CPUs but 8 GiB RAM and no swap: jobs=6 and jobs=4 both OOMed
# before the 8 GiB swapfile was active; jobs=4 + active swap is the proven
# safe configuration from the real smoke build.
export CARGO_BUILD_JOBS=4

[[ "$(uname -m)" == "aarch64" ]] || { echo "requires an aarch64 guest" >&2; exit 1; }
[[ -f "${repo_root}/.github/scripts/install-musl-build-tools.sh" ]] \
  || { echo "gated source missing at ${repo_root}" >&2; exit 1; }

# Fixed, non-secret guest proxy endpoint (already surfaced by the workflow
# preflight; test override: LUMI_SHADOW_APT_PROXY).
LUMI_SHADOW_APT_PROXY="${LUMI_SHADOW_APT_PROXY:-http://192.168.5.2:7890}"

# shellcheck source=lumi_shadow_env.sh
source "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_env.sh"
# shellcheck source=lumi_shadow_install_rust.sh
source "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_install_rust.sh"
# shellcheck source=lumi_shadow_swap.sh
source "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_swap.sh"

# The guest proxy intermittently returns 502/TLS resets; every network curl
# on this path is bounded (finite retries, finite connect/max time) and every
# download is still verified by checksum before use.
curl_retry() {
  curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors \
    --connect-timeout 20 --max-time 300 "$@"
}

# Place the workflow-owned bounded curl wrapper first in PATH so canonical
# bare `curl` calls (install-musl-build-tools.sh libcap download and the
# package builder paths) inherit bounded retries without modifying the
# canonical scripts. The wrapper resolves the real curl itself and never
# logs URLs or proxy values.
lumi_shadow_curl_wrapper="$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_curl.sh"
[[ -f "${lumi_shadow_curl_wrapper}" ]] \
  || { echo "missing bounded curl wrapper: ${lumi_shadow_curl_wrapper}" >&2; exit 1; }
shadow_bin="${work}/shadow-bin"
mkdir -p "${shadow_bin}"
install -m 0755 "${lumi_shadow_curl_wrapper}" "${shadow_bin}/curl"
export PATH="${shadow_bin}:${PATH}"

# apt does not consume the passed proxy env vars, so the exact Acquire proxy
# options are given explicitly to every apt command.
apt_proxy_args=(
  -o "Acquire::http::Proxy=${LUMI_SHADOW_APT_PROXY}"
  -o "Acquire::https::Proxy=${LUMI_SHADOW_APT_PROXY}"
)

echo "::group::Durable guest swap"
sudo -n true || { echo "passwordless sudo required in the VM" >&2; exit 1; }
lumi_shadow_ensure_swap
echo "::endgroup::"

echo "::group::Install toolchain"
sudo apt-get update -y "${apt_proxy_args[@]}" >/dev/null
sudo apt-get install -y --no-install-recommends "${apt_proxy_args[@]}" \
  python3 zstd >/dev/null

# Zig first (pinned + checksummed): install-musl-build-tools.sh probes
# `command -v zig` and only emits its Zig wrappers when Zig is on PATH.
zig_tarball="zig-linux-aarch64-${ZIG_VERSION}.tar.xz"
curl_retry "https://ziglang.org/download/${ZIG_VERSION}/${zig_tarball}" \
  -o "${work}/${zig_tarball}"
echo "${ZIG_SHA256}  ${work}/${zig_tarball}" | sha256sum -c -
tar -xJf "${work}/${zig_tarball}" -C "${work}"
export PATH="${work}/zig-linux-aarch64-${ZIG_VERSION}:${PATH}"

# Canonical musl build tools; its GITHUB_ENV output is parsed without
# eval/source (values contain spaces).
env_file="$(mktemp)"
APT_UPDATE_ARGS="${apt_proxy_args[*]}" \
APT_INSTALL_ARGS="${apt_proxy_args[*]}" \
TARGET="${TARGET}" GITHUB_ENV="${env_file}" \
  bash "${repo_root}/.github/scripts/install-musl-build-tools.sh"
lumi_shadow_load_env_file "${env_file}"

# Rust: dist tarballs are installer trees, not PATH prefixes. After checksum
# verification and extraction, each verified bundled install.sh installs into
# an exact per-run prefix (no sudo, --disable-ldconfig); the musl target std
# goes into the same prefix. No rustup, no curl-piped scripts.
rust_prefix="${work}/rust-prefix"
lumi_shadow_install_rust "${work}" "${rust_prefix}" "${TARGET}"
export PATH="${rust_prefix}/bin:${PATH}"
export CARGO_HOME="${work}/cargo-home"
mkdir -p "${CARGO_HOME}"
: > "${CARGO_HOME}/config.toml"
rustc -V
cargo -V

# Prove the toolchain can compile, link, and run for the target before the
# Codex build (linker env from install-musl-build-tools.sh is in scope).
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
(cd "${work}/hello" && cargo build --target "${TARGET}" --release --quiet)
hello_out="$("${work}/hello/target/${TARGET}/release/shadow-hello")"
[[ "${hello_out}" == "shadow hello" ]]   || { echo "target hello run produced: ${hello_out}" >&2; exit 1; }
echo "Target toolchain proven: rustc/cargo + ${TARGET} hello ran"
echo "::endgroup::"

echo "::group::rusty_v8 artifacts"
# Mirror .github/actions/setup-rusty-v8 (composite actions cannot run inside
# the guest) with the identical commands.
version="$(python3 "${repo_root}/.github/scripts/rusty_v8_bazel.py" \
  resolved-v8-crate-version)"
release_tag="rusty-v8-v${version}"
base_url="https://github.com/openai/codex/releases/download/${release_tag}"
profile="ptrcomp_sandbox_release"
archive_name="librusty_v8_${profile}_${TARGET}.a.gz"
binding_name="src_binding_${profile}_${TARGET}.rs"
checksums_name="rusty_v8_${profile}_${TARGET}.sha256"
binding_dir="${runner_temp}/rusty_v8"
mkdir -p "${binding_dir}"
for f in "${archive_name}" "${binding_name}" "${checksums_name}"; do
  curl_retry "${base_url}/${f}" -o "${binding_dir}/${f}"
done
[[ "$(wc -l < "${binding_dir}/${checksums_name}")" -eq 2 ]] \
  || { echo "expected exactly two checksums for ${TARGET}" >&2; exit 1; }
(cd "${binding_dir}" && tr -d '\r' < "${checksums_name}" | sha256sum -c -)
export RUSTY_V8_ARCHIVE="${binding_dir}/${archive_name}"
export RUSTY_V8_SRC_BINDING_PATH="${binding_dir}/${binding_name}"
echo "::endgroup::"

# Canonical musl build environment (mirrors the lumi-release workflow).
export AWS_LC_SYS_NO_JITTER_ENTROPY=1
export AWS_LC_SYS_NO_JITTER_ENTROPY_AARCH64_UNKNOWN_LINUX_MUSL=1

cd "${repo_root}/codex-rs"

echo "::group::Build bwrap and export digest"
cargo build --target "${TARGET}" --release --timings --bin bwrap
bwrap_path="target/${TARGET}/release/bwrap"
[[ -f "${bwrap_path}" ]] || { echo "bwrap binary not found" >&2; exit 1; }
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
  [[ -f "${binary_path}" ]] || { echo "binary not found: ${binary_path}" >&2; exit 1; }
  strip --strip-debug --strip-unneeded "${binary_path}"
done
echo "::endgroup::"

echo "::group::Fetch rg and zsh through DotSlash manifests"
dotslash_env="${runner_temp}/dotslash.env"
python3 "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_fetch_dotslash.py" \
  --target "${TARGET}" \
  --tools-root "$(dirname "${BASH_SOURCE[0]}")" \
  --zsh-manifest-url "${manifest_url}" \
  --output-dir "${runner_temp}/lumi-shadow-dotslash" \
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
python3 "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_validate_package.py" \
  --archive "${out}/codex-package-${TARGET}.tar.gz" \
  --target "${TARGET}" \
  --expected-version "${version}" \
  --run
echo "::endgroup::"

echo "::group::Write transfer manifest"
(cd "${out}" && sha256sum "codex-package-${TARGET}.tar.gz" > SHA256SUMS)
cat "${out}/SHA256SUMS"
echo "::endgroup::"

echo "Shadow Linux build complete: ${out}/codex-package-${TARGET}.tar.gz"
