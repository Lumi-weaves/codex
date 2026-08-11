#!/bin/sh
# lumi-codex-takeover-helper-v1

# Reversible Lumi CLI and macOS CODEX_CLI_PATH takeover. Never modifies
# CODEX_HOME, shell profiles, or Codex Desktop itself; receipts are parsed as
# data and never sourced or eval'd.

set -eu

RECEIPT_MARKER="lumi-codex-takeover-receipt"
RECEIPT_VERSION="1"
PLIST_LABEL="io.lumi.codex-cli-path"
CLI_NAMES="codex codex-code-mode-host"

LUMI_ROOT="${LUMI_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/lumi-codex}"
LUMI_INSTALL_DIR="${LUMI_INSTALL_DIR:-$HOME/.local/bin}"
STATE_ROOT="${LUMI_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/lumi-codex}"
TAKEOVER_DIR="$STATE_ROOT/takeover"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$LAUNCH_AGENTS_DIR/$PLIST_LABEL.plist"

os=""
LINK_TMP=""
RECEIPT_TMP=""
BACKUP_TMP=""
RESTORE_TMP=""
PLIST_TMP=""
DESKTOP_RECEIPT_TMP=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
}

fail() {
  printf 'takeover.sh: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  for tmp_file in "$LINK_TMP" "$RECEIPT_TMP" "$BACKUP_TMP" "$RESTORE_TMP" "$PLIST_TMP" "$DESKTOP_RECEIPT_TMP"; do
    if [ -n "$tmp_file" ]; then
      rm -f "$tmp_file" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT INT TERM

has_unsafe_chars() {
  value="$1"

  case "$value" in
    *"'"*) return 0 ;;
  esac

  if printf '%s' "$value" | grep -q '[[:cntrl:]]'; then
    return 0
  fi

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
    *) fail "$label must be an absolute path (got: $value)" ;;
  esac
  if has_unsafe_chars "$value"; then
    fail "$label contains control characters or quotes; refusing."
  fi
}

validate_config() {
  validate_abs_path "$LUMI_ROOT" "LUMI_ROOT"
  validate_abs_path "$LUMI_INSTALL_DIR" "LUMI_INSTALL_DIR"
  validate_abs_path "$STATE_ROOT" "Lumi state root"
  if [ -L "$LUMI_ROOT" ]; then
    fail "Refusing to operate on symlinked root $LUMI_ROOT."
  fi
}

detect_os() {
  case "$(uname -s 2>/dev/null || true)" in
    Darwin) os="darwin" ;;
    Linux) os="linux" ;;
    *) os="unknown" ;;
  esac
}

require_canonical_install() {
  if [ ! -d "$LUMI_ROOT" ]; then
    fail "Lumi Codex root $LUMI_ROOT not found; run install.sh first."
  fi
  for name in $CLI_NAMES; do
    backend="$LUMI_ROOT/current/bin/$name"
    if [ ! -f "$backend" ] || [ ! -x "$backend" ]; then
      fail "Canonical install incomplete: $backend is missing or not executable. Run install.sh first."
    fi
  done
}

prepare_install_dir() {
  if [ -L "$LUMI_INSTALL_DIR" ]; then
    fail "Refusing to install links through symlinked directory $LUMI_INSTALL_DIR."
  fi
  if [ -e "$LUMI_INSTALL_DIR" ] && [ ! -d "$LUMI_INSTALL_DIR" ]; then
    fail "$LUMI_INSTALL_DIR exists and is not a directory."
  fi
  mkdir -p "$LUMI_INSTALL_DIR"
}

prepare_state_dir() {
  if [ -L "$STATE_ROOT" ]; then
    fail "Refusing to use symlinked state directory $STATE_ROOT."
  fi
  if [ -e "$STATE_ROOT" ] && [ ! -d "$STATE_ROOT" ]; then
    fail "$STATE_ROOT exists and is not a directory."
  fi
  if [ -L "$TAKEOVER_DIR" ]; then
    fail "Refusing to use symlinked takeover directory $TAKEOVER_DIR."
  fi
  if [ -e "$TAKEOVER_DIR" ] && [ ! -d "$TAKEOVER_DIR" ]; then
    fail "$TAKEOVER_DIR exists and is not a directory."
  fi
  mkdir -p "$TAKEOVER_DIR"
  chmod 700 "$STATE_ROOT" "$TAKEOVER_DIR"
}

file_mode() {
  mode="$(stat -c '%a' "$1" 2>/dev/null || true)"
  if [ -z "$mode" ]; then
    mode="$(stat -f '%Lp' "$1" 2>/dev/null || true)"
  fi
  case "$mode" in
    '' | *[!0-7]*) fail "Could not determine the file mode of $1." ;;
  esac
  printf '%s\n' "$mode"
}

replace_path_with_symlink() {
  link_path="$1"
  link_target="$2"
  tmp_link="$3"

  LINK_TMP="$tmp_link"
  rm -f "$tmp_link"
  ln -s "$link_target" "$tmp_link"

  if mv -Tf "$tmp_link" "$link_path" 2>/dev/null; then
    LINK_TMP=""
    return
  fi

  if mv -hf "$tmp_link" "$link_path" 2>/dev/null; then
    LINK_TMP=""
    return
  fi

  rm -f "$link_path"
  mv -f "$tmp_link" "$link_path"
  LINK_TMP=""
}

receipt_field() {
  receipt_file="$1"
  receipt_key="$2"

  awk -v key="$receipt_key" '
    index($0, key "=") == 1 {
      print substr($0, length(key) + 2)
      exit
    }
  ' "$receipt_file"
}

utc_now() {
  date -u +%Y-%m-%dT%H:%M:%SZ
}

write_cli_receipt() {
  name="$1"
  link_path="$2"
  link_target="$3"
  prior_kind="$4"
  prior_target="$5"
  backup_name="$6"
  prior_mode="$7"

  receipt_path="$TAKEOVER_DIR/cli.$name.receipt"
  RECEIPT_TMP="$TAKEOVER_DIR/.cli.$name.receipt.$$"
  {
    printf 'format=%s\n' "$RECEIPT_MARKER"
    printf 'version=%s\n' "$RECEIPT_VERSION"
    printf 'command=cli\n'
    printf 'name=%s\n' "$name"
    printf 'link_path=%s\n' "$link_path"
    printf 'link_target=%s\n' "$link_target"
    printf 'prior_kind=%s\n' "$prior_kind"
    if [ "$prior_kind" = "symlink" ]; then
      printf 'prior_target=%s\n' "$prior_target"
    fi
    if [ "$prior_kind" = "regular" ]; then
      printf 'backup=%s\n' "$backup_name"
      printf 'prior_mode=%s\n' "$prior_mode"
    fi
    printf 'created_utc=%s\n' "$(utc_now)"
  } >"$RECEIPT_TMP"
  chmod 600 "$RECEIPT_TMP"
  mv -f "$RECEIPT_TMP" "$receipt_path"
  RECEIPT_TMP=""
}

validate_cli_receipt() {
  name="$1"
  receipt_path="$TAKEOVER_DIR/cli.$name.receipt"

  if [ -L "$receipt_path" ] || [ ! -f "$receipt_path" ]; then
    fail "Refusing to read non-regular takeover receipt at $receipt_path."
  fi
  if [ "$(receipt_field "$receipt_path" format)" != "$RECEIPT_MARKER" ] ||
    [ "$(receipt_field "$receipt_path" version)" != "$RECEIPT_VERSION" ] ||
    [ "$(receipt_field "$receipt_path" command)" != "cli" ] ||
    [ "$(receipt_field "$receipt_path" name)" != "$name" ]; then
    fail "Foreign or ambiguous takeover receipt at $receipt_path; refusing."
  fi

  VR_LINK_PATH="$(receipt_field "$receipt_path" link_path)"
  VR_LINK_TARGET="$(receipt_field "$receipt_path" link_target)"
  VR_PRIOR_KIND="$(receipt_field "$receipt_path" prior_kind)"
  VR_PRIOR_TARGET="$(receipt_field "$receipt_path" prior_target)"
  VR_BACKUP="$(receipt_field "$receipt_path" backup)"
  VR_PRIOR_MODE="$(receipt_field "$receipt_path" prior_mode)"

  validate_abs_path "$VR_LINK_PATH" "receipt link_path"
  validate_abs_path "$VR_LINK_TARGET" "receipt link_target"

  case "$VR_PRIOR_KIND" in
    missing) ;;
    symlink)
      case "$VR_PRIOR_TARGET" in
        '') fail "Receipt $receipt_path is missing prior_target." ;;
        -*) fail "Receipt $receipt_path has an unsafe prior_target." ;;
      esac
      if has_unsafe_chars "$VR_PRIOR_TARGET"; then
        fail "Receipt $receipt_path has an unsafe prior_target."
      fi
      ;;
    regular)
      case "$VR_BACKUP" in
        '' | */*) fail "Receipt $receipt_path has an invalid backup name." ;;
      esac
      if has_unsafe_chars "$VR_BACKUP"; then
        fail "Receipt $receipt_path has an invalid backup name."
      fi
      case "$VR_PRIOR_MODE" in
        '' | *[!0-7]*) fail "Receipt $receipt_path has an invalid prior_mode." ;;
      esac
      if [ ! -f "$TAKEOVER_DIR/$VR_BACKUP" ] || [ -L "$TAKEOVER_DIR/$VR_BACKUP" ]; then
        fail "Receipt $receipt_path references missing backup $VR_BACKUP."
      fi
      ;;
    *)
      fail "Foreign or ambiguous takeover receipt at $receipt_path (unknown prior_kind)."
      ;;
  esac
}

cmd_cli() {
  validate_config
  require_canonical_install
  prepare_install_dir
  prepare_state_dir

  plan_actions=""
  for name in $CLI_NAMES; do
    link_path="$LUMI_INSTALL_DIR/$name"
    link_target="$LUMI_ROOT/current/bin/$name"
    receipt_path="$TAKEOVER_DIR/cli.$name.receipt"

    EX_TARGET=""
    if [ -L "$link_path" ]; then
      EX_KIND="symlink"
      EX_TARGET="$(readlink "$link_path" 2>/dev/null || true)"
    elif [ ! -e "$link_path" ]; then
      EX_KIND="missing"
    elif [ -f "$link_path" ]; then
      EX_KIND="regular"
    elif [ -d "$link_path" ]; then
      fail "Refusing to replace directory at $link_path."
    else
      fail "Refusing to replace special file at $link_path."
    fi

    if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
      validate_cli_receipt "$name"
      if [ "$VR_LINK_PATH" != "$link_path" ]; then
        fail "Receipt $receipt_path was written for $VR_LINK_PATH, not $link_path; refusing."
      fi
      if [ "$EX_KIND" = "symlink" ] && [ "$EX_TARGET" = "$VR_LINK_TARGET" ]; then
        step "Already taken over: $link_path -> $VR_LINK_TARGET"
      else
        fail "Takeover receipt exists but $link_path no longer points to $VR_LINK_TARGET; state drifted, refusing."
      fi
    elif [ "$EX_KIND" = "symlink" ] && [ "$EX_TARGET" = "$link_target" ]; then
      plan_actions="$plan_actions $name:record"
    else
      plan_actions="$plan_actions $name:install"
    fi
  done

  for entry in $plan_actions; do
    name="${entry%%:*}"
    action="${entry#*:}"
    link_path="$LUMI_INSTALL_DIR/$name"
    link_target="$LUMI_ROOT/current/bin/$name"

    prior_kind="missing"
    prior_target=""
    backup_name=""
    prior_mode=""
    if [ -L "$link_path" ]; then
      prior_kind="symlink"
      prior_target="$(readlink "$link_path" 2>/dev/null || true)"
      case "$prior_target" in
        -*) fail "Refusing to back up unsafe symlink target at $link_path." ;;
      esac
      if has_unsafe_chars "$prior_target"; then
        fail "Refusing to back up unsafe symlink target at $link_path."
      fi
    elif [ -f "$link_path" ]; then
      prior_kind="regular"
      prior_mode="$(file_mode "$link_path")"
      backup_name="cli.$name.backup"
      BACKUP_TMP="$TAKEOVER_DIR/.$backup_name.$$"
      cp "$link_path" "$BACKUP_TMP"
      chmod 600 "$BACKUP_TMP"
      mv -f "$BACKUP_TMP" "$TAKEOVER_DIR/$backup_name"
      BACKUP_TMP=""
    fi

    write_cli_receipt "$name" "$link_path" "$link_target" "$prior_kind" "$prior_target" "$backup_name" "$prior_mode"

    if [ "$action" = "install" ]; then
      replace_path_with_symlink "$link_path" "$link_target" "$LUMI_INSTALL_DIR/.lumi-takeover.$name.$$"
      step "Installed $link_path -> $link_target"
    else
      step "Recorded existing takeover link $link_path"
    fi
  done

  step "CLI takeover complete. Roll back with: takeover.sh rollback"
}

cli_receipt_cleanup() {
  for name in $CLI_NAMES; do
    receipt_path="$TAKEOVER_DIR/cli.$name.receipt"
    [ -f "$receipt_path" ] || continue
    backup_name="$(receipt_field "$receipt_path" backup)"
    if [ -n "$backup_name" ]; then
      rm -f "$TAKEOVER_DIR/$backup_name"
    fi
    rm -f "$receipt_path"
  done
  rmdir "$TAKEOVER_DIR" 2>/dev/null || true
  rmdir "$STATE_ROOT" 2>/dev/null || true
}

preflight_cli_rollback() {
  CLI_ROLLBACK_ANY=0
  for name in $CLI_NAMES; do
    receipt_path="$TAKEOVER_DIR/cli.$name.receipt"
    link_path="$LUMI_INSTALL_DIR/$name"
    link_target="$LUMI_ROOT/current/bin/$name"

    if [ ! -e "$receipt_path" ] && [ ! -L "$receipt_path" ]; then
      if [ -L "$link_path" ] && [ "$(readlink "$link_path" 2>/dev/null || true)" = "$link_target" ]; then
        fail "Found $link_path pointing to $link_target without a takeover receipt; refusing to touch it."
      fi
      continue
    fi

    validate_cli_receipt "$name"
    if [ "$VR_LINK_PATH" != "$link_path" ]; then
      fail "Receipt $receipt_path was written for $VR_LINK_PATH, not $link_path; refusing."
    fi
    CLI_ROLLBACK_ANY=1

    if [ ! -L "$link_path" ] || [ "$(readlink "$link_path" 2>/dev/null || true)" != "$VR_LINK_TARGET" ]; then
      fail "Takeover link has drifted: $link_path does not point to $VR_LINK_TARGET; refusing to roll back."
    fi
  done
}

rollback_cli() {
  preflight_cli_rollback

  if [ "$CLI_ROLLBACK_ANY" = 0 ]; then
    step "No CLI takeover state found; nothing to roll back."
    return
  fi

  for name in $CLI_NAMES; do
    receipt_path="$TAKEOVER_DIR/cli.$name.receipt"
    [ -e "$receipt_path" ] || [ -L "$receipt_path" ] || continue
    validate_cli_receipt "$name"
    link_path="$LUMI_INSTALL_DIR/$name"

    case "$VR_PRIOR_KIND" in
      missing)
        rm -f "$link_path"
        step "Removed takeover link $link_path"
        ;;
      symlink)
        replace_path_with_symlink "$link_path" "$VR_PRIOR_TARGET" "$LUMI_INSTALL_DIR/.lumi-takeover.$name.$$"
        step "Restored $link_path -> $VR_PRIOR_TARGET"
        ;;
      regular)
        RESTORE_TMP="$LUMI_INSTALL_DIR/.lumi-restore.$name.$$"
        cp "$TAKEOVER_DIR/$VR_BACKUP" "$RESTORE_TMP"
        chmod "$VR_PRIOR_MODE" "$RESTORE_TMP"
        rm -f "$link_path"
        mv -f "$RESTORE_TMP" "$link_path"
        RESTORE_TMP=""
        step "Restored $link_path from backup"
        ;;
    esac
  done

  cli_receipt_cleanup
  step "CLI rollback complete."
}

plist_is_owned_at() {
  plist_path="$1"

  [ -f "$plist_path" ] && [ ! -L "$plist_path" ] || return 1
  grep -Fq -- "<string>$PLIST_LABEL</string>" "$plist_path" || return 1
  grep -Fq -- '<string>/bin/launchctl</string>' "$plist_path" || return 1
  grep -Fq -- '<string>setenv</string>' "$plist_path" || return 1
  grep -Fq -- '<string>CODEX_CLI_PATH</string>' "$plist_path" || return 1
}

plist_points_to() {
  plist_path="$1"
  cli_link="$2"
  grep -Fq -- "<string>$cli_link</string>" "$plist_path"
}

check_competing_desktop_agents() {
  cli_link="$1"
  [ -d "$LAUNCH_AGENTS_DIR" ] || return 0

  for candidate in "$LAUNCH_AGENTS_DIR"/*.plist; do
    [ -f "$candidate" ] || continue
    [ "$candidate" != "$PLIST_PATH" ] || continue
    grep -Fq '<string>setenv</string>' "$candidate" || continue
    grep -Fq '<string>CODEX_CLI_PATH</string>' "$candidate" || continue
    if grep -Fq "<string>$cli_link</string>" "$candidate"; then
      step "Compatible existing CODEX_CLI_PATH LaunchAgent: $candidate"
    else
      fail "Another LaunchAgent manages CODEX_CLI_PATH at $candidate; align or disable it before desktop takeover."
    fi
  done
}

write_plist() {
  cli_link="$1"

  case "$cli_link" in
    *[\<\>\&]*) fail "Path $cli_link contains characters that cannot be represented safely in the LaunchAgent plist." ;;
  esac
  if [ -L "$LAUNCH_AGENTS_DIR" ]; then
    fail "Refusing to write through symlinked directory $LAUNCH_AGENTS_DIR."
  fi
  if [ -e "$LAUNCH_AGENTS_DIR" ] && [ ! -d "$LAUNCH_AGENTS_DIR" ]; then
    fail "$LAUNCH_AGENTS_DIR exists and is not a directory."
  fi
  mkdir -p "$LAUNCH_AGENTS_DIR"

  PLIST_TMP="$LAUNCH_AGENTS_DIR/.$PLIST_LABEL.plist.$$"
  {
    printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>'
    printf '%s\n' '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
    printf '%s\n' '<plist version="1.0">'
    printf '%s\n' '<dict>'
    printf '%s\n' '  <key>Label</key>'
    printf '  <string>%s</string>\n' "$PLIST_LABEL"
    printf '%s\n' '  <key>ProgramArguments</key>'
    printf '%s\n' '  <array>'
    printf '%s\n' '    <string>/bin/launchctl</string>'
    printf '%s\n' '    <string>setenv</string>'
    printf '%s\n' '    <string>CODEX_CLI_PATH</string>'
    printf '    <string>%s</string>\n' "$cli_link"
    printf '%s\n' '  </array>'
    printf '%s\n' '  <key>RunAtLoad</key>'
    printf '%s\n' '  <true/>'
    printf '%s\n' '</dict>'
    printf '%s\n' '</plist>'
  } >"$PLIST_TMP"
  chmod 600 "$PLIST_TMP"
  mv -f "$PLIST_TMP" "$PLIST_PATH"
  PLIST_TMP=""
}

write_desktop_receipt() {
  prior_env_kind="$1"
  prior_env_value="$2"
  prior_plist_kind="$3"
  cli_link="$4"

  receipt_path="$TAKEOVER_DIR/desktop.receipt"
  DESKTOP_RECEIPT_TMP="$TAKEOVER_DIR/.desktop.receipt.$$"
  {
    printf 'format=%s\n' "$RECEIPT_MARKER"
    printf 'version=%s\n' "$RECEIPT_VERSION"
    printf 'command=desktop\n'
    printf 'cli_path=%s\n' "$cli_link"
    printf 'plist_path=%s\n' "$PLIST_PATH"
    printf 'prior_env_kind=%s\n' "$prior_env_kind"
    if [ "$prior_env_kind" = "set" ]; then
      printf 'prior_env_value=%s\n' "$prior_env_value"
    fi
    printf 'prior_plist_kind=%s\n' "$prior_plist_kind"
    printf 'created_utc=%s\n' "$(utc_now)"
  } >"$DESKTOP_RECEIPT_TMP"
  chmod 600 "$DESKTOP_RECEIPT_TMP"
  mv -f "$DESKTOP_RECEIPT_TMP" "$receipt_path"
  DESKTOP_RECEIPT_TMP=""
}

validate_desktop_receipt() {
  receipt_path="$TAKEOVER_DIR/desktop.receipt"

  if [ -L "$receipt_path" ] || [ ! -f "$receipt_path" ]; then
    fail "Refusing to read non-regular desktop receipt at $receipt_path."
  fi
  if [ "$(receipt_field "$receipt_path" format)" != "$RECEIPT_MARKER" ] ||
    [ "$(receipt_field "$receipt_path" version)" != "$RECEIPT_VERSION" ] ||
    [ "$(receipt_field "$receipt_path" command)" != "desktop" ]; then
    fail "Foreign or ambiguous desktop receipt at $receipt_path; refusing."
  fi

  VD_CLI_PATH="$(receipt_field "$receipt_path" cli_path)"
  VD_PLIST_PATH="$(receipt_field "$receipt_path" plist_path)"
  VD_PRIOR_ENV_KIND="$(receipt_field "$receipt_path" prior_env_kind)"
  VD_PRIOR_ENV_VALUE="$(receipt_field "$receipt_path" prior_env_value)"
  VD_PRIOR_PLIST_KIND="$(receipt_field "$receipt_path" prior_plist_kind)"

  validate_abs_path "$VD_CLI_PATH" "desktop receipt cli_path"
  validate_abs_path "$VD_PLIST_PATH" "desktop receipt plist_path"
  case "$VD_PRIOR_ENV_KIND" in
    set)
      validate_abs_path "$VD_PRIOR_ENV_VALUE" "desktop receipt prior_env_value"
      ;;
    unset)
      [ -z "$VD_PRIOR_ENV_VALUE" ] || fail "Desktop receipt has an unexpected prior_env_value."
      ;;
    *) fail "Foreign or ambiguous desktop receipt at $receipt_path (unknown prior_env_kind)." ;;
  esac
  case "$VD_PRIOR_PLIST_KIND" in
    missing) ;;
    *) fail "Foreign or ambiguous desktop receipt at $receipt_path (unknown prior_plist_kind)." ;;
  esac
}

cmd_desktop() {
  detect_os
  if [ "$os" != "darwin" ]; then
    fail "desktop takeover is macOS-only (detected: $(uname -s 2>/dev/null || true))."
  fi
  validate_abs_path "$HOME" "HOME"
  validate_config
  require_canonical_install
  prepare_install_dir
  prepare_state_dir
  command -v launchctl >/dev/null 2>&1 || fail "launchctl is required for desktop takeover."

  cli_link="$LUMI_INSTALL_DIR/codex"
  receipt_path="$TAKEOVER_DIR/desktop.receipt"
  check_competing_desktop_agents "$cli_link"

  prior_env_kind=""
  prior_env_value=""
  prior_plist_kind=""
  if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
    validate_desktop_receipt
    if [ "$VD_CLI_PATH" != "$cli_link" ] || [ "$VD_PLIST_PATH" != "$PLIST_PATH" ]; then
      fail "Desktop receipt was written for a different configuration; refusing."
    fi
    current_env="$(launchctl getenv CODEX_CLI_PATH 2>/dev/null || true)"
    if [ -n "$current_env" ] && [ "$current_env" != "$cli_link" ]; then
      fail "CODEX_CLI_PATH has drifted from $cli_link (now: $current_env); refusing."
    fi
    prior_env_kind="$VD_PRIOR_ENV_KIND"
    prior_env_value="$VD_PRIOR_ENV_VALUE"
    prior_plist_kind="$VD_PRIOR_PLIST_KIND"
  else
    current_env="$(launchctl getenv CODEX_CLI_PATH 2>/dev/null || true)"
    if [ -n "$current_env" ]; then
      validate_abs_path "$current_env" "existing CODEX_CLI_PATH"
      prior_env_kind="set"
      prior_env_value="$current_env"
    else
      prior_env_kind="unset"
    fi
    if [ -e "$PLIST_PATH" ] || [ -L "$PLIST_PATH" ]; then
      fail "Refusing to replace foreign LaunchAgent plist at $PLIST_PATH without a takeover receipt."
    else
      prior_plist_kind="missing"
    fi
  fi

  if [ -e "$PLIST_PATH" ] || [ -L "$PLIST_PATH" ]; then
    plist_is_owned_at "$PLIST_PATH" || fail "Refusing to replace foreign LaunchAgent plist at $PLIST_PATH."
  fi

  step "Ensuring CLI takeover first"
  cmd_cli

  write_desktop_receipt "$prior_env_kind" "$prior_env_value" "$prior_plist_kind" "$cli_link"
  write_plist "$cli_link"
  launchctl bootout "gui/$(id -u)/$PLIST_LABEL" 2>/dev/null || true
  if ! launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"; then
    fail "Could not load the Desktop LaunchAgent. Its receipt and durable plist remain recoverable; retry in the GUI session or run takeover.sh rollback."
  fi
  launchctl setenv CODEX_CLI_PATH "$cli_link"

  step "Set CODEX_CLI_PATH=$cli_link for the current GUI session"
  step "Persisted CODEX_CLI_PATH through $PLIST_PATH"
  step "Restart Codex Desktop normally to pick up the new CLI; this tool will not quit or restart applications."
}

rollback_desktop_if_recorded() {
  receipt_path="$TAKEOVER_DIR/desktop.receipt"

  if [ ! -e "$receipt_path" ] && [ ! -L "$receipt_path" ]; then
    if [ -e "$PLIST_PATH" ] && plist_is_owned_at "$PLIST_PATH"; then
      warn "Found owned LaunchAgent plist $PLIST_PATH without a desktop receipt; leaving it in place."
    fi
    return 0
  fi

  validate_desktop_receipt
  if [ "$VD_CLI_PATH" != "$LUMI_INSTALL_DIR/codex" ] || [ "$VD_PLIST_PATH" != "$PLIST_PATH" ]; then
    fail "Desktop receipt was written for $VD_CLI_PATH / $VD_PLIST_PATH, not the current configuration; refusing."
  fi

  current_env="$(launchctl getenv CODEX_CLI_PATH 2>/dev/null || true)"
  if [ -n "$current_env" ] && [ "$current_env" != "$VD_CLI_PATH" ]; then
    fail "CODEX_CLI_PATH has drifted from $VD_CLI_PATH (now: $current_env); refusing to roll back desktop state."
  fi

  if [ -e "$VD_PLIST_PATH" ] || [ -L "$VD_PLIST_PATH" ]; then
    plist_is_owned_at "$VD_PLIST_PATH" || fail "Refusing to remove foreign plist at $VD_PLIST_PATH."
    launchctl bootout "gui/$(id -u)/$PLIST_LABEL" 2>/dev/null || true
    rm -f "$VD_PLIST_PATH"
    step "Removed $VD_PLIST_PATH"
  fi

  if [ "$VD_PRIOR_ENV_KIND" = "set" ]; then
    launchctl setenv CODEX_CLI_PATH "$VD_PRIOR_ENV_VALUE"
    step "Restored CODEX_CLI_PATH=$VD_PRIOR_ENV_VALUE"
  else
    launchctl unsetenv CODEX_CLI_PATH
    step "Unset CODEX_CLI_PATH"
  fi

  rm -f "$receipt_path"
  step "Desktop rollback complete. Restart Codex Desktop normally to apply."
}

cmd_rollback() {
  validate_config
  detect_os
  if [ "$os" = "darwin" ]; then
    preflight_cli_rollback
    rollback_desktop_if_recorded
  fi
  rollback_cli
}

cmd_status() {
  validate_config
  detect_os

  printf 'Lumi Codex takeover status\n'
  printf 'install dir: %s\n' "$LUMI_INSTALL_DIR"
  printf 'backend root: %s\n' "$LUMI_ROOT"

  for name in $CLI_NAMES; do
    link_path="$LUMI_INSTALL_DIR/$name"
    link_target="$LUMI_ROOT/current/bin/$name"
    receipt_path="$TAKEOVER_DIR/cli.$name.receipt"

    receipt_state="absent"
    if [ -e "$receipt_path" ] || [ -L "$receipt_path" ]; then
      receipt_state="present"
    fi

    if [ -L "$link_path" ]; then
      current_target="$(readlink "$link_path" 2>/dev/null || true)"
      if [ "$current_target" = "$link_target" ]; then
        printf '%s: ok (symlink -> %s, receipt %s)\n' "$name" "$current_target" "$receipt_state"
      else
        printf '%s: DRIFTED (symlink -> %s, expected %s, receipt %s)\n' "$name" "$current_target" "$link_target" "$receipt_state"
      fi
    elif [ ! -e "$link_path" ]; then
      printf '%s: missing (receipt %s)\n' "$name" "$receipt_state"
    else
      printf '%s: FOREIGN (not a symlink, receipt %s)\n' "$name" "$receipt_state"
    fi
  done

  if [ "$os" = "darwin" ]; then
    current_env="$(launchctl getenv CODEX_CLI_PATH 2>/dev/null || true)"
    if [ -n "$current_env" ]; then
      printf 'CODEX_CLI_PATH: %s\n' "$current_env"
    else
      printf 'CODEX_CLI_PATH: unset\n'
    fi

    if [ ! -e "$PLIST_PATH" ] && [ ! -L "$PLIST_PATH" ]; then
      printf 'plist: missing (%s)\n' "$PLIST_PATH"
    elif plist_is_owned_at "$PLIST_PATH"; then
      if plist_points_to "$PLIST_PATH" "$LUMI_INSTALL_DIR/codex"; then
        printf 'plist: owned, points to %s\n' "$LUMI_INSTALL_DIR/codex"
      else
        printf 'plist: owned, DRIFTED (does not point to %s)\n' "$LUMI_INSTALL_DIR/codex"
      fi
    else
      printf 'plist: FOREIGN (%s)\n' "$PLIST_PATH"
    fi
  fi
}

usage() {
  cat <<EOF
Usage: takeover.sh <command>

Commands:
  cli       Point stable codex/host links at \$LUMI_ROOT/current/bin.
  desktop   macOS: ensure CLI takeover and persist CODEX_CLI_PATH for Desktop.
  rollback  Restore receipt-recorded CLI and Desktop backend state.
  status    Report CLI links and macOS Desktop backend state.

Environment:
  LUMI_ROOT         Install root (default: XDG data/lumi-codex).
  LUMI_INSTALL_DIR  Stable-link directory (default: \$HOME/.local/bin).
  XDG_STATE_HOME    Receipt state parent (default: \$HOME/.local/state).

Never modifies CODEX_HOME, shell profiles, or Codex Desktop itself.
EOF
}

main() {
  if [ "$#" -eq 0 ]; then
    usage >&2
    exit 1
  fi

  command="$1"
  shift
  case "$command" in
    cli | desktop | rollback | status)
      if [ "$#" -ne 0 ]; then
        fail "Command '$command' takes no arguments."
      fi
      ;;
    --help | -h | help)
      usage
      exit 0
      ;;
    *)
      printf 'takeover.sh: Unknown command: %s\n' "$command" >&2
      usage >&2
      exit 1
      ;;
  esac

  case "$command" in
    cli) cmd_cli ;;
    desktop) cmd_desktop ;;
    rollback) cmd_rollback ;;
    status) cmd_status ;;
  esac
}

main "$@"
