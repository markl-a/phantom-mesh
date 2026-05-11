#!/bin/bash
set -euo pipefail

# phantom-mesh daemon update script
# Downloads the latest release binary and restarts the service
#
# Environment variables (optional):
#   GITHUB_REPO  — GitHub repo slug, default: markl-a/phantom-mesh

GITHUB_REPO="${GITHUB_REPO:-markl-a/phantom-mesh}"
INSTALL_DIR="/opt/phantom-mesh"
ARCH=$(uname -m)

if [[ "$ARCH" == "aarch64" ]]; then
  BINARY_SUFFIX="aarch64-unknown-linux-gnu"
else
  BINARY_SUFFIX="x86_64-unknown-linux-gnu"
fi

echo "==> Fetching latest phantom-mesh release from ${GITHUB_REPO}..."
LATEST=$(curl -s "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
  | grep "browser_download_url.*$BINARY_SUFFIX" | cut -d'"' -f4)

if [[ -z "$LATEST" ]]; then
  echo "ERROR: Could not find binary for $BINARY_SUFFIX"
  exit 1
fi

echo "==> Stopping phantom-mesh service..."
sudo systemctl stop phantom-mesh

echo "==> Downloading $LATEST..."
sudo curl -L "$LATEST" -o "$INSTALL_DIR/phantom-mesh.new"
sudo chmod +x "$INSTALL_DIR/phantom-mesh.new"

# Atomic swap
sudo mv "$INSTALL_DIR/phantom-mesh" "$INSTALL_DIR/phantom-mesh.bak"
sudo mv "$INSTALL_DIR/phantom-mesh.new" "$INSTALL_DIR/phantom-mesh"

echo "==> Starting phantom-mesh service..."
sudo systemctl start phantom-mesh

echo ""
echo "==> Update complete!"
echo "    Status: sudo systemctl status phantom-mesh"
echo "    Logs:   sudo journalctl -u phantom-mesh -f"
echo "    Backup: $INSTALL_DIR/phantom-mesh.bak (previous binary)"
