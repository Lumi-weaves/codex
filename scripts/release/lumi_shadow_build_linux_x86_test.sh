#!/usr/bin/env bash
# Mock test for lumi_shadow_build_linux_x86.sh: the Omen preflight boundary
# (OS/arch, pinned tool versions, proxy absence, no Docker socket/host
# state, required packages present, bounded direct HTTPS probes) and the
# trusted no-op sudo shim argv contract, using fake tools. Owned
# exclusively by .github/workflows/lumi-release-shadow-worker.yml.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lumi_shadow_build_linux_x86.sh
source "${here}/lumi_shadow_build_linux_x86.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# The test environment may carry proxy vars; the Omen contract requires
# none, so clear them for the success path (and set them only to prove the
# rejection path).
unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY \
  all_proxy ALL_PROXY no_proxy NO_PROXY DOCKER_HOST || true

fakebin="${tmp}/bin"
mkdir -p "${fakebin}"

cat > "${fakebin}/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  -s) echo "${LUMI_SHADOW_FAKE_UNAME_S:-Linux}" ;;
  -m) echo "${LUMI_SHADOW_FAKE_UNAME_M:-x86_64}" ;;
esac
EOF
cat > "${fakebin}/rustc" <<'EOF'
#!/usr/bin/env bash
if [[ "${LUMI_SHADOW_FAKE_RUSTC_NO_STD:-}" == "1" ]]; then
  echo "error: target std not installed" >&2
  exit 1
fi
if [[ "${1:-}" == "-V" ]]; then
  echo "${LUMI_SHADOW_FAKE_RUSTC_V:-rustc 1.95.0 (mock)}"
  exit 0
fi
if [[ "${1:-}" == "--print" ]]; then
  echo "${LUMI_SHADOW_FAKE_RUSTC_LIBDIR:-/opt/rust/lib/rustlib/x86_64-unknown-linux-musl/lib}"
  exit 0
fi
exit 1
EOF
cat > "${fakebin}/cargo" <<'EOF'
#!/usr/bin/env bash
echo "${LUMI_SHADOW_FAKE_CARGO_V:-cargo 1.95.0 (mock)}"
EOF
cat > "${fakebin}/zig" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  version) echo "${LUMI_SHADOW_FAKE_ZIG_V:-0.14.0}" ;;
esac
EOF
cat > "${fakebin}/dpkg-query" <<'EOF'
#!/usr/bin/env bash
pkg="${@: -1}"
if [[ "${pkg}" == "${LUMI_SHADOW_FAKE_DPKG_MISSING:-__none__}" ]]; then
  echo "deinstall ok config-files"
else
  echo "install ok installed"
fi
EOF
cat > "${fakebin}/musl-gcc" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "${fakebin}/cmake" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "${fakebin}/ninja" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "${fakebin}/curl" <<'EOF'
#!/usr/bin/env bash
echo "$*" >> "${LUMI_SHADOW_FAKE_CURL_LOG:?}"
for arg in "$@"; do
  if [[ -n "${LUMI_SHADOW_FAKE_CURL_FAIL:-}" ]] \
    && [[ "${arg}" == https://* ]] \
    && [[ "${arg}" == *"${LUMI_SHADOW_FAKE_CURL_FAIL}"* ]]; then
    exit 22
  fi
done
exit 0
EOF
chmod +x "${fakebin}"/*

curl_log="${tmp}/curl.log"
export LUMI_SHADOW_UNAME="${fakebin}/uname"
export LUMI_SHADOW_RUSTC="${fakebin}/rustc"
export LUMI_SHADOW_CARGO="${fakebin}/cargo"
export LUMI_SHADOW_ZIG="${fakebin}/zig"
export LUMI_SHADOW_DPKG_QUERY="${fakebin}/dpkg-query"
export LUMI_SHADOW_DOCKER_SOCKET="${tmp}/docker.sock"
export LUMI_SHADOW_PROBE_URLS="https://probe.invalid/one https://probe.invalid/two"
export LUMI_SHADOW_REQUIRED_TOOLS="git python3 cmake ninja"
export LUMI_SHADOW_REQUIRED_PACKAGES="pkg-a pkg-b cmake ninja-build"
export LUMI_SHADOW_FAKE_CURL_LOG="${curl_log}"
rust_libdir="${tmp}/rust-libdir"
mkdir -p "${rust_libdir}"
export LUMI_SHADOW_FAKE_RUSTC_LIBDIR="${rust_libdir}"
export PATH="${fakebin}:${PATH}"

# --- Preflight passes under the exact Omen contract. ---------------------
lumi_shadow_x86_preflight >/dev/null 2>&1 || fail "clean preflight failed"
[[ -f "${curl_log}" ]] || fail "probe curl was never invoked"
grep -q -- "--max-time 30" "${curl_log}" || fail "probe curl is not bounded: $(cat "${curl_log}")"
grep -q "https://probe.invalid/one" "${curl_log}" || fail "probe url one missing"
grep -q "https://probe.invalid/two" "${curl_log}" || fail "probe url two missing"

# --- Proxy variables must be absent. -------------------------------------
if http_proxy="http://proxy.invalid:1" lumi_shadow_x86_preflight >/dev/null 2>&1; then
  fail "proxy env accepted"
fi

# --- Wrong OS/arch rejected. ----------------------------------------------
LUMI_SHADOW_FAKE_UNAME_S="Darwin" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "Darwin accepted"
LUMI_SHADOW_FAKE_UNAME_M="aarch64" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "aarch64 accepted"

# --- Pinned tool versions enforced. ---------------------------------------
LUMI_SHADOW_FAKE_RUSTC_V="rustc 1.94.0 (old)" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "rustc 1.94.0 accepted"
LUMI_SHADOW_FAKE_CARGO_V="cargo 1.94.0 (old)" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "cargo 1.94.0 accepted"
LUMI_SHADOW_FAKE_ZIG_V="0.13.0" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "zig 0.13.0 accepted"

# --- Rust target std must be resolvable for the musl target. --------------
LUMI_SHADOW_FAKE_RUSTC_LIBDIR="${tmp}/nonexistent-libdir" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 \
  && fail "missing rustc target libdir accepted"
LUMI_SHADOW_FAKE_RUSTC_NO_STD=1 \
  lumi_shadow_x86_preflight >/dev/null 2>&1 \
  && fail "rustc without musl target std accepted"

# --- No Docker socket / host state. ---------------------------------------
touch "${LUMI_SHADOW_DOCKER_SOCKET}"
lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "docker socket accepted"
rm -f "${LUMI_SHADOW_DOCKER_SOCKET}"
if DOCKER_HOST="unix:///var/run/docker.sock" lumi_shadow_x86_preflight >/dev/null 2>&1; then
  fail "DOCKER_HOST accepted"
fi

# --- Required packages must already be installed. -------------------------
LUMI_SHADOW_FAKE_DPKG_MISSING="pkg-b" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "missing package accepted"

# --- Required tools must be present (cmake/ninja included). ---------------
LUMI_SHADOW_REQUIRED_TOOLS="git python3 cmake ninja no-such-tool" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "missing tool accepted"

# --- Direct HTTPS probes must all succeed. --------------------------------
LUMI_SHADOW_FAKE_CURL_FAIL="probe.invalid/two" \
  lumi_shadow_x86_preflight >/dev/null 2>&1 && fail "unreachable probe accepted"

# --- Trusted no-op sudo shim: exact canonical argv only (fail closed). -----
shimdir="${tmp}/shimdir"
lumi_shadow_x86_write_sudo_shim "${shimdir}"
[[ -x "${shimdir}/sudo" ]] || fail "sudo shim not executable"

bash "${shimdir}/sudo" apt-get update || fail "bare apt-get update rejected"
bash "${shimdir}/sudo" apt-get install -y \
  ca-certificates curl musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils \
  || fail "canonical apt-get install argv rejected"

if bash "${shimdir}/sudo" apt-get update \
  -o "Acquire::http::Proxy=http://192.0.2.1:1" >/dev/null 2>&1; then
  fail "apt-get update with proxy -o accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y \
  -o "Acquire::http::Proxy=http://192.0.2.1:1" \
  ca-certificates curl musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils >/dev/null 2>&1; then
  fail "apt-get install with proxy -o accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y ca-certificates >/dev/null 2>&1; then
  fail "subset accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y \
  ca-certificates ca-certificates curl musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils >/dev/null 2>&1; then
  fail "duplicate package accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y \
  curl ca-certificates musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils >/dev/null 2>&1; then
  fail "reordered argv accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y \
  ca-certificates curl musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils \
  --no-install-recommends >/dev/null 2>&1; then
  fail "extra install flag accepted"
fi
if bash "${shimdir}/sudo" apt-get install -y \
  ca-certificates curl musl-tools pkg-config libcap-dev \
  g++ clang libc++-dev libc++abi-dev lld xz-utils python3 >/dev/null 2>&1; then
  fail "extra package accepted"
fi
bash "${shimdir}/sudo" apt-get upgrade >/dev/null 2>&1 \
  && fail "apt-get upgrade accepted"
bash "${shimdir}/sudo" rm -rf / >/dev/null 2>&1 \
  && fail "rm -rf accepted"
bash "${shimdir}/sudo" >/dev/null 2>&1 \
  && fail "empty argv accepted"
bash "${shimdir}/sudo" apt-get install ca-certificates >/dev/null 2>&1 \
  && fail "install without -y accepted"
bash "${shimdir}/sudo" apt-get install -y evil-package >/dev/null 2>&1 \
  && fail "unknown package accepted"
bash "${shimdir}/sudo" apt-get update --no-install-recommends >/dev/null 2>&1 \
  && fail "extra update flag accepted"
bash "${shimdir}/sudo" apt-get install -y -n ca-certificates >/dev/null 2>&1 \
  && fail "unknown install flag accepted"

# --- Static contract of the helper itself. --------------------------------
helper="${here}/lumi_shadow_build_linux_x86.sh"
for token in \
  'lumi_shadow_x86_preflight' \
  'lumi_shadow_x86_write_sudo_shim' \
  'target-libdir' \
  'cmake' \
  'ninja-build' \
  'install-musl-build-tools.sh' \
  'lumi_shadow_curl.sh' \
  'lumi_shadow_fetch_dotslash.py' \
  'lumi_shadow_validate_package.py' \
  'build-codex-package-archive.sh' \
  'rusty_v8_bazel.py' \
  'CODEX_BWRAP_SHA256' \
  'RUSTY_V8_ARCHIVE' \
  'CARGO_BUILD_JOBS=24' \
  'CARGO_HOME' \
  'CARGO_TARGET_DIR' \
  'sha256sum -c' \
  '--retry 5' \
  '--connect-timeout 20' \
  '--max-time 300' \
  '63c8477512eedd1fa625d8545139435d9773c2fae8f897123dcb643aa4dd7a76' \
  '119,303,356' \
  '35f9bb0540b9f7819a2ec6f88df516773973099d'; do
  grep -Fq -- "${token}" "${helper}" \
    || fail "helper missing ${token}"
done

# The x86 helper must not carry the ARM guest proxy, VM, swap, or rust
# installer contracts, and must not run apt itself.
for token in '192.168.5.2' 'limactl' 'lumi_shadow_ensure_swap' \
  'lumi_shadow_install_rust' 'sudo apt-get'; do
  grep -Fq -- "${token}" "${helper}" && fail "helper must not reference ${token}"
done

# Ordering: bounded curl wrapper before preflight; sudo shim before the
# canonical installer; preflight before any build.
wrapper_line="$(grep -n 'install -m 0755' "${helper}" | head -n1 | cut -d: -f1)"
preflight_call="$(grep -n '^[[:space:]]*lumi_shadow_x86_preflight$' "${helper}" | head -n1 | cut -d: -f1)"
shim_line="$(grep -n 'lumi_shadow_x86_write_sudo_shim "${shadow_bin}"' "${helper}" | head -n1 | cut -d: -f1)"
installer_line="$(grep -n 'install-musl-build-tools.sh' "${helper}" | tail -n1 | cut -d: -f1)"
[[ -n "${wrapper_line}" && -n "${preflight_call}" && -n "${shim_line}" && -n "${installer_line}" ]] \
  || fail "cannot locate helper ordering anchors"
[[ "${wrapper_line}" -lt "${preflight_call}" ]] \
  || fail "curl wrapper must precede preflight"
[[ "${preflight_call}" -lt "${shim_line}" ]] \
  || fail "preflight must precede the sudo shim"
[[ "${shim_line}" -lt "${installer_line}" ]] \
  || fail "sudo shim must precede install-musl-build-tools.sh"

echo "x86 build helper mock test OK"
