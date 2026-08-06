#!/usr/bin/env bash
# Consolidated Android runtime verifier (T-VER-01 V1–V7) for the spectyn-mesh
# Tauri APK on an x86_64 emulator. Encodes the live checks ayaneo-android runs
# against z13-android's output so a full verification is one command.
#
# Companion to scripts/smoke-android-emulator.sh (which builds + does V1–V3);
# this assumes an APK exists and focuses on running the FULL V1–V7 matrix +
# printing a result table. Use after a clean build (ideally from current main —
# the MeshNodeService start-on-launch path lives in the frontend, so a base-
# skewed APK can miss V2/V6; see memory project-ayaneo-android-build-verify).
#
# Usage:
#   ./app/tests/android/verify-runtime.sh [--apk <path>] [--pkg ai.spectynmesh.app] [--avd Pixel_API_36]
#
# Exit 0 if V1+V4 pass (app runs + WebView renders — the hard gates); V2/V3/V5/
# V6/V7 are reported PASS/INFO/FAIL but only V1+V4 fail the script (the rest can
# legitimately depend on onboarding state / manual tile+widget placement).

set -uo pipefail
PKG="ai.spectynmesh.app"; AVD="Pixel_API_36"; APK=""
while [[ $# -gt 0 ]]; do case "$1" in
  --apk) APK="${2:?--apk needs a path}"; shift 2;; --pkg) PKG="${2:?--pkg needs a value}"; shift 2;; --avd) AVD="${2:?--avd needs a value}"; shift 2;;
  *) echo "unknown arg: $1" >&2; exit 2;; esac; done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ADB="$SDK/platform-tools/adb"; [[ -f "$ADB.exe" ]] && ADB="$ADB.exe"
EMU="$SDK/emulator/emulator"; [[ -f "$EMU.exe" ]] && EMU="$EMU.exe"

declare -A R   # results matrix
mark() { R["$1"]="$2 — $3"; printf '  %-4s %-3s %s\n' "$1" "$2" "$3"; }

[[ -z "$APK" ]] && APK=$(find "$REPO_ROOT/app/src-tauri/gen/android/app/build/outputs/apk" -name '*debug*.apk' 2>/dev/null | sort | tail -1)
[[ -f "$APK" ]] || { echo "❌ no APK (pass --apk)"; exit 2; }
echo "◆ APK: $APK"

# ── emulator: reuse or boot ───────────────────────────────────────────────────
if ! "$ADB" devices | grep -qE 'emulator-[0-9]+[[:space:]]+device$'; then
  echo "◆ booting $AVD …"
  "$EMU" -avd "$AVD" -no-snapshot -no-audio -gpu swiftshader_indirect >/tmp/verify-emu.log 2>&1 &
  sleep 2
fi
"$ADB" wait-for-device
export ANDROID_SERIAL="$("$ADB" devices | grep -E 'emulator-[0-9]+[[:space:]]+device$' | head -1 | awk '{print $1}')"
echo "◆ serial: ${ANDROID_SERIAL:-none}"
for i in $(seq 1 60); do [[ "$("$ADB" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]] && break; sleep 5; done
echo "◆ emulator: $("$ADB" shell getprop ro.product.cpu.abi | tr -d '\r')"

# ── install + launch ──────────────────────────────────────────────────────────
"$ADB" uninstall "$PKG" >/dev/null 2>&1 || true
"$ADB" install -r "$APK" 2>&1 | grep -q Success || { echo "❌ install failed"; exit 1; }
"$ADB" logcat -c
"$ADB" shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
sleep 8

echo "── V1–V7 ──────────────────────────────────────────────"
# V1 process alive (hard gate)
if [[ -n "$("$ADB" shell pidof "$PKG" | tr -d '\r')" ]]; then mark V1 PASS "process alive after launch"; else
  "$ADB" logcat -d | grep -iE 'FATAL|AndroidRuntime|abort|UnsatisfiedLink' | tail -5 >&2; mark V1 FAIL "app died on launch"; echo "❌ V1 hard gate failed"; exit 1; fi

# V2 MeshNodeService foreground (INFO: may need onboarding / current-main frontend)
# MeshNodeService is NOT exported (correct), so it can't be started from adb. Trigger it the way
# the app does — the in-app FocusQuickTile onClick, which calls startForegroundService(MeshNodeService).
"$ADB" shell cmd statusbar add-tile "$PKG/.FocusQuickTile" >/dev/null 2>&1 || true
"$ADB" shell cmd statusbar click-tile "$PKG/.FocusQuickTile" >/dev/null 2>&1 || true
sleep 5
if "$ADB" shell dumpsys activity services "$PKG" 2>/dev/null | grep -A12 'MeshNodeService' | grep -q 'isForeground=true'; then mark V2 PASS "MeshNodeService foreground (started via FocusQuickTile, id=1001)"
else mark V2 FAIL "MeshNodeService not foreground after FocusQuickTile trigger"; fi

# V3 deep-link — B2-hardened: pin to the app handler's OWN log line ('deep-link demo-mode
# accepted', emitted by src-tauri/src/lib.rs on_open_url ONLY after the app accepts the URL).
# A loose 'demo-mode|deep-link' grep also matches the OS `am start` line (act=… dat=spectyn://demo-mode)
# → false-green even if the handler never ran. Anchor on 'accepted' so a reject log cannot satisfy it.
"$ADB" logcat -c; "$ADB" shell am start -a android.intent.action.VIEW -d "spectyn://demo-mode" >/dev/null 2>&1; sleep 4
if "$ADB" logcat -d 2>/dev/null | grep -qiE 'deep-link demo-mode accepted'; then mark V3 PASS "spectyn:// deep-link handled (app emitted 'accepted')"; else mark V3 INFO "no app-handler 'accepted' log (handler silent/not invoked — NOT a false PASS)"; fi

# V4 WebView CDP (hard gate)
MJS="$SCRIPT_DIR/webview-cdp-smoke.mjs"; command -v cygpath >/dev/null 2>&1 && MJS="$(cygpath -w "$MJS")"
if node "$MJS" --pkg "$PKG" >/tmp/verify-cdp.log 2>&1; then mark V4 PASS "WebView DOM live ($(grep -o 'title=[^ ]*' /tmp/verify-cdp.log | head -1))"; else mark V4 FAIL "WebView CDP failed (see /tmp/verify-cdp.log)"; echo "❌ V4 hard gate failed"; exit 1; fi

# V5 Focus Quick Settings tile registered
"$ADB" shell dumpsys package "$PKG" 2>/dev/null | grep -q 'FocusQuickTile' && mark V5 PASS "FocusQuickTile registered" || mark V5 FAIL "FocusQuickTile not registered"

# V6 WorkManager job (FocusUpkeepWorker; scheduled from MeshNodeService.onCreate)
if "$ADB" shell dumpsys jobscheduler 2>/dev/null | grep -qiE "$PKG.*(systemjob|androidx.work)"; then mark V6 PASS "WorkManager job scheduled"
else mark V6 INFO "no WorkManager job yet (gated on MeshNodeService.onCreate → V2)"; fi

# V7 Glance habit widget registered
"$ADB" shell dumpsys package "$PKG" 2>/dev/null | grep -q 'HabitChipPaletteWidgetReceiver' && mark V7 PASS "Habit Glance widget registered" || mark V7 FAIL "widget receiver not registered"

# V8 mobile bottom-nav tabs all route correctly — regression guard for dead/mis-routed tabs:
#   /dashboard (習慣, habit-nav fix), /dispatch + /history (route-fix). Each: click its <a href>, confirm pathname.
#   INFO (not FAIL) on miss: a build missing those fixes won't route the tabs — flips to PASS once they land.
nav_fail=""
for tab in /focus /dashboard /cluster /dispatch /history /settings; do
  got="$(node "$MJS" --pkg "$PKG" --expr "(()=>{const a=[...document.querySelectorAll('a')].find(x=>x.getAttribute('href')==='$tab');if(a)a.click();return new Promise(r=>setTimeout(()=>r(location.pathname),700))})()" 2>/dev/null | grep -oE '=> "[^"]*"' | tail -1 | tr -d '=> "')"
  [ "$got" = "$tab" ] || nav_fail="$nav_fail $tab(→${got:-?})"
done
[ -z "$nav_fail" ] && mark V8 PASS "all 6 bottom-nav tabs route correctly" || mark V8 INFO "tabs not routing (needs onboarded app + route-fix/habit-nav landed):$nav_fail"

echo "───────────────────────────────────────────────────────"
echo "RESULT MATRIX:"; for k in V1 V2 V3 V4 V5 V6 V7 V8; do printf '  %-4s %s\n' "$k" "${R[$k]:-?}"; done
# hard gates
[[ "${R[V1]:-}" == PASS* ]] && [[ "${R[V4]:-}" == PASS* ]] || { echo "❌ hard gate (V1/V4) failed"; exit 1; }
echo "✅ hard gates V1+V4 pass (run from current-main build to confirm V2/V6 runtime)"
