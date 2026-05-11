#!/usr/bin/env bash
# Remove the ai.phantommesh.serve LaunchAgent.
#
# Usage: ./scripts/uninstall-launchagent.sh

set -euo pipefail

LABEL="ai.phantommesh.serve"
PLIST_PATH="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  echo "◆ Booting out service…"
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
fi

if [ -f "$PLIST_PATH" ]; then
  echo "◆ Removing $PLIST_PATH"
  rm -f "$PLIST_PATH"
fi

echo "✓ Uninstalled. Any running phantom serve is now stopped."
echo "  (Logs preserved at ~/Library/Logs/phantom-serve.log — delete manually if you want.)"
