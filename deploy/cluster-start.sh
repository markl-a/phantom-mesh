#!/bin/bash
# Phantom Mesh Cluster Start Script
# Starts the hub on this machine and reports status.
# Workers are set up individually on each device using Claude Code + SETUP-WORKER.md.

set -e

HUB_HOST="${HUB_HOST:-0.0.0.0}"
HUB_PORT="${HUB_PORT:-7878}"
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$PROJECT_DIR/target/release/phantom-mesh"

echo "=== Phantom Mesh Cluster Start ==="
echo "Project: $PROJECT_DIR"
echo "Hub: $HUB_HOST:$HUB_PORT"

# 1. Build if needed
if [ ! -f "$BINARY" ] || [ "$PROJECT_DIR/src/main.rs" -nt "$BINARY" ]; then
    echo ""
    echo "[1/3] Building phantom-mesh (release)..."
    cd "$PROJECT_DIR"
    cargo build --release
else
    echo ""
    echo "[1/3] Binary up to date, skipping build"
fi

# 2. Start hub
echo ""
echo "[2/3] Starting hub on $HUB_HOST:$HUB_PORT..."
cd "$PROJECT_DIR"
nohup "$BINARY" daemon --host "$HUB_HOST" --port "$HUB_PORT" > /tmp/phantom-mesh-hub.log 2>&1 &
HUB_PID=$!
echo "Hub PID: $HUB_PID"
echo "$HUB_PID" > /tmp/phantom-mesh-hub.pid

# 3. Wait for startup
echo ""
echo "[3/3] Waiting for hub to start..."
for i in $(seq 1 15); do
    if curl -s "http://127.0.0.1:$HUB_PORT/health" > /dev/null 2>&1; then
        echo "Hub is healthy!"
        break
    fi
    sleep 1
    if [ "$i" -eq 15 ]; then
        echo "WARNING: Hub may not be ready yet. Check /tmp/phantom-mesh-hub.log"
    fi
done

echo ""
echo "=== Hub Started ==="
echo "Health:   http://127.0.0.1:$HUB_PORT/health"
echo "Workers:  http://127.0.0.1:$HUB_PORT/cluster/workers"
echo "Metrics:  http://127.0.0.1:$HUB_PORT/cluster/metrics"
echo "Logs:     /tmp/phantom-mesh-hub.log"
echo ""
echo "Next: Deploy workers on other machines using deploy/cluster-package/"
