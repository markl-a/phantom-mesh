#!/usr/bin/env bash
# matrix-to-backlog.sh — the /goal develop planner (component A).
# FEATURE-MATRIX PARTIAL/STUB rows -> backlog spec .toml -> backlog.sh post.
# Idempotent: a spec already on the backlog (same id) is skipped by backlog.sh.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
MATRIX="${1:-$ROOT/docs/FEATURE-MATRIX.md}"
PHANTOM="${PHANTOM_BIN:-phantom}"
LIMIT="${MATRIX_PLAN_LIMIT:-8}"   # cap specs posted per planner run

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
# Split `phantom matrix-plan` STDOUT into per-spec files at the "=== SPEC <id> ==="
# markers (STDERR advisories are left on the terminal, not captured).
"$PHANTOM" matrix-plan "$MATRIX" | awk -v dir="$tmp" '
  /^=== SPEC / { id=$3; f=dir"/"id".toml"; next }
  f { print >> f }
'
posted=0
for f in "$tmp"/*.toml; do
  [ -f "$f" ] || continue
  [ "$posted" -ge "$LIMIT" ] && break
  if bash "$HERE/backlog.sh" post "$f" 2>/dev/null; then
    posted=$((posted+1))
  fi   # already-posted / gate-rejected specs are skipped, not fatal
done
echo "matrix-to-backlog: posted $posted new spec(s) (limit $LIMIT)"
