#!/usr/bin/env bash
# test-matrix.sh — fan-out selftest across all platforms, merge into one matrix.
#
# WHY THIS EXISTS
#   You already have the engine: scripts/selftest.sh emits --json, and
#   selftest.d/ already encodes the core-vs-platform split (50-snapshot-mac is
#   Darwin-only, 51-service-windows is Windows-only, the rest are shared core).
#   What was missing is the fan-out + aggregation: instead of running selftest
#   by hand on each box and eyeballing the diffs ("single-line"), this runs it
#   on ALL nodes in parallel, merges the JSON, and tells you whether a failure
#   is a CORE bug (fix once, every platform benefits) or a PLATFORM bug.
#
# USAGE
#   scripts/test-matrix.sh                 # all nodes, full matrix
#   scripts/test-matrix.sh --p0-only       # fast gate (skip P1/P2)
#   scripts/test-matrix.sh --feature serve # one feature across all nodes
#   scripts/test-matrix.sh --out matrix.json
#
# NODE CONFIG
#   Reads nodes from $NODES_FILE (default: scripts/test-matrix.nodes).
#   One node per line:  <label> <ssh-target-or-"local"> <port> <platform>
#   e.g.
#     mac     local                7878  darwin
#     node-a  you@100.64.0.11      7879  windows
#     oracle  you@100.64.0.20      7878  linux
#     z13     you@100.64.0.12      7878  linux
#     ios     skip                 -     ios     # ios verified via Xcode flow, see IOS-TEST-FLOW.md
#
#   "local" runs selftest.sh in-process; "skip" records the node as N/A.
#
# OUTPUT
#   - A feature × platform grid (✓ / ✗ / ○skip / -na)
#   - A classification block: CORE bugs (≥2 platforms fail) vs PLATFORM bugs
#   - Optional merged JSON (--out)
#
# EXIT CODES
#   0  no P0 failures anywhere
#   1  at least one P0 failure
#   2  setup error

set -o pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NODES_FILE="${NODES_FILE:-$ROOT_DIR/scripts/test-matrix.nodes}"
SELFTEST_REMOTE_PATH="${SELFTEST_REMOTE_PATH:-scripts/selftest.sh}"   # path on remote nodes
SELFTEST_ARGS=()        # forwarded verbatim to selftest.sh on every node
JSON_OUT=""
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── arg passthrough ───────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --out)   shift; JSON_OUT="$1" ;;
    --out=*) JSON_OUT="${1#--out=}" ;;
    *)       SELFTEST_ARGS+=("$1") ;;   # --p0-only, --feature X, … go to selftest
  esac
  shift
done

[ -f "$NODES_FILE" ] || { echo "node config missing: $NODES_FILE (see header for format)" >&2; exit 2; }
command -v jq >/dev/null || { echo "jq is required (brew install jq)" >&2; exit 2; }

# ── 1. fan-out: run selftest.sh --json on every node, in parallel ──────────────
# Each node writes $TMP/<label>.json (the selftest JSON report) plus
# $TMP/<label>.rc (its exit code). Remote nodes reuse the ControlMaster
# single-password pattern from scripts/smoke-node-a.sh.
run_node() {
  local label="$1" target="$2" port="$3" platform="$4"
  local out="$TMP/$label.json" rc="$TMP/$label.rc"

  if [ "$target" = "skip" ]; then
    echo '{"skipped":true}' > "$out"; echo "na" > "$rc"; return
  fi

  if [ "$target" = "local" ]; then
    COORD="http://127.0.0.1:$port" \
      "$ROOT_DIR/scripts/selftest.sh" --json "${SELFTEST_ARGS[@]}" > "$out" 2>"$TMP/$label.err"
    echo "$?" > "$rc"
    return
  fi

  # remote: one ControlMaster socket per node, password entered at most once
  local sock="$HOME/.ssh/cm-tm-$label-$$"
  ssh -M -S "$sock" -fN -o ControlPersist=5m -o StrictHostKeyChecking=no "$target" 2>/dev/null
  # shellcheck disable=SC2029  # we WANT remote-side expansion of args
  ssh -S "$sock" "$target" \
    "cd phantom-mesh 2>/dev/null && COORD=http://127.0.0.1:$port $SELFTEST_REMOTE_PATH --json ${SELFTEST_ARGS[*]}" \
    > "$out" 2>"$TMP/$label.err"
  echo "$?" > "$rc"
  ssh -S "$sock" -O exit "$target" 2>/dev/null
}

LABELS=(); PLATFORMS=()
while read -r label target port platform; do
  [ -z "$label" ] || [[ "$label" == \#* ]] && continue
  LABELS+=("$label"); PLATFORMS+=("$platform")
  run_node "$label" "$target" "$port" "$platform" &      # ← parallel fan-out
done < "$NODES_FILE"
wait

# ── 2. assemble: tag each node's report with its label, guard bad JSON ────────
# Each node's selftest report (scripts/selftest.sh JSON) gets wrapped as
#   {label, platform, rc, report}
# A "skip" target reports {"skipped":true}; an unreachable node / non-JSON
# stdout collapses to {"_error":true} so the matrix shows it as "err" rather
# than crashing jq.
INPUTS="$TMP/_inputs.json"
{
  printf '[\n'
  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"; platform="${PLATFORMS[$i]}"
    rep="$(cat "$TMP/$label.json" 2>/dev/null)"
    echo "$rep" | jq -e . >/dev/null 2>&1 || rep='{"_error":true}'
    rc="$(cat "$TMP/$label.rc" 2>/dev/null || echo '')"
    [ "$i" -gt 0 ] && printf ',\n'
    jq -n --arg l "$label" --arg p "$platform" --arg rc "$rc" --argjson rep "$rep" \
      '{label:$l, platform:$p, rc:$rc, report:$rep}'
  done
  printf '\n]\n'
} > "$INPUTS"

# Warn about nodes we couldn't reach / that errored, before showing the grid.
jq -r '.[] | select(.report._error==true) | .label' "$INPUTS" | while read -r bad; do
  echo "⚠  $bad: unreachable or selftest failed — see $TMP/$bad.err" >&2
done

# ── 3. merge: build the feature × platform matrix ─────────────────────────────
# Feature-level status is derived from its tests[]: any fail → fail; empty or
# all-skip → skip; else pass. "na" = feature not present on that node (e.g. a
# Darwin-only feature on a Linux box), "err" = node didn't report.
MATRIX="$TMP/_matrix.json"
jq '
  def feat_status($t):
    if   ($t | any(.status=="fail")) then "fail"
    elif ($t | length)==0           then "skip"
    elif ($t | all(.status=="skip")) then "skip"
    else "pass" end;

  . as $nodes
  | ( [ $nodes[] | select(.report.features != null) | .report.features[] ]
      | group_by(.name)
      | map({name: .[0].name, priority: (.[0].priority // "P?")}) ) as $feats
  | {
      nodes: [ $nodes[].label ],
      features: [
        $feats[] as $f
        | {
            name: $f.name,
            priority: $f.priority,
            cells: ( [ $nodes[]
                       | { key: .label,
                           value: (
                             if   (.report._error  == true) then "err"
                             elif (.report.skipped == true) then "na"
                             else ( ((.report.features // []) | map(select(.name==$f.name))) as $m
                                    | if ($m|length)==0 then "na"
                                      else feat_status($m[0].tests // []) end )
                             end ) }
                     ] | from_entries )
          }
      ]
    }
' "$INPUTS" > "$MATRIX"

[ -n "$JSON_OUT" ] && cp "$MATRIX" "$JSON_OUT"

# ── 4. render: grid + classification ──────────────────────────────────────────
echo ""
jq -r '
  def g(s): if s=="pass" then "✓" elif s=="fail" then "✗"
            elif s=="skip" then "○" elif s=="err" then "!" else "-" end;
  .nodes as $n
  | (["FEATURE","PRI"] + $n),
    (.features[] | [.name, .priority] + [ $n[] as $l | g(.cells[$l]) ])
  | @tsv
' "$MATRIX" | column -t -s "$(printf '\t')"

echo ""
echo "legend: ✓ pass   ✗ fail   ○ skip   - n/a (feature absent on node)   ! node error"
echo ""
jq -r '
  [ .features[]
    | . + {fails: [ (.cells | to_entries[] | select(.value=="fail") | .key) ]}
    | select(.fails | length > 0) ] as $fail
  | ($fail | map(select(.fails | length >= 2))) as $core
  | ($fail | map(select(.fails | length == 1))) as $plat
  | "CORE BUGS — appear on ≥2 platforms → fix once in shared core, re-run matrix:",
    ( if ($core|length)==0 then "  (none) ✦"
      else ($core[] | "  ✗ " + .name + " [" + .priority + "] — fails on: " + (.fails | join(", "))) end ),
    "",
    "PLATFORM BUGS — single platform → fix in that platform repo only:",
    ( if ($plat|length)==0 then "  (none)"
      else ($plat[] | "  ✗ " + .name + " [" + .priority + "] — fails on: " + .fails[0]) end )
' "$MATRIX"

# ── 5. exit code: 1 if any P0 feature failed anywhere, or any node errored ────
P0_FAIL="$(jq '[ .features[] | select(.priority=="P0") | .cells | to_entries[] | select(.value=="fail") ] | length' "$MATRIX")"
NODE_ERR="$(jq '[ .[] | select(.report._error==true) ] | length' "$INPUTS")"
echo ""
if [ "${P0_FAIL:-0}" -gt 0 ] || [ "${NODE_ERR:-0}" -gt 0 ]; then
  echo "RESULT: FAIL  (P0 failures: ${P0_FAIL:-0}, unreachable nodes: ${NODE_ERR:-0})"
  exit 1
fi
echo "RESULT: PASS  (no P0 failures across all reachable nodes)"
exit 0
