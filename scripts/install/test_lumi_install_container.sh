#!/bin/sh
# Validate the Lumi Unix installer canary in a disposable Ubuntu container.

set -eu

cli=${LUMI_CONTAINER_CLI:-docker}
image=${LUMI_CONTAINER_IMAGE:-ubuntu:24.04}
name=${LUMI_CONTAINER_NAME:-lumi-installer-canary-$$}
repo=${LUMI_CONTAINER_REPO:-}
container_created=0

fail() {
    printf '%s\n' "lumi installer container validation: $*" >&2
    exit 1
}

cleanup() {
    if [ "$container_created" -eq 1 ]; then
        "$cli" rm -f "$name" >/dev/null 2>&1 || true
    fi
}

trap cleanup EXIT HUP INT TERM

command -v "$cli" >/dev/null 2>&1 || fail "container CLI not found: $cli"
[ -n "$repo" ] || repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
[ -d "$repo" ] || fail "repository directory is missing: $repo"
[ -r "$repo/scripts/install/lumi-install.sh" ] || fail "missing scripts/install/lumi-install.sh"
[ -r "$repo/scripts/install/test_lumi_install_sh.py" ] || fail "missing scripts/install/test_lumi_install_sh.py"
[ -r "$repo/scripts/install/test_install_sh.py" ] || fail "missing scripts/install/test_install_sh.py"

case "$name" in
    ''|*[!A-Za-z0-9_.-]*) fail "invalid container name: $name" ;;
esac

container_created=1
"$cli" create --name "$name" -v "$repo:/workspace:ro" "$image" sh -ceu '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -o Acquire::Retries=2
    apt-get install -y --no-install-recommends ca-certificates python3
    rm -rf /var/lib/apt/lists/*
    cd /workspace
    sh -n scripts/install/lumi-install.sh
    python3 -m unittest scripts/install/test_lumi_install_sh.py
    python3 scripts/install/test_install_sh.py
' >/dev/null || fail "failed to create container (image or mount unavailable)"

printf '==> starting disposable container %s (%s)\n' "$name" "$image"
"$cli" start -a "$name"
printf '%s\n' 'Lumi installer container validation passed.'
