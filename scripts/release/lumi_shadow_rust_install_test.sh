#!/usr/bin/env bash
# Mock test for lumi_shadow_install_rust.sh: proves the exact dist-tarball
# install shape (download+verify+extract, bundled install.sh into a per-run
# prefix with --disable-ldconfig, no sudo) using fake local tarballs and a
# file:// dist base. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lumi_shadow_install_rust.sh
source "${here}/lumi_shadow_install_rust.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

base="${tmp}/dist"
work="${tmp}/work"
prefix="${tmp}/rust-prefix"
mkdir -p "${base}" "${work}"

# Fake installer tree for the host toolchain: install.sh records its exact
# args and provisions bin/rustc + bin/cargo.
host_dir="rust-1.95.0-aarch64-unknown-linux-gnu"
mkdir -p "${base}/${host_dir}"
cat > "${base}/${host_dir}/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$@" > "${INSTALL_ARGS_FILE:?}"
prefix=""
for arg in "$@"; do
  case "${arg}" in
    --prefix=*) prefix="${arg#--prefix=}" ;;
  esac
done
[[ -n "${prefix}" ]] || { echo "missing --prefix" >&2; exit 1; }
mkdir -p "${prefix}/bin"
cat > "${prefix}/bin/rustc" <<'INNER'
#!/usr/bin/env bash
echo "rustc 1.95.0 (mock)"
INNER
cp "${prefix}/bin/rustc" "${prefix}/bin/cargo"
chmod +x "${prefix}/bin/rustc" "${prefix}/bin/cargo"
EOF
chmod +x "${base}/${host_dir}/install.sh"
(cd "${base}" && tar -cJf "${host_dir}.tar.xz" "${host_dir}")
(cd "${base}" && sha256sum "${host_dir}.tar.xz" > "${host_dir}.tar.xz.sha256")

# Fake installer tree for the musl target std.
std_dir="rust-std-1.95.0-aarch64-unknown-linux-musl"
mkdir -p "${base}/${std_dir}"
cat > "${base}/${std_dir}/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$@" > "${STD_ARGS_FILE:?}"
prefix=""
for arg in "$@"; do
  case "${arg}" in
    --prefix=*) prefix="${arg#--prefix=}" ;;
  esac
done
[[ -n "${prefix}" ]] || { echo "missing --prefix" >&2; exit 1; }
mkdir -p "${prefix}/lib/rustlib/aarch64-unknown-linux-musl"
touch "${prefix}/lib/rustlib/aarch64-unknown-linux-musl/INSTALLED"
EOF
chmod +x "${base}/${std_dir}/install.sh"
(cd "${base}" && tar -cJf "${std_dir}.tar.xz" "${std_dir}")
(cd "${base}" && sha256sum "${std_dir}.tar.xz" > "${std_dir}.tar.xz.sha256")

export RUST_DIST_BASE="file://${base}"
export INSTALL_ARGS_FILE="${tmp}/host-args.txt"
export STD_ARGS_FILE="${tmp}/std-args.txt"

lumi_shadow_install_rust "${work}" "${prefix}" aarch64-unknown-linux-musl \
  || fail "install function failed"

[[ -x "${prefix}/bin/rustc" && -x "${prefix}/bin/cargo" ]] \
  || fail "prefix/bin/rustc or cargo missing"
[[ -f "${prefix}/lib/rustlib/aarch64-unknown-linux-musl/INSTALLED" ]] \
  || fail "musl std not installed into the prefix"

[[ "$(cat "${INSTALL_ARGS_FILE}")" == "--prefix=${prefix} --disable-ldconfig" ]] \
  || fail "host install.sh args wrong: $(cat "${INSTALL_ARGS_FILE}")"
[[ "$(cat "${STD_ARGS_FILE}")" == "--prefix=${prefix} --disable-ldconfig" ]] \
  || fail "std install.sh args wrong: $(cat "${STD_ARGS_FILE}")"

echo "rust installer mock test OK"
