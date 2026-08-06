#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "RPC HMAC roundtrip — ASCII prompt against local serve"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

step "dispatching agent.master with simple ASCII prompt"
job=$(rpc_dispatch master "Reply with the word OK and nothing else.")

if [ -z "$job" ]; then
  fail "rpc_dispatch returned empty job_id (HMAC or serve issue)"
  exit 1
fi
pass "got job_id: $job"

step "waiting up to 60s for done"
if rpc_wait_done "$job" 60; then
  pass "job reached terminal state: done"
else
  ec=$?
  if [ $ec -eq 1 ]; then
    fail "job ended in failed/error state: $(rpc_error "$job")"
  else
    fail "job did not finish within 60s (still: $(rpc_state "$job"))"
  fi
  exit 1
fi

out=$(rpc_output "$job")
ASSERT_CONTAINS "$out" "OK" "agent output contains OK"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
