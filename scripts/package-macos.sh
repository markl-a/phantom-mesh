#!/usr/bin/env bash
# scripts/package-macos.sh — build + sign a macOS .dmg of the phantom-mesh
# desktop "AI terminal" (the Tauri GUI app), with a Gatekeeper smoke check.
#
# Wave H3.5 (task-2026052613). Produces a distributable disk image for the
# macOS app. Signing tiers, in order of preference:
#   1. "Developer ID Application" cert  → notarizable, passes Gatekeeper (prod)
#   2. "Apple Development" cert         → runs locally; Gatekeeper REJECTS for
#                                         distribution (dev path — current state)
#   3. ad-hoc ("-")                     → local-only, no distribution
#
# Production notarization (xcrun notarytool) is wired but only runs when a
# Developer ID identity + ASC credentials are present; otherwise it is skipped
# with a clear note. This matches the brief: ship the dev/self-cert path now,
# defer the production Apple Distribution cert to the release window.
#
# Usage:
#   scripts/package-macos.sh                    # build, auto-pick best identity, package
#   scripts/package-macos.sh --no-build         # reuse existing bundle
#   scripts/package-macos.sh --identity "Developer ID Application: …"
#   scripts/package-macos.sh --adhoc            # force ad-hoc signing
#   scripts/package-macos.sh --notarize         # attempt notarytool (needs Dev ID + creds)
#   scripts/package-macos.sh --out DIR          # copy the .dmg here (default: dist/)
#
# Exit: 0 ok, 1 build/sign/package error, 2 bad args.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$REPO_ROOT/app"
OUT_DIR="$REPO_ROOT/dist"
DO_BUILD=1
DO_NOTARIZE=0
FORCE_ADHOC=0
IDENTITY=""

while [ $# -gt 0 ]; do
  case "$1" in
    --no-build)  DO_BUILD=0; shift ;;
    --notarize)  DO_NOTARIZE=1; shift ;;
    --adhoc)     FORCE_ADHOC=1; shift ;;
    --identity)  IDENTITY="${2:?--identity needs a value}"; shift 2 ;;
    --out)       OUT_DIR="${2:?--out needs a dir}"; shift 2 ;;
    --help|-h)   sed -n '2,30p' "$0"; exit 0 ;;
    *)           echo "package-macos: unknown arg '$1'" >&2; exit 2 ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || { echo "FATAL: package-macos.sh must run on macOS (host $(uname -s))" >&2; exit 1; }
command -v hdiutil  >/dev/null 2>&1 || { echo "FATAL: hdiutil not found" >&2; exit 1; }
command -v codesign >/dev/null 2>&1 || { echo "FATAL: codesign not found" >&2; exit 1; }

# ── Resolve the signing identity ───────────────────────────────────────────
# Preference: explicit --identity > Developer ID > Apple Development > ad-hoc.
pick_identity() {
  [ -n "$IDENTITY" ] && { echo "$IDENTITY"; return; }
  [ "$FORCE_ADHOC" = 1 ] && { echo "-"; return; }
  local list; list="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  local devid; devid="$(echo "$list" | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)"
  [ -n "$devid" ] && { echo "$devid"; return; }
  local appledev; appledev="$(echo "$list" | sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' | head -1)"
  [ -n "$appledev" ] && { echo "$appledev"; return; }
  echo "-"
}
SIGN_ID="$(pick_identity)"
case "$SIGN_ID" in
  "Developer ID Application:"*) TIER="developer-id (notarizable)" ;;
  "Apple Development:"*)        TIER="apple-development (dev path — Gatekeeper rejects for distribution)" ;;
  "-")                          TIER="ad-hoc (local-only)" ;;
  *)                            TIER="custom" ;;
esac

echo "package-macos: phantom-mesh desktop app"
echo "  identity : $SIGN_ID"
echo "  tier     : $TIER"
echo "  out      : $OUT_DIR"
echo

# ── 1. Build the Tauri bundle (frontend + .app + .dmg) ─────────────────────
if [ "$DO_BUILD" = 1 ]; then
  echo "  [1/5] tauri build (frontend + macOS bundle)…"
  # Let Tauri sign the app during bundling so the .dmg carries a valid
  # signature (re-signing inside an existing .dmg would need a rebuild anyway).
  export APPLE_SIGNING_IDENTITY="$SIGN_ID"
  ( cd "$APP_DIR" && npm run tauri:build -- --bundles app,dmg )
else
  echo "  [1/5] skip build (--no-build)"
fi

# ── 2. Locate the produced .app and .dmg ───────────────────────────────────
echo "  [2/5] locate bundle artifacts"
BUNDLE_DIR="$APP_DIR/src-tauri/target/release/bundle"
APP_PATH="$(find "$BUNDLE_DIR/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1 || true)"
DMG_PATH="$(find "$BUNDLE_DIR/dmg"   -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)"
[ -n "$APP_PATH" ] || { echo "  ✗ no .app under $BUNDLE_DIR/macos (build failed?)" >&2; exit 1; }
echo "    app : $APP_PATH"
echo "    dmg : ${DMG_PATH:-<none — will create>}"

# ── 3. Ensure the .app is signed (defensive re-sign) ───────────────────────
echo "  [3/5] codesign --deep --force --sign \"$SIGN_ID\""
codesign --deep --force --options runtime --sign "$SIGN_ID" "$APP_PATH" 2>&1 | sed 's/^/    /' || \
  codesign --deep --force --sign "$SIGN_ID" "$APP_PATH" 2>&1 | sed 's/^/    /'
echo "    ── codesign -dv ──"
codesign -dv --verbose=2 "$APP_PATH" 2>&1 | sed 's/^/    /'

# If Tauri did not emit a .dmg, build one from the signed .app.
if [ -z "$DMG_PATH" ]; then
  echo "    creating .dmg via hdiutil from signed .app"
  VER="$(awk -F' *= *' '/^\[package\]/{p=1} p&&/^version/{gsub(/"/,"",$2); print $2; exit}' "$REPO_ROOT/core/Cargo.toml")"
  DMG_PATH="$BUNDLE_DIR/dmg/phantom-mesh_${VER:-0.0.0}_aarch64.dmg"
  mkdir -p "$(dirname "$DMG_PATH")"
  hdiutil create -volname "Phantom Mesh" -srcfolder "$APP_PATH" -ov -format UDZO "$DMG_PATH" >/dev/null
fi

# ── 4. Gatekeeper assessment (honest reporting) ────────────────────────────
echo "  [4/5] spctl --assess (Gatekeeper)"
set +e
SPCTL_OUT="$(spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG_PATH" 2>&1)"
SPCTL_RC=$?
set -e
echo "$SPCTL_OUT" | sed 's/^/    /'
if [ "$SPCTL_RC" -eq 0 ]; then
  echo "    ✓ Gatekeeper accepts the .dmg"
else
  echo "    ⚠ Gatekeeper rejects (rc=$SPCTL_RC) — expected for $TIER."
  echo "      Distribution needs a Developer ID cert + notarization (deferred)."
fi

# ── 4b. Optional notarization (only with Developer ID + creds) ─────────────
if [ "$DO_NOTARIZE" = 1 ]; then
  if [[ "$SIGN_ID" == "Developer ID Application:"* ]] && command -v xcrun >/dev/null 2>&1; then
    echo "    notarytool submit (requires keychain profile 'phantom-notary')"
    xcrun notarytool submit "$DMG_PATH" --keychain-profile "phantom-notary" --wait 2>&1 | sed 's/^/    /' \
      && xcrun stapler staple "$DMG_PATH" 2>&1 | sed 's/^/    /' \
      || echo "    ⚠ notarization failed/skipped — check ASC credentials"
  else
    echo "    ⚠ --notarize requested but no Developer ID identity / xcrun — skipped"
  fi
fi

# ── 5. Publish to dist/ ────────────────────────────────────────────────────
echo "  [5/5] copy .dmg → $OUT_DIR"
mkdir -p "$OUT_DIR"
cp "$DMG_PATH" "$OUT_DIR/"
FINAL="$OUT_DIR/$(basename "$DMG_PATH")"
echo "    ✓ $FINAL ($(du -h "$FINAL" | awk '{print $1}'))"

echo
echo "  ✓ macOS package complete."
echo "    DMG  : $FINAL"
echo "    Tier : $TIER"
[ "$SPCTL_RC" -eq 0 ] || echo "    Note : dev-signed — for local install only until Developer ID + notarization land."
