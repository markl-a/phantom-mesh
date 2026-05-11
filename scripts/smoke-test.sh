#!/bin/bash
HOST=${1:-localhost}
PORT=${2:-7878}
BASE="http://$HOST:$PORT"

echo "Testing phantom-mesh at $BASE"
curl -sf "$BASE/health" | jq . || { echo "FAIL: /health"; exit 1; }
echo "OK: /health"

curl -sf "$BASE/rpc/peers" | jq . || { echo "FAIL: /rpc/peers"; exit 1; }
echo "OK: /rpc/peers"

curl -sf "$BASE/tools" | jq '.tools | length' || { echo "FAIL: /tools"; exit 1; }
echo "OK: /tools"

echo "All checks passed!"
