#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/inspect.sh"

scenario "events.jsonl — grows after a spectyn command"
require_cmd "$SPECTYN_BIN"

before=$(events_count)
[ -z "$before" ] && before=0
step "events.jsonl currently has $before lines"

step "running a trivial spectyn command (--version) to trigger an event"
"$SPECTYN_BIN" --version >/dev/null 2>&1 || true
sleep 1  # give the diag writer a moment to flush

after=$(events_count)
[ -z "$after" ] && after=0

if [ "$after" -gt "$before" ]; then
  pass "events.jsonl: $before → $after lines (+$((after - before)))"
else
  fail "events.jsonl did not grow after spectyn command (still $after lines)"
fi

# Verify the most recent event's argv mentions our command.
last_summary=$(tail -1 "$SPECTYN_CONFIG_DIR/events.jsonl" 2>/dev/null \
  | python -c "import json,sys; print(json.loads(sys.stdin.read()).get('summary',''))" 2>/dev/null)

ASSERT_CONTAINS "$last_summary" "version" "latest event mentions --version"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
