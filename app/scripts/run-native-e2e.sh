#!/usr/bin/env bash
# run-native-e2e.sh — orchestrate the macOS native-window E2E:
#   1. ensure a debug app binary WITH the webdriver plugin exists
#   2. start the vite dev server on :5173 (debug app loads frontend from devUrl)
#   3. start tauri-wd on :4444 (W3C bridge → in-app plugin)
#   4. run wdio (launches the real native window, drives WKWebView)
#   5. tear everything down
#
# Honest: every step is gated; a missing binary / unreachable bridge / wdio
# failure makes the script exit non-zero. Output is verbose so a CI/agent can
# read the real pass/fail. Requires: tauri-wd on PATH (cargo install
# tauri-webdriver-automation), node_modules with wdio.
set -uo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$APP_DIR"
export PATH="$HOME/.cargo/bin:$PATH"

BIN="src-tauri/target/debug/phantom-mesh-app"
VITE_PID=""; WD_PID=""
cleanup() {
  [ -n "$WD_PID" ] && kill "$WD_PID" 2>/dev/null || true
  [ -n "$VITE_PID" ] && kill "$VITE_PID" 2>/dev/null || true
  # tauri-wd kills the app on session delete, but belt-and-suspenders:
  pkill -f "target/debug/phantom-mesh-app" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

note() { printf '%s\n' "$*"; }
fail() { note "✗ FAIL: $*"; exit 1; }

command -v tauri-wd >/dev/null 2>&1 || fail "tauri-wd not on PATH (cargo install tauri-webdriver-automation)"
[ -x "$BIN" ] || fail "debug app binary missing at $BIN — build it WITH the webdriver feature: (cd src-tauri && cargo build --bin phantom-mesh-app --features e2e-webdriver)"
[ -x node_modules/.bin/wdio ] || fail "wdio not installed (npm i -D webdriverio @wdio/cli @wdio/local-runner @wdio/mocha-framework)"
note "✓ prereqs: tauri-wd, debug binary, wdio all present"

# vite_up — true if :5173 answers on EITHER IPv4 or IPv6. vite announces
# "Local: http://localhost:5173/" and on macOS may bind only ::1 (localhost
# resolves to IPv6 first), so a hardcoded 127.0.0.1 probe gives a false negative
# even while vite is serving. Check localhost (resolver-chosen) + both literals.
vite_up() {
  curl -s "http://localhost:5173" >/dev/null 2>&1 \
    || curl -s "http://127.0.0.1:5173" >/dev/null 2>&1 \
    || curl -gs "http://[::1]:5173" >/dev/null 2>&1
}

# 1. vite on :5173 (reuse if already up)
if vite_up; then
  note "✓ vite already running on :5173"
else
  note "starting vite on :5173 ..."
  # Use the LOCAL vite binary (node_modules/.bin/vite) — starts in <1s; avoids
  # any `npx` registry round-trip.
  ./node_modules/.bin/vite --port 5173 --strictPort --host >/tmp/native-e2e-vite.log 2>&1 &
  VITE_PID=$!
  for _ in $(seq 1 120); do   # up to 60s
    vite_up && break
    sleep 0.5
  done
  vite_up || fail "vite never came up on :5173 (see /tmp/native-e2e-vite.log)"
  note "✓ vite up on :5173"
fi

# 2. tauri-wd on :4444 (reuse if already up)
if curl -s http://127.0.0.1:4444/status >/dev/null 2>&1; then
  note "✓ tauri-wd already running on :4444"
else
  note "starting tauri-wd on :4444 ..."
  tauri-wd --port 4444 >/tmp/native-e2e-wd.log 2>&1 &
  WD_PID=$!
  for _ in $(seq 1 30); do
    curl -s http://127.0.0.1:4444/status >/dev/null 2>&1 && break
    sleep 0.5
  done
  curl -s http://127.0.0.1:4444/status >/dev/null 2>&1 || fail "tauri-wd never came up on :4444 (see /tmp/native-e2e-wd.log)"
  note "✓ tauri-wd up on :4444"
fi

# 3. run the programmatic WebdriverIO driver (plain node — no mocha/tsx/spec-glob,
#    which mis-resolved the .mjs spec file:// URL under wdio's loader).
note "── running native-window driver (programmatic webdriverio) ──"
node tests/wdio/native-window.mjs
rc=$?
note ""
note "NATIVE-E2E RESULT: $([ $rc -eq 0 ] && echo PASS || echo FAIL) (wdio rc=$rc)"
exit $rc
