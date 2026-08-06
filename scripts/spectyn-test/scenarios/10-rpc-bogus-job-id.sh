#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "RPC status — querying a bogus job_id returns clean error"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

bogus="00000000-0000-0000-0000-000000000000"
step "GET /rpc/task/status/$bogus"

resp=$(curl -sS --max-time 5 -w "\n--HTTP-%{http_code}--" \
  "$(rpc_url)/rpc/task/status/$bogus")

http_code=$(printf '%s' "$resp" | grep -oE 'HTTP-[0-9]+' | tr -dc '0-9')
body=$(printf '%s' "$resp" | sed 's/--HTTP-[0-9]*--//')

step "got HTTP $http_code"

# Acceptable: 404, OR 200 with an explicit "not found" / "error" field in JSON.
case "$http_code" in
  404) pass "404 Not Found — clean signal for missing job" ;;
  200)
    state=$(printf '%s' "$body" | python -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    print(d.get('error') or d.get('status') or '')
except: print('')
" 2>/dev/null)
    if [ -n "$state" ]; then
      pass "200 OK with state/error field: $state"
    else
      fail "200 OK but body has no state/error field: ${body:0:200}"
    fi
    ;;
  *) fail "unexpected HTTP $http_code: ${body:0:200}" ;;
esac

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
