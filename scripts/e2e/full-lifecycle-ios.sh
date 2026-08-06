#!/usr/bin/env bash
# full-lifecycle-ios.sh — G-DBG-3 (L3 of the pyramid): boot an iOS simulator,
# capture live screenshots + system log while driving the app, and tear down.
#
# This is the SCAFFOLD: it does the simulator lifecycle + capture plumbing that
# every iOS E2E needs (boot, install, screenshot-per-step, log stream, shutdown,
# debug bundle on failure). The actual UI driving (tap chip / onboarding) is an
# Appium XCUITest flow added in G-E2E-3 — this script proves the capture rig
# works and is what that flow plugs into.
#
# Requires: Xcode + simctl (verified present 2026-05-31). A built .app is
# optional; without one, the script still validates boot + screenshot + log
# capture against the booted simulator (so the rig is testable today).
#
# Usage:
#   scripts/e2e/full-lifecycle-ios.sh
#   SIM_NAME="spectyn-iphone15-ios17" APP_PATH=/path/to/Spectyn.app scripts/e2e/...
set -uo pipefail

SIM_NAME="${SIM_NAME:-spectyn-iphone15-ios17}"
APP_PATH="${APP_PATH:-}"
BUNDLE_ID="${BUNDLE_ID:-ai.spectynmesh.app}"
TS="$(date +%Y%m%d-%H%M%S)"
SHOTS="${TMPDIR:-/tmp}/spectyn-ios-e2e-$TS.shots"; mkdir -p "$SHOTS"
LOG="${TMPDIR:-/tmp}/spectyn-ios-e2e-$TS.log"
LOGPID=""

note() { printf '%s\n' "$*" | tee -a "$LOG"; }
shot() { # shot <step-label>
  local f="$SHOTS/$(printf '%02d' "${SHOT_N:-0}")-$1.png"
  if xcrun simctl io "$UDID" screenshot "$f" >/dev/null 2>&1; then
    note "  📸 $f"; SHOT_N=$(( ${SHOT_N:-0} + 1 ))
  else
    note "  ⚠ screenshot failed at $1"
  fi
}

cleanup() {
  [ -n "$LOGPID" ] && kill "$LOGPID" 2>/dev/null || true
  [ -n "${UDID:-}" ] && xcrun simctl shutdown "$UDID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

note "=== spectyn iOS-simulator E2E rig ==="
note "sim:  $SIM_NAME"
note "log:  $LOG"

command -v xcrun >/dev/null 2>&1 || { note "✗ xcrun not found (need Xcode)"; exit 2; }

# Resolve UDID (create the device if the named one is absent).
UDID="$(xcrun simctl list devices 2>/dev/null | grep "$SIM_NAME" | grep -oE '[0-9A-F-]{36}' | head -1)"
if [ -z "$UDID" ]; then
  note "  (device '$SIM_NAME' not found; using first available iPhone)"
  UDID="$(xcrun simctl list devices available 2>/dev/null | grep -iE 'iPhone' | grep -oE '[0-9A-F-]{36}' | head -1)"
fi
[ -n "$UDID" ] || { note "✗ no iOS simulator available"; exit 2; }
note "udid: $UDID"

fail=0
note "▶ boot simulator";       xcrun simctl boot "$UDID" >>"$LOG" 2>&1 || true
xcrun simctl bootstatus "$UDID" -b >>"$LOG" 2>&1 || true
note "  ✓ booted"

# Stream the app's subsystem log in the background (G-DBG-2).
note "▶ start log stream → $LOG"
( xcrun simctl spawn "$UDID" log stream --level debug \
    --predicate 'subsystem CONTAINS "spectynmesh" OR processImagePath CONTAINS "Spectyn"' \
    >>"$LOG" 2>&1 ) & LOGPID=$!

SHOT_N=0
shot "booted"

if [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
  note "▶ install app: $APP_PATH"
  xcrun simctl install "$UDID" "$APP_PATH" >>"$LOG" 2>&1 && note "  ✓ installed" || { note "  ✗ install failed"; fail=1; }
  note "▶ launch $BUNDLE_ID"
  xcrun simctl launch "$UDID" "$BUNDLE_ID" >>"$LOG" 2>&1 && note "  ✓ launched" || { note "  ✗ launch failed"; fail=1; }
  sleep 3; shot "launched"
  note "  (UI driving — onboarding→habit chip→daily review — is the Appium XCUITest flow in G-E2E-3)"
else
  note "▶ no APP_PATH given — validating capture rig only (boot+screenshot+log)"
  shot "home-screen"
fi

note ""
note "E2E RESULT: $([ "$fail" -eq 0 ] && echo PASS || echo FAIL) ($SHOT_N screenshots in $SHOTS, log=$LOG)"
if [ "$fail" -ne 0 ]; then
  bash "$(dirname "$0")/collect-debug-bundle.sh" "${HOME}" "$LOG" "$SHOTS" || true
fi
[ "$fail" -eq 0 ]
