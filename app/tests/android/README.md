# Android verification harness

This directory **builds, runs, and proves** the Android app on a real x86_64
emulator — it does not author the app, it verifies it (SPEC-33 / SPEC-34).

> Trust only **live** verification: show real `adb` output / screenshots, never
> a "looks fine" claim. A successful build is not verification.

## Two complementary layers

| Layer | Script | Proves | Blind to |
|---|---|---|---|
| **Native** | [`scripts/smoke-android-emulator.sh`](../../../scripts/smoke-android-emulator.sh) | process survives launch (no native crash), `MeshNodeService` foreground (SPEC-33 §6), `spectyn://` deep-link (SPEC-33 G6) | the WebView DOM |
| **WebView** | [`webview-cdp-smoke.mjs`](./webview-cdp-smoke.mjs) | real DOM rendered (not a white screen), buttons/testids present, optional per-flow `--expr` runs a real `safeInvoke→native` path | native service/process state |
| **Runtime** | [`verify-runtime.sh`](./verify-runtime.sh) | V1–V8 matrix: launch / FG service / deep-link / WebView DOM / QS tile / WorkManager / Glance widget / bottom-nav route sweep | — |

`uiautomator dump` is **useless** here (the Tauri WebView is opaque) and
coordinate taps are flaky — hence CDP (Chrome DevTools Protocol).

## ABI gotcha (do not skip)

Stock emulators are **x86_64**. An aarch64-only APK *installs* but **crashes on
launch** under arm64→x86 translation (Tauri/Rust `_start_app` abort). Always
build/verify **x86_64** for the emulator; use the device's real ABI for handsets.

## Run it

```bash
# full native smoke (builds x86_64 debug APK, boots Pixel_API_36, asserts)
bash scripts/smoke-android-emulator.sh

# reuse a built APK / a running emulator window, keep it alive
bash scripts/smoke-android-emulator.sh --apk <path.apk> --window --keep

# then drive the WebView DOM (app must already be launched)
node app/tests/android/webview-cdp-smoke.mjs
node app/tests/android/webview-cdp-smoke.mjs --testid mesh-status
node app/tests/android/webview-cdp-smoke.mjs --expr "location.pathname"

# full V1–V8 runtime matrix against a running emulator
bash app/tests/android/verify-runtime.sh
```

Prereqs: Android SDK + NDK, an AVD named `Pixel_API_36` (x86_64), Node ≥ 22.

## Verification matrix

| # | Feature (SPEC) | Live check |
|---|---|---|
| V1 | App launches, no native crash | `smoke-android-emulator.sh` launch step |
| V2 | `MeshNodeService` foreground (SPEC-33 §6) | smoke FG step (`isForeground=true`) |
| V3 | `spectyn://` deep-link (SPEC-33 G6) | smoke deep-link step (app-handler log line) |
| V4 | WebView UI renders (not blank) | `webview-cdp-smoke.mjs` core probe |
| V5 | Focus Quick Settings tile | `adb shell cmd statusbar ...` + tile toggles FSM |
| V6 | WorkManager job scheduled | `adb shell dumpsys jobscheduler \| grep <pkg>` |
| V7 | Glance widget registered | `adb shell dumpsys appwidget` |
| V8 | Bottom-nav routes (no dead/crashing tab) | CDP nav sweep over each tab |

New flows land here as `app/tests/android/<flow>.mjs`.

## Notes for driving the WebView

- The app exposes **no `data-testid`** (`testids=0`), so `--testid` will not
  match — drive the WebView by **text content** instead, e.g.
  `--expr "[...document.querySelectorAll('button')].find(b=>b.textContent.includes('立即試用')).click(),true"`.
- The landing surface on a fresh install is the first-launch onboarding
  (3 entry paths: free-shared-LLM demo / join-cluster / phantommesh.io login).
- Heavy build/install/reboot cycles can wedge the emulator's WebView CDP socket
  (CDP `--expr` returns nothing); recover with a cold boot:
  `emulator -avd Pixel_API_36 -no-snapshot -wipe-data`.
