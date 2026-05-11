#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/cluster-rpc.sh"

scenario "RPC HMAC — wrong cluster secret rejected"

step "dispatching with deliberately wrong secret"
job=$(rpc_dispatch_with_secret "wrong-secret-on-purpose" master "anything")

# Expected: rpc_dispatch echoes empty (server returned unauthorized, no job_id)
if [ -z "$job" ]; then
  pass "no job_id returned — server rejected the wrong secret"
else
  fail "server accepted wrong secret and assigned job_id=$job (auth bypass!)"
fi

# Verify the response body explicitly contained 'unauthorized'.
secret="wrong-secret-on-purpose"
body='{"agent":"master","prompt":"x"}'
auth=$(_hmac_hex "$body" "$secret")
resp=$(curl -sS --max-time 5 \
  -X POST "$(rpc_url)/rpc/task/assign" \
  -H "X-Cluster-Auth: $auth" \
  -H "Content-Type: application/json" \
  -d "$body" 2>&1)

ASSERT_CONTAINS "$resp" "unauthorized" "response body says unauthorized"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
