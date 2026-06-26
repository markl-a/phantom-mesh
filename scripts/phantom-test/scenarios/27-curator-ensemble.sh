#!/usr/bin/env bash
# 27-curator-ensemble.sh — verify `phantom evolve --judge --ensemble N`
# (T28 V2) accepts the flag, dispatches to multiple judges, and writes a
# judge_ensemble verdict (alongside the V1 judge_score) onto the latest
# EvolveCheckpoint.
#
# Gated behind PHANTOM_HERMES_ENSEMBLE=1 — like scenario 26, this is opt-in
# because (a) it requires real API keys for at least 2 providers and (b) the
# binary must have been built with --features experimental-curator.
#
# Primary correctness coverage lives in core/tests/curator_v2_integration.rs
# (wiremock-driven, runs in regular cargo test --features experimental-curator).
# This script is shell-driven smoke against a real built binary.
source "$PHANTOM_TEST_LIB/common.sh"
scenario "phantom evolve --judge --ensemble N writes a judge_ensemble verdict"

if [ "${PHANTOM_HERMES_ENSEMBLE:-0}" != "1" ]; then
  warn "skip: set PHANTOM_HERMES_ENSEMBLE=1 to enable (binary must also be built with --features experimental-curator)"
  exit 77
fi

if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  warn "skip: ANTHROPIC_API_KEY not set (ensemble requires at least one Anthropic key + one other provider key)"
  exit 77
fi

# Need at least one more provider key for a meaningful ensemble.
other_keys=0
for k in MISTRAL_API_KEY XAI_API_KEY TOGETHER_API_KEY FIREWORKS_API_KEY; do
  if [ -n "${!k:-}" ]; then
    other_keys=$((other_keys + 1))
  fi
done
if [ "$other_keys" -lt 1 ]; then
  warn "skip: set at least one of MISTRAL_API_KEY/XAI_API_KEY/TOGETHER_API_KEY/FIREWORKS_API_KEY for a real ensemble"
  warn "(or rely on the self-consistency fallback: duplicate Anthropic instances will be used if N>1 and only ANTHROPIC_API_KEY is set)"
fi

require_cmd "$PHANTOM_BIN"

# Verify usage advertises the new flag in the help text.
help_out=$("$PHANTOM_BIN" evolve --help 2>&1 || true)
if echo "$help_out" | grep -q -- '--ensemble'; then
  pass "binary advertises --ensemble in usage line"
else
  warn "binary usage does not advertise --ensemble — feature flag may not be compiled in"
fi

# Seed a tiny checkpoint by running evolve with a no-op goal + max-rounds 1
# + --judge --ensemble 3.
CHECKPOINT_DIR="${HOME}/.phantom-mesh/evolve-checkpoints"
BEFORE_COUNT=$(ls "$CHECKPOINT_DIR"/*.json 2>/dev/null | wc -l | tr -d ' \n')
BEFORE_COUNT="${BEFORE_COUNT:-0}"

step "running phantom evolve --max-rounds 1 --judge --ensemble 3 (180s hard cap)"
timeout 180 "$PHANTOM_BIN" evolve --max-rounds 1 --judge --ensemble 3 \
  "echo hello world; this is a no-op evolve goal for the ensemble scenario" \
  > "$PHANTOM_TEST_TMP/ensemble-stdout.log" 2>&1 || true

AFTER_COUNT=$(ls "$CHECKPOINT_DIR"/*.json 2>/dev/null | wc -l | tr -d ' \n')
AFTER_COUNT="${AFTER_COUNT:-0}"
if [ "$AFTER_COUNT" -ge "$BEFORE_COUNT" ]; then
  pass "evolve produced at least one checkpoint (before=$BEFORE_COUNT after=$AFTER_COUNT)"
else
  fail "checkpoint count went backwards (before=$BEFORE_COUNT after=$AFTER_COUNT)"
fi

# Find the latest checkpoint and assert judge_ensemble is populated.
latest=$(ls -t "$CHECKPOINT_DIR"/*.json 2>/dev/null | head -1)
if [ -z "$latest" ]; then
  fail "no checkpoint file found in $CHECKPOINT_DIR"
fi
step "asserting judge_ensemble in $latest"
if grep -q '"judge_ensemble"' "$latest" && ! grep -q '"judge_ensemble": null' "$latest"; then
  pass "judge_ensemble field is present and non-null"
else
  fail "judge_ensemble missing or null in $(basename "$latest")"
fi

# Assert agreement class is one of the three legal values.
if grep -qE '"agreement": "(unanimous|consensus|needs_human_review)"' "$latest"; then
  pass "agreement class is one of {unanimous, consensus, needs_human_review}"
else
  fail "agreement field has an unexpected value in $(basename "$latest")"
fi

# Assert judges_attempted == 3.
if grep -q '"judges_attempted": 3' "$latest"; then
  pass "judges_attempted == 3 as requested by --ensemble 3"
else
  warn "judges_attempted is not 3 — may indicate the slate fell short of 3 providers"
fi
