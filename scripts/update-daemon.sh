#!/bin/bash
set -euo pipefail

# spectyn-mesh daemon update script
# Downloads the latest release binary and restarts the service
#
# Environment variables (optional):
#   GITHUB_REPO  — GitHub repo slug, default: markl-a/spectyn-mesh

GITHUB_REPO="${GITHUB_REPO:-markl-a/spectyn-mesh}"
INSTALL_DIR="/opt/spectyn-mesh"
ARCH=$(uname -m)

if [[ "$ARCH" == "aarch64" ]]; then
  BINARY_SUFFIX="aarch64-unknown-linux-gnu"
else
  BINARY_SUFFIX="x86_64-unknown-linux-gnu"
fi

# Load the SHA256 + HTTPS verification helper.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd 2>/dev/null || echo "")"
VERIFY_HELPER=""
if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/_verify-download.sh" ]; then
    VERIFY_HELPER="$SCRIPT_DIR/_verify-download.sh"
else
    VERIFY_HELPER="$(mktemp -t spectyn-verify.XXXXXX)"
    HELPER_URL="https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/_verify-download.sh"
    if ! curl -fsSL --max-time 10 "$HELPER_URL" -o "$VERIFY_HELPER"; then
        echo "ERROR: Could not load $HELPER_URL — refusing to update unverified binary" >&2
        exit 1
    fi
fi
# shellcheck disable=SC1090
. "$VERIFY_HELPER"

echo "==> Fetching latest spectyn-mesh release from ${GITHUB_REPO}..."
LATEST=$(curl -s "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
  | grep "browser_download_url.*$BINARY_SUFFIX" | cut -d'"' -f4)

if [[ -z "$LATEST" ]]; then
  echo "ERROR: Could not find binary for $BINARY_SUFFIX"
  exit 1
fi

# Enforce HTTPS on the GitHub release URL. (GitHub always redirects to
# https, but the API response could be tampered with on a hostile network —
# this gives us a definite negative if the URL ever became plain http.)
require_https "$LATEST" || exit 1

echo "==> Stopping spectyn-mesh service..."
sudo systemctl stop spectyn-mesh

echo "==> Downloading $LATEST..."
sudo curl -fsSL "$LATEST" -o "$INSTALL_DIR/spectyn-mesh.new"
# Verify SHA256 BEFORE chmod +x or atomic swap. verify_sha256 deletes the
# .new binary on mismatch; we then need to clean up + restart spectyn-mesh
# with the old binary so the service comes back online.
if ! verify_sha256 "$INSTALL_DIR/spectyn-mesh.new" "$LATEST"; then
  echo "==> Verification failed — restarting old spectyn-mesh and aborting" >&2
  sudo systemctl start spectyn-mesh
  exit 1
fi
sudo chmod +x "$INSTALL_DIR/spectyn-mesh.new"

# Atomic swap
sudo mv "$INSTALL_DIR/spectyn-mesh" "$INSTALL_DIR/spectyn-mesh.bak"
sudo mv "$INSTALL_DIR/spectyn-mesh.new" "$INSTALL_DIR/spectyn-mesh"

echo "==> Starting spectyn-mesh service..."
sudo systemctl start spectyn-mesh

echo ""
echo "==> Update complete!"
echo "    Status: sudo systemctl status spectyn-mesh"
echo "    Logs:   sudo journalctl -u spectyn-mesh -f"
echo "    Backup: $INSTALL_DIR/spectyn-mesh.bak (previous binary)"
