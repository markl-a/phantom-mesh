#!/bin/bash
# Setup iOS sandbox app for simulator testing
# Run: chmod +x scripts/setup-ios-sandbox.sh && ./scripts/setup-ios-sandbox.sh

set -e

ARCHIVE_APP="app/src-tauri/gen/apple/build/spectyn-mesh-app_iOS.xcarchive/Products/Applications/Spectyn Mesh.app"
SANDBOX_DIR="ios-sandbox"

mkdir -p "$SANDBOX_DIR"

echo "Copying app bundle..."
cp -R "$ARCHIVE_APP" "$SANDBOX_DIR/"

# Remove device signing (keep unsigned for simulator)
rm -rf "$SANDBOX_DIR/Spectyn Mesh.app/_CodeSignature"
rm -f "$SANDBOX_DIR/Spectyn Mesh.app/embedded.mobileprovision"

echo "Verifying app..."
codesign -dv "$SANDBOX_DIR/Spectyn Mesh.app" 2>&1 || echo "(unsigned expected)"

echo ""
echo "✅ iOS sandbox app ready: $SANDBOX_DIR/Spectyn Mesh.app"
echo ""
echo "To install on simulator:"
echo "  SIM_ID=\$(xcrun simctl list devices available | grep 'iPhone 15 Pro' | grep -oE '[A-F0-9-]{36}' | head -1)"
echo "  xcrun simctl install \$SIM_ID '$SANDBOX_DIR/Spectyn Mesh.app'"
echo "  xcrun launch \$SIM_ID ai.spectynmesh.app"