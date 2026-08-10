#!/usr/bin/env bash
# Static test: the shadow workflow must use a deterministic per-run
# self-hosted label and must never contain a fixed reusable label. Owned
# exclusively by .github/workflows/lumi-release-shadow-worker.yml.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workflow="${here}/../../.github/workflows/lumi-release-shadow-worker.yml"
config="${here}/lumi_shadow_actionlint.yaml"

[[ -f "${workflow}" ]] || { echo "FAIL: workflow file missing" >&2; exit 1; }

# 1. The build job must run on the deterministic per-run label.
grep -q 'runs-on: lumi-shadow-arm64-\${{ github.run_id }}-\${{ github.run_attempt }}' "${workflow}" \
  || { echo "FAIL: per-run runs-on label missing" >&2; exit 1; }

# 2. No fixed reusable label: any runs-on value starting with the shadow
#    prefix must contain a ${{ expression }} (no static label remains).
if grep -E 'runs-on: lumi-shadow-arm64-[^$]+$' "${workflow}"; then
  echo "FAIL: fixed reusable self-hosted label found" >&2
  exit 1
fi

# 3. The actionlint config must not declare a fixed label.
if grep -q 'lumi-shadow-arm64-mac-mini' "${config}"; then
  echo "FAIL: actionlint config still declares a fixed label" >&2
  exit 1
fi

# 4. The workflow must not reference the legacy static label anywhere.
if grep -q 'lumi-shadow-arm64-mac-mini' "${workflow}"; then
  echo "FAIL: workflow still references the legacy static label" >&2
  exit 1
fi

echo "workflow static label test OK"
