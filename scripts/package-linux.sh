#!/usr/bin/env bash
# scripts/package-linux.sh — build Linux packages for phantom (.deb / .rpm / AppImage)
#
# Wave H3.1 (Linux half) + LIN-PKG-1/LIN-PKG-2. Multiple artefacts; the CLI
# variants ship the same /usr/bin/phantom + systemd unit so they install the
# headless CLI/TUI/serve daemon end-to-end:
#
#   • CLI .deb      → package `phantom-mesh-cli`, ships /usr/bin/phantom
#                     (headless CLI/TUI/serve). Built with dpkg-deb (default).
#   • CLI .rpm      → package `phantom-mesh-cli`, mirrors the .deb tree
#                     (/usr/bin/phantom + systemd unit + %post useradd/enable
#                     + %preun disable). Built via rpmbuild or fpm. Use --rpm.
#   • CLI AppImage  → self-contained dist/phantom-mesh-<version>-<arch>.AppImage
#                     wrapping /usr/bin/phantom via an AppRun + .desktop, built
#                     with linuxdeploy/appimagetool. Use --appimage.
#   • GUI .deb      → package `phantom-mesh` (Tauri desktop), ships
#                     /usr/bin/phantom-mesh-app + .desktop + icons. Built via
#                     `tauri build --bundles deb` (needs webkit2gtk-4.1 / gtk-3
#                     / pnpm). Use --gui.
#
# Usage:
#   scripts/package-linux.sh                 # build CLI release binary then package .deb
#   scripts/package-linux.sh --no-build      # CLI, use existing core/target/release/phantom
#   scripts/package-linux.sh --rpm           # build the CLI .rpm (rpmbuild or fpm)
#   scripts/package-linux.sh --appimage      # build the CLI AppImage (appimagetool)
#   scripts/package-linux.sh --gui           # build the Tauri desktop GUI .deb instead
#   scripts/package-linux.sh --out DIR       # output dir (default: dist/)
#   scripts/package-linux.sh --arch amd64    # override detected arch (CLI only)
#
# Output: <out>/phantom-mesh-cli_<version>_<arch>.deb          (CLI .deb)
#         <out>/phantom-mesh-cli-<version>.<arch>.rpm           (--rpm)
#         <out>/phantom-mesh-<version>-<arch>.AppImage          (--appimage)
#         <out>/Phantom_Mesh_<version>_<arch>.deb               (--gui)
# Exit:   0 ok, 1 build/package/tool-missing error, 2 bad args.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DO_BUILD=1
DO_GUI=0
DO_RPM=0
DO_APPIMAGE=0
OUT_DIR="$REPO_ROOT/dist"
ARCH=""

while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) DO_BUILD=0; shift ;;
    --gui)      DO_GUI=1; shift ;;
    --rpm)      DO_RPM=1; shift ;;
    --appimage) DO_APPIMAGE=1; shift ;;
    --out)      OUT_DIR="${2:?--out needs a dir}"; shift 2 ;;
    --arch)     ARCH="${2:?--arch needs a value}"; shift 2 ;;
    --help|-h)  sed -n '2,35p' "$0"; exit 0 ;;
    *)          echo "package-linux: unknown arg '$1'" >&2; exit 2 ;;
  esac
done

# .deb tooling is only required for the dpkg-built variants (default CLI + GUI).
# The --rpm / --appimage modes check for their own tools further down.
if [ "$DO_RPM" = 0 ] && [ "$DO_APPIMAGE" = 0 ]; then
  command -v dpkg-deb >/dev/null 2>&1 || { echo "FATAL: dpkg-deb not found (apt-get install dpkg)" >&2; exit 1; }
fi

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

# Map the Debian arch (amd64/arm64) onto the RPM/AppImage convention (x86_64/
# aarch64). Reuses the .deb arch detection above so all three artefacts agree
# on the target; only the spelling differs per ecosystem.
case "$ARCH" in
  amd64) RPMARCH=x86_64 ;;
  arm64) RPMARCH=aarch64 ;;
  *)     RPMARCH="$ARCH" ;;   # already an rpm-style arch (e.g. --arch x86_64)
esac

# ── RPM mode: mirror the .deb tree as an .rpm (LIN-PKG-2) ──────────────────
# Ships the same /usr/bin/phantom + systemd unit, and reproduces the .deb's
# postinst/prerm as RPM scriptlets: %post creates the unprivileged `phantom`
# user + enables the unit, %preun disables it on uninstall. Prefers rpmbuild
# (spec file); falls back to fpm if only that is present.
if [ "$DO_RPM" = 1 ]; then
  PKG="phantom-mesh-cli"
  # RPM Version: must not contain '-'; map a -rc.N pre-release onto ~rc.N which
  # rpm sorts *below* the final release, mirroring the .deb DEB_VERSION rule.
  RPM_VERSION="${VERSION/-rc./~rc}"

  HAVE_RPMBUILD=0; command -v rpmbuild >/dev/null 2>&1 && HAVE_RPMBUILD=1
  HAVE_FPM=0;      command -v fpm      >/dev/null 2>&1 && HAVE_FPM=1
  if [ "$HAVE_RPMBUILD" = 0 ] && [ "$HAVE_FPM" = 0 ]; then
    echo "FATAL: no RPM packaging tool found — need 'rpmbuild' or 'fpm'." >&2
    echo "  Install one of:" >&2
    echo "    sudo dnf install rpm-build        # rpmbuild (Fedora/RHEL/openSUSE)" >&2
    echo "    sudo apt-get install rpm          # rpmbuild (Debian/Ubuntu)" >&2
    echo "    gem install fpm                   # fpm (any distro w/ Ruby)" >&2
    exit 1
  fi

  RPM_DEST="$OUT_DIR/${PKG}-${RPM_VERSION}.${RPMARCH}.rpm"
  mkdir -p "$OUT_DIR"
  echo "package-linux: building CLI .rpm $PKG $RPM_VERSION ($RPMARCH)…"

  if [ "$HAVE_RPMBUILD" = 0 ]; then
    # fpm fallback path (taken only when rpmbuild is absent — rpmbuild is the
    # preferred native path below). Build a buildroot mirroring the .deb tree,
    # then let fpm emit the .rpm with the same scriptlet behaviour via
    # --rpm-* / --*-script hooks.
    ROOT="$(mktemp -d)"; SCR="$(mktemp -d)"
    trap 'rm -rf "$ROOT" "$SCR"' EXIT
    install -Dm755 "$BIN" "$ROOT/usr/bin/phantom"
    install -Dm644 "$SCRIPT_DIR/phantom-mesh.service" \
      "$ROOT/usr/lib/systemd/system/phantom-mesh.service"
    cat > "$SCR/after-install.sh" <<'POST'
#!/bin/sh
set -e
if ! getent passwd phantom >/dev/null 2>&1; then
    useradd --system --home-dir /home/phantom --create-home --shell /usr/sbin/nologin phantom || true
fi
install -d -o phantom -g phantom -m 700 /home/phantom/.phantom-mesh || true
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
    systemctl enable phantom-mesh.service || true
fi
POST
    cat > "$SCR/before-remove.sh" <<'PREUN'
#!/bin/sh
set -e
# $1 == 0 on final erase (not on upgrade) — match the .deb prerm semantics.
if [ "$1" = 0 ] && [ -d /run/systemd/system ]; then
    systemctl disable --now phantom-mesh.service || true
fi
PREUN
    rm -f "$RPM_DEST"
    fpm -s dir -t rpm -n "$PKG" -v "$RPM_VERSION" -a "$RPMARCH" \
      --license 'AGPL-3.0-only' \
      --maintainer 'phantom-mesh maintainers <noreply@phantom-mesh.local>' \
      --url 'https://github.com/markl-a/phantom-mesh' \
      --description 'Phantom Mesh — AI agent mesh CLI / terminal' \
      --after-install "$SCR/after-install.sh" \
      --before-remove "$SCR/before-remove.sh" \
      -p "$RPM_DEST" \
      -C "$ROOT" usr
  else
    # rpmbuild path — write a minimal spec under a private %_topdir and build a
    # binary RPM from a pre-staged tree (no %prep/%build compile).
    #
    # The staged tree lives in a SOURCE dir ($STAGE), NOT the rpm buildroot:
    # rpmbuild empties %{buildroot} at the top of %install, so staging directly
    # into the buildroot (and passing --buildroot) both wipes the files and makes
    # the %install `cp` a no-op self-copy ("are the same file"). Stage separately
    # and let %install copy $STAGE into the rpmbuild-managed %{buildroot}.
    TOP="$(mktemp -d)"
    trap 'rm -rf "$TOP"' EXIT
    mkdir -p "$TOP/BUILD" "$TOP/RPMS" "$TOP/SOURCES" "$TOP/SPECS"
    STAGE="$TOP/stage"
    install -Dm755 "$BIN" "$STAGE/usr/bin/phantom"
    install -Dm644 "$SCRIPT_DIR/phantom-mesh.service" \
      "$STAGE/usr/lib/systemd/system/phantom-mesh.service"
    SPEC="$TOP/SPECS/${PKG}.spec"
    cat > "$SPEC" <<SPEC
Name:           $PKG
Version:        $RPM_VERSION
Release:        1
Summary:        Phantom Mesh — AI agent mesh CLI / terminal
License:        AGPLv3
URL:            https://github.com/markl-a/phantom-mesh
BuildArch:      $RPMARCH
%global debug_package %{nil}

%description
Phantom is a peer-to-peer AI agent mesh. This package installs the headless
phantom CLI + TUI terminal (interactive REPL, headless exec, and the serve
daemon) for Linux. The desktop GUI is packaged separately.

%install
# Copy the package-linux.sh-staged tree into the rpmbuild-managed %{buildroot}.
# (rpmbuild has already emptied %{buildroot} before this runs.)
mkdir -p %{buildroot}
cp -a "$STAGE"/. %{buildroot}/

%files
/usr/bin/phantom
/usr/lib/systemd/system/phantom-mesh.service

%post
if ! getent passwd phantom >/dev/null 2>&1; then
    useradd --system --home-dir /home/phantom --create-home --shell /usr/sbin/nologin phantom || true
fi
install -d -o phantom -g phantom -m 700 /home/phantom/.phantom-mesh || true
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
    systemctl enable phantom-mesh.service || true
fi

%preun
# \$1 == 0 on final erase (not on upgrade) — match the .deb prerm semantics.
if [ "\$1" = 0 ] && [ -d /run/systemd/system ]; then
    systemctl disable --now phantom-mesh.service || true
fi
SPEC
    rm -f "$RPM_DEST"
    # No --buildroot: let rpmbuild manage %{buildroot} under $TOP. %install copies
    # the staged tree in (passing --buildroot here would resurrect the self-copy bug).
    rpmbuild --define "_topdir $TOP" -bb "$SPEC"
    BUILT="$(find "$TOP/RPMS" -name '*.rpm' -type f | head -1)"
    [ -n "$BUILT" ] && [ -f "$BUILT" ] || { echo "FATAL: rpmbuild produced no .rpm" >&2; exit 1; }
    cp "$BUILT" "$RPM_DEST"
  fi

  [ -f "$RPM_DEST" ] || { echo "FATAL: rpm build produced no artefact at $RPM_DEST" >&2; exit 1; }
  echo "package-linux: wrote $RPM_DEST ($(du -h "$RPM_DEST" | cut -f1))"
  if command -v rpm >/dev/null 2>&1; then
    echo "── rpm -qip ──"
    rpm -qip "$RPM_DEST" 2>/dev/null | sed 's/^/  /' || true
    echo "── rpm -qlp ──"
    rpm -qlp "$RPM_DEST" 2>/dev/null | sed 's/^/  /' || true
  fi
  exit 0
fi

# ── AppImage mode: wrap /usr/bin/phantom in a relocatable AppImage (LIN-PKG-1) ─
# Builds an AppDir { AppRun, .desktop, icon, usr/bin/phantom } and folds it into
# a single self-contained dist/phantom-mesh-<version>-<arch>.AppImage. Prefers
# linuxdeploy (which also pulls runtime libs); falls back to a plain
# appimagetool run over the hand-built AppDir.
if [ "$DO_APPIMAGE" = 1 ]; then
  HAVE_LINUXDEPLOY=0; command -v linuxdeploy   >/dev/null 2>&1 && HAVE_LINUXDEPLOY=1
  HAVE_APPIMAGETOOL=0; command -v appimagetool >/dev/null 2>&1 && HAVE_APPIMAGETOOL=1
  if [ "$HAVE_LINUXDEPLOY" = 0 ] && [ "$HAVE_APPIMAGETOOL" = 0 ]; then
    echo "FATAL: no AppImage tool found — need 'appimagetool' or 'linuxdeploy'." >&2
    echo "  Install one of (then re-run with --appimage):" >&2
    echo "    # appimagetool" >&2
    echo "    wget -O ~/.local/bin/appimagetool \\" >&2
    echo "      https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" >&2
    echo "    chmod +x ~/.local/bin/appimagetool" >&2
    echo "    # linuxdeploy" >&2
    echo "    wget -O ~/.local/bin/linuxdeploy \\" >&2
    echo "      https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" >&2
    echo "    chmod +x ~/.local/bin/linuxdeploy" >&2
    exit 1
  fi

  APPDIR="$(mktemp -d)/phantom-mesh.AppDir"
  trap 'rm -rf "$(dirname "$APPDIR")"' EXIT
  install -Dm755 "$BIN" "$APPDIR/usr/bin/phantom"

  # .desktop — required by appimagetool; categories/keys mirror the Tauri entry.
  install -d "$APPDIR/usr/share/applications"
  cat > "$APPDIR/usr/share/applications/phantom-mesh.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Phantom Mesh
Comment=Phantom Mesh — AI agent mesh CLI / terminal
Exec=phantom
Icon=phantom-mesh
Categories=Utility;Development;
Terminal=true
DESKTOP
  # appimagetool wants the .desktop at the AppDir root too.
  cp "$APPDIR/usr/share/applications/phantom-mesh.desktop" "$APPDIR/phantom-mesh.desktop"

  # Icon — reuse the Tauri app icon if present, else synthesise a 1x1 placeholder
  # so appimagetool (which requires an icon) still succeeds.
  ICON_SRC="$REPO_ROOT/app/src-tauri/icons/128x128.png"
  install -d "$APPDIR/usr/share/icons/hicolor/128x128/apps"
  if [ -f "$ICON_SRC" ]; then
    install -Dm644 "$ICON_SRC" "$APPDIR/usr/share/icons/hicolor/128x128/apps/phantom-mesh.png"
    cp "$ICON_SRC" "$APPDIR/phantom-mesh.png"
  else
    # 1x1 transparent PNG (base64) — minimal valid icon.
    printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC' \
      | base64 -d > "$APPDIR/phantom-mesh.png" 2>/dev/null || true
    cp "$APPDIR/phantom-mesh.png" "$APPDIR/usr/share/icons/hicolor/128x128/apps/phantom-mesh.png" 2>/dev/null || true
  fi

  # AppRun — entrypoint that execs the bundled CLI, forwarding all args.
  cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/phantom" "$@"
APPRUN
  chmod 755 "$APPDIR/AppRun"

  mkdir -p "$OUT_DIR"
  APPIMAGE_DEST="$OUT_DIR/phantom-mesh-${VERSION}-${RPMARCH}.AppImage"
  rm -f "$APPIMAGE_DEST"
  echo "package-linux: building CLI AppImage phantom-mesh $VERSION ($RPMARCH)…"

  if [ "$HAVE_LINUXDEPLOY" = 1 ]; then
    # linuxdeploy with its appimage output plugin folds the AppDir + deps into
    # the final image; point OUTPUT at our dist name.
    ( cd "$OUT_DIR" && OUTPUT="$(basename "$APPIMAGE_DEST")" \
        linuxdeploy --appdir "$APPDIR" \
          --desktop-file "$APPDIR/usr/share/applications/phantom-mesh.desktop" \
          --icon-file "$APPDIR/phantom-mesh.png" \
          --output appimage )
  else
    # appimagetool consumes the hand-built AppDir directly. ARCH env tells it
    # which runtime to embed.
    ARCH="$RPMARCH" appimagetool "$APPDIR" "$APPIMAGE_DEST"
  fi

  [ -f "$APPIMAGE_DEST" ] || { echo "FATAL: AppImage build produced no artefact at $APPIMAGE_DEST" >&2; exit 1; }
  chmod +x "$APPIMAGE_DEST" 2>/dev/null || true
  echo "package-linux: wrote $APPIMAGE_DEST ($(du -h "$APPIMAGE_DEST" | cut -f1))"
  # Self-check the LIN-PKG-1 acceptance: the artefact must run on a glibc host
  # and print its version. Best-effort — a direct run needs FUSE, so fall back to
  # the FUSE-free extract-and-run path; never fail the build over a sandbox that
  # has neither (the artefact is already written and valid).
  echo "── AppImage --version self-check ──"
  VER_OUT="$("$APPIMAGE_DEST" --version 2>/dev/null \
            || APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE_DEST" --version 2>/dev/null || true)"
  if [ -n "$VER_OUT" ]; then
    echo "  $VER_OUT"
  else
    echo "  (could not self-run here — needs FUSE or APPIMAGE_EXTRACT_AND_RUN=1; artefact still written)"
  fi
  exit 0
fi

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
# (SCRIPT_DIR is resolved once near the top of this script.)
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

# Copyright (AGPL-3.0-only per core/Cargo.toml — this packages the `phantom`
# CLI built from core/, which is AGPL; the permissive pm-types SDK crate is not
# shipped as a standalone artifact here.)
install -d "$STAGE/usr/share/doc/$PKG"
cat > "$STAGE/usr/share/doc/$PKG/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: phantom-mesh
Source: https://github.com/markl-a/phantom-mesh

Files: *
License: AGPL-3.0-only
 This package is licensed under the GNU Affero General Public License v3.0.
 See the upstream repository for the full license text.
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
