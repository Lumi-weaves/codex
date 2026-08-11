#!/bin/sh

# Entry point for a manually built Lumi Codex installer kit. The kit carries
# one canonical package plus its checksum manifest, so installation itself is
# offline. Takeover remains an explicit, reversible choice after the verified
# side-by-side install succeeds.

set -eu

script_dir="$(CDPATH= cd -P -- "$(dirname -- "$0")" && pwd)"
version_file="$script_dir/VERSION"
target_file="$script_dir/TARGET"
package_checksums="$script_dir/codex-package_SHA256SUMS"
canonical_installer="$script_dir/install.sh"
takeover_helper="$script_dir/takeover.sh"

prompt=auto
takeover_cli=ask
takeover_desktop=ask
side_by_side=no
explicit_takeover=no
skipped_prompt=no

usage() {
  cat <<'EOF'
Usage: install-lumi.sh [OPTIONS]

Installs the package carried by this manual CI kit. By default an interactive
terminal is asked whether Lumi should also take over the `codex` command and,
on macOS, the open-source backend launched by Codex Desktop.

Options:
  --side-by-side       Install only `lumi-codex`; do not offer takeover.
  --takeover-cli       Take over the `codex` command without prompting.
  --takeover-desktop   Take over the macOS Desktop backend without prompting
                       (also ensures CLI takeover).
  --no-prompt          Do not ask questions; apply only explicit takeover flags.
  -h, --help           Show this help.

Environment:
  LUMI_ROOT, LUMI_INSTALL_DIR, XDG_DATA_HOME, and XDG_STATE_HOME select the
  same user-owned paths documented by the canonical Lumi installer.

The installer never modifies CODEX_HOME, authentication, history, shell
profiles, or the signed Codex Desktop application.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --side-by-side)
      prompt=no
      takeover_cli=no
      takeover_desktop=no
      side_by_side=yes
      ;;
    --takeover-cli)
      takeover_cli=yes
      explicit_takeover=yes
      ;;
    --takeover-desktop)
      takeover_desktop=yes
      explicit_takeover=yes
      ;;
    --no-prompt)
      prompt=no
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

if [ "$side_by_side" = yes ] && [ "$explicit_takeover" = yes ]; then
  echo "--side-by-side cannot be combined with takeover flags." >&2
  exit 1
fi

for required in \
  "$version_file" \
  "$target_file" \
  "$package_checksums" \
  "$canonical_installer" \
  "$takeover_helper"
do
  if [ ! -f "$required" ] || [ -L "$required" ]; then
    echo "Installer kit is incomplete or unsafe: $required" >&2
    exit 1
  fi
done

version="$(cat "$version_file")"
target="$(cat "$target_file")"
package_archive="$script_dir/codex-package-$target.tar.gz"
if [ ! -f "$package_archive" ] || [ -L "$package_archive" ]; then
  echo "Installer kit package is missing or unsafe: $package_archive" >&2
  exit 1
fi

sh "$canonical_installer" \
  --release "$version" \
  --target "$target" \
  --package-archive "$package_archive" \
  --checksum-manifest "$package_checksums"

root="${LUMI_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex}"
manager_dir="$root/tools"
manager="$manager_dir/takeover.sh"
manager_marker="$manager_dir/.lumi-owner"
if [ -L "$manager_dir" ] || { [ -e "$manager_dir" ] && [ ! -d "$manager_dir" ]; }; then
  echo "Refusing unsafe takeover helper directory: $manager_dir" >&2
  exit 1
fi
if [ -d "$manager_dir" ]; then
  if [ ! -f "$manager_marker" ] || [ -L "$manager_marker" ] || \
    [ "$(cat "$manager_marker" 2>/dev/null || true)" != "lumi-codex-takeover-tools-v1" ]; then
    echo "Refusing to replace an unowned takeover helper directory: $manager_dir" >&2
    exit 1
  fi
else
  mkdir -p "$manager_dir"
  printf '%s\n' "lumi-codex-takeover-tools-v1" >"$manager_marker"
  chmod 0600 "$manager_marker"
fi
if [ -e "$manager" ] || [ -L "$manager" ]; then
  if [ -L "$manager" ] || [ ! -f "$manager" ]; then
    echo "Refusing unsafe takeover helper path: $manager" >&2
    exit 1
  fi
  if ! grep -Fxq '# lumi-codex-takeover-helper-v1' "$manager"; then
    echo "Refusing to replace foreign takeover helper: $manager" >&2
    exit 1
  fi
fi
manager_tmp="$manager_dir/.takeover.$$"
trap 'rm -f "$manager_tmp"' EXIT INT TERM
cp "$takeover_helper" "$manager_tmp"
chmod 0755 "$manager_tmp"
mv -f "$manager_tmp" "$manager"
trap - EXIT INT TERM

ask_yes_no() {
  question="$1"
  printf '%s [y/N] ' "$question"
  if ! IFS= read -r answer; then
    answer=""
  fi
  case "$answer" in
    y | Y | yes | YES | Yes) return 0 ;;
    *) return 1 ;;
  esac
}

host_os="$(uname -s)"
if [ "$host_os" != Darwin ] && [ "$takeover_desktop" = ask ]; then
  takeover_desktop=no
fi

if [ "$prompt" = auto ]; then
  if [ -t 0 ] && [ -t 1 ]; then
    if [ "$takeover_cli" = ask ]; then
      if ask_yes_no "Let Lumi Codex take over the 'codex' command?"; then
        takeover_cli=yes
      else
        takeover_cli=no
      fi
    fi

    if [ "$host_os" = Darwin ] && [ "$takeover_desktop" = ask ]; then
      if ask_yes_no "Let Codex Desktop launch the Lumi Codex backend (also installs the stable 'codex' link)?"; then
        takeover_desktop=yes
      else
        takeover_desktop=no
      fi
    fi
  else
    skipped_prompt=yes
    [ "$takeover_cli" != ask ] || takeover_cli=no
    [ "$takeover_desktop" != ask ] || takeover_desktop=no
  fi
fi

if [ "$takeover_desktop" = yes ]; then
  sh "$manager" desktop
elif [ "$takeover_cli" = yes ]; then
  sh "$manager" cli
fi

if [ "$skipped_prompt" = yes ]; then
  printf '%s\n' "Interactive takeover questions were skipped in this non-interactive terminal."
  printf '%s\n' "Run '$manager cli' or '$manager desktop' when you are ready."
fi

printf '%s\n' "Rollback any accepted takeover with: $manager rollback"
