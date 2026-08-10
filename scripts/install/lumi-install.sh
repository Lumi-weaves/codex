#!/bin/sh

# Lumi Codex standalone installer/manager (Unix canary).
#
# Installs official Codex CLI package releases from the Lumi-weaves/codex
# GitHub Releases into an independent, immutable root under XDG_DATA_HOME
# (or ~/.local/share), alongside any package-managed official Codex install.
#
# The visible `lumi-codex` launcher (installed at
# ${LUMI_INSTALL_DIR:-$HOME/.local/bin}/lumi-codex) normally execs
# <root>/current/bin/codex and intercepts only `lumi-codex manage <action>`:
#   install (default), doctor, list, rollback, activate, deactivate, uninstall
#
# Safety model:
#   - fork-only downloads from Lumi-weaves/codex GitHub Releases
#   - canonical codex-package archives verified against codex-package_SHA256SUMS
#     and the GitHub release-metadata digests (the canary trust anchor: no
#     artifact signing yet, so the GitHub release digest is the anchor)
#   - the manager copy is the verified `lumi-install.sh` release asset
#     (works when bootstrapped via `curl ... | sh`; $0 is never copied)
#   - immutable release directories under <root>/releases
#   - strict schema/magic receipts prove ownership of every managed path
#   - lock + staging + archive-member + package-metadata + completeness
#     validation + atomic symlink switch (fails closed; no non-atomic fallback)
#   - a prepared-operation journal is written before switching `current`, so a
#     SIGKILL immediately after the switch cannot lose the old release as the
#     rollback target; valid journals are reconciled deterministically,
#     tampered journals fail closed
#   - default installs are side-by-side: no PATH changes and no `codex`
#     shadowing; `--activate`/`manage activate` opt in to the owned shim PATH
#     block, `manage deactivate` removes exactly that block
#   - uninstall touches only receipt-proven owned paths (including the visible
#     launcher) and never CODEX_HOME or official package-managed binaries

set -eu

SCHEMA_VERSION=1
RECEIPT_MAGIC="LUMI-CODEX-RECEIPT-V1"
JOURNAL_MAGIC="LUMI-CODEX-JOURNAL-V1"
PROFILE_BEGIN="# >>> Lumi Codex managed PATH >>>"
PROFILE_END="# <<< Lumi Codex managed PATH <<<"
RELEASES_API_BASE="https://api.github.com/repos/Lumi-weaves/codex"
RELEASES_DOWNLOAD_BASE="https://github.com/Lumi-weaves/codex/releases/download"
RELEASES_CONNECT_TIMEOUT=10
RELEASES_METADATA_TIMEOUT=30
RELEASES_ASSET_TIMEOUT=300
LOCK_STALE_AFTER_SECS=600
MANAGER_ASSET="lumi-install.sh"
TARGET_ALLOWLIST="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

RELEASE_RECEIPT_KEYS="schema root tag version target archive archive_sha256 bin_sha256 release_dir"
CURRENT_RECEIPT_KEYS="$RELEASE_RECEIPT_KEYS current previous activated profile manager launcher shim shim_dir install_dir releases_dir receipts_dir tmp_dir"
JOURNAL_KEYS="schema op current_old current_new previous_old previous_new"

root=""
releases_dir=""
receipts_dir=""
mode=""
action=""
release="${LUMI_RELEASE:-latest}"
target="${LUMI_TARGET:-}"
install_dir=""
launcher=""
activate_install=0
profile=""
path_line=""
tmp_dir=""
lock_kind=""
tag=""
version=""
package_asset=""
checksum_asset=""
release_metadata=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
}

die() {
  printf 'lumi-codex: %s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<EOF
Lumi Codex standalone installer/manager (Unix canary)

Usage:
  lumi-codex                           Run the installed Codex CLI (execs <root>/current/bin/codex)
  lumi-codex manage install [OPTIONS]  Install/update the latest release (default action)
  lumi-codex manage doctor             Verify confinement, packages, receipts, launcher and PATH
  lumi-codex manage list               List managed releases and activation state
  lumi-codex manage rollback           Switch back to the previous release
  lumi-codex manage activate           Add the owned Lumi Codex shim PATH block (shadows codex)
  lumi-codex manage deactivate         Remove exactly the owned Lumi Codex PATH block
  lumi-codex manage uninstall          Remove the managed install (never touches official Codex)

install options:
  --release VERSION   Release to install (default: latest; env: LUMI_RELEASE)
                      Accepts tags such as rust-v0.147.0-lumi.1
  --target TARGET     Package target (default: x86_64-unknown-linux-musl; env: LUMI_TARGET)
                      Linux canary allowlist: x86_64-unknown-linux-musl,
                      aarch64-unknown-linux-musl
  --activate          Opt in to the owned shim PATH block after install
                      (default is side-by-side: no PATH change, codex is not shadowed)

Environment:
  LUMI_ROOT           Override the managed root (default: \${XDG_DATA_HOME:-\$HOME/.local/share}/lumi-codex)
  LUMI_INSTALL_DIR    Where the visible lumi-codex launcher is installed
                      (default: \$HOME/.local/bin)
  LUMI_PROFILE        Override the shell profile to activate (default: \$HOME/.bashrc, .zshrc, or .profile)
  LUMI_DEV_MANAGER_SELF  Developer/test mode: use this local file as the manager
                      source instead of the verified lumi-install.sh release asset

The installer only downloads from Lumi-weaves/codex GitHub Releases and never
touches CODEX_HOME or package-managed official Codex binaries. The GitHub
release asset digest is the canary trust anchor (no artifact signing yet).
EOF
}

normalize_version() {
  case "$1" in
    "" | latest)
      printf 'latest\n'
      ;;
    rust-v*)
      printf '%s\n' "${1#rust-v}"
      ;;
    v*)
      printf '%s\n' "${1#v}"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

validate_version() {
  version="$1"

  if [ "$version" = "latest" ]; then
    return
  fi

  # Codex SemVer plus optional Lumi suffix, e.g. 0.147.0-lumi.1.
  if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-alpha(\.[0-9]+){0,2}|-beta(\.[0-9]+)?)?(-lumi\.[0-9]+)?$'; then
    echo "Invalid Codex release version: $version. Expected latest or x.y.z[-alpha[.N[.M]]|-beta[.N]][-lumi.N]." >&2
    return 1
  fi
}

safe_rel_name() {
  # Strict safe name for receipt/journal values that become path components:
  # no empty, no leading dot, no slash, only [A-Za-z0-9._-].
  case "$1" in
    "" | .* | */* | *[!A-Za-z0-9._-]*)
      return 1
      ;;
  esac
}

has_control_chars() {
  value="$1"
  if printf '%s' "$value" | grep -q '[[:cntrl:]]'; then
    return 0
  fi
  # grep is line-oriented and never sees the line terminator, so count
  # embedded newlines explicitly.
  if [ "$(printf '%s' "$value" | tr -cd '\n' | wc -c)" != 0 ]; then
    return 0
  fi
  return 1
}

validate_abs_path() {
  value="$1"
  label="$2"

  case "$value" in
    /*) ;;
    *) die "$label must be an absolute path (got: $value)" ;;
  esac
  if has_control_chars "$value"; then
    die "$label contains control characters; refusing."
  fi
}

validate_target() {
  case " $TARGET_ALLOWLIST " in
    *" $target "*)
      ;;
    *)
      die "Unsupported target: $target. Linux canary allowlist: $TARGET_ALLOWLIST (use --target or LUMI_TARGET)."
      ;;
  esac
  printf '%s' "$target" | grep -Eq '^[a-z0-9_]+(-[a-z0-9_]+)*$' ||
    die "Target contains unsafe characters: $target"
}

download_file() {
  url="$1"
  output="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout "$RELEASES_CONNECT_TIMEOUT" --max-time "$RELEASES_ASSET_TIMEOUT" "$url" -o "$output"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -q -t 1 -T "$RELEASES_ASSET_TIMEOUT" -O "$output" "$url"
    return
  fi

  echo "curl or wget is required to install Lumi Codex." >&2
  exit 1
}

download_text() {
  url="$1"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout "$RELEASES_CONNECT_TIMEOUT" --max-time "$RELEASES_METADATA_TIMEOUT" "$url"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -q -t 1 -T "$RELEASES_METADATA_TIMEOUT" -O - "$url"
    return
  fi

  echo "curl or wget is required to install Lumi Codex." >&2
  exit 1
}

parse_release_metadata() {
  # Bound awk's record size so compact, single-line JSON stays fast on every
  # supported awk implementation. JSON strings cannot contain literal newlines,
  # so the record boundaries inserted by fold do not change the document.
  LC_ALL=C fold -b -w 4096 | LC_ALL=C awk '
    function finish_string(value) {
      if (object_depth == 1 && key == "tag_name") {
        print "tag_name\t" value
      } else if (object_depth == asset_object_depth) {
        if (key == "name") {
          asset_name = value
        } else if (key == "digest") {
          asset_digest = value
        }
      }

      expecting_value = 0
      key = ""
    }

    {
      for (i = 1; i <= length($0); i++) {
        char = substr($0, i, 1)

        if (in_string) {
          if (escaped) {
            token = token "\\" char
            escaped = 0
          } else if (char == "\\") {
            escaped = 1
          } else if (char == "\"") {
            in_string = 0
            if (string_is_value) {
              finish_string(token)
            } else {
              pending_key = token
            }
          } else {
            token = token char
          }
          continue
        }

        if (char == "\"") {
          in_string = 1
          token = ""
          escaped = 0
          string_is_value = expecting_value
        } else if (char == ":" && pending_key != "") {
          key = pending_key
          pending_key = ""
          expecting_value = 1
        } else if (char == "{") {
          object_depth++
          if (assets_array_depth != 0 &&
              array_depth == assets_array_depth &&
              asset_object_depth == 0) {
            asset_object_depth = object_depth
            asset_name = ""
            asset_digest = ""
          }
          expecting_value = 0
          key = ""
        } else if (char == "}") {
          if (object_depth == asset_object_depth) {
            if (asset_name != "" && asset_digest != "") {
              print "asset\t" asset_name "\t" asset_digest
            }
            asset_object_depth = 0
            asset_name = ""
            asset_digest = ""
          }
          object_depth--
          expecting_value = 0
          key = ""
          pending_key = ""
        } else if (char == "[") {
          array_depth++
          if (expecting_value && key == "assets" && object_depth == 1) {
            assets_array_depth = array_depth
          }
          expecting_value = 0
          key = ""
        } else if (char == "]") {
          if (array_depth == assets_array_depth) {
            assets_array_depth = 0
          }
          array_depth--
          expecting_value = 0
          key = ""
          pending_key = ""
        } else if (char == ",") {
          expecting_value = 0
          key = ""
          pending_key = ""
        }
      }
    }

    END {
      if (in_string || object_depth != 0 || array_depth != 0) {
        exit 1
      }
    }
  '
}

release_url_for_asset() {
  asset="$1"
  printf '%s/rust-v%s/%s\n' "$RELEASES_DOWNLOAD_BASE" "$version" "$asset"
}

release_asset_digest_or_empty() {
  asset="$1"

  digest="$(printf '%s\n' "$release_metadata" | awk -F '\t' -v asset="$asset" '
    $1 == "asset" && $2 == asset {
      print $3
      exit
    }
  ')"

  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????)
      digest="${digest#sha256:}"
      case "$digest" in
        *[!0-9a-fA-F]*) return 1 ;;
      esac
      printf '%s\n' "$digest"
      ;;
    *)
      return 1
      ;;
  esac
}

release_asset_exists() {
  asset="$1"

  release_asset_digest_or_empty "$asset" >/dev/null 2>&1
}

release_asset_digest() {
  asset="$1"

  digest="$(release_asset_digest_or_empty "$asset" || true)"
  if [ -z "$digest" ]; then
    echo "Could not find SHA-256 digest for release asset $asset." >&2
    exit 1
  fi

  printf '%s\n' "$digest"
}

package_archive_digest() {
  asset="$1"
  manifest_path="$2"

  digest="$(awk -v asset="$asset" '
    $2 == asset && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ {
      print tolower($1)
      found = 1
      exit
    }
    END {
      if (!found) {
        exit 1
      }
    }
  ' "$manifest_path" 2>/dev/null || true)"

  if [ -z "$digest" ]; then
    echo "Could not find SHA-256 digest for $asset in codex-package_SHA256SUMS." >&2
    return 1
  fi

  printf '%s\n' "$digest"
}

file_sha256() {
  path="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
    return
  fi

  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$path" | sed 's/^.*= //'
    return
  fi

  echo "sha256sum, shasum, or openssl is required to verify the Lumi Codex download." >&2
  exit 1
}

verify_archive_digest() {
  archive_path="$1"
  expected_digest="$2"
  actual_digest="$(file_sha256 "$archive_path")"

  if [ "$actual_digest" != "$expected_digest" ]; then
    echo "Downloaded archive checksum did not match expected digest." >&2
    echo "expected: $expected_digest" >&2
    echo "actual:   $actual_digest" >&2
    return 1
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required by Lumi Codex." >&2
    exit 1
  fi
}

pick_profile() {
  if [ -n "${LUMI_PROFILE:-}" ]; then
    printf '%s\n' "$LUMI_PROFILE"
    return
  fi

  case "${SHELL:-}" in
    */zsh)
      printf '%s\n' "$HOME/.zshrc"
      ;;
    */bash)
      printf '%s\n' "$HOME/.bashrc"
      ;;
    *)
      printf '%s\n' "$HOME/.profile"
      ;;
  esac
}

resolve_root() {
  if [ -n "${LUMI_ROOT:-}" ]; then
    root="$LUMI_ROOT"
  else
    root="${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex"
  fi

  validate_abs_path "$root" "LUMI_ROOT"

  releases_dir="$root/releases"
  receipts_dir="$root/receipts"
}

resolve_install_dir() {
  if [ -n "${LUMI_INSTALL_DIR:-}" ]; then
    install_dir="$LUMI_INSTALL_DIR"
  else
    install_dir="$HOME/.local/bin"
  fi

  validate_abs_path "$install_dir" "LUMI_INSTALL_DIR"
  launcher="$install_dir/lumi-codex"
}

validate_root_for_mutation() {
  if [ -L "$root" ]; then
    die "Refusing to operate on symlinked root $root (remove the symlink or point LUMI_ROOT at a real directory)."
  fi
}

validate_install_dir_for_mutation() {
  if [ -L "$install_dir" ]; then
    die "Refusing to install the launcher through symlinked directory $install_dir."
  fi
}

mkdir_lock_is_stale() {
  [ -d "$root/install.lock.d" ] || return 1

  pid="$(cat "$root/install.lock.d/pid" 2>/dev/null || true)"
  started_at="$(cat "$root/install.lock.d/started_at" 2>/dev/null || true)"
  now="$(date +%s 2>/dev/null || printf '0')"

  case "$started_at" in
    '' | *[!0-9]*)
      started_at=0
      ;;
  esac

  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    return 1
  fi

  if [ "$started_at" -eq 0 ] || [ "$now" -eq 0 ]; then
    return 0
  fi

  [ $((now - started_at)) -ge "$LOCK_STALE_AFTER_SECS" ]
}

acquire_install_lock() {
  mkdir -p "$root"

  if command -v flock >/dev/null 2>&1; then
    exec 9>"$root/install.lock"
    flock 9
    lock_kind="flock"
    return
  fi

  while ! mkdir "$root/install.lock.d" 2>/dev/null; do
    if mkdir_lock_is_stale; then
      warn "Removing stale Lumi Codex lock at $root/install.lock.d"
      rm -rf "$root/install.lock.d"
      continue
    fi
    sleep 1
  done

  printf '%s\n' "$$" >"$root/install.lock.d/pid"
  date +%s >"$root/install.lock.d/started_at" 2>/dev/null || true
  lock_kind="mkdir"
}

release_install_lock() {
  if [ "$lock_kind" = "mkdir" ]; then
    rm -rf "$root/install.lock.d" 2>/dev/null || true
  elif [ "$lock_kind" = "flock" ]; then
    exec 9>&- 2>/dev/null || true
  fi
  lock_kind=""
}

cleanup_stale_artifacts() {
  if [ -d "$root/tmp" ]; then
    find "$root/tmp" -mindepth 1 -maxdepth 1 \
      \( -name '.staging.*' -o -name '.current.*' -o -name '.previous.*' \) \
      -exec rm -rf {} + 2>/dev/null || true
  fi
}

switch_link() {
  name="$1"
  link_target="$2"
  mkdir -p "$root/tmp"
  tmp_link="$root/tmp/.$name.$$"

  rm -f "$tmp_link"
  ln -s "$link_target" "$tmp_link"

  if mv -Tf "$tmp_link" "$root/$name" 2>/dev/null; then
    return
  fi

  if mv -hf "$tmp_link" "$root/$name" 2>/dev/null; then
    return
  fi

  rm -f "$tmp_link"
  die "Failed to atomically switch $name; aborting without a non-atomic fallback."
}

ensure_owned_symlink() {
  path="$1"
  link_target="$2"

  if [ -L "$path" ]; then
    current_target="$(readlink "$path" 2>/dev/null || true)"
    if [ "$current_target" = "$link_target" ]; then
      return 0
    fi
    die "Refusing to overwrite unknown symlink $path -> $current_target"
  fi

  if [ -e "$path" ]; then
    die "Refusing to overwrite non-symlink path $path"
  fi

  ln -s "$link_target" "$path"
}

profile_block_state() {
  if [ ! -f "$profile" ]; then
    printf 'none\n'
    return
  fi

  awk -v begin="$PROFILE_BEGIN" -v end="$PROFILE_END" -v line="$path_line" '
    { lines[NR] = $0 }
    END {
      n = 0
      for (i = 1; i <= NR; i++) {
        if (lines[i] == begin && lines[i+1] == line && lines[i+2] == end) {
          n++
        }
      }
      state = "none"
      if (n == 1) {
        state = "exact"
      } else if (n > 1) {
        state = "drift"
      } else {
        for (i = 1; i <= NR; i++) {
          if (lines[i] == begin || lines[i] == end) {
            state = "drift"
            break
          }
        }
      }
      print state
    }
  ' "$profile"
}

assert_profile_not_symlink() {
  if [ -L "$profile" ]; then
    die "Refusing to modify symlinked profile $profile (replace the symlink with a regular file or set LUMI_PROFILE)."
  fi
}

profile_activate() {
  assert_profile_not_symlink
  case "$(profile_block_state)" in
    exact)
      step "Lumi Codex PATH already configured in $profile"
      return 0
      ;;
    drift)
      die "Profile $profile contains a Lumi Codex block that does not match the owned block; refusing to modify it."
      ;;
    none)
      ;;
  esac

  tmp_profile="$profile.lumi-codex.$$"
  rm -f "$tmp_profile"
  if [ -f "$profile" ]; then
    cp -p "$profile" "$tmp_profile"
  else
    : >"$tmp_profile"
  fi
  {
    printf '\n%s\n' "$PROFILE_BEGIN"
    printf '%s\n' "$path_line"
    printf '%s\n' "$PROFILE_END"
  } >>"$tmp_profile"
  mv -f "$tmp_profile" "$profile"
  step "Added Lumi Codex PATH block to $profile"
}

profile_deactivate() {
  assert_profile_not_symlink
  case "$(profile_block_state)" in
    none)
      step "Lumi Codex PATH is not configured (nothing to remove)."
      return 0
      ;;
    drift)
      die "Profile $profile contains a drifted Lumi Codex block; refusing to remove it."
      ;;
    exact)
      ;;
  esac

  tmp_profile="$profile.lumi-codex.$$"
  rm -f "$tmp_profile"
  awk -v begin="$PROFILE_BEGIN" -v end="$PROFILE_END" -v line="$path_line" '
    BEGIN { removed = 0 }
    { lines[NR] = $0 }
    END {
      for (i = 1; i <= NR; i++) {
        if (!removed && lines[i] == begin && lines[i+1] == line && lines[i+2] == end) {
          removed = 1
          i += 2
          continue
        }
        print lines[i]
      }
      if (!removed) exit 1
    }
  ' "$profile" >"$tmp_profile"
  mv -f "$tmp_profile" "$profile"
  step "Removed Lumi Codex PATH block from $profile"
}

validate_receipt() {
  path="$1"
  required="$2"
  allowed="$3"

  [ -f "$path" ] || return 1
  [ "$(sed -n '1p' "$path")" = "$RECEIPT_MAGIC" ] || return 1

  awk -v required="$required" -v allowed="$allowed" '
    NR == 1 { next }
    {
      line = $0
      eq = index(line, "=")
      if (eq <= 1) { bad = 1; next }
      key = substr(line, 1, eq - 1)
      value = substr(line, eq + 1)
      if (value == "") { bad = 1 }
      if (seen[key]++) { bad = 1 }
      if (!((" " allowed " ") ~ (" " key " "))) { bad = 1 }
      keys = keys " " key
    }
    END {
      if (bad) exit 1
      n = split(required, req, " ")
      for (i = 1; i <= n; i++) {
        if (!((" " keys " ") ~ (" " req[i] " "))) exit 1
      }
      exit 0
    }
  ' "$path"
}

receipt_key() {
  path="$1"
  key="$2"
  awk -v key="$key" -F= '
    NR > 1 && $1 == key {
      print substr($0, index($0, "=") + 1)
      exit
    }
  ' "$path" 2>/dev/null || true
}

write_release_receipt() {
  rel="$1"
  bin_digest="$2"
  archive_digest="$3"

  safe_rel_name "$rel" || die "Refusing to write release receipt for unsafe name: $rel"
  mkdir -p "$root/tmp" "$root/receipts"
  tmp="$root/tmp/release.receipt.$$"
  {
    printf '%s\n' "$RECEIPT_MAGIC"
    printf 'schema=%s\n' "$SCHEMA_VERSION"
    printf 'root=%s\n' "$root"
    printf 'tag=%s\n' "$tag"
    printf 'version=%s\n' "$version"
    printf 'target=%s\n' "$target"
    printf 'archive=%s\n' "$package_asset"
    printf 'archive_sha256=%s\n' "$archive_digest"
    printf 'bin_sha256=%s\n' "$bin_digest"
    printf 'release_dir=%s\n' "$rel"
  } >"$tmp"
  mv -f "$tmp" "$root/receipts/$rel.receipt"
}

write_current_receipt() {
  cur_dir="$1"
  prev_dir="$2"
  activated_state="$3"
  profile_path="$4"

  safe_rel_name "$cur_dir" || die "Refusing to write current receipt for unsafe name: $cur_dir"
  case "$prev_dir" in
    -) ;;
    *) safe_rel_name "$prev_dir" || die "Refusing to write current receipt for unsafe previous: $prev_dir" ;;
  esac

  rpath="$root/receipts/$cur_dir.receipt"
  [ -f "$rpath" ] || die "Missing release receipt $rpath; cannot write current receipt."

  mkdir -p "$root/tmp" "$root/receipts"
  tmp="$root/tmp/current.receipt.$$"
  {
    printf '%s\n' "$RECEIPT_MAGIC"
    printf 'schema=%s\n' "$SCHEMA_VERSION"
    printf 'root=%s\n' "$root"
    printf 'tag=%s\n' "$(receipt_key "$rpath" tag)"
    printf 'version=%s\n' "$(receipt_key "$rpath" version)"
    printf 'target=%s\n' "$(receipt_key "$rpath" target)"
    printf 'archive=%s\n' "$(receipt_key "$rpath" archive)"
    printf 'archive_sha256=%s\n' "$(receipt_key "$rpath" archive_sha256)"
    printf 'bin_sha256=%s\n' "$(receipt_key "$rpath" bin_sha256)"
    printf 'release_dir=%s\n' "$(receipt_key "$rpath" release_dir)"
    printf 'current=%s\n' "$cur_dir"
    printf 'previous=%s\n' "$prev_dir"
    printf 'activated=%s\n' "$activated_state"
    printf 'profile=%s\n' "$profile_path"
    printf 'manager=%s\n' 'manager/lumi-install.sh'
    printf 'launcher=%s\n' "$launcher"
    printf 'shim=%s\n' 'shim/codex'
    printf 'shim_dir=%s\n' 'shim'
    printf 'install_dir=%s\n' "$install_dir"
    printf 'releases_dir=%s\n' 'releases'
    printf 'receipts_dir=%s\n' 'receipts'
    printf 'tmp_dir=%s\n' 'tmp'
  } >"$tmp"
  mv -f "$tmp" "$root/receipts/current.receipt"
}

update_activation() {
  new_activated="$1"
  new_profile="$2"

  receipt="$root/receipts/current.receipt"
  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  safe_rel_name "$cur" || die "current.receipt contains an unsafe current value; refusing."
  case "$prev" in
    -) ;;
    *) safe_rel_name "$prev" || die "current.receipt contains an unsafe previous value; refusing." ;;
  esac
  write_current_receipt "$cur" "$prev" "$new_activated" "$new_profile"
}

version_from_binary() {
  codex_path="$1"

  if [ ! -x "$codex_path" ]; then
    return 1
  fi

  "$codex_path" --version 2>/dev/null | sed -n 's/.* \([0-9][0-9A-Za-z.+-]*\)$/\1/p' | head -n 1
}

validate_archive_members() {
  archive="$1"

  # Names: reject absolute members, empty names, and any `..` component.
  if ! tar -tzf "$archive" 2>/dev/null | awk '
    {
      name = $0
      if (name == "" || substr(name, 1, 1) == "/") { bad = 1 }
      n = split(name, parts, "/")
      for (i = 1; i <= n; i++) {
        if (parts[i] == "..") { bad = 1 }
      }
      if (bad) exit 1
    }
    END { if (bad) exit 1 }
  '; then
    echo "Package archive contains unsafe member paths (absolute or .. traversal); refusing." >&2
    return 1
  fi

  # Types: only regular files (-) and directories (d); reject symlinks,
  # hardlinks, devices, and fifos.
  if ! tar -tvzf "$archive" 2>/dev/null | awk '
    {
      type = substr($1, 1, 1)
      if (type != "-" && type != "d") { exit 1 }
    }
  '; then
    echo "Package archive contains unsafe entry types (symlinks or special files); refusing." >&2
    return 1
  fi
}

validate_package_metadata() {
  pkgjson="$1"
  expected_version="$2"
  expected_target="$3"

  [ -f "$pkgjson" ] && [ ! -L "$pkgjson" ] || return 1
  # Strict parser for the canonical pretty-printed top-level JSON.  Only the
  # five fields are allowed, each exactly once, on lines of the exact form
  #   "key": "value"
  # Values are compared as exact strings (no regex interpolation), and any
  # duplicate, unknown, decoy, or malformed line rejects the package.
  awk -v expected_version="$expected_version" -v expected_target="$expected_target" '
    BEGIN {
      bad = 0
      nfields = 0
    }
    {
      line = $0
      sub(/[[:space:]]*$/, "", line)
      sub(/,$/, "", line)
      sub(/[[:space:]]*$/, "", line)
      if (line == "{" || line == "}" || line == "") next

      n = split(line, parts, "\"")
      if (n != 5) { bad = 1; next }
      if (parts[1] !~ /^[[:space:]]*$/) { bad = 1; next }
      if (parts[3] !~ /^[[:space:]]*:[[:space:]]*$/) { bad = 1; next }
      if (parts[5] != "") { bad = 1; next }
      key = parts[2]
      val = parts[4]
      if (key != "distribution" && key != "variant" && key != "entrypoint" &&
          key != "version" && key != "target") { bad = 1; next }
      if (seen[key]++) { bad = 1; next }
      value[key] = val
      nfields++
    }
    END {
      if (bad || nfields != 5) exit 1
      if (value["distribution"] != "lumi") exit 1
      if (value["variant"] != "codex") exit 1
      if (value["entrypoint"] != "bin/codex") exit 1
      if (value["version"] != expected_version) exit 1
      if (value["target"] != expected_target) exit 1
      exit 0
    }
  ' "$pkgjson"
}

release_dir_is_complete() {
  dir="$1"
  expected_version="$2"
  expected_target="$3"

  [ -d "$dir" ] || return 1
  [ -f "$dir/codex-package.json" ] && [ ! -L "$dir/codex-package.json" ] &&
    [ -f "$dir/bin/codex" ] && [ ! -L "$dir/bin/codex" ] &&
    [ -f "$dir/bin/codex-code-mode-host" ] && [ ! -L "$dir/bin/codex-code-mode-host" ] &&
    [ -f "$dir/codex-path/rg" ] && [ ! -L "$dir/codex-path/rg" ] &&
    [ -f "$dir/codex-resources/bwrap" ] && [ ! -L "$dir/codex-resources/bwrap" ] &&
    [ -x "$dir/codex" ] &&
    [ "$(readlink "$dir/codex" 2>/dev/null || true)" = "bin/codex" ] || return 1

  validate_package_metadata "$dir/codex-package.json" "$expected_version" "$expected_target" || return 1

  installed_version="$(version_from_binary "$dir/bin/codex" || true)"
  [ -n "$installed_version" ] || return 1
  [ "$installed_version" = "$expected_version" ]
}

resolve_release() {
  normalized="$(normalize_version "$release")"
  validate_version "$normalized"

  if [ "$normalized" = "latest" ]; then
    metadata_url="$RELEASES_API_BASE/releases/latest"
  else
    metadata_url="$RELEASES_API_BASE/releases/tags/rust-v$normalized"
  fi

  step "Resolving release metadata from $metadata_url"
  if ! release_json="$(download_text "$metadata_url")"; then
    die "Could not fetch Lumi-weaves/codex release metadata. GitHub API may be unavailable or rate limited."
  fi

  if ! release_metadata="$(printf '%s\n' "$release_json" | parse_release_metadata)"; then
    die "Could not parse release metadata JSON from Lumi-weaves/codex."
  fi

  tag="$(printf '%s\n' "$release_metadata" | awk -F '\t' '$1 == "tag_name" { print $2; exit }')"
  case "$tag" in
    rust-v*)
      version="${tag#rust-v}"
      ;;
    *)
      die "Release metadata did not include a valid rust-v tag."
      ;;
  esac
  validate_version "$version"

  package_asset="codex-package-$target.tar.gz"
  checksum_asset="codex-package_SHA256SUMS"

  if ! release_asset_exists "$package_asset"; then
    die "Release $tag has no package asset for target $target ($package_asset)."
  fi
  if ! release_asset_exists "$checksum_asset"; then
    die "Release $tag is missing the checksum manifest $checksum_asset."
  fi
  if [ -z "${LUMI_DEV_MANAGER_SELF:-}" ] && ! release_asset_exists "$MANAGER_ASSET"; then
    die "Release $tag is missing the bootstrap asset $MANAGER_ASSET (needed to install a verified manager copy)."
  fi
}

release_is_proven() {
  rel="$1"

  safe_rel_name "$rel" || return 1
  [ -d "$releases_dir/$rel" ] || return 1
  rpath="$receipts_dir/$rel.receipt"
  validate_receipt "$rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" || return 1
  [ "$(receipt_key "$rpath" version)" = "$version" ] || return 1
  [ "$(receipt_key "$rpath" target)" = "$target" ] || return 1
  [ "$(receipt_key "$rpath" release_dir)" = "$rel" ] || return 1
  release_dir_is_complete "$releases_dir/$rel" "$version" "$target" || return 1
  [ "$(file_sha256 "$releases_dir/$rel/bin/codex")" = "$(receipt_key "$rpath" bin_sha256)" ] || return 1
}

download_and_stage() {
  rel="$1"

  checksum_digest="$(release_asset_digest "$checksum_asset")"
  download_file "$(release_url_for_asset "$checksum_asset")" "$tmp_dir/checksum" ||
    die "Could not download $checksum_asset."
  verify_archive_digest "$tmp_dir/checksum" "$checksum_digest" ||
    die "Downloaded $checksum_asset digest did not match release metadata."

  expected_digest="$(package_archive_digest "$package_asset" "$tmp_dir/checksum")" ||
    die "Checksum manifest has no digest for $package_asset."

  step "Downloading $package_asset"
  download_file "$(release_url_for_asset "$package_asset")" "$tmp_dir/archive.tar.gz" ||
    die "Could not download $package_asset."
  verify_archive_digest "$tmp_dir/archive.tar.gz" "$expected_digest" ||
    die "Downloaded archive checksum did not match codex-package_SHA256SUMS; aborting without switching."

  if [ -z "${LUMI_DEV_MANAGER_SELF:-}" ]; then
    step "Downloading $MANAGER_ASSET"
    download_file "$(release_url_for_asset "$MANAGER_ASSET")" "$tmp_dir/lumi-install.sh" ||
      die "Could not download $MANAGER_ASSET."
    verify_archive_digest "$tmp_dir/lumi-install.sh" "$(release_asset_digest "$MANAGER_ASSET")" ||
      die "Downloaded $MANAGER_ASSET digest did not match release metadata."
    sh -n "$tmp_dir/lumi-install.sh" ||
      die "Downloaded $MANAGER_ASSET fails the shell syntax check; refusing to install it as the manager."
  fi

  validate_archive_members "$tmp_dir/archive.tar.gz" ||
    die "Aborting without switching."

  mkdir -p "$root/tmp"
  stage="$root/tmp/.staging.$rel.$$"
  rm -rf "$stage"
  mkdir -p "$stage"
  if ! tar --no-same-owner --no-same-permissions -xzf "$tmp_dir/archive.tar.gz" -C "$stage" 2>/dev/null; then
    rm -rf "$stage"
    die "Package extraction with hardened tar options failed; aborting without switching (no fallback extraction)."
  fi
  [ -f "$stage/bin/codex" ] && chmod 0755 "$stage/bin/codex"
  [ -f "$stage/bin/codex-code-mode-host" ] && chmod 0755 "$stage/bin/codex-code-mode-host"
  [ -f "$stage/codex-path/rg" ] && chmod 0755 "$stage/codex-path/rg"
  if [ -f "$stage/codex-resources/bwrap" ]; then
    chmod 0755 "$stage/codex-resources/bwrap"
  fi
  ln -sf bin/codex "$stage/codex"

  if ! release_dir_is_complete "$stage" "$version" "$target"; then
    rm -rf "$stage"
    die "Staged package failed completeness or version validation; aborting without switching."
  fi

  bin_digest="$(file_sha256 "$stage/bin/codex")"
  write_release_receipt "$rel" "$bin_digest" "$expected_digest"

  mkdir -p "$releases_dir"
  if [ -e "$releases_dir/$rel" ] || [ -L "$releases_dir/$rel" ]; then
    rm -rf "$releases_dir/$rel"
  fi
  mv "$stage" "$releases_dir/$rel"
}

write_journal() {
  op="$1"
  cur_old="$2"
  cur_new="$3"
  prev_old="$4"
  prev_new="$5"

  mkdir -p "$root/tmp"
  tmp="$root/tmp/pending.journal.$$"
  {
    printf '%s\n' "$JOURNAL_MAGIC"
    printf 'schema=%s\n' "$SCHEMA_VERSION"
    printf 'op=%s\n' "$op"
    printf 'current_old=%s\n' "$cur_old"
    printf 'current_new=%s\n' "$cur_new"
    printf 'previous_old=%s\n' "$prev_old"
    printf 'previous_new=%s\n' "$prev_new"
  } >"$tmp"
  mv -f "$tmp" "$root/tmp/pending.journal"
}

validate_journal() {
  path="$1"

  [ -f "$path" ] || return 1
  [ "$(sed -n '1p' "$path")" = "$JOURNAL_MAGIC" ] || return 1

  if ! awk -v allowed="$JOURNAL_KEYS" '
    NR == 1 { next }
    {
      line = $0
      eq = index(line, "=")
      if (eq <= 1) { bad = 1; next }
      key = substr(line, 1, eq - 1)
      value = substr(line, eq + 1)
      if (value == "") { bad = 1 }
      if (seen[key]++) { bad = 1 }
      if (!((" " allowed " ") ~ (" " key " "))) { bad = 1 }
      keys = keys " " key
    }
    END {
      if (bad) exit 1
      n = split(allowed, req, " ")
      for (i = 1; i <= n; i++) {
        if (!((" " keys " ") ~ (" " req[i] " "))) exit 1
      }
      exit 0
    }
  ' "$path"; then
    return 1
  fi

  op="$(receipt_key "$path" op)"
  case "$op" in
    install | rollback) ;;
    *) return 1 ;;
  esac
  for key in current_old current_new previous_old previous_new; do
    value="$(receipt_key "$path" "$key")"
    case "$value" in
      -) ;;
      *) safe_rel_name "$value" || return 1 ;;
    esac
  done
}

journal_value() {
  receipt_key "$root/tmp/pending.journal" "$1"
}

current_symlink_rel() {
  value="$(readlink "$root/current" 2>/dev/null || true)"
  case "$value" in
    releases/*) printf '%s\n' "$(basename "$value")" ;;
    "") printf '%s\n' "-" ;;
    *) printf '%s\n' "$value" ;;
  esac
}

previous_symlink_rel() {
  value="$(readlink "$root/previous" 2>/dev/null || true)"
  case "$value" in
    releases/*) printf '%s\n' "$(basename "$value")" ;;
    "") printf '%s\n' "-" ;;
    *) printf '%s\n' "$value" ;;
  esac
}

reconcile_journal() {
  journal="$root/tmp/pending.journal"
  [ -f "$journal" ] || return 0

  if ! validate_journal "$journal"; then
    die "Pending operation journal at $journal is missing, invalid, or tampered; refusing to continue (run doctor for details)."
  fi

  op="$(journal_value op)"
  cur_old="$(journal_value current_old)"
  cur_new="$(journal_value current_new)"
  prev_old="$(journal_value previous_old)"
  prev_new="$(journal_value previous_new)"
  actual_cur="$(current_symlink_rel)"
  actual_prev="$(previous_symlink_rel)"

  value_in_set() {
    value="$1"
    set_a="$2"
    set_b="$3"
    [ "$value" = "$set_a" ] || [ "$value" = "$set_b" ]
  }

  if value_in_set "$actual_cur" "$cur_old" "$cur_new" &&
    value_in_set "$actual_prev" "$prev_old" "$prev_new"; then
    :
  else
    die "Pending operation journal state does not match the actual symlinks; refusing to reconcile tampered state."
  fi

  if [ "$actual_cur" = "$cur_old" ] && [ "$actual_prev" = "$prev_old" ]; then
    # The first switch never happened; the operation was abandoned before mutation.
    rm -f "$journal"
    step "Reconciled abandoned journal (no switch happened); removed it."
    return 0
  fi

  # At least the current switch happened. Validate the intended target release
  # before finalizing so a corrupt target cannot be accepted.
  cur_rpath="$receipts_dir/$cur_new.receipt"
  cur_version="$(receipt_key "$cur_rpath" version)"
  cur_target="$(receipt_key "$cur_rpath" target)"
  if ! release_dir_is_complete "$releases_dir/$cur_new" "$cur_version" "$cur_target"; then
    die "Journal target release $cur_new is incomplete or invalid; refusing to finalize."
  fi

  if [ "$actual_prev" != "$prev_new" ]; then
    # Crash between the current switch and the previous switch: complete it.
    if [ "$prev_new" = "-" ]; then
      rm -f "$root/previous"
    else
      switch_link previous "releases/$prev_new"
    fi
    step "Reconciled interrupted journal: completed previous switch to $prev_new."
  fi

  receipt="$root/receipts/current.receipt"
  if validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS"; then
    activated="$(receipt_key "$receipt" activated)"
    profile="$(receipt_key "$receipt" profile)"
  else
    activated="no"
    profile="-"
  fi
  write_current_receipt "$cur_new" "$prev_new" "$activated" "$profile"
  rm -f "$journal"
  step "Reconciled journal: current=$cur_new previous=$prev_new."
}

ensure_manager_and_shim() {
  mkdir -p "$root/manager" "$root/shim"

  manager_source=""
  if [ -n "${LUMI_DEV_MANAGER_SELF:-}" ]; then
    manager_source="$LUMI_DEV_MANAGER_SELF"
    [ -r "$manager_source" ] || die "LUMI_DEV_MANAGER_SELF is not a readable file: $manager_source"
    sh -n "$manager_source" || die "LUMI_DEV_MANAGER_SELF fails the shell syntax check."
    warn "Developer mode: installing manager copy from $manager_source instead of the verified release asset."
  elif [ -f "$tmp_dir/lumi-install.sh" ]; then
    manager_source="$tmp_dir/lumi-install.sh"
  elif [ -f "$root/manager/lumi-install.sh" ] && sh -n "$root/manager/lumi-install.sh" 2>/dev/null; then
    manager_source="$root/manager/lumi-install.sh"
  else
    die "No verified manager source available (release asset missing or manager copy invalid); reinstall required."
  fi

  if [ "$manager_source" != "$root/manager/lumi-install.sh" ]; then
    cp "$manager_source" "$root/manager/lumi-install.sh"
  fi
  chmod 0755 "$root/manager/lumi-install.sh"

  mkdir -p "$install_dir"
  ensure_owned_symlink "$launcher" "$root/manager/lumi-install.sh"
  ensure_owned_symlink "$root/shim/lumi-codex" "../manager/lumi-install.sh"
  ensure_owned_symlink "$root/shim/codex" "../current/bin/codex"
}

install_dir_on_path() {
  old_ifs="$IFS"
  IFS=:
  for dir in $PATH; do
    if [ -n "$dir" ] && [ "$dir" = "$install_dir" ]; then
      IFS="$old_ifs"
      return 0
    fi
  done
  IFS="$old_ifs"
  return 1
}

cmd_install() {
  case "$(uname -s)" in
    Linux) ;;
    *)
      die "Lumi Codex installer supports Linux only in this canary (detected $(uname -s))."
      ;;
  esac

  if [ -z "$target" ]; then
    arch="$(uname -m)"
    case "$arch" in
      x86_64 | amd64)
        target="x86_64-unknown-linux-musl"
        ;;
      aarch64 | arm64)
        target="aarch64-unknown-linux-musl"
        ;;
      *)
        die "Unsupported architecture: $arch. Linux x86_64 is the canary target; use --target or LUMI_TARGET to select another known target."
        ;;
    esac
  fi
  validate_target

  resolve_root
  resolve_install_dir
  validate_root_for_mutation
  validate_install_dir_for_mutation

  mkdir -p "$root"
  acquire_install_lock
  cleanup_stale_artifacts
  reconcile_journal

  resolve_release

  step "Resolved release: $tag"
  step "Target: $target"

  release_dir="$version-$target"
  safe_rel_name "$release_dir" || die "Refusing to install release with unsafe name: $release_dir"

  if release_is_proven "$release_dir"; then
    step "Release $release_dir is already installed, complete, and digest-verified; reusing it."
  else
    if [ -e "$releases_dir/$release_dir" ] || [ -L "$releases_dir/$release_dir" ]; then
      warn "Existing release $release_dir is incomplete, unproven, or tampered; reinstalling."
      rm -rf "$releases_dir/$release_dir"
    fi
    download_and_stage "$release_dir"
  fi

  release_dir_is_complete "$releases_dir/$release_dir" "$version" "$target" ||
    die "Installed release $release_dir failed final validation."

  old_current="$(current_symlink_rel)"
  old_previous="$(previous_symlink_rel)"
  case "$old_current" in
    -) old_current="" ;;
  esac

  new_previous="$old_previous"
  if [ -n "$old_current" ] && [ "$old_current" != "$release_dir" ]; then
    new_previous="$old_current"
  fi

  if [ "$old_current" != "$release_dir" ]; then
    cur_old="$old_current"
    [ -n "$cur_old" ] || cur_old="-"
    write_journal install "$cur_old" "$release_dir" "$old_previous" "$new_previous"
    switch_link current "releases/$release_dir"
    if [ -n "${LUMI_TEST_SLOW_AFTER_SWITCH:-}" ]; then
      sleep "$LUMI_TEST_SLOW_AFTER_SWITCH"
    fi
    if [ "$new_previous" != "$old_previous" ]; then
      switch_link previous "releases/$new_previous"
    fi
  fi

  ensure_manager_and_shim

  activated="no"
  profile="-"
  if [ "$activate_install" = 1 ]; then
    profile="$(pick_profile)"
    validate_abs_path "$profile" "LUMI_PROFILE"
    path_line="export PATH=\"$root/shim:\$PATH\""
    profile_activate
    activated="yes"
  fi
  write_current_receipt "$release_dir" "$new_previous" "$activated" "$profile"
  rm -f "$root/tmp/pending.journal"

  step "Lumi Codex CLI $version installed (root: $root)."
  step "Launcher: $launcher"
  if install_dir_on_path; then
    step "Run: lumi-codex   (execs the installed CLI; new terminals pick it up automatically)"
  else
    warn "$install_dir is not on PATH."
    warn "Add $install_dir to PATH, or run the launcher directly: $launcher manage install|doctor|list|rollback|activate|deactivate|uninstall"
    warn "Run the installed CLI with: $launcher <args>"
  fi
  if [ "$activated" = "yes" ]; then
    step "Activated: $root/shim added to PATH in $profile (codex now resolves to the managed CLI)."
  else
    step "Not activated: codex still resolves to the official install (side-by-side)."
    step "Run: lumi-codex manage activate   to shadow codex with the managed CLI"
  fi
}

find_official_codex() {
  old_ifs="$IFS"
  IFS=:
  for dir in $PATH; do
    if [ -n "$dir" ] && [ "$dir" != "$root/shim" ] && [ -x "$dir/codex" ]; then
      IFS="$old_ifs"
      printf '%s\n' "$dir/codex"
      return 0
    fi
  done
  IFS="$old_ifs"
  return 1
}

cmd_doctor() {
  step "Lumi Codex doctor"
  step "root: $root"

  if [ ! -e "$root" ]; then
    step "Lumi Codex is not installed (root does not exist)."
    return 0
  fi

  problems=0
  report() {
    if [ "$2" = 0 ]; then
      printf '[ok] %s\n' "$1"
    else
      printf '[FAIL] %s\n' "$1"
      problems=1
    fi
  }

  expected_root="${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex"
  if [ -n "${LUMI_ROOT:-}" ]; then
    expected_root="$LUMI_ROOT"
  fi
  if [ "$root" = "$expected_root" ]; then
    report "root derivation matches environment" 0
  else
    report "root derivation mismatch (root=$root expected=$expected_root)" 1
  fi
  case "$root" in
    /*) report "root is absolute" 0 ;;
    *) report "root is not absolute" 1 ;;
  esac
  if [ -L "$root" ]; then
    report "root is a symlink" 1
  else
    report "root is not a symlink" 0
  fi
  if [ "$(basename "$root")" = "lumi-codex" ]; then
    report "root basename is lumi-codex" 0
  else
    report "root basename is not lumi-codex" 1
  fi

  # Reconcile a valid pending journal deterministically; a tampered journal
  # fails closed (reported below and no mutation is attempted).
  if [ -f "$root/tmp/pending.journal" ]; then
    if validate_journal "$root/tmp/pending.journal"; then
      acquire_install_lock
      reconcile_journal
      release_install_lock
      step "doctor reconciled a valid pending journal."
    else
      report "pending journal is invalid or tampered" 1
    fi
  fi

  receipt="$root/receipts/current.receipt"
  if validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS"; then
    report "current.receipt is valid" 0
  else
    report "current.receipt is missing or invalid" 1
    step "doctor: PROBLEMS FOUND (cannot verify the install without a valid receipt)"
    return 1
  fi

  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  release_dir="$(receipt_key "$receipt" release_dir)"
  version="$(receipt_key "$receipt" version)"
  target="$(receipt_key "$receipt" target)"
  bin_sha256="$(receipt_key "$receipt" bin_sha256)"
  activated="$(receipt_key "$receipt" activated)"
  profile="$(receipt_key "$receipt" profile)"
  launcher="$(receipt_key "$receipt" launcher)"
  install_dir="$(receipt_key "$receipt" install_dir)"

  cur_prev_safe=1
  safe_rel_name "$cur" || cur_prev_safe=0
  if [ "$prev" != "-" ]; then
    safe_rel_name "$prev" || cur_prev_safe=0
  fi
  if [ "$cur_prev_safe" = 1 ]; then
    report "receipt current/previous names are safe" 0
  else
    report "receipt current/previous names are unsafe" 1
  fi
  if safe_rel_name "$release_dir"; then
    report "receipt release_dir name is safe" 0
  else
    report "receipt release_dir name is unsafe" 1
  fi

  if [ "$release_dir" = "$cur" ]; then
    report "receipt release_dir matches current" 0
  else
    report "receipt release_dir ($release_dir) does not match current ($cur)" 1
  fi

  actual_current="$(readlink "$root/current" 2>/dev/null || true)"
  if safe_rel_name "$cur" && [ "$actual_current" = "releases/$cur" ]; then
    report "current symlink points to releases/$cur" 0
  else
    report "current symlink drift (got: $actual_current)" 1
  fi

  if safe_rel_name "$cur" && release_dir_is_complete "$root/releases/$cur" "$version" "$target"; then
    report "current release directory is complete" 0
  else
    report "current release directory is incomplete or missing" 1
  fi

  cur_rpath="$root/receipts/$cur.receipt"
  if safe_rel_name "$cur" &&
    validate_receipt "$cur_rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" &&
    [ "$(receipt_key "$cur_rpath" version)" = "$version" ] &&
    [ "$(receipt_key "$cur_rpath" target)" = "$target" ]; then
    report "current release receipt matches" 0
  else
    report "current release receipt is missing, invalid, or mismatched" 1
  fi

  actual_bin_sha="$(file_sha256 "$root/releases/$cur/bin/codex" 2>/dev/null || true)"
  if [ -n "$bin_sha256" ] && [ "$actual_bin_sha" = "$bin_sha256" ]; then
    report "current bin/codex sha256 matches receipt" 0
  else
    report "current bin/codex sha256 does not match receipt" 1
  fi

  actual_version="$(version_from_binary "$root/current/bin/codex" || true)"
  if [ -n "$version" ] && [ "$actual_version" = "$version" ]; then
    report "reported version matches receipt ($version)" 0
  else
    report "reported version mismatch (got: $actual_version, expected: $version)" 1
  fi

  if [ "$prev" = "-" ]; then
    if [ -e "$root/previous" ] || [ -L "$root/previous" ]; then
      report "previous symlink exists but receipt records no previous release" 1
    else
      report "no previous release (receipt consistent)" 0
    fi
  elif safe_rel_name "$prev"; then
    prev_rpath="$root/receipts/$prev.receipt"
    prev_version="$(receipt_key "$prev_rpath" version)"
    prev_target="$(receipt_key "$prev_rpath" target)"
    actual_prev="$(readlink "$root/previous" 2>/dev/null || true)"
    if [ "$actual_prev" = "releases/$prev" ] &&
      validate_receipt "$prev_rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" &&
      release_dir_is_complete "$root/releases/$prev" "$prev_version" "$prev_target"; then
      report "previous release ($prev) is consistent" 0
    else
      report "previous release ($prev) is missing, drifted, or incomplete" 1
    fi
  else
    report "previous release name from receipt is unsafe" 1
  fi

  if [ -f "$root/manager/lumi-install.sh" ] && [ -x "$root/manager/lumi-install.sh" ] &&
    sh -n "$root/manager/lumi-install.sh" 2>/dev/null; then
    report "manager copy exists, is executable, and passes sh -n" 0
  else
    report "manager copy is missing, not executable, or fails sh -n" 1
  fi

  launcher_safe=1
  case "$launcher" in
    /*) ;;
    *) launcher_safe=0 ;;
  esac
  if has_control_chars "$launcher"; then
    launcher_safe=0
  fi
  if [ "$launcher_safe" = 1 ]; then
    report "visible launcher path is absolute and safe" 0
  else
    report "visible launcher path is not absolute or contains control characters" 1
  fi
  if [ -L "$launcher" ] && [ "$(readlink "$launcher" 2>/dev/null || true)" = "$root/manager/lumi-install.sh" ]; then
    report "visible launcher $launcher is the owned symlink" 0
  else
    report "visible launcher $launcher is missing or not the owned symlink" 1
  fi

  if [ -L "$root/shim/lumi-codex" ] && [ "$(readlink "$root/shim/lumi-codex" 2>/dev/null || true)" = "../manager/lumi-install.sh" ]; then
    report "shim lumi-codex launcher is the owned symlink" 0
  else
    report "shim lumi-codex launcher is missing or not the owned symlink" 1
  fi

  if [ -L "$root/shim/codex" ] && [ "$(readlink "$root/shim/codex" 2>/dev/null || true)" = "../current/bin/codex" ]; then
    report "shim codex is the owned symlink" 0
  else
    report "shim codex is missing, drifted, or an unknown symlink" 1
  fi

  path_line="export PATH=\"$root/shim:\$PATH\""
  if [ "$activated" = "yes" ]; then
    if [ -n "$profile" ] && [ "$profile" != "-" ] && [ ! -L "$profile" ] && [ -f "$profile" ] &&
      [ "$(profile_block_state)" = "exact" ]; then
      report "activation block in $profile is the exact owned block" 0
    else
      report "receipt says activated but the owned PATH block is missing or drifted" 1
    fi
  elif [ "$activated" = "no" ]; then
    if [ "$(profile_block_state)" = "none" ]; then
      report "deactivated and no owned PATH block present" 0
    else
      report "receipt says deactivated but a Lumi Codex PATH block is present" 1
    fi
  else
    report "invalid activated field in receipt ($activated)" 1
  fi

  if install_dir_on_path; then
    step "launcher directory $install_dir is on PATH"
  else
    step "launcher directory $install_dir is NOT on PATH; run: $launcher manage ..."
  fi

  official="$(find_official_codex || true)"
  if [ -n "$official" ]; then
    step "official codex (outside shim): $official"
  else
    step "official codex (outside shim): none found"
  fi

  if [ "$problems" = 1 ]; then
    step "doctor: PROBLEMS FOUND"
    return 1
  fi
  step "doctor: OK"
  return 0
}

cmd_list() {
  step "Lumi Codex managed releases (root: $root)"

  if [ ! -e "$root" ]; then
    step "Lumi Codex is not installed."
    return 0
  fi

  receipt="$root/receipts/current.receipt"
  if validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS"; then
    cur="$(receipt_key "$receipt" current)"
    prev="$(receipt_key "$receipt" previous)"
    activated="$(receipt_key "$receipt" activated)"
    profile="$(receipt_key "$receipt" profile)"
    launcher="$(receipt_key "$receipt" launcher)"
    step "current: $cur"
    if [ "$prev" != "-" ] && [ -n "$prev" ]; then
      step "previous: $prev"
    fi
    if [ "$activated" = "yes" ] && [ -n "$profile" ] && [ "$profile" != "-" ]; then
      step "activation: enabled (profile: $profile)"
    else
      step "activation: disabled"
    fi
    step "launcher: $launcher"
  fi

  if [ -d "$root/releases" ]; then
    for rel in "$root/releases"/*; do
      [ -d "$rel" ] || continue
      name="$(basename "$rel")"
      printf '  - %s\n' "$name"
    done
  fi
}

cmd_rollback() {
  [ -e "$root/current" ] || die "Lumi Codex is not installed."
  validate_root_for_mutation
  acquire_install_lock
  reconcile_journal

  receipt="$root/receipts/current.receipt"
  validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "Refusing rollback: current.receipt is missing or invalid."

  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  safe_rel_name "$cur" || die "Refusing rollback: current release name from receipt is unsafe."
  [ -n "$cur" ] || die "Refusing rollback: current release missing from receipt."
  if [ -z "$prev" ] || [ "$prev" = "-" ]; then
    die "Nothing to roll back to (no previous release recorded)."
  fi
  safe_rel_name "$prev" || die "Refusing rollback: previous release name from receipt is unsafe."
  [ "$cur" != "$prev" ] || die "Refusing rollback: current and previous are the same release."

  [ "$(readlink "$root/current" 2>/dev/null || true)" = "releases/$cur" ] ||
    die "Refusing rollback: current symlink drifted from the receipt."

  cur_version="$(receipt_key "$receipt" version)"
  cur_target="$(receipt_key "$receipt" target)"
  release_dir_is_complete "$root/releases/$cur" "$cur_version" "$cur_target" ||
    die "Refusing rollback: current release is incomplete."

  prev_rpath="$root/receipts/$prev.receipt"
  validate_receipt "$prev_rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "Refusing rollback: previous release receipt is missing or invalid."
  prev_version="$(receipt_key "$prev_rpath" version)"
  prev_target="$(receipt_key "$prev_rpath" target)"
  release_dir_is_complete "$root/releases/$prev" "$prev_version" "$prev_target" ||
    die "Refusing rollback: previous release is incomplete."

  activated="$(receipt_key "$receipt" activated)"
  profile="$(receipt_key "$receipt" profile)"

  write_journal rollback "$cur" "$prev" "$prev" "$cur"
  switch_link current "releases/$prev"
  switch_link previous "releases/$cur"
  write_current_receipt "$prev" "$cur" "$activated" "$profile"
  rm -f "$root/tmp/pending.journal"

  step "Rolled back: current=$prev previous=$cur"
}

cmd_activate() {
  [ -L "$root/current" ] || die "Lumi Codex is not installed; run: lumi-codex manage install"
  validate_root_for_mutation
  validate_receipt "$root/receipts/current.receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "current.receipt is missing or invalid; run: lumi-codex manage doctor"
  [ "$(readlink "$root/shim/lumi-codex" 2>/dev/null || true)" = "../manager/lumi-install.sh" ] ||
    die "Lumi Codex shim launcher is missing or not the owned symlink; run: lumi-codex manage install"
  [ "$(readlink "$root/shim/codex" 2>/dev/null || true)" = "../current/bin/codex" ] ||
    die "Lumi Codex shim codex is missing, drifted, or an unknown symlink; run: lumi-codex manage doctor"

  acquire_install_lock
  profile="$(pick_profile)"
  validate_abs_path "$profile" "LUMI_PROFILE"
  path_line="export PATH=\"$root/shim:\$PATH\""
  profile_activate
  update_activation yes "$profile"
  step "Lumi Codex activated: $root/shim added to PATH in $profile (codex is now shadowed)."
}

cmd_deactivate() {
  [ -e "$root/current" ] || die "Lumi Codex is not installed."
  validate_root_for_mutation
  validate_receipt "$root/receipts/current.receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "current.receipt is missing or invalid; run: lumi-codex manage doctor"

  acquire_install_lock
  profile="$(receipt_key "$root/receipts/current.receipt" profile)"
  case "$profile" in
    "" | -) profile="$(pick_profile)" ;;
  esac
  validate_abs_path "$profile" "profile path from receipt"
  path_line="export PATH=\"$root/shim:\$PATH\""
  profile_deactivate
  update_activation no "-"
  step "Lumi Codex deactivated: PATH block removed from $profile (official codex resolution restored)."
}

cmd_uninstall() {
  if [ ! -e "$root" ]; then
    step "Lumi Codex is not installed (root $root does not exist)."
    return 0
  fi

  case "$root" in
    "" | /) die "Refusing to uninstall root $root" ;;
  esac
  case "$root" in
    /*) ;;
    *) die "Refusing to uninstall non-absolute root $root" ;;
  esac
  [ "$(basename "$root")" = "lumi-codex" ] ||
    die "Refusing to uninstall unexpected root $root (basename is not lumi-codex)."
  [ "$root" != "$HOME" ] || die "Refusing to uninstall HOME as the Lumi Codex root."
  [ "$root" != "${XDG_DATA_HOME:-$HOME/.local/share}" ] ||
    die "Refusing to uninstall the XDG data root itself."
  validate_root_for_mutation

  acquire_install_lock

  if [ -f "$root/tmp/pending.journal" ]; then
    die "Refusing uninstall: a pending operation journal exists; run: lumi-codex manage doctor (reconciles valid journals) and retry."
  fi

  receipt="$root/receipts/current.receipt"
  validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "Refusing uninstall: current.receipt is missing or invalid; ownership cannot be proven."

  [ "$(receipt_key "$receipt" root)" = "$root" ] ||
    die "Refusing uninstall: receipt root does not match $root."

  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  activated="$(receipt_key "$receipt" activated)"
  profile="$(receipt_key "$receipt" profile)"
  launcher="$(receipt_key "$receipt" launcher)"
  install_dir="$(receipt_key "$receipt" install_dir)"
  [ -n "$cur" ] || die "Refusing uninstall: current release missing from receipt."
  safe_rel_name "$cur" || die "Refusing uninstall: current release name from receipt is unsafe."
  if [ "$prev" != "-" ]; then
    safe_rel_name "$prev" || die "Refusing uninstall: previous release name from receipt is unsafe."
  fi

  [ "$(readlink "$root/current" 2>/dev/null || true)" = "releases/$cur" ] ||
    die "Refusing uninstall: current symlink does not match the receipt."
  [ -d "$root/releases/$cur" ] || die "Refusing uninstall: current release directory is missing."
  [ "$(readlink "$root/shim/codex" 2>/dev/null || true)" = "../current/bin/codex" ] ||
    die "Refusing uninstall: shim/codex is not the owned symlink (unknown symlink)."
  [ "$(readlink "$root/shim/lumi-codex" 2>/dev/null || true)" = "../manager/lumi-install.sh" ] ||
    die "Refusing uninstall: shim/lumi-codex is not the owned symlink (unknown symlink)."
  [ -f "$root/manager/lumi-install.sh" ] || die "Refusing uninstall: manager copy is missing."

  case "$launcher" in
    /*) ;;
    *) die "Refusing uninstall: visible launcher path from receipt is not absolute." ;;
  esac
  if has_control_chars "$launcher"; then
    die "Refusing uninstall: visible launcher path from receipt contains control characters."
  fi
  [ "$(readlink "$launcher" 2>/dev/null || true)" = "$root/manager/lumi-install.sh" ] ||
    die "Refusing uninstall: visible launcher is not the owned symlink (unknown target)."

  if [ "$prev" != "-" ] && [ -n "$prev" ]; then
    [ "$(readlink "$root/previous" 2>/dev/null || true)" = "releases/$prev" ] ||
      die "Refusing uninstall: previous symlink does not match the receipt."
  elif [ -e "$root/previous" ] || [ -L "$root/previous" ]; then
    die "Refusing uninstall: previous symlink exists but the receipt records no previous release."
  fi

  # Activation state must match the profile exactly before any mutation.
  path_line="export PATH=\"$root/shim:\$PATH\""
  case "$activated" in
    yes)
      case "$profile" in
        "" | -) die "Refusing uninstall: receipt says activated but records no profile." ;;
      esac
      validate_abs_path "$profile" "profile path from receipt"
      [ -L "$profile" ] && die "Refusing uninstall: activation profile $profile is a symlink."
      [ "$(profile_block_state)" = "exact" ] ||
        die "Refusing uninstall: activation profile block is missing or drifted."
      profile_deactivate
      ;;
    no)
      if [ -n "$profile" ] && [ "$profile" != "-" ]; then
        validate_abs_path "$profile" "profile path from receipt"
        [ -L "$profile" ] && die "Refusing uninstall: activation profile $profile is a symlink."
        [ "$(profile_block_state)" = "none" ] ||
          die "Refusing uninstall: profile block state does not match the deactivated receipt."
      fi
      ;;
    *)
      die "Refusing uninstall: invalid activated field in receipt."
      ;;
  esac

  rm -f "$launcher"
  rm -rf "$root/tmp" "$root/receipts" "$root/releases" "$root/manager" "$root/shim" \
    "$root/current" "$root/previous"
  release_install_lock
  rm -f "$root/install.lock"
  if rmdir "$root" 2>/dev/null; then
    step "Lumi Codex uninstalled (removed $root and launcher $launcher)."
  else
    step "Lumi Codex uninstalled; left non-empty root at $root."
  fi
}

parse_args() {
  mode="manager"
  if [ "${1:-}" = "manage" ]; then
    shift
  elif [ "$(basename "$0")" = "lumi-codex" ]; then
    mode="launcher"
    return
  fi

  action="${1:-install}"
  if [ "$action" = "--help" ] || [ "$action" = "-h" ]; then
    usage
    exit 0
  fi
  [ $# -gt 0 ] && shift

  case "$action" in
    install)
      while [ $# -gt 0 ]; do
        case "$1" in
          --release)
            [ $# -ge 2 ] || die "--release requires a value"
            shift
            release="$1"
            ;;
          --target)
            [ $# -ge 2 ] || die "--target requires a value"
            shift
            target="$1"
            ;;
          --activate)
            activate_install=1
            ;;
          --help | -h)
            usage
            exit 0
            ;;
          *)
            die "Unknown install argument: $1"
            ;;
        esac
        shift
      done
      ;;
    doctor | list | rollback | activate | deactivate | uninstall)
      [ $# -eq 0 ] || die "Unknown argument for $action: $1"
      ;;
    *)
      die "Unknown Lumi Codex action: $action (expected install, doctor, list, rollback, activate, deactivate, uninstall)"
      ;;
  esac
}

parse_args "$@"

if [ "$mode" = "launcher" ]; then
  resolve_root
  if [ ! -x "$root/current/bin/codex" ]; then
    printf 'lumi-codex: no managed Codex at %s/current/bin/codex; run: lumi-codex manage install\n' "$root" >&2
    exit 127
  fi
  exec "$root/current/bin/codex" "$@"
fi

resolve_root
resolve_install_dir
require_command mktemp
require_command tar

tmp_dir="$(mktemp -d)" || die "mktemp -d failed; cannot create a temporary directory."

cleanup() {
  release_install_lock
  if [ -n "$tmp_dir" ] && [ -d "$tmp_dir" ]; then
    rm -rf "$tmp_dir"
  fi
}
trap cleanup EXIT INT TERM

case "$action" in
  install) cmd_install ;;
  doctor) cmd_doctor ;;
  list) cmd_list ;;
  rollback) cmd_rollback ;;
  activate) cmd_activate ;;
  deactivate) cmd_deactivate ;;
  uninstall) cmd_uninstall ;;
esac
