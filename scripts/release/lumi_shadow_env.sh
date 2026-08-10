#!/usr/bin/env bash
# Safe loader for GITHUB_ENV-style files. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
#
# The canonical musl build tools script appends NAME=value lines (values may
# contain spaces) to a GITHUB_ENV file. Loading it with `source` or `eval`
# would execute embedded shell; this loader parses each line at the first `=`,
# validates the name as a shell identifier, and assigns the literal value with
# `printf -v` (no eval, no source). Compatible with bash 3.2+.
set -euo pipefail

lumi_shadow_load_env_file() {
  local file="${1:?env file path required}"
  local line name value
  [[ -r "${file}" ]] || { echo "lumi_shadow_load_env_file: not readable: ${file}" >&2; return 1; }
  while IFS= read -r line || [[ -n "${line}" ]]; do
    [[ -n "${line}" && "${line}" != \#* ]] || continue
    [[ "${line}" == *=* ]] \
      || { echo "lumi_shadow_load_env_file: malformed line (no '='): ${line}" >&2; return 1; }
    name="${line%%=*}"
    value="${line#*=}"
    [[ "${name}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] \
      || { echo "lumi_shadow_load_env_file: invalid identifier: ${name}" >&2; return 1; }
    printf -v "${name}" '%s' "${value}"
    export "${name}"
  done < "${file}"
}
