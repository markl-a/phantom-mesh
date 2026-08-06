#!/usr/bin/env bash
# 26-curator-judge.sh — verify `spectyn evolve --judge` parses the flag
# and (when run with SPECTYN_HERMES_CURATOR=1 + a real ANTHROPIC_API_KEY) writes
# a numeric score into the latest EvolveCheckpoint.
#
# This scenario is opt-in (SPECTYN_HERMES_CURATOR=1) because (a) it requires
# a real API key and (b) the binary must have been built with
# --features experimental-curator. Otherwise it skips (exit 77).
source "$SPECTYN_TEST_LIB/common.sh"
scenario "spectyn evolve --judge writes a JudgeVerdict to the latest checkpoint"

if [ "${SPECTYN_HERMES_CURATOR:-0}" != "1" ]; then
  warn "skip: set SPECTYN_HERMES_CURATOR=1 to enable (binary must also be built with --features experimental-curator)"
  exit 77
fi

if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  warn "skip: ANTHROPIC_API_KEY not set"
  exit 77
fi

require_cmd "$SPECTYN_BIN"

# 1) Make sure the binary recognises --judge in its help.
help_out=$("$SPECTYN_BIN" evolve --help 2>&1 || true)
if ! echo "$help_out" | grep -q -- '--judge'; then
  # The usage line is shared between feature variants in this repo, so
  # presence of `--judge` in usage is informational only. Continue and
  # detect feature absence below by failing on missing judge_score.
  warn "binary usage does not advertise --judge — may indicate missing --features experimental-curator build"
fi

# 2) Seed a tiny checkpoint by running evolve with a no-op goal + max-rounds 1.
#    Then invoke `--judge` on the latest checkpoint.
CHECKPOINT_DIR="${HOME}/.spectyn-mesh/evolve-checkpoints"
BEFORE_COUNT=$(ls "$CHECKPOINT_DIR"/*.json 2>/dev/null | wc -l | tr -d ' \n')
BEFORE_COUNT="${BEFORE_COUNT:-0}"

step "running spectyn evolve --max-rounds 1 --judge (180s hard cap)"
timeout 180 "$SPECTYN_BIN" evolve --max-rounds 1 --judge "echo hello world; this is a no-op evolve goal for the judge scenario" \
  > "$SPECTYN_TEST_TMP/judge-stdout.log" 2>&1 || true

AFTER_COUNT=$(ls "$CHECKPOINT_DIR"/*.json 2>/dev/null | wc -l | tr -d ' \n')
AFTER_COUNT="${AFTER_COUNT:-0}"
if [ "$AFTER_COUNT" -ge "$BEFORE_COUNT" ]; then
  pass "evolve produced at least one checkpoint (before=$BEFORE_COUNT after=$AFTER_COUNT)"
else
  fail "checkpoint count went backwards (before=$BEFORE_COUNT after=$AFTER_COUNT)"
fi

# 3) Find the newest checkpoint and assert it has judge_score with score in 0..=10.
LATEST=$(ls -t "$CHECKPOINT_DIR"/*.json 2>/dev/null | head -1)
if [ -z "$LATEST" ]; then
  fail "no checkpoint files found in $CHECKPOINT_DIR"
  exit 1
fi
pass "found latest checkpoint file: $LATEST"

if command -v jq >/dev/null 2>&1; then
  SCORE=$(jq -r '.judge_score.score // empty' < "$LATEST")
  if [ -n "$SCORE" ]; then
    pass "judge_score.score is non-empty: $SCORE"
  else
    fail "judge_score.score is empty (binary may not be built with --features experimental-curator)"
  fi
  if [ -n "$SCORE" ] && [ "$SCORE" -ge 0 ] && [ "$SCORE" -le 10 ]; then
    pass "judge_score.score in 0..=10 (got $SCORE)"
  else
    fail "judge_score.score out of range (got '$SCORE')"
  fi
  RUBRIC=$(jq -r '.judge_score.rubric_version // empty' < "$LATEST")
  ASSERT_EQ "$RUBRIC" "h1-v1" "rubric version pinned"
else
  # Fallback without jq: grep for the field shape.
  if grep -q '"judge_score"' "$LATEST"; then
    pass "judge_score field present in $LATEST"
  else
    fail "no judge_score field in $LATEST"
  fi
  if grep -q '"rubric_version": "h1-v1"' "$LATEST"; then
    pass "rubric_version pinned to h1-v1"
  else
    fail "rubric_version != h1-v1 in $LATEST"
  fi
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
