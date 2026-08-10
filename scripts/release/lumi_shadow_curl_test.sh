#!/usr/bin/env bash
# Mock test for lumi_shadow_curl.sh. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# Proves:
#   * bounded defaults (retry/connect/max-time) come first, caller args pass
#     through;
#   * a bare `curl` invocation resolves through the wrapper when the wrapper
#     is first in PATH (the exact mechanism by which the canonical
#     setup-rusty-v8 composite action on the host and
#     install-musl-build-tools.sh in the guest inherit the bounds);
#   * the recursion guard refuses a real-curl path pointing at the wrapper;
#   * a missing real curl fails loudly instead of recursing.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="${here}/lumi_shadow_curl.sh"
[[ -f "${wrapper}" ]] || { echo "FAIL: wrapper missing" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

real_curl="${tmp}/real-curl"
cat > "${real_curl}" <<'EOF'
#!/usr/bin/env bash
echo "$@" > "${LUMI_SHADOW_CURL_LOG}"
EOF
chmod +x "${real_curl}"

export LUMI_SHADOW_REAL_CURL="${real_curl}"
export LUMI_SHADOW_CURL_LOG="${tmp}/curl-args.log"

# 1. Direct invocation: bounded defaults precede the caller's args, which
#    are passed through untouched (caller flags may override defaults).
bash "${wrapper}" -fsSL https://example.invalid/file -o "${tmp}/out"
args="$(cat "${LUMI_SHADOW_CURL_LOG}")"
for flag in "--retry 5" "--retry-delay 2" "--retry-all-errors" \
  "--connect-timeout 20" "--max-time 300"; do
  grep -Fq -- "${flag}" <<<"${args}" || fail "missing ${flag}: ${args}"
done
grep -Fq -- "-fsSL https://example.invalid/file -o ${tmp}/out" <<<"${args}" \
  || fail "caller args not passed through: ${args}"
[[ "${args}" == *"--max-time 300"*"-fsSL"* ]] \
  || fail "bounded defaults do not precede caller args: ${args}"

# 2. Bare `curl` with the wrapper first in PATH resolves through it (the
#    canonical action/installer context): the fake real curl receives the
#    bounded flags plus the bare caller's args.
bin="${tmp}/bin"
mkdir -p "${bin}"
cp "${wrapper}" "${bin}/curl"
chmod +x "${bin}/curl"
: > "${LUMI_SHADOW_CURL_LOG}"
PATH="${bin}:${PATH}" curl -fsSL https://example.invalid/file2 -o "${tmp}/out2"
args2="$(cat "${LUMI_SHADOW_CURL_LOG}")"
grep -Fq -- "--retry 5" <<<"${args2}" \
  || fail "bare curl did not resolve through the wrapper: ${args2}"
grep -Fq -- "-fsSL https://example.invalid/file2 -o ${tmp}/out2" <<<"${args2}" \
  || fail "bare curl args lost through the wrapper: ${args2}"

# 3. Recursion guard: pointing the real-curl resolution at the wrapper
#    itself must fail loudly without hanging.
LUMI_SHADOW_REAL_CURL="${wrapper}" bash "${wrapper}" https://example.invalid \
  && fail "recursion guard did not trigger"

# 4. Missing real curl fails loudly.
LUMI_SHADOW_REAL_CURL="${tmp}/nonexistent" bash "${wrapper}" https://example.invalid \
  && fail "missing real curl accepted"

echo "curl wrapper mock test OK"
