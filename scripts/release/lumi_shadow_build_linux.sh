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
# The VM has 6 CPUs; keep parallel cargo jobs bounded.
export CARGO_BUILD_JOBS=6

[[ "$(uname -m)" == "aarch64" ]] || { echo "requires an aarch64 guest" >&2; exit 1; }
[[ -f "${repo_root}/.github/scripts/install-musl-build-tools.sh" ]] \
  || { echo "gated source missing at ${repo_root}" >&2; exit 1; }

# shellcheck source=lumi_shadow_env.sh
source "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_env.sh"
# shellcheck source=lumi_shadow_install_rust.sh
source "$(dirname "${BASH_SOURCE[0]}")/lumi_shadow_install_rust.sh"

echo "::group::Install toolchain"
sudo -n true || { echo "passwordless sudo required in the VM" >&2; exit 1; }
sudo apt-get update -y >/dev/null
sudo apt-get install -y --no-install-recommends python3 zstd >/dev/null

# Zig first (pinned + checksummed): install-musl-build-tools.sh probes
# `command -v zig` and only emits its Zig wrappers when Zig is on PATH.
zig_tarball="zig-linux-aarch64-${ZIG_VERSION}.tar.xz"
curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/${zig_tarball}" \
  -o "${work}/${zig_tarball}"
echo "${ZIG_SHA256}  ${work}/${zig_tarball}" | sha256sum -c -
tar -xJf "${work}/${zig_tarball}" -C "${work}"
export PATH="${work}/zig-linux-aarch64-${ZIG_VERSION}:${PATH}"

# Canonical musl build tools; its GITHUB_ENV output is parsed without
# eval/source (values contain spaces).
env_file="$(mktemp)"
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
  curl -fsSL "${base_url}/${f}" -o "${binding_dir}/${f}"
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

echo "::group::Download packaged zsh manifest"
curl -fsSL "${manifest_url}" -o "${runner_temp}/codex-zsh"
echo "::endgroup::"

echo "::group::Build Codex package archive"
cd "${repo_root}"
bash .github/scripts/build-codex-package-archive.sh \
  --target "${TARGET}" \
  --bundle primary \
  --entrypoint-dir "codex-rs/target/${TARGET}/release" \
  --archive-dir "${out}" \
  --zsh-manifest "${runner_temp}/codex-zsh"
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
