#!/usr/bin/env bash
# tdd-status.sh — print P0 progress summary.

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INDEX="$SCRIPT_DIR/../../docs/tdd/INDEX.md"

if [[ ! -f "$INDEX" ]]; then
  echo "✗ INDEX.md not found at $INDEX" >&2
  exit 2
fi

total=$(grep -c '^- \[[ x]\]' "$INDEX" || true)
done=$(grep -c '^- \[x\]' "$INDEX" || true)
remaining=$((total - done))
pct=0
if [[ $total -gt 0 ]]; then pct=$(( done * 100 / total )); fi

# per-platform breakdown
for plat in WIN LIN MAC AND SHARED; do
  p_total=$(grep -c "^- \[[ x]\] $plat " "$INDEX" || true)
  p_done=$(grep -c "^- \[x\] $plat " "$INDEX" || true)
  printf '  %-7s %d/%d\n' "$plat" "$p_done" "$p_total"
done > /tmp/_tdd_per_plat.txt

printf 'P0 progress: %d/%d (%d%%)  remaining: %d\n\n' "$done" "$total" "$pct" "$remaining"
cat /tmp/_tdd_per_plat.txt
rm -f /tmp/_tdd_per_plat.txt

printf '\nNext 5 red tests:\n'
grep -m5 '^- \[ \] ' "$INDEX" | sed 's/^- \[ \] /  - /'
