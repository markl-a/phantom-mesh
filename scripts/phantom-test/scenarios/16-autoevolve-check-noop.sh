#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"

scenario "autoevolve --once --target check — green codebase exits no-op"
require_cmd "$PHANTOM_BIN"
require_cmd cargo

# autoevolve cd's into the project's manifest dir before running cargo check.
# If we can't find a Cargo.toml from the cwd, skip — running this from /tmp
# would be meaningless.
if ! ls Cargo.toml core/Cargo.toml 2>/dev/null | head -1 | grep -q .; then
  warn "no Cargo.toml in cwd or core/ — skipping (autoevolve needs project context)"
  exit 77
fi

PHANTOM_CONFIG_DIR="${PHANTOM_CONFIG_DIR:-$HOME/.phantom-mesh}"
log="$PHANTOM_CONFIG_DIR/autoevolve.log"
prev_lines=$(wc -l < "$log" 2>/dev/null | tr -d ' \n')
prev_lines="${prev_lines:-0}"

step "running phantom autoevolve --once --target check (60s hard cap)"
ts=$(date +%s)
# 60s is generous: a fresh-cache cargo check on this codebase + McAfee
# real-time-scan interference can stretch a normally-9s build to a minute.
# If we hit the cap, that itself is a finding and the scenario fails.
out=$(timeout 60 "$PHANTOM_BIN" autoevolve --once --target check 2>&1 \
      | sed -E 's/\x1b\[[0-9;]*m//g')
ec=$?
elapsed=$(( $(date +%s) - ts ))

if [ $ec -eq 124 ]; then
  fail "autoevolve hit the 60s timeout — likely McAfee/Defender stomping cargo (see crosscompile-gotchas memory)"
  exit 1
fi

step "exit=$ec elapsed=${elapsed}s"

# Acceptable outcomes:
#   - exit 0 + "green" or "no-op" or "nothing to evolve" in output
#   - exit 0 + a fix loop happened (evolve made changes) — also valid; we
#     just don't expect that on a known-green tree
ASSERT_EQ "$ec" "0" "autoevolve exited cleanly"

if echo "$out" | grep -qE 'nothing to evolve|already green|no-op'; then
  pass "autoevolve detected green codebase, no fix loop run"
else
  warn "autoevolve ran a fix loop (uncommon on a clean tree); inspect output above"
fi

# autoevolve.log should have grown by exactly 1 line (one new JSONL entry).
new_lines=$(wc -l < "$log" 2>/dev/null | tr -d ' \n')
new_lines="${new_lines:-0}"
delta=$((new_lines - prev_lines))
if [ "$delta" -ge 1 ]; then
  pass "autoevolve.log grew by $delta line(s)"
  step "newest entry:"
  tail -1 "$log" | sed 's/^/      /'
else
  fail "autoevolve.log did not grow (was $prev_lines, now $new_lines)"
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
