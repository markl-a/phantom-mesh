#!/bin/bash
# Phantom Mesh Cluster Status Script
# Checks hub health and worker status.

HUB_PORT="${HUB_PORT:-7878}"
HUB_URL="http://127.0.0.1:$HUB_PORT"

echo "=== Phantom Mesh Cluster Status ==="

# Check hub
echo ""
echo "--- Hub ---"
if curl -s "$HUB_URL/health" > /dev/null 2>&1; then
    echo "Status: ONLINE"
    curl -s "$HUB_URL/health" | python3 -m json.tool 2>/dev/null || curl -s "$HUB_URL/health"
else
    echo "Status: OFFLINE"
    echo "Hub is not responding on port $HUB_PORT"
    exit 1
fi

# Check workers
echo ""
echo "--- Workers ---"
WORKERS=$(curl -s "$HUB_URL/cluster/workers" 2>/dev/null)
if [ -n "$WORKERS" ]; then
    echo "$WORKERS" | python3 -m json.tool 2>/dev/null || echo "$WORKERS"
else
    echo "No worker data available"
fi

# Check metrics
echo ""
echo "--- Metrics ---"
METRICS=$(curl -s "$HUB_URL/cluster/metrics" 2>/dev/null)
if [ -n "$METRICS" ]; then
    echo "$METRICS" | python3 -m json.tool 2>/dev/null || echo "$METRICS"
else
    echo "No metrics data available"
fi

# Check e-stop
echo ""
echo "--- E-Stop ---"
ESTOP=$(curl -s "$HUB_URL/estop" 2>/dev/null)
if [ -n "$ESTOP" ]; then
    echo "$ESTOP" | python3 -m json.tool 2>/dev/null || echo "$ESTOP"
fi
