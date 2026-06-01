#!/usr/bin/env bash
# tdd-mark-done.sh — flip `- [ ]` to `- [x]` for matching test name + append
# timestamp to results.log.
#
# Usage:
#   tdd-mark-done.sh <test::path::name>

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INDEX="$SCRIPT_DIR/../../docs/tdd/INDEX.md"
RESULTS="$SCRIPT_DIR/../../docs/tdd/results.log"
TEST_NAME="${1:-}"

if [[ -z "$TEST_NAME" ]]; then
  echo "usage: tdd-mark-done.sh <test_name>" >&2
  exit 2
fi

if [[ ! -f "$INDEX" ]]; then
  echo "✗ INDEX.md not found at $INDEX" >&2
  exit 2
fi

# Flip the FIRST matching `- [ ]` line for this test_name.
# Portable across GNU sed (Linux) and BSD sed (macOS) via awk one-shot —
# the previous `sed "0,/PATTERN/s//repl/"` form silently no-ops on BSD
# sed (the script printed "marked done" but the checkbox stayed `[ ]`).
if awk -v pat="$TEST_NAME" '
     /^- \[ \] / && index($0, pat) { found=1; exit }
     END { exit !found }
   ' "$INDEX"; then
  tmp="${INDEX}.tmp.$$"
  awk -v pat="$TEST_NAME" '
    !done && /^- \[ \] / && index($0, pat) { sub(/^- \[ \]/, "- [x]"); done=1 }
    { print }
  ' "$INDEX" > "$tmp" && mv "$tmp" "$INDEX"

  ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  printf '%s | %s | %s | green\n' "$ts" "${USER:-unknown}" "$TEST_NAME" >> "$RESULTS"
  echo "✓ marked done: $TEST_NAME"
elif grep -qF -- "$TEST_NAME" "$INDEX" && grep -q "^- \[x\] " "$INDEX"; then
  echo "ℹ already marked done: $TEST_NAME"
  exit 0
else
  echo "✗ test not found in INDEX.md: $TEST_NAME" >&2
  exit 1
fi
