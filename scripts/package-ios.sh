#!/usr/bin/env bash
# Build and package phantom for iOS (Tauri v2 app + IPA).
# Output: dist/phantom-mesh-ios.ipa  (signed)
#         dist/phantom-mesh-ios-sim.app (simulator build, unsigned)
#
# Usage:
#   cd phantom-mesh
#   ./scripts/package-ios.sh [--sim]     # --sim for simulator build only
#
# Requirements:
#   - Xcode 15+ with iOS platform (xcodebuild -downloadPlatform iOS)
#   - Apple Developer account (DEVELOPMENT_TEAM must be set)
#   - Node/npm + Rust + cargo installed
#   - tauri-cli: cargo install tauri-cli

set -euo pipefail

SIM_ONLY=false
for arg in "$@"; do
  [[ "$arg" == "--sim" ]] && SIM_ONLY=true
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
APP_DIR="$REPO_ROOT/app"
APPLE_DIR="$APP_DIR/src-tauri/gen/apple"
DIST="$REPO_ROOT/dist"

TEAM="${APPLE_TEAM_ID:-${DEVELOPMENT_TEAM:-}}"

mkdir -p "$DIST"

# ── Ensure iOS platform / simulator runtime is installed ─────────────────────
echo "◆ Checking iOS platform components …"
if ! xcrun simctl list runtimes 2>/dev/null | grep -q "iOS 26"; then
  echo "◆ Downloading iOS platform (this may take several minutes) …"
  xcodebuild -downloadPlatform iOS
fi

# ── Init Tauri iOS project if needed ─────────────────────────────────────────
if [[ ! -d "$APPLE_DIR/phantom-mesh-app.xcodeproj" ]]; then
  echo "◆ Initialising Tauri iOS project …"
  cd "$APP_DIR"
  npm install
  npx tauri ios init
fi

# ── Ensure xcodegen prerequisites exist ──────────────────────────────────────
# xcodegen project.yml lists `Externals` as a target source, so the dir tree
# must exist before `xcodegen generate` runs (otherwise: "Spec validation
# error: missing source directory"). Create only the arm64 dirs — Apple
# Silicon Mac never builds x86_64 for sim/device, but if the x86_64 dirs
# exist xcode's `Build Rust Code` script phase still declares
# `Externals/x86_64/${CONFIGURATION}/libapp.a` as a SCRIPT_OUTPUT_FILE,
# which auto-registers a CpResource rule and then trips on duplicate-output
# against arm64's libapp.a (both copy to the same `.app/libapp.a`).
# x86_64 is excluded for both iphoneos AND iphonesimulator on this Mac.
for cfg in debug release; do
  mkdir -p "$APPLE_DIR/Externals/arm64/$cfg"
done

cd "$APPLE_DIR"
if command -v xcodegen &>/dev/null; then
  echo "◆ Regenerating Xcode project …"
  xcodegen generate --spec project.yml
fi

# ── Simulator build (--sim flag) ──────────────────────────────────────────────
# Drive the build via `tauri ios build` rather than raw xcodebuild — the
# `Build Rust Code` xcode phase calls `tauri ios xcode-script`, which
# panics on a missing addr file unless tauri's CLI sets one up first.
if $SIM_ONLY; then
  echo "◆ Building for iOS Simulator (arm64) …"

  # Pick newest available iOS 26 simulator just for the post-build hint.
  SIM_ID=$(xcrun simctl list devices available 2>/dev/null \
    | grep -E "iPhone.*Shutdown" \
    | grep -oE "[A-F0-9-]{36}" | tail -1)

  # xcodegen project.yml uses `path: Externals` (recursive resource copy),
  # so a stale libapp.a from a different configuration causes a duplicate
  # output collision in the .app bundle. Wipe the release dir entirely
  # (file + dir; xcode also caches the dir's existence in DerivedData).
  rm -rf "$APPLE_DIR/Externals/arm64/release" \
         "$APPLE_DIR/Externals/x86_64/release"

  # Tauri renames the freshly built .app onto the same path, which fails
  # when a previous build (esp. a device archive) left content behind.
  rm -rf "$APPLE_DIR/build/phantom-mesh-app_iOS.xcarchive" \
         "$APPLE_DIR/build/arm64-sim"

  # DerivedData caches the Externals copy plan; force a clean re-evaluate.
  rm -rf ~/Library/Developer/Xcode/DerivedData/phantom-mesh-app-*

  # Re-stub the debug libapp.a (the wipe above also removed dirs); cargo
  # will overwrite during the build phase. See pre-xcodegen block for why.
  #
  # Sim path: only stub arm64 (the only --target aarch64-sim ships).
  # Same fix as commit 3a0c7cf for the device path — stubbing BOTH arches
  # makes Xcode generate two CpResource tasks both producing the same
  # `Phantom Mesh.app/libapp.a` and the build fails with
  # "duplicate output file" / "Multiple commands produce libapp.a".
  mkdir -p "$APPLE_DIR/Externals/arm64/debug"
  touch "$APPLE_DIR/Externals/arm64/debug/libapp.a"

  # RE-RUN xcodegen now so project.yml's `path: Externals` recursive walk
  # captures ONLY debug/libapp.a (release was wiped above at line 84).
  # Without this, the initial xcodegen at script start (line ~65) baked
  # BOTH debug + release paths into the .xcodeproj's CpResource rules
  # (because BOTH dirs existed at that moment per the pre-xcodegen loop
  # at line 58-60), and xcode dies on `CpResource
  # Externals/arm64/release/libapp.a` (no such file). Stubbing the
  # release file would instead trip "Multiple commands produce libapp.a"
  # (both copy → same `.app/libapp.a` dest). Mirror of the device-path
  # fix at line ~193.
  if [[ -d "$APPLE_DIR" ]] && command -v xcodegen &>/dev/null; then
    (cd "$APPLE_DIR" && xcodegen generate --spec project.yml --quiet 2>/dev/null) || true
  fi

  cd "$APP_DIR"
  # VITE_DEMO_MODE=1 (only honored by --sim builds) auto-bypasses the
  # MobileOnboardingV2 gate + lands directly on /settings/cluster so QA
  # / screenshot demos can reach the cluster UI without OAuth. App
  # source checks `import.meta.env.VITE_DEMO_MODE === "1"`. Production
  # IPA builds (the device path further down) do NOT set this var.
  VITE_DEMO_MODE=1 npx tauri ios build --debug --target aarch64-sim --ci \
    --ignore-version-mismatches

  # Tauri places the built bundle under
  #   src-tauri/gen/apple/build/arm64-sim/Phantom Mesh.app   (--debug → debug-iphonesimulator)
  # Hunt for it without assuming the exact intermediate name.
  SIM_APP=$(find "$APPLE_DIR/build" -type d -name "*.app" \
    -path "*iphonesimulator*" 2>/dev/null | head -1)
  if [[ -z "$SIM_APP" ]]; then
    SIM_APP=$(find "$APPLE_DIR" -type d -name "Phantom Mesh.app" 2>/dev/null | head -1)
  fi
  if [[ -n "$SIM_APP" ]]; then
    rm -rf "$DIST/phantom-mesh-ios-sim.app"
    cp -r "$SIM_APP" "$DIST/phantom-mesh-ios-sim.app"
    echo ""
    echo "✓ Simulator app: $DIST/phantom-mesh-ios-sim.app"
    if [[ -n "$SIM_ID" ]]; then
      echo "  Install: xcrun simctl install $SIM_ID '$DIST/phantom-mesh-ios-sim.app'"
      echo "  Launch:  xcrun simctl launch $SIM_ID ai.phantommesh.app"
    fi
  else
    echo "⚠  Build finished but no .app bundle was found under $APPLE_DIR"
    exit 1
  fi
  exit 0
fi

if [[ -z "$TEAM" ]]; then
  echo "❌  Missing Apple team id. Set APPLE_TEAM_ID (or DEVELOPMENT_TEAM) before device build."
  exit 1
fi

# ── Pre-flight: Xcode must be signed into the Apple ID owning the team ───────
# Without this, automatic provisioning fails late with a cryptic
# "No profiles for 'ai.phantommesh.app'" deep inside xcodebuild output.
XCODE_PLIST="$HOME/Library/Preferences/com.apple.dt.Xcode.plist"
if ! /usr/libexec/PlistBuddy -c "Print :IDEProvisioningTeams" "$XCODE_PLIST" 2>/dev/null \
     | grep -q "$TEAM"; then
  cat <<EOF
❌  Xcode is not signed into the Apple ID for team $TEAM.
    One-time setup:
      1. Open Xcode → Settings (⌘,) → Accounts
      2. Click + → Apple ID → sign in with the account that owns team $TEAM
      3. Re-run this script

    (Free dev certs require a logged-in Apple ID; the keychain identity
     alone is not enough for automatic provisioning.)
EOF
  exit 1
fi

# ── Frontend deps (Tauri's beforeBuildCommand needs node_modules) ─────────────
if [[ ! -d "$APP_DIR/node_modules" ]]; then
  echo "◆ Installing frontend deps …"
  (cd "$APP_DIR" && npm install)
fi

# ── Device build (archive + export IPA via tauri CLI) ────────────────────────
# Same constraint as the sim path: the xcode `Build Rust Code` phase invokes
# `tauri ios xcode-script`, which needs the addr file set up by a parent
# `tauri ios build`. Driving xcodebuild directly panics in that script.
echo "◆ Archiving for iOS device (arm64) …"

# Drop the entire stale debug directory so xcode's "Externals" resource
# walk doesn't trip on it (just deleting the .a file leaves the dir,
# which xcode still has cached in DerivedData as a copy input).
rm -rf "$APPLE_DIR/Externals/arm64/debug" \
       "$APPLE_DIR/Externals/x86_64/debug"

# Same rename collision the sim path hits.
rm -rf "$APPLE_DIR/build/phantom-mesh-app_iOS.xcarchive" \
       "$APPLE_DIR/build/arm64"

# DerivedData caches the Externals copy plan; force a clean re-evaluate.
rm -rf ~/Library/Developer/Xcode/DerivedData/phantom-mesh-app-*

# Re-stub the release libapp.a (wipe above removed it); cargo will
# overwrite during build phase. See pre-xcodegen block for why.
#
# Device path: only stub arm64 (the only release target we ship).
# Stubbing both arm64 + x86_64 caused the same dup-output failure
# the sim path hit (CpResource rules for both arches → same
# `.app/libapp.a` dest). Match the `--target aarch64` flag below.
mkdir -p "$APPLE_DIR/Externals/arm64/release"
touch "$APPLE_DIR/Externals/arm64/release/libapp.a"

# RE-RUN xcodegen now so project.yml's `path: Externals` recursive walk
# captures ONLY release/libapp.a (debug was wiped). Without this, the
# initial xcodegen at script start (line ~65) baked BOTH debug + release
# paths into the .xcodeproj's CpResource rules. Once both files exist,
# xcodebuild fires "Multiple commands produce libapp.a" (both copy to
# the same `.app/libapp.a`). When only release exists, it's "file not
# found" for debug. Re-running xcodegen here flips the project to
# release-only view.
if [[ -d "$APPLE_DIR" ]] && command -v xcodegen &>/dev/null; then
  (cd "$APPLE_DIR" && xcodegen generate --spec project.yml --quiet 2>/dev/null) || true
fi

# Export method (computed FIRST so the build-number stamp below keys off the
# EFFECTIVE method, not just the TESTFLIGHT shortcut):
#   debugging        - development cert; install via xcrun devicectl (default,
#                      requires devices to be paired/registered)
#   app-store-connect - distribution cert; for TestFlight or App Store upload;
#                      install via TestFlight app on any user's iOS device
# Toggle via TESTFLIGHT=1 env: TESTFLIGHT=1 APPLE_TEAM_ID=... bash scripts/package-ios.sh
EXPORT_METHOD="${EXPORT_METHOD:-debugging}"
if [[ "${TESTFLIGHT:-0}" == "1" ]]; then
  EXPORT_METHOD="app-store-connect"
fi
echo "◆ Export method: $EXPORT_METHOD"

# Build number: App Store Connect rejects a DUPLICATE CFBundleVersion within the same
# marketing version, so EVERY upload (weekly auto-rebuild OR a same-commit rebuild)
# must be unique + strictly increasing. Gate on the EFFECTIVE export method so BOTH
# the TESTFLIGHT=1 shortcut AND a direct EXPORT_METHOD=app-store-connect upload get a
# fresh number (single build runner assumed). Epoch seconds: always larger than the
# last build, unique, a valid integer well under Apple's 2^32 ceiling — beats a git
# commit-count (which collides on a same-commit rebuild). Stamp the freshly generated
# Info.plist, then READ BACK and fail-fast if it didn't take (silently shipping the
# static "1" would re-collide).
if [[ "$EXPORT_METHOD" == "app-store-connect" ]]; then
  BUILD_NUMBER="$(date +%s)"
  PLIST="$APPLE_DIR/phantom-mesh-app_iOS/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$PLIST" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $BUILD_NUMBER" "$PLIST" 2>/dev/null
  GOT="$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$PLIST" 2>/dev/null)"
  if [[ "$GOT" != "$BUILD_NUMBER" ]]; then
    echo "❌  Could not stamp CFBundleVersion in $PLIST — an App Store Connect upload" >&2
    echo "    would collide on the build number. Fix the Info.plist path first." >&2
    exit 1
  fi
  echo "◆ App Store Connect build number: $BUILD_NUMBER"
fi

cd "$APP_DIR"
DEVELOPMENT_TEAM="$TEAM" APPLE_TEAM_ID="$TEAM" \
  npx tauri ios build --target aarch64 --ci \
  --ignore-version-mismatches \
  --export-method "$EXPORT_METHOD"

# Tauri places the device IPA at:
#   src-tauri/gen/apple/build/arm64/Phantom Mesh.ipa  (release config)
IPA=$(find "$APPLE_DIR/build" -type f -name "*.ipa" 2>/dev/null \
       | grep -v "sim" | head -1)
if [[ -z "$IPA" ]]; then
  IPA=$(find "$APPLE_DIR" -type f -name "*.ipa" 2>/dev/null | head -1)
fi

if [[ -n "$IPA" ]]; then
  cp "$IPA" "$DIST/phantom-mesh-ios.ipa"
  SIZE=$(du -sh "$DIST/phantom-mesh-ios.ipa" | cut -f1)
  echo ""
  echo "✓ IPA ready:"
  echo "  $DIST/phantom-mesh-ios.ipa  ($SIZE)"
  echo "  Export method: $EXPORT_METHOD"
  echo ""
  if [[ "$EXPORT_METHOD" == "app-store-connect" ]]; then
    echo "Upload to TestFlight (App Store Connect must already have the app record):"
    echo "  xcrun altool --upload-app -f '$DIST/phantom-mesh-ios.ipa' \\"
    echo "    --type ios --apiKey \$ASC_API_KEY_ID --apiIssuer \$ASC_API_KEY_ISSUER_ID"
    echo ""
    echo "Or via Xcode:"
    echo "  open -a Xcode '$IPA' (Window → Organizer → Distribute App → App Store Connect → Upload)"
  else
    echo "Install on already-paired device:"
    echo "  xcrun devicectl device install app --device <UDID> '$DIST/phantom-mesh-ios.ipa'"
  fi
else
  echo "❌  IPA not found under $APPLE_DIR — build may have failed."
  exit 1
fi
