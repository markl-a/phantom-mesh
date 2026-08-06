#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"

scenario "spectyn peer list — agents.toml peers visible + statuses present"
require_cmd "$SPECTYN_BIN"

step "running spectyn peer list"
out=$("$SPECTYN_BIN" peer list 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

ASSERT_CONTAINS "$out" "STATUS" "table header includes STATUS column"

# Each peer is either online or offline — both are valid; the absence of
# either means the command produced no peer rows, which is the failure case.
status_count=$(echo "$out" | grep -cE 'online|offline' || true)
if [ "$status_count" -ge 1 ]; then
  pass "found $status_count peer status row(s)"
else
  fail "no online/offline rows in peer list — agents.toml [cluster].peers might be empty"
fi

# If an agents.toml has explicit peer URLs, at least one should appear in
# the output. We grep loosely for an http URL.
if echo "$out" | grep -qE 'https?://'; then
  pass "peer URL(s) present in output"
else
  warn "no http URL in peer list — peers may be configured via discovery only"
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
