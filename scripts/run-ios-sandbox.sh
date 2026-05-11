#!/bin/bash
# Install and launch iOS sandbox app on simulator
# Run: ./scripts/run-ios-sandbox.sh

set -e

SANDBOX_APP="ios-sandbox/Phantom Mesh.app"
SIM_ID="FACB6871-1346-4761-8425-A858E1B39CCE"  # iPhone 15 Pro

echo "Installing on simulator..."
xcrun simctl install "$SIM_ID" "$SANDBOX_APP"

echo "Launching Phantom Mesh..."
xcrun simctl launch "$SIM_ID" ai.phantommesh.app --console-pty &

echo ""
echo "✅ App launched!"
echo ""
echo "View logs:"
echo "  log show --predicate 'subsystem == \"ai.phantommesh.app\"' --last 5m"