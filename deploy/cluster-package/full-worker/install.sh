#!/bin/bash
# Full Worker Installation Script
# Run this on macOS/Linux machines with Rust installed.

set -e

echo "=== Clawtex Full Worker Setup ==="

# Check Rust
if ! command -v cargo &> /dev/null; then
    echo "Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "Rust version: $(rustc --version)"

# Build
if [ -d "clawtex-core" ]; then
    echo "Building clawtex-core..."
    cd clawtex-core
    cargo build --release
    echo ""
    echo "Build complete: ./target/release/clawtex-core"
    echo ""
    echo "Start with:"
    echo "  ./target/release/clawtex-core worker --hub http://<HUB_IP>:7878 --name <NAME> --port 7879"
else
    echo "ERROR: clawtex-core source not found."
    echo "Clone the repo first, then run this script from the parent directory."
fi
