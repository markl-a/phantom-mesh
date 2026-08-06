#!/usr/bin/env bash
# Android emulator sideload smoke test for the spectyn-mesh Tauri APK
# (task-2026052620). Boots an AVD, installs the APK, launches it, and asserts:
#   1. the process survives launch (no native crash),
#   2. MeshNodeService comes up as a foreground service (SPEC-33 §6 + the
#      MainActivity→service wiring + inject.sh manifest registration),
#   3. the spectyn:// deep-link reaches the app (SPEC-33 G6 intent-filter).
#
# IMPORTANT — ABI: stock Android emulators are x86_64. An aarch64-only APK
# *installs* (the system image lists arm64-v8a) but CRASHES on launch inside
# the Tauri/Rust runtime under arm64→x86 translation. So this smoke defaults to
# the x86_64 build. Use the matching device ABI for a real handset.
#
# Usage:
#   ./scripts/smoke-android-emulator.sh                      # build x86_64, boot AVD, smoke
#   ./scripts/smoke-android-emulator.sh --apk path/to.apk    # skip build, use this APK
#   ./scripts/smoke-android-emulator.sh --avd Pixel_API_36 --abi x86_64 --window --keep

# set -e: a bare `am start` / `logcat -c` that errors (e.g. device dropped off,
# wrong package) must ABORT the smoke, not silently continue to a later assert
# that then passes on stale data (false-green). Commands whose non-zero exit is
# acceptable are explicitly guarded with `|| true` / `if`.
set -euo pipefail

AVD="Pixel_API_36"
ABI="x86_64"
APK=""
WINDOW=0
KEEP=0
PKG="ai.spectynmesh.app"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --avd) AVD="$2"; shift 2 ;;
    --abi) ABI="$2"; shift 2 ;;
    --apk) APK="$2"; shift 2 ;;
    --window) WINDOW=1; shift ;;
    --keep) KEEP=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ADB="$SDK/platform-tools/adb"; [[ -f "$ADB.exe" ]] && ADB="$ADB.exe"
EMU="$SDK/emulator/emulator"; [[ -f "$EMU.exe" ]] && EMU="$EMU.exe"

fail() { echo "❌ SMOKE FAIL: $*" >&2; exit 1; }
ok()   { echo "✓ $*"; }

# ── 1. APK (build for the target ABI unless one was provided) ─────────────────
BUILDER="$SCRIPT_DIR/package-android-apk.sh"
if [[ -z "$APK" ]]; then
  if [[ ! -f "$BUILDER" ]]; then
    fail "APK builder missing: $BUILDER
       It is tracked under scripts/** (node-a-winlinux scope) and was lost from main
       in the 2026-05-29 archive sweep. Restore it (git show 4d641de4:scripts/package-android-apk.sh)
       or pass a prebuilt APK with --apk <path> to run the smoke without building."
  fi
  echo "◆ building $ABI debug APK …"
  bash "$BUILDER" --target "$ABI" >/dev/null || fail "APK build failed"
  APK=$(find "$REPO_ROOT/app/src-tauri/gen/android/app/build/outputs/apk" -name '*.apk' -path '*debug*' | sort | tail -1)
fi
[[ -f "$APK" ]] || fail "APK not found: $APK"
ok "APK: $APK"

# ── 2. emulator (reuse a running one, else boot the AVD) ──────────────────────
if "$ADB" devices | grep -q 'emulator-.*device'; then
  ok "reusing running emulator"
else
  echo "◆ booting AVD $AVD …"
  WIN_FLAGS=(-no-snapshot -no-audio); [[ "$WINDOW" == "0" ]] && WIN_FLAGS+=(-no-window -no-boot-anim)
  "$EMU" -avd "$AVD" "${WIN_FLAGS[@]}" -gpu swiftshader_indirect >/tmp/smoke-emu.log 2>&1 &
fi
"$ADB" wait-for-device
for i in $(seq 1 60); do
  [[ "$("$ADB" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]] && break
  sleep 5
done
[[ "$("$ADB" shell getprop sys.boot_completed | tr -d '\r')" == "1" ]] || fail "emulator did not finish booting"
ok "emulator booted ($("$ADB" shell getprop ro.product.cpu.abi | tr -d '\r'))"

# ── 3. install + launch ───────────────────────────────────────────────────────
"$ADB" uninstall "$PKG" >/dev/null 2>&1 || true
"$ADB" install -r "$APK" 2>&1 | grep -q Success || fail "install failed"
ok "installed"
"$ADB" logcat -c || fail "logcat -c failed (device dropped off?)"
"$ADB" shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1 \
  || fail "am start ($PKG/.MainActivity) failed — activity missing or device gone"
sleep 8

# ── 4. assert: alive (no crash) ───────────────────────────────────────────────
if [[ -z "$("$ADB" shell pidof "$PKG" | tr -d '\r')" ]]; then
  "$ADB" logcat -d 2>/dev/null | grep -iE 'FATAL|AndroidRuntime|abort|UnsatisfiedLink' | tail -10 >&2
  fail "app died on launch (see logcat above — wrong ABI for this device?)"
fi
ok "process alive after launch"

# ── 5. assert: MeshNodeService is foreground ──────────────────────────────────
"$ADB" shell dumpsys activity services "$PKG" 2>/dev/null | grep -q 'isForeground=true' \
  || fail "MeshNodeService is not a running foreground service"
ok "MeshNodeService foreground service up"

# ── 6. assert: spectyn:// deep-link is HANDLED by our app (not just dispatched) ─
# B2 FIX: the old assertion grepped logcat for 'demo-mode|deep-link', which ALSO
# matches Android's own ActivityManager `am start` line ("Starting: Intent {
# act=… dat=spectyn://demo-mode }"). That made the assert pass even when the app
# never ran its handler — a false-green. Pin to the app-specific handler log line
# the Rust deep-link callback emits (target: "spectyn-app", "deep-link demo-mode
# accepted" — see app/src-tauri/src/lib.rs on_open_url). That line is emitted
# ONLY after our code accepts the URL, so it proves real handling, not dispatch.
"$ADB" logcat -c || fail "logcat -c failed before deep-link probe"
"$ADB" shell am start -a android.intent.action.VIEW -d "spectyn://demo-mode" >/dev/null 2>&1 \
  || fail "am start (VIEW spectyn://demo-mode) failed to dispatch"
sleep 4
# Match the handler's exact phrasing; anchor on "accepted" so the rejection log
# ("deep-link rejected: …") can never satisfy this assert.
if "$ADB" logcat -d 2>/dev/null | grep -qiE 'deep-link demo-mode accepted'; then
  ok "spectyn:// deep-link handled (app emitted 'deep-link demo-mode accepted')"
else
  "$ADB" logcat -d 2>/dev/null | grep -iE 'spectyn-app|deep-link' | tail -10 >&2 || true
  fail "spectyn:// deep-link reached the OS but the app handler never accepted it (no 'deep-link demo-mode accepted' in logcat)"
fi

[[ "$KEEP" == "0" ]] && "$ADB" emu kill >/dev/null 2>&1 || true
echo ""
echo "✅ SMOKE PASS — install + launch + MeshNodeService FG + spectyn:// deep-link"
