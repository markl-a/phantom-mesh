#!/usr/bin/env bash
# Copy spectyn-mesh binary into src-tauri/binaries/ for Tauri sidecar bundling.
# Usage: ./scripts/copy-sidecar.sh [release|debug]
#
# Searches exFAT-workaround target dirs first (target2, target3), then standard target/.

set -euo pipefail

PROFILE="${1:-release}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(dirname "$SCRIPT_DIR")"
CORE_DIR="$(dirname "$DESKTOP_DIR")/core"
DEST_DIR="$DESKTOP_DIR/src-tauri/binaries"

# Detect target triple
case "$(uname -s)-$(uname -m)" in
  MINGW*|MSYS*|CYGWIN*) TRIPLE="x86_64-pc-windows-msvc"; EXT=".exe" ;;
  Darwin-arm64)          TRIPLE="aarch64-apple-darwin";    EXT="" ;;
  Darwin-x86_64)         TRIPLE="x86_64-apple-darwin";     EXT="" ;;
  Linux-x86_64)          TRIPLE="x86_64-unknown-linux-gnu"; EXT="" ;;
  *)                     echo "Unsupported platform"; exit 1 ;;
esac

SRC_NAME="spectyn-mesh${EXT}"
DEST_NAME="spectyn-mesh-${TRIPLE}${EXT}"

# Search order: exFAT workaround dirs, then standard
for DIR in "$CORE_DIR/target2/$PROFILE" "$CORE_DIR/target3/$PROFILE" "$CORE_DIR/target/$PROFILE"; do
  if [ -f "$DIR/$SRC_NAME" ]; then
    mkdir -p "$DEST_DIR"
    cp "$DIR/$SRC_NAME" "$DEST_DIR/$DEST_NAME"
    echo "Copied $DIR/$SRC_NAME -> $DEST_DIR/$DEST_NAME"
    exit 0
  fi
done

echo "ERROR: spectyn-mesh binary not found in any target dir for profile=$PROFILE"
echo "Build it first: cd core && CARGO_TARGET_DIR=target2 cargo build --release"
exit 1
