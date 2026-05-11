#!/usr/bin/env bash
# Build and package phantom for Android (Termux / ARM64).
# Output: dist/phantom-android-arm64.tar.gz
#
# Usage:
#   cd phantom-mesh
#   ./scripts/package-android.sh
#
# Requirements:
#   - Android NDK installed (via Android Studio or sdkmanager)
#   - cargo-ndk: cargo install cargo-ndk
#   - ANDROID_NDK_HOME / ANDROID_NDK_ROOT set, or NDK discoverable under
#     one of the platform-default Android SDK locations (see below).
#
# Hosts supported: Linux, macOS, Windows (Git Bash / MSYS / Cygwin).

set -euo pipefail

# ── Detect host toolchain prebuilt directory ──────────────────────────────────
# NDK ships prebuilt LLVM under toolchains/llvm/prebuilt/<host>.
case "$(uname -s)" in
  Linux*)                NDK_HOST="linux-x86_64" ;;
  Darwin*)               NDK_HOST="darwin-x86_64" ;;
  MINGW*|MSYS*|CYGWIN*)  NDK_HOST="windows-x86_64" ;;
  *)
    echo "⚠  Unknown host $(uname -s); assuming linux-x86_64 NDK toolchain."
    NDK_HOST="linux-x86_64"
    ;;
esac

# ── Locate NDK ────────────────────────────────────────────────────────────────
# Honour ANDROID_NDK_HOME, then ANDROID_NDK_ROOT, then a platform-default
# search across known SDK locations. We pick the highest-versioned NDK found.
NDK_CANDIDATES=()
[[ -n "${ANDROID_NDK_HOME:-}" ]] && NDK_CANDIDATES+=("$ANDROID_NDK_HOME")
[[ -n "${ANDROID_NDK_ROOT:-}" ]] && NDK_CANDIDATES+=("$ANDROID_NDK_ROOT")

# `ndk` subdirs of any SDK root we know about.
NDK_BASES=()
[[ -n "${ANDROID_HOME:-}"     ]] && NDK_BASES+=("${ANDROID_HOME}/ndk")
[[ -n "${ANDROID_SDK_ROOT:-}" ]] && NDK_BASES+=("${ANDROID_SDK_ROOT}/ndk")
NDK_BASES+=(
  "${HOME}/Library/Android/sdk/ndk"   # macOS Android Studio default
  "${HOME}/Android/Sdk/ndk"           # Linux Android Studio default
)
# Windows: %LOCALAPPDATA%\Android\Sdk\ndk under MSYS path translation.
[[ -n "${LOCALAPPDATA:-}" ]] && NDK_BASES+=("${LOCALAPPDATA}/Android/Sdk/ndk")

for base in "${NDK_BASES[@]}"; do
  [[ -d "$base" ]] || continue
  pick=$(ls -1d "$base"/*/ 2>/dev/null | sort -V | tail -1 || true)
  [[ -n "$pick" ]] && NDK_CANDIDATES+=("${pick%/}")
done

ANDROID_NDK_HOME=""
for cand in "${NDK_CANDIDATES[@]}"; do
  if [[ -d "$cand/toolchains/llvm/prebuilt/$NDK_HOST" ]]; then
    ANDROID_NDK_HOME="$cand"
    break
  fi
done

if [[ -z "$ANDROID_NDK_HOME" ]]; then
  echo "❌  Cannot find Android NDK with toolchain prebuilt for host '$NDK_HOST'."
  echo "    Set ANDROID_NDK_HOME, or install NDK via Android Studio →"
  echo "    SDK Manager → SDK Tools → NDK (Side by side)."
  exit 1
fi
export ANDROID_NDK_HOME ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"

echo "◆ Host toolchain: $NDK_HOST"
echo "◆ Using NDK: $ANDROID_NDK_HOME"

# ── Build ─────────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
CORE="$REPO_ROOT/core"
TARGET="aarch64-linux-android"
# Honour CARGO_TARGET_DIR if set (lets callers redirect target/ outside the
# worktree, which avoids Windows-AV access-denied on .worktrees/<x>/target/
# — see AGENTS.md §8).
TARGET_BASE="${CARGO_TARGET_DIR:-$CORE/target}"
RELEASE="$TARGET_BASE/$TARGET/release"

echo "◆ Building phantom (release) for $TARGET …"
echo "◆ Target dir: $TARGET_BASE"
cd "$CORE"
cargo ndk -t arm64-v8a -P 21 build --release --bin phantom

# ── Strip ─────────────────────────────────────────────────────────────────────
STRIP_BIN="llvm-strip"
[[ "$NDK_HOST" == "windows-x86_64" ]] && STRIP_BIN="llvm-strip.exe"
STRIP="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST/bin/$STRIP_BIN"

if [[ -x "$STRIP" ]]; then
  echo "◆ Stripping debug symbols …"
  "$STRIP" "$RELEASE/phantom" -o "$RELEASE/phantom-stripped"
  BINARY="$RELEASE/phantom-stripped"
else
  echo "⚠  llvm-strip not found at $STRIP — shipping unstripped binary"
  BINARY="$RELEASE/phantom"
fi

echo "◆ Binary: $(du -sh "$BINARY" | cut -f1)  →  $BINARY"

# ── Assemble dist/ ────────────────────────────────────────────────────────────
DIST="$REPO_ROOT/dist"
STAGING="$DIST/phantom-android-arm64"
mkdir -p "$STAGING"

cp "$BINARY"                          "$STAGING/phantom"
cp "$REPO_ROOT/agents.toml.example"   "$STAGING/agents.toml.example"

cat > "$STAGING/install.sh" << 'INSTALL'
#!/usr/bin/env sh
# phantom install script for Termux (Android ARM64)
# Run inside Termux: sh install.sh
set -e

DEST="$HOME/.local/bin"
CONFIG_DIR="$HOME/.phantom-mesh"

# Termux uses $PREFIX/bin instead of ~/.local/bin
if [ -n "$PREFIX" ] && [ -d "$PREFIX/bin" ]; then
  DEST="$PREFIX/bin"
fi

mkdir -p "$DEST" "$CONFIG_DIR"

echo "Installing phantom → $DEST/phantom"
cp phantom "$DEST/phantom"
chmod +x "$DEST/phantom"

if [ ! -f "$CONFIG_DIR/agents.toml" ]; then
  cp agents.toml.example "$CONFIG_DIR/agents.toml"
  echo "Config created → $CONFIG_DIR/agents.toml"
  echo "  Edit it and set your API key:"
  echo "  nano $CONFIG_DIR/agents.toml"
fi

echo ""
echo "✓ phantom installed. Run: phantom"
echo ""
echo "To start the serve daemon:"
echo "  phantom serve"
echo ""
echo "To use as an MCP server:"
echo "  phantom mcp"
INSTALL
chmod +x "$STAGING/install.sh"

cat > "$STAGING/README.txt" << 'README'
phantom-mesh — Android / Termux
================================

Requirements
------------
  - Termux (https://termux.dev) on Android 5.0+
  - API key for at least one LLM provider

Quick Start
-----------
  1. Copy this folder to your phone (adb push, WiFi, or GitHub Releases)
  2. Open Termux
  3. cd /path/to/phantom-android-arm64
  4. sh install.sh
  5. Set your API key:
       export ANTHROPIC_API_KEY=sk-ant-...
     or edit ~/.phantom-mesh/agents.toml
  6. phantom                   # interactive REPL
     phantom "list src files"  # one-shot
     phantom serve             # WebSocket daemon (ws://phone-ip:7878/ws)
     phantom mcp               # MCP stdio server
     phantom evolve            # self-iteration loop

Connect from Mac/PC
-------------------
  Start daemon on phone:  phantom serve
  Connect via curl:       curl http://PHONE_IP:7878/healthz

  With Tailscale:
    curl http://100.x.x.x:7878/healthz

MCP integration
---------------
  Add to your Claude Code / Cursor / Goose config:
  { "mcpServers": { "phantom-android": {
      "command": "ssh", "args": ["phone", "phantom mcp"] } } }
README

# ── Compress ──────────────────────────────────────────────────────────────────
TARBALL="$DIST/phantom-android-arm64.tar.gz"
cd "$DIST"
tar czf "$TARBALL" phantom-android-arm64/
rm -rf "$STAGING"

SIZE=$(du -sh "$TARBALL" | cut -f1)
echo ""
echo "✓ Package ready:"
echo "  $TARBALL  ($SIZE)"
echo ""
echo "Install on phone:"
echo "  adb push $TARBALL /sdcard/"
echo "  # then in Termux:"
echo "  cd /sdcard && tar xzf phantom-android-arm64.tar.gz"
echo "  cd phantom-android-arm64 && sh install.sh"
