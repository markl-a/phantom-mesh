#!/usr/bin/env bash
# scripts/package-linux.sh — build Debian (.deb) packages for phantom on Linux
#
# Wave H3.1 (Linux half). Two artefacts, two package names (so both can be
# installed side-by-side without a dpkg conflict):
#
#   • CLI  → package `phantom-mesh-cli`, ships /usr/bin/phantom (headless
#            CLI/TUI/serve). Built here with dpkg-deb (no cargo-deb dep).
#   • GUI  → package `phantom-mesh` (Tauri desktop), ships /usr/bin/phantom-mesh-app
#            + .desktop + icons. Built via `tauri build --bundles deb`
#            (needs webkit2gtk-4.1 / gtk-3 / pnpm). Use --gui.
#
# Usage:
#   scripts/package-linux.sh                 # build CLI release binary then package
#   scripts/package-linux.sh --no-build      # CLI, use existing core/target/release/phantom
#   scripts/package-linux.sh --gui           # build the Tauri desktop GUI .deb instead
#   scripts/package-linux.sh --out DIR       # output dir (default: dist/)
#   scripts/package-linux.sh --arch amd64    # override detected arch (CLI only)
#
# Output: <out>/phantom-mesh-cli_<version>_<arch>.deb   (CLI)
#         <out>/Phantom_Mesh_<version>_<arch>.deb        (--gui)
# Exit:   0 ok, 1 build/package error, 2 bad args.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DO_BUILD=1
DO_GUI=0
OUT_DIR="$REPO_ROOT/dist"
ARCH=""

while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) DO_BUILD=0; shift ;;
    --gui)      DO_GUI=1; shift ;;
    --out)      OUT_DIR="${2:?--out needs a dir}"; shift 2 ;;
    --arch)     ARCH="${2:?--arch needs a value}"; shift 2 ;;
    --help|-h)  sed -n '2,24p' "$0"; exit 0 ;;
    *)          echo "package-linux: unknown arg '$1'" >&2; exit 2 ;;
  esac
done

command -v dpkg-deb >/dev/null 2>&1 || { echo "FATAL: dpkg-deb not found (apt-get install dpkg)" >&2; exit 1; }

# ── GUI mode: delegate to Tauri's deb bundler ─────────────────────────────
# The Tauri desktop .deb (package `phantom-mesh`) is produced by the tauri CLI,
# which compiles the GUI app crate + bundles /usr/bin/phantom-mesh-app, a
# .desktop entry, and hicolor icons. Needs the Tauri-v2 Linux build deps.
if [ "$DO_GUI" = 1 ]; then
  APP_DIR="$REPO_ROOT/app"
  command -v corepack >/dev/null 2>&1 || {
    echo "FATAL: corepack not found — install Node (nvm) so corepack/pnpm is on PATH" >&2; exit 1; }
  if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    echo "FATAL: webkit2gtk-4.1 dev libs missing. Install Tauri-v2 Linux build deps:" >&2
    echo "  sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential libxdo-dev \\" >&2
    echo "    libssl-dev libayatana-appindicator3-dev librsvg2-dev libgtk-3-dev patchelf" >&2
    exit 1
  fi
  echo "package-linux: building Tauri desktop GUI .deb (compiles app crate — several minutes)…"
  BUNDLE_DIR="$APP_DIR/src-tauri/target/release/bundle/deb"
  # Drop stale debs so we never copy a previous version's artefact.
  rm -f "$BUNDLE_DIR"/*.deb 2>/dev/null || true
  ( cd "$APP_DIR" \
      && corepack pnpm install --frozen-lockfile \
      && CI=true corepack pnpm exec tauri build --bundles deb )
  # Pick the most recently modified .deb (defensive if multiple linger).
  BUILT="$(find "$BUNDLE_DIR" -maxdepth 1 -name '*.deb' -printf '%T@ %p\n' 2>/dev/null \
            | sort -rn | head -1 | cut -d' ' -f2-)"
  [ -n "$BUILT" ] && [ -f "$BUILT" ] || { echo "FATAL: tauri build produced no .deb" >&2; exit 1; }
  mkdir -p "$OUT_DIR"
  # Normalise spaces in Tauri's "Phantom Mesh_<ver>_amd64.deb" to underscores.
  GUI_DEST="$OUT_DIR/$(basename "$BUILT" | tr ' ' '_')"
  cp "$BUILT" "$GUI_DEST"
  echo "package-linux: wrote GUI $GUI_DEST ($(du -h "$GUI_DEST" | cut -f1))"
  echo "── dpkg-deb --info ──"
  dpkg-deb --info "$GUI_DEST" | sed 's/^/  /'
  exit 0
fi

# ── Resolve metadata from Cargo.toml ──────────────────────────────────────
CARGO_TOML="$REPO_ROOT/core/Cargo.toml"
VERSION="$(awk -F' *= *' '/^\[package\]/{p=1} p&&/^version/{gsub(/"/,"",$2); print $2; exit}' "$CARGO_TOML")"
[ -n "$VERSION" ] || { echo "FATAL: could not read version from $CARGO_TOML" >&2; exit 1; }
# Debian versions disallow '-' in the upstream part for some tooling; keep rc as ~rc
DEB_VERSION="${VERSION/-rc./~rc}"

if [ -z "$ARCH" ]; then
  case "$(uname -m)" in
    x86_64)  ARCH=amd64 ;;
    aarch64) ARCH=arm64 ;;
    *)       ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)" ;;
  esac
fi

echo "package-linux: phantom-mesh $VERSION ($DEB_VERSION) arch=$ARCH"

# ── Build (unless --no-build) ─────────────────────────────────────────────
BIN="$REPO_ROOT/core/target/release/phantom"
if [ "$DO_BUILD" = 1 ]; then
  echo "package-linux: building release binary…"
  ( cd "$REPO_ROOT/core" && cargo build --release --bin phantom )
fi
[ -x "$BIN" ] || { echo "FATAL: phantom binary not found at $BIN (run without --no-build)" >&2; exit 1; }

# ── Stage debian tree ─────────────────────────────────────────────────────
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"   # mktemp -d is 700; dpkg root dir must be 755
PKG="phantom-mesh-cli"

install -Dm755 "$BIN" "$STAGE/usr/bin/phantom"

# ── systemd service (T-WLA-07: make the advertised "serve daemon" installable
# end-to-end). Ship the unit + a postinst that creates the unprivileged service
# user and registers the unit. The unit's ExecStart=/usr/bin/phantom serve
# matches the binary path installed above, and it carries Alias=phantom.service.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
install -Dm644 "$SCRIPT_DIR/phantom-mesh.service" "$STAGE/usr/lib/systemd/system/phantom-mesh.service"

mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
# Unprivileged system user the unit runs as (User=phantom, HOME=/home/phantom).
if ! getent passwd phantom >/dev/null 2>&1; then
    useradd --system --home-dir /home/phantom --create-home --shell /usr/sbin/nologin phantom || true
fi
install -d -o phantom -g phantom -m 700 /home/phantom/.phantom-mesh || true
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
    # Enable on boot, but do NOT start now — operator must write agents.toml first.
    systemctl enable phantom-mesh.service || true
fi
POSTINST
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
    systemctl disable --now phantom-mesh.service || true
fi
PRERM
chmod 755 "$STAGE/DEBIAN/prerm"

# Control file
INSTALLED_KB="$(du -k "$STAGE/usr/bin/phantom" | cut -f1)"
mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $DEB_VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $INSTALLED_KB
Maintainer: phantom-mesh maintainers <noreply@phantom-mesh.local>
Homepage: https://github.com/markl-a/phantom-mesh
Description: Phantom Mesh — AI agent mesh CLI / terminal
 Phantom is a peer-to-peer AI agent mesh. This package installs the headless
 phantom CLI + TUI terminal (interactive REPL, headless exec, and the
 serve daemon) for Linux. The desktop GUI is packaged separately.
EOF

# Copyright (dual MIT/Apache-2.0 per core/Cargo.toml)
install -d "$STAGE/usr/share/doc/$PKG"
cat > "$STAGE/usr/share/doc/$PKG/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: phantom-mesh
Source: https://github.com/markl-a/phantom-mesh

Files: *
License: MIT or Apache-2.0
 This package is dual-licensed under the MIT license and the Apache License 2.0.
 See the upstream repository for full license texts.
EOF

# ── Build the .deb ────────────────────────────────────────────────────────
mkdir -p "$OUT_DIR"
DEB_PATH="$OUT_DIR/${PKG}_${DEB_VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB_PATH" >/dev/null

echo "package-linux: wrote $DEB_PATH ($(du -h "$DEB_PATH" | cut -f1))"
echo "── dpkg-deb --info ──"
dpkg-deb --info "$DEB_PATH" | sed 's/^/  /'
echo "── contents ──"
dpkg-deb --contents "$DEB_PATH" | sed 's/^/  /'
