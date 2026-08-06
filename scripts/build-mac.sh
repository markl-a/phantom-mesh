#!/usr/bin/env bash
# Mac release-build helper for spectyn-mesh.
#
# What it does, in order:
#   1. cargo build --release --bin spectyn (target = host arch by default;
#      override via TARGET=<triple>)
#   2. ad-hoc codesign the resulting binary — `cp` into dist/ would strip
#      signature and amfid would silently SIGKILL on launch (commit 85c8377).
#      Signing happens at the source path, so any later cp/mv carries a
#      valid signature.
#   3. smoke-verify by spawning `spectyn --version` with a 5s timeout. If
#      the kernel SIGKILLs (exit 137 / no output), we fail loud here, not
#      after the user has scp'd the binary somewhere and wondered why
#      launchd looks alive but no daemon answers.
#   4. mirror into dist/spectyn-<triple> so install-mac.sh + `spectyn selftest`
#      pick it up via the coordinator's /dist/ HTTP route.
#
# Usage:
#   ./scripts/build-mac.sh                 # builds for host arch
#   TARGET=aarch64-apple-darwin ./scripts/build-mac.sh
#   TARGET=x86_64-apple-darwin  ./scripts/build-mac.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The cargo package lives in core/, not the repo root.
CARGO_DIR="$REPO_ROOT/core"
cd "$CARGO_DIR"

# Detect host triple if TARGET unset.
if [ -z "${TARGET:-}" ]; then
  case "$(uname -m)" in
    arm64)  TARGET="aarch64-apple-darwin" ;;
    x86_64) TARGET="x86_64-apple-darwin" ;;
    *)      echo "✗ unsupported host arch: $(uname -m)"; exit 1 ;;
  esac
fi

if [ "$(uname -s)" != "Darwin" ]; then
  echo "✗ build-mac.sh must run on macOS — host is $(uname -s)"
  exit 1
fi

echo "  ◆ spectyn-mesh — Mac release build"
echo "    target : $TARGET"
echo "    repo   : $REPO_ROOT"
echo

# ── 1. cargo build ──────────────────────────────────────────────────────
echo "  [1/4] cargo build --release --bin spectyn --target $TARGET"
cargo build --release --bin spectyn --target "$TARGET"

OUT_BIN="$CARGO_DIR/target/$TARGET/release/spectyn"
if [ ! -x "$OUT_BIN" ]; then
  echo "  ✗ binary missing at $OUT_BIN"
  exit 1
fi
echo "    ✓ built $OUT_BIN ($(du -h "$OUT_BIN" | awk '{print $1}'))"

# ── 2. ad-hoc codesign ──────────────────────────────────────────────────
echo "  [2/4] codesign --force --sign -"
codesign --force --sign - "$OUT_BIN"
codesign --verify --verbose "$OUT_BIN" 2>&1 | sed 's/^/    /'

# ── 3. smoke-verify launch (catches amfid SIGKILL) ──────────────────────
echo "  [3/4] launch smoke (5s timeout)"
# `gtimeout` from coreutils, falls back to background+kill if absent.
if command -v gtimeout >/dev/null 2>&1; then
  RUN_OUT="$(gtimeout 5 "$OUT_BIN" --version 2>&1)" || RUN_RC=$?
elif command -v timeout >/dev/null 2>&1; then
  RUN_OUT="$(timeout 5 "$OUT_BIN" --version 2>&1)" || RUN_RC=$?
else
  RUN_OUT="$("$OUT_BIN" --version 2>&1)" || RUN_RC=$?
fi
RUN_RC="${RUN_RC:-0}"
if [ "$RUN_RC" -eq 137 ] || [ "$RUN_RC" -eq 9 ]; then
  echo "  ✗ binary SIGKILL'd by kernel (exit $RUN_RC) — codesign signature rejected by amfid"
  echo "    Check Console.app for amfid messages around $(date '+%H:%M')"
  exit 1
fi
if [ "$RUN_RC" -ne 0 ]; then
  echo "  ✗ binary --version exited $RUN_RC: $RUN_OUT"
  exit 1
fi
echo "    ✓ $RUN_OUT"

# ── 4. mirror into dist/ for distribution ───────────────────────────────
echo "  [4/4] mirror → dist/spectyn-$TARGET"
mkdir -p "$REPO_ROOT/dist"
cp "$OUT_BIN" "$REPO_ROOT/dist/spectyn-$TARGET"
# cp into dist/ strips signature on macOS — re-sign in place so HTTP
# distribution carries a valid signature. install-mac.sh ALSO re-signs
# defensively because curl-download likewise breaks the signature.
codesign --force --sign - "$REPO_ROOT/dist/spectyn-$TARGET"
echo "    ✓ dist/spectyn-$TARGET ($(du -h "$REPO_ROOT/dist/spectyn-$TARGET" | awk '{print $1}'))"

echo
echo "  ✓ Mac release build complete."
echo "    Source : $OUT_BIN"
echo "    Dist   : $REPO_ROOT/dist/spectyn-$TARGET"
echo
echo "  Next:"
echo "    spectyn selftest                          # full feature self-test (or scripts/selftest.sh)"
echo "    cp dist/spectyn-$TARGET ~/.cargo/bin/spectyn   # adopt locally"
