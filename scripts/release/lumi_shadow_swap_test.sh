#!/usr/bin/env bash
# Mock test for lumi_shadow_swap.sh: fresh creation, idempotent reactivation,
# strict validation of any existing object (regular file, owner, 0600, exact
# size), and the never-delete contract, using fake fallocate/mkswap/swapon
# tools and a fake swaps table. Owned exclusively by
# .github/workflows/lumi-release-shadow-worker.yml.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lumi_shadow_swap.sh
source "${here}/lumi_shadow_swap.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

state="${tmp}/state"
swaps="${tmp}/swaps"
: > "${swaps}"

# Fake tools record their invocations; the fake swapon updates the swaps
# table like the real one (one line per path).
mkdir -p "${tmp}/bin"
cat > "${tmp}/bin/fallocate" <<'EOF'
#!/usr/bin/env bash
echo "fallocate $*" >> "${LUMI_SHADOW_LOG}"
truncate -s "$2" "$3"
EOF
cat > "${tmp}/bin/mkswap" <<'EOF'
#!/usr/bin/env bash
echo "mkswap $*" >> "${LUMI_SHADOW_LOG}"
touch "${LUMI_SHADOW_MKSWAP_RAN}"
EOF
cat > "${tmp}/bin/swapon" <<'EOF'
#!/usr/bin/env bash
echo "swapon $*" >> "${LUMI_SHADOW_LOG}"
path="$1"
size_kib=$((LUMI_SHADOW_SWAP_SIZE_MIB * 1024))
awk -v p="${path}" '$1 != p' "${LUMI_SHADOW_SWAPS_FILE}" \
  > "${LUMI_SHADOW_SWAPS_FILE}.new"
mv "${LUMI_SHADOW_SWAPS_FILE}.new" "${LUMI_SHADOW_SWAPS_FILE}"
printf '%s swap %s 0 -2\n' "${path}" "${size_kib}" >> "${LUMI_SHADOW_SWAPS_FILE}"
EOF
chmod +x "${tmp}/bin/fallocate" "${tmp}/bin/mkswap" "${tmp}/bin/swapon"

export LUMI_SHADOW_STATE_ROOT="${state}"
export LUMI_SHADOW_SWAPFILE="${state}/swapfile"
export LUMI_SHADOW_SWAP_SIZE_MIB=8
export LUMI_SHADOW_SWAPS_FILE="${swaps}"
export LUMI_SHADOW_SUDO=""          # fake tools need no sudo
export LUMI_SHADOW_FALLOCATE="${tmp}/bin/fallocate"
export LUMI_SHADOW_MKSWAP="${tmp}/bin/mkswap"
export LUMI_SHADOW_SWAPON="${tmp}/bin/swapon"
export LUMI_SHADOW_LOG="${tmp}/log"
export LUMI_SHADOW_MKSWAP_RAN="${tmp}/mkswap-ran"
: > "${LUMI_SHADOW_LOG}"

# --- Fresh creation: fallocate -> chmod 0600 -> mkswap -> swapon -> verify.
lumi_shadow_ensure_swap || fail "fresh swap creation"
[[ -f "${LUMI_SHADOW_SWAPFILE}" ]] || fail "swapfile not created"
[[ "$(stat -c %s "${LUMI_SHADOW_SWAPFILE}")" -eq $((8 * 1024 * 1024)) ]] \
  || fail "swapfile size wrong after creation"
[[ "$(stat -c %a "${LUMI_SHADOW_SWAPFILE}")" == "600" ]] \
  || fail "swapfile mode not 0600 after creation"
[[ -f "${LUMI_SHADOW_MKSWAP_RAN}" ]] || fail "mkswap not invoked"
grep -q "mkswap ${LUMI_SHADOW_SWAPFILE}" "${LUMI_SHADOW_LOG}" \
  || fail "mkswap invoked with wrong path"
grep -q "swapon ${LUMI_SHADOW_SWAPFILE}" "${LUMI_SHADOW_LOG}" \
  || fail "swapon invoked with wrong path"
grep -q "${LUMI_SHADOW_SWAPFILE} swap 8192" "${swaps}" \
  || fail "swaps table not updated"

# --- Idempotent re-run with the swapfile already active: no mkswap/swapon.
: > "${LUMI_SHADOW_LOG}"
rm -f "${LUMI_SHADOW_MKSWAP_RAN}"
lumi_shadow_ensure_swap || fail "active re-run failed"
[[ ! -f "${LUMI_SHADOW_MKSWAP_RAN}" ]] || fail "mkswap rerun while active"
[[ ! -s "${LUMI_SHADOW_LOG}" ]] || fail "tools invoked while swap active"
lumi_shadow_ensure_swap >/dev/null 2>&1 || fail "active re-run (again) failed"
[[ ! -s "${LUMI_SHADOW_LOG}" ]] || fail "tools invoked while swap active (2)"
[[ "$(grep -c "${LUMI_SHADOW_SWAPFILE}" "${swaps}")" -eq 1 ]] \
  || fail "swaps table should have exactly one line for the swapfile"

# --- Existing valid but inactive swapfile: reactivated without re-creating.
rm -f "${LUMI_SHADOW_MKSWAP_RAN}"
: > "${swaps}"
: > "${LUMI_SHADOW_LOG}"
lumi_shadow_ensure_swap || fail "reactivation of valid inactive swapfile"
[[ -f "${LUMI_SHADOW_MKSWAP_RAN}" ]] || fail "mkswap not invoked for inactive swapfile"
if grep -q "fallocate" "${LUMI_SHADOW_LOG}"; then
  fail "fallocate invoked for an existing valid swapfile"
fi

# --- Strict validation of any existing object; nothing is ever deleted.
for case in symlink mode size owner; do
  rm -f "${LUMI_SHADOW_SWAPFILE}"
  : > "${swaps}"
  case "${case}" in
    symlink)
      ln -s "${tmp}/target" "${LUMI_SHADOW_SWAPFILE}"
      ;;
    mode)
      : > "${LUMI_SHADOW_SWAPFILE}"
      chmod 0644 "${LUMI_SHADOW_SWAPFILE}"
      ;;
    size)
      : > "${LUMI_SHADOW_SWAPFILE}"
      chmod 0600 "${LUMI_SHADOW_SWAPFILE}"
      truncate -s 1024 "${LUMI_SHADOW_SWAPFILE}"
      ;;
    owner)
      : > "${LUMI_SHADOW_SWAPFILE}"
      chmod 0600 "${LUMI_SHADOW_SWAPFILE}"
      LUMI_SHADOW_SWAP_OWNER=12345 lumi_shadow_ensure_swap \
        && fail "wrong-owner swapfile accepted"
      ;;
  esac
  if [[ "${case}" != "owner" ]]; then
    lumi_shadow_ensure_swap && fail "${case} mismatch accepted"
  fi
  [[ -e "${LUMI_SHADOW_SWAPFILE}" || -L "${LUMI_SHADOW_SWAPFILE}" ]] \
    || fail "${case}: existing object was deleted"
done

# --- Real default uses `sudo -n` (the VM contract), proven hermetically
#     with a fake sudo on PATH that records its invocation and then runs
#     the fake mkswap/swapon tools.
unset LUMI_SHADOW_SUDO
rm -f "${LUMI_SHADOW_SWAPFILE}"
: > "${swaps}"
: > "${LUMI_SHADOW_LOG}"
rm -f "${LUMI_SHADOW_MKSWAP_RAN}"
cat > "${tmp}/bin/sudo" <<'EOF'
#!/usr/bin/env bash
echo "sudo $*" >> "${LUMI_SHADOW_LOG}"
# Bash `exec` treats a leading "-" as an option; the helper invokes us as
# `sudo -n <tool> <args>`, so strip the -n (already logged) before running.
shift
exec "$@"
EOF
chmod +x "${tmp}/bin/sudo"
PATH="${tmp}/bin:${PATH}" lumi_shadow_ensure_swap \
  || fail "default sudo -n swap failed"
grep -Eq '^sudo -n ' "${LUMI_SHADOW_LOG}" \
  || fail "sudo -n default not used: $(cat "${LUMI_SHADOW_LOG}")"
grep -Fq "sudo -n ${LUMI_SHADOW_MKSWAP} ${LUMI_SHADOW_SWAPFILE}" "${LUMI_SHADOW_LOG}" \
  || fail "mkswap not run via sudo -n: $(cat "${LUMI_SHADOW_LOG}")"
grep -Fq "sudo -n ${LUMI_SHADOW_SWAPON} ${LUMI_SHADOW_SWAPFILE}" "${LUMI_SHADOW_LOG}" \
  || fail "swapon not run via sudo -n: $(cat "${LUMI_SHADOW_LOG}")"
[[ -f "${LUMI_SHADOW_MKSWAP_RAN}" ]] || fail "mkswap did not run under sudo"

echo "swap helper mock test OK"
