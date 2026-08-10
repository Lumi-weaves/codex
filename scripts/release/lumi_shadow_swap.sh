#!/usr/bin/env bash
# Durable workflow-owned swapfile for the Lumi shadow ARM Linux guest. Owned
# exclusively by .github/workflows/lumi-release-shadow-worker.yml.
#
# Runs inside the persistent Lima VM lumi-codex-arm-builder (default user,
# passwordless sudo). The guest has 8 GiB RAM and no swap; the real aarch64
# musl smoke build OOMed at both CARGO_BUILD_JOBS=6 and =4 until an 8 GiB
# swapfile was activated, after which the build succeeded with
# CARGO_BUILD_JOBS=4. This helper provisions exactly one durable swapfile
# under an exact workflow-owned state root:
#
#   $HOME/.local/state/lumi-codex-arm-builder/swapfile   (8 GiB default)
#
# The swapfile persists across runs (durable) and activation is idempotent:
#   * an existing object must be a regular file owned by the build user, not
#     a symlink, mode 0600, with exactly the expected size; any mismatch
#     fails loudly (this helper never deletes, truncates, or overwrites);
#   * mkswap/swapon run only when the exact swapfile is not already active;
#   * after activation the exact path and size are re-verified in the swaps
#     table.
#
# Usage: lumi_shadow_ensure_swap
#
# Test/mock overrides (never set by the workflow):
#   LUMI_SHADOW_STATE_ROOT, LUMI_SHADOW_SWAPFILE, LUMI_SHADOW_SWAP_SIZE_MIB,
#   LUMI_SHADOW_SWAP_OWNER (uid), LUMI_SHADOW_SWAPS_FILE (default /proc/swaps),
#   LUMI_SHADOW_SUDO (unset: real default `sudo -n`; empty: disable sudo;
#   non-empty: exactly one command word, no eval/splitting),
#   LUMI_SHADOW_FALLOCATE, LUMI_SHADOW_DD, LUMI_SHADOW_MKSWAP,
#   LUMI_SHADOW_SWAPON
set -euo pipefail

lumi_shadow_ensure_swap() {
  local size_mib="${LUMI_SHADOW_SWAP_SIZE_MIB:-8192}"
  local state_root
  local swapfile
  local owner
  local swaps_file
  local expected_bytes
  local expected_kib
  local swap_kib
  local available_kib
  local fallocate
  local dd
  local mkswap
  local swapon
  local sudo_cmd=()

  state_root="${LUMI_SHADOW_STATE_ROOT:-${HOME}/.local/state/lumi-codex-arm-builder}"
  swapfile="${LUMI_SHADOW_SWAPFILE:-${state_root}/swapfile}"
  owner="${LUMI_SHADOW_SWAP_OWNER:-$(id -u)}"
  swaps_file="${LUMI_SHADOW_SWAPS_FILE:-/proc/swaps}"
  fallocate="${LUMI_SHADOW_FALLOCATE:-fallocate}"
  dd="${LUMI_SHADOW_DD:-dd}"
  mkswap="${LUMI_SHADOW_MKSWAP:-mkswap}"
  swapon="${LUMI_SHADOW_SWAPON:-swapon}"
  if [[ -n "${LUMI_SHADOW_SUDO+x}" ]]; then
    # Explicit test seam: an empty value disables sudo entirely; a non-empty
    # value is exactly one command word (no eval, no word splitting).
    if [[ -n "${LUMI_SHADOW_SUDO}" ]]; then
      sudo_cmd=("${LUMI_SHADOW_SUDO}")
    fi
  else
    # Real default: passwordless sudo, the workflow contract for the VM.
    sudo_cmd=(sudo -n)
  fi

  [[ "${size_mib}" =~ ^[0-9]+$ && "${size_mib}" -ge 1 ]] \
    || { echo "lumi_shadow_ensure_swap: invalid size: ${size_mib}" >&2; return 1; }
  expected_bytes=$((size_mib * 1024 * 1024))
  # swapon reports size in KiB; mkswap reserves header/bad-block pages, so a
  # small allowance (1 MiB) is tolerated when verifying the active swap line.
  expected_kib=$((size_mib * 1024))

  # Exact workflow-owned state root only; never touches anything outside it.
  mkdir -p "${state_root}"

  if [[ -e "${swapfile}" || -L "${swapfile}" ]]; then
    # Validate any existing object before touching it. Nothing is deleted,
    # truncated, or overwritten on mismatch: the run fails loudly instead.
    [[ -f "${swapfile}" && ! -L "${swapfile}" ]] \
      || { echo "lumi_shadow_ensure_swap: not a regular file (or symlink): ${swapfile}" >&2; return 1; }
    [[ "$(stat -c %u "${swapfile}")" == "${owner}" ]] \
      || { echo "lumi_shadow_ensure_swap: not owned by build user: ${swapfile}" >&2; return 1; }
    [[ "$(stat -c %a "${swapfile}")" == "600" ]] \
      || { echo "lumi_shadow_ensure_swap: mode is not 0600: ${swapfile}" >&2; return 1; }
    [[ "$(stat -c %s "${swapfile}")" == "${expected_bytes}" ]] \
      || { echo "lumi_shadow_ensure_swap: unexpected size: ${swapfile}" >&2; return 1; }
  else
    # Create exactly one swapfile at the exact path. fallocate preallocates
    # real blocks; dd is the portable fallback (bounded to size_mib).
    available_kib="$(df -Pk "${state_root}" | awk 'NR==2 {print $4}')"
    if [[ -n "${available_kib}" ]] \
      && [[ "${available_kib}" -lt $((expected_kib + 256 * 1024)) ]]; then
      echo "lumi_shadow_ensure_swap: insufficient space in ${state_root}" >&2
      return 1
    fi
    if command -v "${fallocate}" >/dev/null 2>&1; then
      "${fallocate}" -l "${expected_bytes}" "${swapfile}"
    else
      "${dd}" if=/dev/zero of="${swapfile}" bs=1M count="${size_mib}" status=none
    fi
    chmod 600 "${swapfile}"
  fi

  # Idempotent activation: nothing to do when the exact swapfile is already
  # active with the expected size.
  if [[ -r "${swaps_file}" ]] && grep -Fq "${swapfile}" "${swaps_file}"; then
    swap_kib="$(awk -v p="${swapfile}" '$1 == p {print $3}' "${swaps_file}" | head -n1)"
    if [[ -n "${swap_kib}" ]] \
      && [[ "${swap_kib}" -ge $((expected_kib - 1024)) ]]; then
      echo "swap active: ${swapfile} (${swap_kib} KiB)"
      return 0
    fi
  fi

  # Re-signing an owned, validated swapfile is safe and idempotent; swapon
  # then activates it. Both need root (passwordless sudo in the VM).
  "${sudo_cmd[@]}" "${mkswap}" "${swapfile}"
  "${sudo_cmd[@]}" "${swapon}" "${swapfile}"

  swap_kib="$(awk -v p="${swapfile}" '$1 == p {print $3}' "${swaps_file}" | head -n1)"
  [[ -n "${swap_kib}" && "${swap_kib}" -ge $((expected_kib - 1024)) ]] \
    || { echo "lumi_shadow_ensure_swap: swap activation not confirmed: ${swapfile}" >&2; return 1; }
  echo "swap active: ${swapfile} (${swap_kib} KiB)"
}
