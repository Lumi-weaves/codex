#!/usr/bin/env bash
# Tests for lumi_shadow_env.sh: values with spaces round-trip; hostile
# payloads are stored literally and never executed; invalid lines are
# rejected. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lumi_shadow_env.sh
source "${here}/lumi_shadow_env.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

cat > "${tmp}/spaces.env" <<'EOF'
CFLAGS=-pthread -Wno-error=frame-larger-than
CMAKE_ARGS=-DCMAKE_HAVE_THREADS_LIBRARY=1 -DCMAKE_USE_PTHREADS_INIT=1 -DCMAKE_THREAD_LIBS_INIT=-pthread -DTHREADS_PREFER_PTHREAD_FLAG=ON
EMPTY=
EOF
lumi_shadow_load_env_file "${tmp}/spaces.env" || fail "load spaces.env"
[[ "${CFLAGS}" == "-pthread -Wno-error=frame-larger-than" ]] \
  || fail "CFLAGS not round-tripped: ${CFLAGS}"
[[ "${CMAKE_ARGS}" == "-DCMAKE_HAVE_THREADS_LIBRARY=1 -DCMAKE_USE_PTHREADS_INIT=1 -DCMAKE_THREAD_LIBS_INIT=-pthread -DTHREADS_PREFER_PTHREAD_FLAG=ON" ]] \
  || fail "CMAKE_ARGS not round-tripped"
[[ -z "${EMPTY:-}" ]] || fail "EMPTY should be empty"

cat > "${tmp}/hostile.env" <<'EOF'
EVIL=$(touch /tmp/lumi_shadow_env_pwned)
PWN=1; touch /tmp/lumi_shadow_env_pwned2
X=$(echo pwned)
Y=`touch /tmp/lumi_shadow_env_pwned3`
EOF
lumi_shadow_load_env_file "${tmp}/hostile.env" || fail "load hostile.env"
[[ "${EVIL}" == '$(touch /tmp/lumi_shadow_env_pwned)' ]] || fail "EVIL executed or mangled"
[[ "${PWN}" == '1; touch /tmp/lumi_shadow_env_pwned2' ]] || fail "PWN executed or mangled"
[[ "${X}" == '$(echo pwned)' ]] || fail "X executed or mangled"
[[ "${Y}" == '`touch /tmp/lumi_shadow_env_pwned3`' ]] || fail "Y executed or mangled"
for f in /tmp/lumi_shadow_env_pwned /tmp/lumi_shadow_env_pwned2 /tmp/lumi_shadow_env_pwned3; do
  [[ ! -e "${f}" ]] || fail "payload created ${f}"
done

cat > "${tmp}/bad1.env" <<'EOF'
BAD NAME=value
EOF
if lumi_shadow_load_env_file "${tmp}/bad1.env"; then fail "identifier with space accepted"; fi

cat > "${tmp}/bad2.env" <<'EOF'
1BAD=value
EOF
if lumi_shadow_load_env_file "${tmp}/bad2.env"; then fail "leading-digit identifier accepted"; fi

cat > "${tmp}/bad3.env" <<'EOF'
NOVALUE
EOF
if lumi_shadow_load_env_file "${tmp}/bad3.env"; then fail "line without '=' accepted"; fi

bash -c '[[ "${CFLAGS}" == *"frame-larger-than"* ]]' || fail "export not visible to child shell"

echo "env loader tests OK"
