#!/usr/bin/env bash
# Install the pinned Rust toolchain and musl target std from official dist
# tarballs. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# The dist tarballs are installer trees, not ready PATH prefixes: after
# checksum verification and extraction, the verified bundled install.sh
# installs into an exact per-run prefix (no sudo, --disable-ldconfig). The
# host (gnu) toolchain provides rustc/cargo; rust-std for the musl target is
# installed into the same prefix. No rustup, no curl-piped scripts; official
# .sha256 files are verified with sha256sum -c.
#
# Env overrides (used by the mock test): RUST_VERSION, RUST_DIST_BASE.
set -euo pipefail

lumi_shadow_install_rust() {
  local work="${1:?work dir required}"
  local prefix="${2:?rust prefix required}"
  local target="${3:?rust target required}"
  local version="${RUST_VERSION:-1.95.0}"
  local base="${RUST_DIST_BASE:-https://static.rust-lang.org/dist}"
  # Host toolchain arch matches the aarch64 guest (gnu), as in the canonical
  # release flow (rustup default-toolchain + target).
  local host_tarball="rust-${version}-aarch64-unknown-linux-gnu.tar.xz"
  local std_tarball="rust-std-${version}-${target}.tar.xz"

  mkdir -p "${prefix}"

  fetch_verify_extract() {
    local tarball="${1:?tarball required}"
    curl -fsSL "${base}/${tarball}" -o "${work}/${tarball}"
    curl -fsSL "${base}/${tarball}.sha256" -o "${work}/${tarball}.sha256"
    (cd "${work}" && sha256sum -c "${tarball}.sha256")
    tar -xJf "${work}/${tarball}" -C "${work}"
  }

  fetch_verify_extract "${host_tarball}"
  bash "${work}/rust-${version}-aarch64-unknown-linux-gnu/install.sh" \
    --prefix="${prefix}" --disable-ldconfig

  fetch_verify_extract "${std_tarball}"
  bash "${work}/rust-std-${version}-${target}/install.sh" \
    --prefix="${prefix}" --disable-ldconfig
}
