#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/inspect.sh"

scenario "events.jsonl — grows after a phantom command"
require_cmd "$PHANTOM_BIN"

before=$(events_count)
[ -z "$before" ] && before=0
step "events.jsonl currently has $before lines"

step "running a trivial phantom command (--version) to trigger an event"
"$PHANTOM_BIN" --version >/dev/null 2>&1 || true
sleep 1  # give the diag writer a moment to flush

after=$(events_count)
[ -z "$after" ] && after=0

if [ "$after" -gt "$before" ]; then
  pass "events.jsonl: $before → $after lines (+$((after - before)))"
else
  fail "events.jsonl did not grow after phantom command (still $after lines)"
fi

# Verify the most recent event's argv mentions our command.
last_summary=$(tail -1 "$PHANTOM_CONFIG_DIR/events.jsonl" 2>/dev/null \
  | python -c "import json,sys; print(json.loads(sys.stdin.read()).get('summary',''))" 2>/dev/null)

ASSERT_CONTAINS "$last_summary" "version" "latest event mentions --version"

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
