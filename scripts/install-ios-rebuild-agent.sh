#!/usr/bin/env bash
# Install a per-user macOS LaunchAgent that rebuilds + re-signs the iOS IPA
# every Sunday 03:30, so the free-cert 7-day expiry never bites a sideloaded
# device.
#
# Usage:
#   APPLE_TEAM_ID=YX7U4J39PX ./scripts/install-ios-rebuild-agent.sh
#
# Find your team id: security find-identity -v -p codesigning | grep Apple
# (the 10-char string in parentheses).
#
# Caveats:
#   - The Mac must be awake at the scheduled time (or wake from sleep — pmset
#     can wake for launchd jobs; outside scope of this installer).
#   - Login keychain must be unlocked when the job runs, since codesign needs
#     the private key. If you auto-lock keychain on screensaver, this will
#     prompt the next morning instead.
#   - Apple's automatic provisioning requires network + a logged-in Apple ID
#     in Xcode preferences. This script does not validate that — first run
#     `make ios-rebuild` interactively to confirm it works.

set -euo pipefail

LABEL="ai.phantommesh.ios-rebuild"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$LAUNCH_AGENTS_DIR/$LABEL.plist"
LOG_DIR="$HOME/Library/Logs"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
TMPL="$REPO_ROOT/templates/ai.phantommesh.ios-rebuild.plist.tmpl"

if [ -z "${APPLE_TEAM_ID:-}" ]; then
  echo "✗ Set APPLE_TEAM_ID before running." >&2
  echo "  security find-identity -v -p codesigning | grep Apple" >&2
  exit 1
fi

if [ ! -f "$TMPL" ]; then
  echo "✗ Template missing: $TMPL" >&2
  exit 1
fi

mkdir -p "$LAUNCH_AGENTS_DIR" "$LOG_DIR"

sed \
  -e "s|__WORK_DIR__|$REPO_ROOT|g" \
  -e "s|__HOME__|$HOME|g" \
  -e "s|__APPLE_TEAM_ID__|$APPLE_TEAM_ID|g" \
  "$TMPL" > "$PLIST_PATH"

# Reload (unload-then-load is idempotent; bootout silently no-ops if missing).
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"

echo "✓ Installed $LABEL"
echo "  Plist:    $PLIST_PATH"
echo "  Schedule: every Sunday 03:30 (StartCalendarInterval)"
echo "  Log:      $LOG_DIR/phantom-ios-rebuild.log"
echo ""
echo "Manual run now (to verify it works end-to-end):"
echo "  launchctl kickstart -k gui/$(id -u)/$LABEL"
echo "  tail -f $LOG_DIR/phantom-ios-rebuild.log"
echo ""
echo "Uninstall:"
echo "  launchctl bootout gui/$(id -u)/$LABEL && rm $PLIST_PATH"
