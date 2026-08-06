#!/usr/bin/env bash
# run_tier1.sh — half-automated Tier 1 (8 scenario) runner
#
# For each scenario, prints the steps and waits for the operator to
# mark PASS / PARTIAL / FAIL. Writes a results file under
# tests-e2e/results/<ISO-week>/.
#
# Usage:
#   ./tests-e2e/run_tier1.sh                    # all 8 scenarios
#   ./tests-e2e/run_tier1.sh T1.8               # specific
#   ./tests-e2e/run_tier1.sh T1.1 T1.5          # selection
#
# Env:
#   SPECTYN_BINARY  override path (default: $(which spectyn))
#   OPERATOR        operator name for the result file (default: $USER)
#   MACHINE         machine label (default: $(hostname))

set -u

SCENARIOS_DIR="$(cd "$(dirname "$0")/scenarios" && pwd)"
RESULTS_BASE="$(cd "$(dirname "$0")" && pwd)/results"

SPECTYN_BINARY="${SPECTYN_BINARY:-$(command -v spectyn 2>/dev/null || true)}"
OPERATOR="${OPERATOR:-$USER}"
MACHINE="${MACHINE:-$(hostname)}"

# pre-flight: verify-binary
echo "=== Pre-flight: verify-binary --quick ==="
if [[ -z "$SPECTYN_BINARY" ]]; then
  echo "✗ spectyn not on PATH and SPECTYN_BINARY unset; aborting" >&2
  exit 2
fi
VERIFY_SCRIPT="$(cd "$(dirname "$0")/.." && pwd)/scripts/verify-binary.sh"
if [[ -x "$VERIFY_SCRIPT" ]]; then
  if ! "$VERIFY_SCRIPT" "$SPECTYN_BINARY" --quick; then
    echo "✗ verify-binary failed; aborting Tier 1 run" >&2
    exit 1
  fi
else
  echo "⚠ verify-binary.sh not found or not executable at $VERIFY_SCRIPT — continuing without pre-flight" >&2
fi

# scenario selection
ALL=(T1.1 T1.2 T1.3 T1.4 T1.5 T1.6 T1.7 T1.8)
if [[ $# -gt 0 ]]; then
  SELECTION=("$@")
else
  SELECTION=("${ALL[@]}")
fi

# ISO week for results dir
WEEK=$(date +%G-W%V)
RESULTS_DIR="$RESULTS_BASE/$WEEK"
mkdir -p "$RESULTS_DIR"

echo ""
echo "=== Running ${#SELECTION[@]} Tier 1 scenarios ==="
echo "Binary:  $SPECTYN_BINARY ($($SPECTYN_BINARY --version --short))"
echo "Results: $RESULTS_DIR/"
echo "Operator: $OPERATOR  Machine: $MACHINE"
echo ""

# loop
PASS=0
FAIL=0
SKIP=0
PARTIAL=0

for id in "${SELECTION[@]}"; do
  SCENARIO_FILE=$(ls "$SCENARIOS_DIR"/$id-*.md 2>/dev/null | head -1)
  if [[ -z "$SCENARIO_FILE" ]]; then
    echo "--- $id: no scenario file matching ${id}-*.md, skipping ---"
    SKIP=$((SKIP+1))
    continue
  fi

  echo ""
  echo "======================================================================"
  echo "  $id  ($SCENARIO_FILE)"
  echo "======================================================================"
  cat "$SCENARIO_FILE"
  echo ""
  echo "----------------------------------------------------------------------"
  read -r -p "Result [p=pass, x=fail, ~=partial, s=skip, q=quit]: " result

  case "$result" in
    p|P) status="PASS"; PASS=$((PASS+1)) ;;
    x|X) status="FAIL"; FAIL=$((FAIL+1)) ;;
    "~") status="PARTIAL"; PARTIAL=$((PARTIAL+1)) ;;
    s|S) status="SKIP"; SKIP=$((SKIP+1)); continue ;;
    q|Q) echo "quit by operator"; break ;;
    *) status="UNKNOWN"; SKIP=$((SKIP+1)) ;;
  esac

  read -r -p "Notes (1 line; leave blank for none): " notes

  RESULT_FILE="$RESULTS_DIR/${id}-$(date +%Y-%m-%d).md"
  cat > "$RESULT_FILE" <<EOF
# $id — $(date +%Y-%m-%d)

Run by: $OPERATOR
Machine: $MACHINE
Binary version: $($SPECTYN_BINARY --version 2>/dev/null | head -1)

## Result
- [$( [[ "$status" == "PASS" ]] && echo "x" || echo " " )] PASS
- [$( [[ "$status" == "PARTIAL" ]] && echo "x" || echo " " )] PARTIAL
- [$( [[ "$status" == "FAIL" ]] && echo "x" || echo " " )] FAIL

## Notes
$notes

## V-matrix update
(see scenario file for the associated V<N> — update doc 29 §4 if status changed)
EOF
  echo "→ saved $RESULT_FILE"
done

echo ""
echo "======================================================================"
echo "  Summary"
echo "======================================================================"
echo "  PASS:    $PASS"
echo "  PARTIAL: $PARTIAL"
echo "  FAIL:    $FAIL"
echo "  SKIP:    $SKIP"
echo "  Results: $RESULTS_DIR/"

[[ $FAIL -eq 0 ]] && exit 0 || exit 1
