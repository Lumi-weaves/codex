#!/bin/sh

# Lumi Codex standalone installer/manager (Unix canary).
#
# Installs official Codex CLI package releases from the Lumi-weaves/codex
# GitHub Releases into an independent, immutable root under XDG_DATA_HOME
# (or ~/.local/share), alongside any package-managed official Codex install.
#
# The visible `lumi-codex` launcher normally execs <root>/current/bin/codex
# and intercepts only `lumi-codex manage <action>`:
#   install (default), doctor, list, rollback, activate, deactivate, uninstall
#
# Safety model:
#   - fork-only downloads from Lumi-weaves/codex GitHub Releases
#   - canonical codex-package archives verified against codex-package_SHA256SUMS
#   - immutable release directories under <root>/releases
#   - strict schema/magic receipts prove ownership of every managed path
#   - lock + staging + complete-package validation + atomic symlink switch
#     (fails closed; never falls back to a non-atomic switch)
#   - activation only appends an exactly owned, uniquely marked PATH block
#     for the Lumi shim directory; official binaries are never overwritten
#   - uninstall touches only receipt-proven owned paths and never CODEX_HOME
#     or official package-managed Codex binaries

set -eu

SCHEMA_VERSION=1
RECEIPT_MAGIC="LUMI-CODEX-RECEIPT-V1"
PROFILE_BEGIN="# >>> Lumi Codex managed PATH >>>"
PROFILE_END="# <<< Lumi Codex managed PATH <<<"
RELEASES_API_BASE="https://api.github.com/repos/Lumi-weaves/codex"
RELEASES_DOWNLOAD_BASE="https://github.com/Lumi-weaves/codex/releases/download"
RELEASES_CONNECT_TIMEOUT=10
RELEASES_METADATA_TIMEOUT=30
RELEASES_ASSET_TIMEOUT=300
LOCK_STALE_AFTER_SECS=600

RELEASE_RECEIPT_KEYS="schema root tag version target archive archive_sha256 bin_sha256 release_dir"
CURRENT_RECEIPT_KEYS="$RELEASE_RECEIPT_KEYS current previous activated profile manager launcher shim shim_dir releases_dir receipts_dir tmp_dir"

root=""
self_path=""
mode=""
action=""
release="${LUMI_RELEASE:-latest}"
target=""
no_activate=0
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
  lumi-codex manage activate           Add the owned Lumi Codex shim PATH block
  lumi-codex manage deactivate         Remove exactly the owned Lumi Codex PATH block
  lumi-codex manage uninstall          Remove the managed install (never touches official Codex)

install options:
  --release VERSION   Release to install (default: latest; env: LUMI_RELEASE)
  --target TARGET     Package target (default: x86_64-unknown-linux-musl; env: LUMI_TARGET)
  --no-activate       Install without modifying any shell profile

Environment:
  LUMI_ROOT           Override the managed root (default: \${XDG_DATA_HOME:-\$HOME/.local/share}/lumi-codex)
  LUMI_PROFILE        Override the shell profile to activate (default: \$HOME/.bashrc, .zshrc, or .profile)

The installer only downloads from Lumi-weaves/codex GitHub Releases and never
touches CODEX_HOME or package-managed official Codex binaries.
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

  if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-alpha(\.[0-9]+){0,2}|-beta(\.[0-9]+)?)?$'; then
    echo "Invalid Codex release version: $version. Expected latest or x.y.z[-alpha[.N[.M]]|-beta[.N]]." >&2
    return 1
  fi
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

  case "$root" in
    /*) ;;
    *) die "LUMI_ROOT must be an absolute path (got: $root)" ;;
  esac

  releases_dir="$root/releases"
  receipts_dir="$root/receipts"
}

resolve_self() {
  if command -v readlink >/dev/null 2>&1; then
    self_path="$(readlink -f "$0" 2>/dev/null || true)"
  fi
  if [ -z "$self_path" ]; then
    self_path="$0"
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
  target="$2"
  mkdir -p "$root/tmp"
  tmp_link="$root/tmp/.$name.$$"

  rm -f "$tmp_link"
  ln -s "$target" "$tmp_link"

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
  target="$2"

  if [ -L "$path" ]; then
    current_target="$(readlink "$path" 2>/dev/null || true)"
    if [ "$current_target" = "$target" ]; then
      return 0
    fi
    die "Refusing to overwrite unknown symlink $path -> $current_target"
  fi

  if [ -e "$path" ]; then
    die "Refusing to overwrite non-symlink path $path"
  fi

  ln -s "$target" "$path"
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

profile_activate() {
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
    printf 'launcher=%s\n' 'shim/lumi-codex'
    printf 'shim=%s\n' 'shim/codex'
    printf 'shim_dir=%s\n' 'shim'
    printf 'releases_dir=%s\n' 'releases'
    printf 'receipts_dir=%s\n' 'receipts'
    printf 'tmp_dir=%s\n' 'tmp'
  } >"$tmp"
  mv -f "$tmp" "$root/receipts/current.receipt"
}

update_activation() {
  new_activated="$1"
  new_profile="$2"

  cur="$(receipt_key "$root/receipts/current.receipt" current)"
  prev="$(receipt_key "$root/receipts/current.receipt" previous)"
  [ -n "$cur" ] || die "current.receipt is invalid."
  case "$prev" in
    - | "") prev="-" ;;
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

release_dir_is_complete() {
  dir="$1"
  expected_version="$2"
  expected_target="$3"

  [ -d "$dir" ] || return 1
  [ -f "$dir/codex-package.json" ] &&
    [ -x "$dir/bin/codex" ] &&
    [ -x "$dir/bin/codex-code-mode-host" ] &&
    [ -x "$dir/codex" ] &&
    [ -x "$dir/codex-path/rg" ] &&
    [ -x "$dir/codex-resources/bwrap" ] || return 1

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
}

release_is_proven() {
  rel="$1"

  [ -d "$releases_dir/$rel" ] || return 1
  release_dir_is_complete "$releases_dir/$rel" "$version" "$target" || return 1
  rpath="$receipts_dir/$rel.receipt"
  validate_receipt "$rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" || return 1
  [ "$(receipt_key "$rpath" version)" = "$version" ] || return 1
  [ "$(receipt_key "$rpath" target)" = "$target" ] || return 1
  [ "$(receipt_key "$rpath" release_dir)" = "$rel" ] || return 1
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

  mkdir -p "$root/tmp"
  stage="$root/tmp/.staging.$rel.$$"
  rm -rf "$stage"
  mkdir -p "$stage"
  tar -xzf "$tmp_dir/archive.tar.gz" -C "$stage"
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

ensure_manager_and_shim() {
  mkdir -p "$root/manager" "$root/shim"
  if [ "$self_path" != "$root/manager/lumi-install.sh" ]; then
    cp "$self_path" "$root/manager/lumi-install.sh"
  fi
  chmod 0755 "$root/manager/lumi-install.sh"
  ensure_owned_symlink "$root/shim/lumi-codex" "../manager/lumi-install.sh"
  ensure_owned_symlink "$root/shim/codex" "../current/bin/codex"
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
        die "Unsupported architecture: $arch. Linux x86_64 is the canary target; pass --target to select another known target."
        ;;
    esac
  fi

  mkdir -p "$root"
  acquire_install_lock
  cleanup_stale_artifacts

  resolve_release

  step "Resolved release: $tag"
  step "Target: $target"

  release_dir="$version-$target"

  if release_is_proven "$release_dir"; then
    step "Release $release_dir is already installed and complete; reusing it."
  else
    if [ -e "$releases_dir/$release_dir" ] || [ -L "$releases_dir/$release_dir" ]; then
      warn "Existing release $release_dir is incomplete or unproven; reinstalling."
      rm -rf "$releases_dir/$release_dir"
    fi
    download_and_stage "$release_dir"
  fi

  release_dir_is_complete "$releases_dir/$release_dir" "$version" "$target" ||
    die "Installed release $release_dir failed final validation."

  old_current=""
  if [ -L "$root/current" ]; then
    old_current="$(readlink "$root/current" 2>/dev/null || true)"
    case "$old_current" in
      releases/*) ;;
      *) old_current="" ;;
    esac
  fi

  switch_link current "releases/$release_dir"
  if [ -n "$old_current" ] && [ "$old_current" != "releases/$release_dir" ]; then
    switch_link previous "$old_current"
  elif [ -z "$old_current" ]; then
    rm -f "$root/previous"
  fi

  ensure_manager_and_shim

  prev_target="$(readlink "$root/previous" 2>/dev/null || true)"
  case "$prev_target" in
    releases/*) prev_target="$(basename "$prev_target")" ;;
    *) prev_target="-" ;;
  esac

  activated="no"
  profile="-"
  if [ "$no_activate" != 1 ]; then
    profile="$(pick_profile)"
    path_line="export PATH=\"$root/shim:\$PATH\""
    profile_activate
    activated="yes"
  fi
  write_current_receipt "$release_dir" "$prev_target" "$activated" "$profile"

  if [ "$activated" = "yes" ]; then
    step "Lumi Codex CLI $version installed and activated (PATH configured in $profile)."
    step "Run: lumi-codex   (new terminals pick it up automatically)"
  else
    step "Lumi Codex CLI $version installed (not activated)."
    step "Run: $root/shim/lumi-codex manage activate"
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
  if [ "$(basename "$root")" = "lumi-codex" ]; then
    report "root basename is lumi-codex" 0
  else
    report "root basename is not lumi-codex" 1
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
  version="$(receipt_key "$receipt" version)"
  target="$(receipt_key "$receipt" target)"
  bin_sha256="$(receipt_key "$receipt" bin_sha256)"
  release_dir="$(receipt_key "$receipt" release_dir)"
  activated="$(receipt_key "$receipt" activated)"
  profile="$(receipt_key "$receipt" profile)"

  if [ "$release_dir" = "$cur" ]; then
    report "receipt release_dir matches current" 0
  else
    report "receipt release_dir ($release_dir) does not match current ($cur)" 1
  fi

  actual_current="$(readlink "$root/current" 2>/dev/null || true)"
  if [ "$actual_current" = "releases/$cur" ]; then
    report "current symlink points to releases/$cur" 0
  else
    report "current symlink drift (got: $actual_current)" 1
  fi

  if release_dir_is_complete "$root/releases/$cur" "$version" "$target"; then
    report "current release directory is complete" 0
  else
    report "current release directory is incomplete or missing" 1
  fi

  cur_rpath="$root/receipts/$cur.receipt"
  if validate_receipt "$cur_rpath" "$RELEASE_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" &&
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
  else
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
  fi

  if [ -f "$root/manager/lumi-install.sh" ] && [ -x "$root/manager/lumi-install.sh" ]; then
    report "manager copy exists and is executable" 0
  else
    report "manager copy is missing or not executable" 1
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
    if [ -n "$profile" ] && [ "$profile" != "-" ] && [ -f "$profile" ] &&
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
    step "current: $cur"
    if [ "$prev" != "-" ] && [ -n "$prev" ]; then
      step "previous: $prev"
    fi
    if [ "$activated" = "yes" ] && [ -n "$profile" ] && [ "$profile" != "-" ]; then
      step "activation: enabled (profile: $profile)"
    else
      step "activation: disabled"
    fi
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
  acquire_install_lock

  receipt="$root/receipts/current.receipt"
  validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "Refusing rollback: current.receipt is missing or invalid."

  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  [ -n "$cur" ] || die "Refusing rollback: current release missing from receipt."
  if [ -z "$prev" ] || [ "$prev" = "-" ]; then
    die "Nothing to roll back to (no previous release recorded)."
  fi

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

  switch_link current "releases/$prev"
  if [ "$cur" != "$prev" ]; then
    switch_link previous "releases/$cur"
  fi
  write_current_receipt "$prev" "$cur" "$activated" "$profile"

  step "Rolled back: current=$prev previous=$cur"
}

cmd_activate() {
  [ -L "$root/current" ] || die "Lumi Codex is not installed; run: lumi-codex manage install"
  validate_receipt "$root/receipts/current.receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "current.receipt is missing or invalid; run: lumi-codex manage doctor"
  [ "$(readlink "$root/shim/lumi-codex" 2>/dev/null || true)" = "../manager/lumi-install.sh" ] ||
    die "Lumi Codex shim launcher is missing or not the owned symlink; run: lumi-codex manage install"
  [ "$(readlink "$root/shim/codex" 2>/dev/null || true)" = "../current/bin/codex" ] ||
    die "Lumi Codex shim codex is missing, drifted, or an unknown symlink; run: lumi-codex manage doctor"

  acquire_install_lock
  profile="$(pick_profile)"
  path_line="export PATH=\"$root/shim:\$PATH\""
  profile_activate
  update_activation yes "$profile"
  step "Lumi Codex activated: $root/shim added to PATH in $profile"
}

cmd_deactivate() {
  [ -e "$root/current" ] || die "Lumi Codex is not installed."
  validate_receipt "$root/receipts/current.receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "current.receipt is missing or invalid; run: lumi-codex manage doctor"

  acquire_install_lock
  profile="$(receipt_key "$root/receipts/current.receipt" profile)"
  case "$profile" in
    "" | -) profile="$(pick_profile)" ;;
  esac
  path_line="export PATH=\"$root/shim:\$PATH\""
  profile_deactivate
  update_activation no "-"
  step "Lumi Codex deactivated: PATH block removed from $profile"
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

  acquire_install_lock

  receipt="$root/receipts/current.receipt"
  validate_receipt "$receipt" "$CURRENT_RECEIPT_KEYS" "$CURRENT_RECEIPT_KEYS" ||
    die "Refusing uninstall: current.receipt is missing or invalid; ownership cannot be proven."

  [ "$(receipt_key "$receipt" root)" = "$root" ] ||
    die "Refusing uninstall: receipt root does not match $root."

  cur="$(receipt_key "$receipt" current)"
  prev="$(receipt_key "$receipt" previous)"
  profile="$(receipt_key "$receipt" profile)"
  [ -n "$cur" ] || die "Refusing uninstall: current release missing from receipt."

  [ "$(readlink "$root/current" 2>/dev/null || true)" = "releases/$cur" ] ||
    die "Refusing uninstall: current symlink does not match the receipt."
  [ -d "$root/releases/$cur" ] || die "Refusing uninstall: current release directory is missing."
  [ "$(readlink "$root/shim/codex" 2>/dev/null || true)" = "../current/bin/codex" ] ||
    die "Refusing uninstall: shim/codex is not the owned symlink (unknown symlink)."
  [ "$(readlink "$root/shim/lumi-codex" 2>/dev/null || true)" = "../manager/lumi-install.sh" ] ||
    die "Refusing uninstall: shim/lumi-codex is not the owned symlink (unknown symlink)."
  [ -f "$root/manager/lumi-install.sh" ] || die "Refusing uninstall: manager copy is missing."

  if [ "$prev" != "-" ] && [ -n "$prev" ]; then
    [ "$(readlink "$root/previous" 2>/dev/null || true)" = "releases/$prev" ] ||
      die "Refusing uninstall: previous symlink does not match the receipt."
  elif [ -e "$root/previous" ] || [ -L "$root/previous" ]; then
    die "Refusing uninstall: previous symlink exists but the receipt records no previous release."
  fi

  path_line="export PATH=\"$root/shim:\$PATH\""
  if [ -n "$profile" ] && [ "$profile" != "-" ]; then
    case "$(profile_block_state)" in
      exact)
        profile_deactivate
        ;;
      drift)
        warn "Leaving drifted PATH block in $profile; it is not the exactly owned block."
        ;;
      none)
        ;;
    esac
  fi

  rm -rf "$root/tmp" "$root/receipts" "$root/releases" "$root/manager" "$root/shim" \
    "$root/current" "$root/previous" "$root/install.lock" "$root/install.lock.d"
  if rmdir "$root" 2>/dev/null; then
    step "Lumi Codex uninstalled (removed $root)."
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
          --no-activate)
            no_activate=1
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
resolve_self
require_command mktemp
require_command tar

tmp_dir="$(mktemp -d 2>/dev/null || printf '%s\n' "${TMPDIR:-/tmp}/lumi-codex.$$")"
mkdir -p "$tmp_dir"

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
