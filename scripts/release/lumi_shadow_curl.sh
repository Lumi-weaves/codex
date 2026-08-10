#!/usr/bin/env bash
# Workflow-owned bounded curl wrapper for the Lumi shadow ARM workflow. Owned
# exclusively by .github/workflows/lumi-release-shadow-worker.yml.
#
# Canonical code on this path calls bare `curl` and must not be modified:
# .github/actions/setup-rusty-v8 (host) and
# .github/scripts/install-musl-build-tools.sh (guest, libcap download). The
# guest proxy intermittently produced 502/TLS resets, so this wrapper is put
# first in PATH (host: before preflight/setup-rusty-v8; guest: before the
# canonical install-musl/build flow) and every bare `curl` resolves through
# it, inheriting finite retry/connect/max-time bounds.
#
# The wrapper never recurses: it resolves the real curl binary explicitly
# (default /usr/bin/curl, override LUMI_SHADOW_REAL_CURL for tests) and execs
# it with bounded defaults first, so explicit caller flags (for example a
# health-check --max-time) take precedence. It logs nothing: no URLs, no
# proxy values, no credentials.
set -euo pipefail

real_curl="${LUMI_SHADOW_REAL_CURL:-/usr/bin/curl}"
if [[ ! -x "${real_curl}" ]]; then
  echo "lumi_shadow_curl: real curl not found or not executable: ${real_curl}" >&2
  exit 1
fi

# Refuse to exec ourselves (PATH could resolve `curl` to this wrapper).
wrapper_real="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/$(basename "${BASH_SOURCE[0]}")"
curl_real="$(cd "$(dirname "${real_curl}")" && pwd -P)/$(basename "${real_curl}")"
if [[ "${wrapper_real}" == "${curl_real}" ]]; then
  echo "lumi_shadow_curl: refusing recursion; resolve a real curl binary" >&2
  exit 1
fi

exec "${real_curl}" \
  --retry 5 \
  --retry-delay 2 \
  --retry-all-errors \
  --connect-timeout 20 \
  --max-time 300 \
  "$@"
