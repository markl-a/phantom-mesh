#!/usr/bin/env bash
# tdd-next.sh — print the next red P0 test from docs/tdd/INDEX.md.
#
# Exit codes:
#   0   a red test found, printed to stdout
#   1   all P0 tests green (printed reminder)
#   2   INDEX.md missing
#
# Output format (when a red test exists):
#   PLATFORM | test::path::name | V-track | est-time

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INDEX="$SCRIPT_DIR/../../docs/tdd/INDEX.md"

if [[ ! -f "$INDEX" ]]; then
  echo "✗ INDEX.md not found at $INDEX" >&2
  exit 2
fi

# pick the first `- [ ]` line, strip the checkbox prefix
next=$(grep -m1 '^- \[ \] ' "$INDEX" | sed 's/^- \[ \] //')

if [[ -z "$next" ]]; then
  echo "✓ All P0 tests green. Next step: run doc 29 §4 V-matrix ship readiness check."
  exit 1
fi

echo "$next"
