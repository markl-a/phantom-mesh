#!/usr/bin/env bash
# Install phantom serve as a per-user macOS LaunchAgent so it auto-starts at
# login and is restarted (with throttle) if it ever exits non-zero.
#
# Usage:
#   ./scripts/install-launchagent.sh [/path/to/phantom]
#
# If PHANTOM_BIN env or arg 1 is given, that binary is wired into the plist;
# otherwise we try (in order): $HOME/.cargo/bin/phantom, ./core/target/release/phantom,
# `which phantom`.

set -euo pipefail

LABEL="ai.phantommesh.serve"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$LAUNCH_AGENTS_DIR/$LABEL.plist"
LOG_DIR="$HOME/Library/Logs"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
TMPL="$REPO_ROOT/templates/ai.phantommesh.serve.plist.tmpl"

# ── locate phantom binary ────────────────────────────────────────────────────
PHANTOM_BIN="${PHANTOM_BIN:-${1:-}}"
if [ -z "$PHANTOM_BIN" ]; then
  if [ -x "$HOME/.cargo/bin/phantom" ]; then
    PHANTOM_BIN="$HOME/.cargo/bin/phantom"
  elif [ -x "$REPO_ROOT/core/target/release/phantom" ]; then
    PHANTOM_BIN="$REPO_ROOT/core/target/release/phantom"
  else
    PHANTOM_BIN="$(command -v phantom || true)"
  fi
fi

if [ -z "$PHANTOM_BIN" ] || [ ! -x "$PHANTOM_BIN" ]; then
  echo "✗ Cannot locate the phantom binary." >&2
  echo "  Pass it explicitly: ./scripts/install-launchagent.sh /path/to/phantom" >&2
  exit 1
fi

# Resolve symlink to absolute target — launchd does not follow user symlinks
# when the symlink is in $HOME/.cargo/bin and the agent's HOME var resolves
# late.
if [ -L "$PHANTOM_BIN" ]; then
  PHANTOM_BIN="$(readlink -f "$PHANTOM_BIN" 2>/dev/null || readlink "$PHANTOM_BIN")"
fi

WORK_DIR="$REPO_ROOT"

mkdir -p "$LAUNCH_AGENTS_DIR" "$LOG_DIR"

# ── render template ──────────────────────────────────────────────────────────
if [ ! -f "$TMPL" ]; then
  echo "✗ Template not found: $TMPL" >&2
  exit 1
fi

sed \
  -e "s|__PHANTOM_BIN__|$PHANTOM_BIN|g" \
  -e "s|__WORK_DIR__|$WORK_DIR|g" \
  -e "s|__HOME__|$HOME|g" \
  "$TMPL" > "$PLIST_PATH"

echo "◆ Wrote $PLIST_PATH"
echo "  binary:    $PHANTOM_BIN"
echo "  cwd:       $WORK_DIR"
echo "  log:       $LOG_DIR/phantom-serve.log"

# ── (re)load via launchctl ───────────────────────────────────────────────────
DOMAIN="gui/$(id -u)"

# Bootout any prior instance so we always pick up the fresh plist.
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  echo "◆ Booting out existing service…"
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
fi

echo "◆ Bootstrapping service…"
launchctl bootstrap "$DOMAIN" "$PLIST_PATH"
launchctl enable "$DOMAIN/$LABEL"
launchctl kickstart -kp "$DOMAIN/$LABEL" >/dev/null 2>&1 || true

# ── verify ───────────────────────────────────────────────────────────────────
sleep 2
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  PID=$(launchctl print "$DOMAIN/$LABEL" 2>/dev/null | awk '/^\tpid =/ {print $3; exit}')
  echo "✓ Service running (pid: ${PID:-?})"
  echo
  echo "  Verify:    curl http://127.0.0.1:7878/healthz"
  echo "  Logs:      tail -f $LOG_DIR/phantom-serve.log"
  echo "  Uninstall: ./scripts/uninstall-launchagent.sh"
else
  echo "✗ Service did not register. Check $LOG_DIR/phantom-serve.log" >&2
  exit 1
fi
