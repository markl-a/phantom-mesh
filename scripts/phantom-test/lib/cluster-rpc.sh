# shellcheck shell=bash
# cluster-rpc.sh — HMAC dispatch + status polling for phantom serve.
# Wraps:  POST /rpc/task/assign   GET /rpc/task/status/<id>
#
# Public surface:
#   rpc_dispatch <agent> <prompt>              -> echoes job_id (or empty on fail)
#   rpc_dispatch_with_secret <secret> <agent> <prompt>
#   rpc_status <job_id>                        -> echoes raw JSON
#   rpc_state <job_id>                         -> echoes "running" / "done" / "failed"
#   rpc_output <job_id>                        -> echoes the agent's text output
#   rpc_wait_done <job_id> <max_seconds>       -> 0 on done, 1 on failed, 2 on timeout
#   rpc_url                                    -> echoes "http://<host>:<port>"

require_cmd curl
require_cmd openssl

rpc_url() {
  printf 'http://%s:%s\n' "$PHANTOM_HOST" "$PHANTOM_PORT"
}

# Compute HMAC-SHA256 over body bytes. Echo hex.
_hmac_hex() {
  local body="$1" secret="$2"
  printf '%s' "$body" | openssl dgst -sha256 -hmac "$secret" -hex | awk '{print $2}'
}

# rpc_dispatch <agent> <prompt> — POST, echo job_id
rpc_dispatch() {
  rpc_dispatch_with_secret "$PHANTOM_CLUSTER_SECRET" "$@"
}

rpc_dispatch_with_secret() {
  local secret="$1" agent="$2" prompt="$3"
  # Build JSON body manually (avoid jq dep). Caller must ASCII-escape prompt
  # if it contains quotes — keep prompts simple in scenarios.
  local body="{\"agent\":\"${agent}\",\"prompt\":\"${prompt}\"}"
  local auth resp
  auth=$(_hmac_hex "$body" "$secret")
  resp=$(curl -sS --max-time 10 \
    -X POST "$(rpc_url)/rpc/task/assign" \
    -H "X-Cluster-Auth: $auth" \
    -H "Content-Type: application/json" \
    -d "$body" 2>&1)
  # Echo job_id only; if there's an error or no job_id, echo empty.
  printf '%s\n' "$resp" | python -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    print(d.get('job_id', ''))
except Exception:
    print('')
" 2>/dev/null
}

# Echo raw JSON (caller parses).
rpc_status() {
  local job="$1"
  curl -sS --max-time 5 "$(rpc_url)/rpc/task/status/$job"
}

rpc_state() {
  rpc_status "$1" | python -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    print(d.get('state', d.get('status', '')))
except Exception:
    print('')
" 2>/dev/null
}

rpc_output() {
  rpc_status "$1" | python -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    print(d.get('output', ''))
except Exception:
    print('')
" 2>/dev/null
}

rpc_error() {
  rpc_status "$1" | python -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    e = d.get('error')
    print(e if e else '')
except Exception:
    print('')
" 2>/dev/null
}

# rpc_wait_done <job_id> <max_seconds>
rpc_wait_done() {
  local job="$1" deadline=$(( $(date +%s) + ${2:-30} ))
  local s
  while [ "$(date +%s)" -lt "$deadline" ]; do
    s=$(rpc_state "$job")
    case "$s" in
      done|completed) return 0 ;;
      failed|error)   return 1 ;;
    esac
    sleep 2
  done
  return 2
}
