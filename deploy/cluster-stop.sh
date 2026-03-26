#!/bin/bash
# Phantom Mesh Cluster Stop Script
# Stops the hub on this machine.

echo "=== Phantom Mesh Cluster Stop ==="

if [ -f /tmp/phantom-mesh-hub.pid ]; then
    HUB_PID=$(cat /tmp/phantom-mesh-hub.pid)
    if kill -0 "$HUB_PID" 2>/dev/null; then
        echo "Stopping hub (PID: $HUB_PID)..."
        kill "$HUB_PID"
        sleep 2
        if kill -0 "$HUB_PID" 2>/dev/null; then
            echo "Force killing..."
            kill -9 "$HUB_PID"
        fi
        echo "Hub stopped."
    else
        echo "Hub process $HUB_PID not running."
    fi
    rm -f /tmp/phantom-mesh-hub.pid
else
    echo "No hub PID file found. Searching for process..."
    pkill -f "phantom-mesh daemon" 2>/dev/null && echo "Killed." || echo "No process found."
fi
