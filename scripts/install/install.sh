#!/bin/sh

# Lumi Codex standalone installer (canonical Codex package flow, fork-aware).
#
# Installs Lumi Codex package releases published on the Lumi-weaves/codex
# GitHub Releases into an independent Lumi-owned root
# (${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex), side-by-side with any
# official or package-managed Codex install. It never writes to CODEX_HOME,
# never touches a `codex` binary or shell profile, and installs only a visible
# `lumi-codex` launcher that execs <root>/current/bin/codex so packaged
# resources and the code-mode host stay adjacent to the real binary.
#
# The flow is the canonical precompiled-package installer: GitHub release
# metadata (tag + per-asset SHA-256 digests), a codex-package_SHA256SUMS
# checksum manifest, staged immutable version directories under
# <root>/releases, an atomic `current` symlink switch, an install lock, and
# exact binary-version verification. The GitHub release-metadata digest is
# the trust anchor (no artifact signing yet) and the checksum manifest is the
# second layer; downloads that fail one layer are re-verified against the
# GitHub release metadata before the install fails closed.

set -eu

RELEASE="${LUMI_RELEASE:-latest}"
RELEASES_CONNECT_TIMEOUT=10
RELEASES_METADATA_TIMEOUT=30
RELEASES_ASSET_TIMEOUT=300
TARGET_ALLOWLIST="x86_64-unknown-linux-musl aarch64-unknown-linux-musl aarch64-apple-darwin"
LOCK_STALE_AFTER_SECS=600

BIN_DIR="${LUMI_INSTALL_DIR:-$HOME/.local/bin}"
BIN_PATH="$BIN_DIR/lumi-codex"
ROOT="${LUMI_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex}"
RELEASES_DIR="$ROOT/releases"
CURRENT_LINK="$ROOT/current"
LOCK_FILE="$ROOT/install.lock"
LOCK_DIR="$ROOT/install.lock.d"

target="${LUMI_TARGET:-}"
local_package="${LUMI_PACKAGE_ARCHIVE:-}"
local_checksums="${LUMI_CHECKSUM_MANIFEST:-}"
platform_label=""
lock_kind=""
tmp_dir=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
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

  # Codex SemVer plus the optional Lumi canary suffix, e.g. 0.147.0-lumi.1.
  if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-alpha(\.[0-9]+){0,2}|-beta(\.[0-9]+)?)?(-lumi\.[0-9]+)?$'; then
    echo "Invalid Codex release version: $version. Expected latest or x.y.z[-alpha[.N[.M]]|-beta[.N]][-lumi.N]." >&2
    return 1
  fi
}

has_unsafe_chars() {
  value="$1"

  case "$value" in
    *"'"*) return 0 ;;
  esac

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
    *) echo "$label must be an absolute path (got: $value)" >&2; exit 1 ;;
  esac
  if has_unsafe_chars "$value"; then
    echo "$label contains control characters or quotes; refusing." >&2
    exit 1
  fi
}

validate_target() {
  case " $TARGET_ALLOWLIST " in
    *" $target "*) ;;
    *)
      echo "Unsupported target: $target. Supported targets: $TARGET_ALLOWLIST." >&2
      exit 1
      ;;
  esac
  printf '%s' "$target" | grep -Eq '^[a-z0-9_]+(-[a-z0-9_]+)*$' ||
    { echo "Target contains unsafe characters: $target" >&2; exit 1; }
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --release)
        if [ "$#" -lt 2 ]; then
          echo "--release requires a value." >&2
          exit 1
        fi
        RELEASE="$2"
        shift
        ;;
      --target)
        if [ "$#" -lt 2 ]; then
          echo "--target requires a value." >&2
          exit 1
        fi
        target="$2"
        shift
        ;;
      --package-archive)
        if [ "$#" -lt 2 ]; then
          echo "--package-archive requires a value." >&2
          exit 1
        fi
        local_package="$2"
        shift
        ;;
      --checksum-manifest)
        if [ "$#" -lt 2 ]; then
          echo "--checksum-manifest requires a value." >&2
          exit 1
        fi
        local_checksums="$2"
        shift
        ;;
      --help | -h)
        cat <<EOF
Usage: install.sh [--release VERSION] [--target TARGET]
                  [--package-archive PATH --checksum-manifest PATH]

Environment:
  LUMI_RELEASE          Version to install; overridden by --release. Accepts
                        latest or Lumi tags such as x.y.z-lumi.N or
                        rust-vx.y.z-lumi.N.
  LUMI_TARGET           Package target to install (default: detected
                        platform). One of: $TARGET_ALLOWLIST
  LUMI_INSTALL_DIR      Directory for the visible lumi-codex launcher
                        (default: \$HOME/.local/bin).
  LUMI_ROOT             Lumi Codex install root (default:
                        \${XDG_DATA_HOME:-\$HOME/.local/share}/lumi-codex).
  LUMI_PACKAGE_ARCHIVE  Verified local package archive (offline kit mode).
  LUMI_CHECKSUM_MANIFEST
                        Local codex-package_SHA256SUMS for offline kit mode.

Installs side-by-side: no PATH or shell profile changes, no official codex
binary, CODEX_HOME, or auth/config is touched.
EOF
        exit 0
        ;;
      *)
        echo "Unknown argument: $1" >&2
        exit 1
        ;;
    esac
    shift
  done
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

# Download and verify one asset. GitHub Releases is the only source, so when
# the first-layer digest (or the required manifest lookup) fails, re-resolve
# the GitHub release metadata and verify the already-downloaded bytes against
# the GitHub release-asset digest -- the fork's trust anchor -- before
# failing closed.
download_file_with_fallback() {
  primary_url="$1"
  output="$2"
  expected_digest="$3"
  fallback_asset="$4"
  required_manifest_asset="${5:-}"

  if download_file "$primary_url" "$output" &&
    verify_archive_digest "$output" "$expected_digest" &&
    { [ -z "$required_manifest_asset" ] || package_archive_digest "$required_manifest_asset" "$output" >/dev/null; }; then
    return
  fi

  warn "Could not download or verify $primary_url; re-verifying against GitHub release metadata."
  resolve_release_from_github "$resolved_version"
  fallback_digest="$(release_asset_digest "$fallback_asset")"
  verify_archive_digest "$output" "$fallback_digest"
  if [ -n "$required_manifest_asset" ]; then
    package_archive_digest "$required_manifest_asset" "$output" >/dev/null
  fi
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
  resolved_version="$2"

  printf 'https://github.com/Lumi-weaves/codex/releases/download/rust-v%s/%s\n' "$resolved_version" "$asset"
}

release_metadata_url() {
  resolved_version="$1"

  printf 'https://api.github.com/repos/Lumi-weaves/codex/releases/tags/rust-v%s\n' "$resolved_version"
}

parse_downloaded_release_metadata() {
  requested_release="$1"
  source_name="$2"
  if ! release_metadata="$(printf '%s\n' "$release_json" | parse_release_metadata)"; then
    echo "Could not parse $source_name release metadata for Lumi Codex $requested_release." >&2
    return 1
  fi
}

resolve_metadata_version() {
  release_tag="$(printf '%s\n' "$release_metadata" | awk -F '\t' '$1 == "tag_name" { print $2; exit }')"
  case "$release_tag" in
    rust-v*) metadata_version="${release_tag#rust-v}" ;;
    *) metadata_version="" ;;
  esac
  if [ -z "$metadata_version" ]; then
    echo "Failed to resolve the latest Lumi Codex release version." >&2
    return 1
  fi
  validate_version "$metadata_version"
}

resolve_release_from_github() {
  normalized_version="$1"
  if [ "$normalized_version" = "latest" ]; then
    requested_release="latest"
    metadata_url="https://api.github.com/repos/Lumi-weaves/codex/releases/latest"
  else
    resolved_version="$normalized_version"
    requested_release="$resolved_version"
    metadata_url="$(release_metadata_url "$resolved_version")"
  fi

  if ! release_json="$(download_text "$metadata_url")"; then
    if [ "$normalized_version" = "latest" ]; then
      echo "Could not resolve a stable Lumi Codex release. GitHub excludes prereleases from /releases/latest; pin a canary with --release x.y.z-lumi.N." >&2
      exit 1
    fi
    echo "Could not fetch GitHub release metadata for Lumi Codex $requested_release. GitHub API may be unavailable or rate limited." >&2
    exit 1
  fi

  parse_downloaded_release_metadata "$requested_release" "GitHub"

  if [ "$normalized_version" = "latest" ]; then
    resolve_metadata_version
    resolved_version="$metadata_version"
  fi
}

resolve_release() {
  normalized_version="$(normalize_version "$RELEASE")"
  validate_version "$normalized_version"

  if [ -n "$local_package" ] || [ -n "$local_checksums" ]; then
    if [ -z "$local_package" ] || [ -z "$local_checksums" ]; then
      echo "--package-archive and --checksum-manifest must be provided together." >&2
      exit 1
    fi
    if [ "$normalized_version" = "latest" ]; then
      echo "Offline package installation requires an exact --release version." >&2
      exit 1
    fi
    validate_abs_path "$local_package" "Package archive path"
    validate_abs_path "$local_checksums" "Checksum manifest path"
    if [ ! -f "$local_package" ] || [ -L "$local_package" ]; then
      echo "Package archive must be a regular, non-symlink file: $local_package" >&2
      exit 1
    fi
    if [ ! -f "$local_checksums" ] || [ -L "$local_checksums" ]; then
      echo "Checksum manifest must be a regular, non-symlink file: $local_checksums" >&2
      exit 1
    fi

    resolved_version="$normalized_version"
    asset="codex-package-$target.tar.gz"
    checksum_asset="codex-package_SHA256SUMS"
    if [ "${local_package##*/}" != "$asset" ]; then
      echo "Offline package filename must be $asset." >&2
      exit 1
    fi
    if [ "${local_checksums##*/}" != "$checksum_asset" ]; then
      echo "Offline checksum filename must be $checksum_asset." >&2
      exit 1
    fi
    return
  fi

  resolve_release_from_github "$normalized_version"
  select_release_assets
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

select_release_assets() {
  package_asset="codex-package-$target.tar.gz"
  checksum_asset="codex-package_SHA256SUMS"

  if ! release_asset_exists "$package_asset" ||
    ! release_asset_exists "$checksum_asset"; then
    echo "Could not find Codex package or checksum manifest release assets for Lumi Codex $resolved_version (target $target)." >&2
    return 1
  fi

  asset="$package_asset"
  download_url="$(release_url_for_asset "$asset" "$resolved_version")"
  checksum_url="$(release_url_for_asset "$checksum_asset" "$resolved_version")"
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
    echo "Downloaded Lumi Codex archive checksum did not match expected digest." >&2
    echo "expected: $expected_digest" >&2
    echo "actual:   $actual_digest" >&2
    return 1
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to install Lumi Codex." >&2
    exit 1
  fi
}

mkdir_lock_is_stale() {
  [ -d "$LOCK_DIR" ] || return 1

  pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
  started_at="$(cat "$LOCK_DIR/started_at" 2>/dev/null || true)"
  now="$(date +%s 2>/dev/null || printf '0')"

  case "$started_at" in
    ''|*[!0-9]*)
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
  mkdir -p "$ROOT"

  if [ "$os" = "darwin" ] && command -v lockf >/dev/null 2>&1; then
    : >>"$LOCK_FILE"
    exec 9<>"$LOCK_FILE"
    lockf 9
    lock_kind="lockf"
    return
  fi

  if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK_FILE"
    flock 9
    lock_kind="flock"
    return
  fi

  while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    if mkdir_lock_is_stale; then
      warn "Removing stale installer lock at $LOCK_DIR"
      rm -rf "$LOCK_DIR"
      continue
    fi
    sleep 1
  done

  printf '%s\n' "$$" >"$LOCK_DIR/pid"
  date +%s >"$LOCK_DIR/started_at" 2>/dev/null || true
  lock_kind="mkdir"
}

release_install_lock() {
  if [ "$lock_kind" = "mkdir" ]; then
    rm -rf "$LOCK_DIR" 2>/dev/null || true
  elif [ "$lock_kind" = "flock" ] || [ "$lock_kind" = "lockf" ]; then
    exec 9>&- 2>/dev/null || true
  fi
  lock_kind=""
}

cleanup_stale_install_artifacts() {
  mkdir -p "$RELEASES_DIR" "$ROOT"

  find "$RELEASES_DIR" -mindepth 1 -maxdepth 1 -name '.staging.*' -exec rm -rf {} +
  find "$ROOT" -mindepth 1 -maxdepth 1 -name '.current.*' -exec rm -f {} +

  if [ -d "$BIN_DIR" ]; then
    find "$BIN_DIR" -mindepth 1 -maxdepth 1 -name '.lumi-codex.*' -exec rm -f {} +
  fi
}

replace_path_with_symlink() {
  link_path="$1"
  link_target="$2"
  tmp_link="$3"

  rm -f "$tmp_link"
  ln -s "$link_target" "$tmp_link"

  if mv -Tf "$tmp_link" "$link_path" 2>/dev/null; then
    return
  fi

  if mv -hf "$tmp_link" "$link_path" 2>/dev/null; then
    return
  fi

  rm -f "$link_path"
  mv -f "$tmp_link" "$link_path"
}

version_from_binary() {
  codex_path="$1"

  if [ ! -x "$codex_path" ]; then
    return 1
  fi

  "$codex_path" --version 2>/dev/null | sed -n 's/.* \([0-9][0-9A-Za-z.+-]*\)$/\1/p' | head -n 1
}

current_installed_version() {
  version="$(version_from_binary "$CURRENT_LINK/bin/codex" || true)"
  if [ -n "$version" ]; then
    printf '%s\n' "$version"
    return 0
  fi

  version="$(version_from_binary "$CURRENT_LINK/codex" || true)"
  if [ -n "$version" ]; then
    printf '%s\n' "$version"
    return 0
  fi

  return 0
}

# Reject absolute members, empty names, and any `..` component, and allow
# only regular files and directories inside package archives. This keeps the
# canonical tar extraction from ever writing outside the staging directory.
validate_archive_members() {
  archive="$1"

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

install_package_release() {
  release_dir="$1"
  archive_path="$2"
  stage_release="$RELEASES_DIR/.staging.$(basename "$release_dir").$$"

  mkdir -p "$RELEASES_DIR"
  rm -rf "$stage_release"
  mkdir -p "$stage_release"
  tar -xzf "$archive_path" -C "$stage_release"
  chmod 0755 \
    "$stage_release/bin/codex" \
    "$stage_release/bin/codex-code-mode-host" \
    "$stage_release/codex-path/rg"
  if [ -f "$stage_release/codex-resources/bwrap" ]; then
    chmod 0755 "$stage_release/codex-resources/bwrap"
  fi
  ln -sf "bin/codex" "$stage_release/codex"
  printf '%s\n' "lumi-codex-release-v1" >"$stage_release/.lumi-owner"
  chmod 0600 "$stage_release/.lumi-owner"

  # Fail closed instead of deleting a foreign file or symlink at the release
  # path; a plain directory is our own incomplete state and is replaced.
  if [ -L "$release_dir" ] || { [ -e "$release_dir" ] && [ ! -d "$release_dir" ]; }; then
    echo "Refusing to replace unexpected non-directory at $release_dir" >&2
    exit 1
  fi
  if [ -e "$release_dir" ]; then
    if { [ ! -f "$release_dir/.lumi-owner" ] || [ -L "$release_dir/.lumi-owner" ]; } &&
      { [ ! -f "$release_dir/codex-package.json" ] || [ -L "$release_dir/codex-package.json" ] ||
        ! grep -Eq '"distribution"[[:space:]]*:[[:space:]]*"lumi"' "$release_dir/codex-package.json"; }; then
      echo "Refusing to remove unowned existing release directory at $release_dir" >&2
      exit 1
    fi
    rm -rf "$release_dir"
  fi
  mv "$stage_release" "$release_dir"
}

release_dir_is_complete() {
  release_dir="$1"
  expected_version="$2"
  expected_target="$3"

  [ -d "$release_dir" ] &&
    [ "$(basename "$release_dir")" = "$expected_version-$expected_target" ] ||
    return 1

  # Archive-shipped executables must be regular files, not symlinks.
  [ -f "$release_dir/codex-package.json" ] && [ ! -L "$release_dir/codex-package.json" ] &&
    [ -f "$release_dir/bin/codex" ] && [ ! -L "$release_dir/bin/codex" ] &&
    [ -x "$release_dir/bin/codex" ] &&
    [ -f "$release_dir/bin/codex-code-mode-host" ] && [ ! -L "$release_dir/bin/codex-code-mode-host" ] &&
    [ -x "$release_dir/bin/codex-code-mode-host" ] &&
    [ -x "$release_dir/codex" ] &&
    [ -f "$release_dir/codex-path/rg" ] && [ ! -L "$release_dir/codex-path/rg" ] &&
    [ -x "$release_dir/codex-path/rg" ] ||
    return 1

  case "$expected_target" in
    *linux*)
      [ -f "$release_dir/codex-resources/bwrap" ] && [ ! -L "$release_dir/codex-resources/bwrap" ] &&
        [ -x "$release_dir/codex-resources/bwrap" ] ||
        return 1
      ;;
  esac

  installed_version="$(version_from_binary "$release_dir/bin/codex" || true)"
  [ "$installed_version" = "$expected_version" ]
}

update_current_link() {
  release_dir="$1"

  # Fail closed on foreign or non-symlink state at <root>/current; only a
  # symlink pointing inside our releases directory may be retargeted.
  if [ -e "$CURRENT_LINK" ] || [ -L "$CURRENT_LINK" ]; then
    if [ ! -L "$CURRENT_LINK" ]; then
      echo "Refusing to replace non-symlink at $CURRENT_LINK" >&2
      exit 1
    fi
    case "$(readlink "$CURRENT_LINK" 2>/dev/null || true)" in
      "$RELEASES_DIR"/*) ;;
      *)
        echo "Refusing to retarget foreign symlink at $CURRENT_LINK" >&2
        exit 1
        ;;
    esac
  fi

  tmp_link="$ROOT/.current.$$"
  replace_path_with_symlink "$CURRENT_LINK" "$release_dir" "$tmp_link"
}

launcher_contents() {
  printf '%s\n' '#!/bin/sh'
  printf '%s\n' '# Lumi Codex launcher: execs the verified package entrypoint so'
  printf '%s\n' '# code-mode host and packaged resources stay adjacent to the real binary.'
  printf "exec '%s/current/bin/codex' \"\$@\"\n" "$ROOT"
}

install_visible_launcher() {
  mkdir -p "$BIN_DIR"
  desired="$(launcher_contents)"

  if [ -e "$BIN_PATH" ] || [ -L "$BIN_PATH" ]; then
    if [ -L "$BIN_PATH" ] || [ ! -f "$BIN_PATH" ]; then
      echo "Refusing to replace non-regular file at $BIN_PATH (not a Lumi Codex launcher)." >&2
      exit 1
    fi
    if [ "$(cat "$BIN_PATH" 2>/dev/null || true)" != "$desired" ]; then
      echo "Refusing to overwrite unexpected file at $BIN_PATH; remove it or point LUMI_INSTALL_DIR elsewhere." >&2
      exit 1
    fi
  fi

  tmp_launcher="$BIN_DIR/.lumi-codex.$$"
  rm -f "$tmp_launcher"
  printf '%s\n' "$desired" >"$tmp_launcher"
  chmod 0755 "$tmp_launcher"
  mv -f "$tmp_launcher" "$BIN_PATH"
}

verify_visible_command() {
  "$BIN_PATH" --version >/dev/null
  if [ "$os" = "darwin" ]; then
    [ -x "$CURRENT_LINK/bin/codex-code-mode-host" ]
  fi
}

print_launch_instructions() {
  case ":$PATH:" in
    *":$BIN_DIR:"*)
      step "Current terminal: lumi-codex"
      step "Future terminals: open a new terminal and run: lumi-codex"
      ;;
    *)
      step "Add $BIN_DIR to your PATH, or run the launcher directly:"
      step "  $BIN_PATH"
      ;;
  esac
}

parse_args "$@"

require_command mktemp
require_command tar

validate_abs_path "$ROOT" "LUMI_ROOT"
validate_abs_path "$BIN_DIR" "LUMI_INSTALL_DIR"
if [ -L "$ROOT" ]; then
  echo "Refusing to operate on symlinked root $ROOT (remove the symlink or point LUMI_ROOT at a real directory)." >&2
  exit 1
fi
if [ -L "$BIN_DIR" ]; then
  echo "Refusing to install the launcher through symlinked directory $BIN_DIR." >&2
  exit 1
fi

os=""
case "$(uname -s)" in
  Darwin)
    os="darwin"
    ;;
  Linux)
    os="linux"
    ;;
esac

if [ -z "$target" ]; then
  if [ -z "$os" ]; then
    echo "Lumi Codex packages support macOS and Linux only." >&2
    exit 1
  fi

  case "$(uname -m)" in
    x86_64 | amd64)
      arch="x86_64"
      ;;
    arm64 | aarch64)
      arch="aarch64"
      ;;
    *)
      echo "Unsupported architecture: $(uname -m)" >&2
      exit 1
      ;;
  esac

  if [ "$os" = "darwin" ] && [ "$arch" = "x86_64" ]; then
    echo "Lumi Codex does not publish x86_64 (Intel) macOS binaries; build from source instead." >&2
    exit 1
  fi

  if [ "$os" = "darwin" ]; then
    target="aarch64-apple-darwin"
  else
    if [ "$arch" = "aarch64" ]; then
      target="aarch64-unknown-linux-musl"
    else
      target="x86_64-unknown-linux-musl"
    fi
  fi
fi

validate_target

case "$target" in
  x86_64-unknown-linux-musl) platform_label="Linux (x64)" ;;
  aarch64-unknown-linux-musl) platform_label="Linux (ARM64)" ;;
  aarch64-apple-darwin) platform_label="macOS (Apple Silicon)" ;;
esac

resolve_release
release_name="$resolved_version-$target"
release_dir="$RELEASES_DIR/$release_name"
current_version="$(current_installed_version)"

if [ -n "$current_version" ] && [ "$current_version" != "$resolved_version" ]; then
  step "Updating Lumi Codex CLI from $current_version to $resolved_version"
elif [ -n "$current_version" ]; then
  step "Updating Lumi Codex CLI"
else
  step "Installing Lumi Codex CLI"
fi
step "Detected platform: $platform_label"
step "Resolved version: $resolved_version"

tmp_dir="$(mktemp -d)"
cleanup() {
  release_install_lock
  if [ -n "$tmp_dir" ]; then
    rm -rf "$tmp_dir"
  fi
}
trap cleanup EXIT INT TERM

acquire_install_lock
cleanup_stale_install_artifacts

if ! release_dir_is_complete "$release_dir" "$resolved_version" "$target"; then
  if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
    warn "Found incomplete existing release at $release_dir; reinstalling."
  fi

  if [ -n "$local_package" ]; then
    archive_path="$local_package"
    checksum_path="$local_checksums"
    step "Verifying local Lumi Codex package"
    expected_digest="$(package_archive_digest "$asset" "$checksum_path")"
    verify_archive_digest "$archive_path" "$expected_digest"
  else
    archive_path="$tmp_dir/$asset"
    checksum_path="$tmp_dir/$checksum_asset"

    step "Downloading Lumi Codex CLI"
    checksum_digest="$(release_asset_digest "$checksum_asset")"
    download_file_with_fallback "$checksum_url" "$checksum_path" "$checksum_digest" "$checksum_asset" "$asset"
    expected_digest="$(package_archive_digest "$asset" "$checksum_path")"
    download_file_with_fallback "$download_url" "$archive_path" "$expected_digest" "$asset"
  fi
  validate_archive_members "$archive_path"

  step "Installing standalone package to $release_dir"
  install_package_release "$release_dir" "$archive_path"
fi
if ! release_dir_is_complete "$release_dir" "$resolved_version" "$target"; then
  echo "Installed Codex command did not report expected version $resolved_version." >&2
  exit 1
fi
update_current_link "$release_dir"
install_visible_launcher
verify_visible_command
release_install_lock

print_launch_instructions

printf 'Lumi Codex CLI %s installed successfully.\n' "$resolved_version"
