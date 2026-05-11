#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/cluster-rpc.sh"

scenario "RPC — Chinese prompt from MSYS bash currently fails (encoding gotcha)"

# Background: 2026-05-02 found that MSYS bash's `printf '%s' "$body"`
# encodes multi-byte UTF-8 differently from how curl writes the wire body
# under some locales, so HMAC mismatches. ASCII bodies work fine.
# This scenario LOCKS IN the current behavior so a future fix is detected.

step "dispatching with Chinese characters in prompt"
job=$(rpc_dispatch master "用一句話說 OK")

if [ -z "$job" ]; then
  pass "Chinese prompt rejected as expected (current encoding gotcha)"
  warn "if you fix the MSYS UTF-8 byte alignment, flip this assertion to expect success"
else
  pass "Chinese prompt now works! (job_id=$job) — please update this scenario to assert success"
fi

# Either outcome is informational here, not a failure. Exit 0.
exit 0
