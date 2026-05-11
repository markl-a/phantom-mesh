#!/usr/bin/env bash
# Day 2 helper: verify Windows WSL2 worker reachability and HMAC task dispatch.
#
# Usage:
#   scripts/day2-windows-verify.sh \
#     --secret 'phantom-cluster-2026' \
#     100.87.70.65:7879 100.106.176.125:7878 100.107.205.98:7878
#
# Or let it read cluster_secret from ~/.phantom-mesh/agents.toml:
#   scripts/day2-windows-verify.sh 100.87.70.65:7879

set -euo pipefail

SECRET=""
PROMPT='hostname; uname -a'
POLL_SECS=2
POLL_COUNT=3
NODES=()

usage() {
  cat <<'EOF'
Usage: day2-windows-verify.sh [options] <node:port> [node:port ...]

Options:
  --secret <value>    Cluster secret to use for HMAC auth.
  --prompt <value>    Prompt sent through /rpc/task/assign.
  --poll-secs <n>     Sleep between status polls (default: 2).
  --poll-count <n>    Number of status polls per job (default: 3).
  -h, --help          Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --secret) SECRET="$2"; shift 2 ;;
    --prompt) PROMPT="$2"; shift 2 ;;
    --poll-secs) POLL_SECS="$2"; shift 2 ;;
    --poll-count) POLL_COUNT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) NODES+=("$1"); shift ;;
  esac
done

if [[ ${#NODES[@]} -eq 0 ]]; then
  usage
  exit 1
fi

if [[ -z "$SECRET" && -f "$HOME/.phantom-mesh/agents.toml" ]]; then
  SECRET="$(grep cluster_secret "$HOME/.phantom-mesh/agents.toml" | head -1 | cut -d'"' -f2 || true)"
fi

if [[ -z "$SECRET" ]]; then
  echo "error: cluster secret not provided and not found in ~/.phantom-mesh/agents.toml" >&2
  exit 1
fi

for node in "${NODES[@]}"; do
  echo "=== $node ==="

  echo "-- healthz"
  curl -s --max-time 5 "http://$node/healthz" || true
  echo

  BODY=$(python3 -c 'import json,sys; print(json.dumps({"agent":"master","prompt":sys.argv[1]}))' "$PROMPT")
  AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')

  echo "-- task assign"
  RESP=$(curl -s --max-time 30 -X POST "http://$node/rpc/task/assign" \
    -H "X-Cluster-Auth: $AUTH" \
    -H "Content-Type: application/json" \
    -d "$BODY")
  echo "$RESP"

  JOB_ID=$(printf '%s' "$RESP" | python3 -c 'import json,sys; 
try:
    print(json.load(sys.stdin).get("job_id",""))
except Exception:
    print("")')

  if [[ -n "$JOB_ID" ]]; then
    for ((i=1; i<=POLL_COUNT; i++)); do
      sleep "$POLL_SECS"
      echo "-- status poll $i"
      curl -s --max-time 10 "http://$node/rpc/task/status/$JOB_ID" || true
      echo
    done
  fi

  echo
done
