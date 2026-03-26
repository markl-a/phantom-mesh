#!/bin/bash
# Full Worker Installation Script
# Run this on macOS/Linux machines with Rust installed.

set -e

echo "=== Phantom Mesh Full Worker Setup ==="

# Check Rust
if ! command -v cargo &> /dev/null; then
    echo "Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "Rust version: $(rustc --version)"

# Build
if [ -d "phantom-mesh" ]; then
    echo "Building phantom-mesh..."
    cd phantom-mesh
    cargo build --release
    echo ""
    echo "Build complete: ./target/release/phantom-mesh"
    echo ""
    echo "Start with:"
    echo "  ./target/release/phantom-mesh worker --hub http://<HUB_IP>:7878 --name <NAME> --port 7879"
else
    echo "ERROR: phantom-mesh source not found."
    echo "Clone the repo first, then run this script from the parent directory."
fi
