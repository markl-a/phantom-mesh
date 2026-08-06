#!/usr/bin/env bash
# goal-setup.sh — /goal Setup-stage readiness probe (ECOSYSTEM-MASTER-PLAN §3-S).
#
# Answers ONE question: "is the fleet ready for distributed development?" For each
# node it runs `node-check.sh` LOCALLY (piped to the node's LOGIN shell over ssh —
# no cross-shell quoting, real PATH), parses the report, and prints a readiness
# table; with --smoke it also round-trips a trivial spec through the shared backlog.
#
# READ-ONLY: it only inspects. The /goal Setup skill fixes gaps with the documented
# per-node commands. --smoke is the only mutation and cleans up after itself.
#
# Fleet list (IPs NEVER hardcoded here — they live OUTSIDE the repo so they can't
# leak): $SPECTYN_FLEET_NODES (default: ~/.spectyn-mesh/fleet.nodes), each line:
#   <label>  <target|local>  <shell: local|mac|win>  <caps,csv>  <role: primary|backup>
# A `backup` node (e.g. M1, taken to work / drops) is probed but NOT required for
# the M-S gate; macOS/iOS work is pinned to the primary mac (M5).
#
# Usage:  bash scripts/dev-loop/goal-setup.sh [--smoke]
# Exit: 0 = ≥2 PRIMARY nodes READY (+ smoke clean if asked); 1 = gaps; 2 = setup error.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CHECK="$HERE/node-check.sh"
SSH="ssh -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new"
GITBASH='"C:\Program Files\Git\bin\bash.exe"'
NODES_FILE="${SPECTYN_FLEET_NODES:-$HOME/.spectyn-mesh/fleet.nodes}"
[ -f "$NODES_FILE" ] || NODES_FILE="$ROOT/scripts/test-matrix.nodes"
SMOKE=0; [ "${1:-}" = "--smoke" ] && SMOKE=1

[ -f "$CHECK" ]      || { echo "goal-setup: missing $CHECK" >&2; exit 2; }
[ -f "$NODES_FILE" ] || { echo "goal-setup: no fleet list ($NODES_FILE) — create ~/.spectyn-mesh/fleet.nodes" >&2; exit 2; }

# Run node-check.sh ON a node, in its native LOGIN shell (script arrives via stdin).
run_check() { # <shell-kind> <target>
  case "$1" in
    local) sh "$CHECK" 2>/dev/null ;;
    mac)   $SSH "$2" "zsh -ls"        < "$CHECK" 2>/dev/null ;;
    win)   $SSH "$2" "$GITBASH -ls"   < "$CHECK" 2>/dev/null ;;
    *)     return 1 ;;
  esac
}

field() { printf '%s\n' "$1" | sed -n "s/^$2=//p" | head -1; }
mark() { [ "$1" = 1 ] && printf '✅' || printf '❌'; }

printf '%-8s %-7s %-6s %-26s %-16s %-5s %-5s %s\n' NODE ROLE REACH "AIs" CAPS SRC CACHE READY
printf '%.0s─' {1..86}; echo

prim_ready=0 prim_total=0; gaps=()
while read -r label target shell caps role _rest; do
  case "$label" in ''|\#*) continue;; esac
  role="${role:-primary}"
  [ "$role" = primary ] && prim_total=$((prim_total+1))

  out="$(run_check "$shell" "$target" | tr -d '\r')"   # strip CRLF so Windows output can't garble the table
  if [ -z "$out" ]; then
    printf '%-8s %-7s  %s   UNREACHABLE (%s) — debug: ssh %s true\n' "$label" "$role" "$(mark 0)" "$target" "$target"
    [ "$role" = primary ] && gaps+=("$label:unreachable")
    continue
  fi
  ais="$(field "$out" AIS)"; ncaps="$(field "$out" CAPS)"; repo="$(field "$out" REPO)"; cache="$(field "$out" CACHE)"

  # ≥2 AIs needed for a double-gate
  ai_n=0; [ -n "$ais" ] && ai_n=$(printf '%s' "$ais" | tr ',' '\n' | grep -c .)
  ai_ok=0;   [ "$ai_n" -ge 2 ] && ai_ok=1
  caps_ok=0; [ -n "${ncaps// /}" ] && caps_ok=1
  src_ok=0;  [ -n "$repo" ] && src_ok=1
  cache_ok=0; [ "$cache" = yes ] && cache_ok=1
  ready=0; [ "$ai_ok" = 1 ] && [ "$caps_ok" = 1 ] && [ "$src_ok" = 1 ] && ready=1
  [ "$ready" = 1 ] && [ "$role" = primary ] && prim_ready=$((prim_ready+1))

  printf '%-8s %-7s  %s    %-26s %s %-14s %s     %s     %s\n' \
    "$label" "$role" "$(mark 1)" "${ais:-—}" "$(mark $caps_ok)" "${ncaps:-—}" "$(mark $src_ok)" "$(mark $cache_ok)" "$(mark $ready)"

  # Gaps: only block on PRIMARY nodes; backup gaps are advisory.
  pfx=""; [ "$role" = backup ] && pfx="(backup) "
  [ "$ai_ok"  = 1 ] || gaps+=("$pfx$label:need≥2-AI(have:${ais:-none})")
  [ "$caps_ok" = 1 ] || gaps+=("$pfx$label:no-caps(node-setup.sh --caps $caps)")
  [ "$src_ok" = 1 ] || gaps+=("$pfx$label:no-source(seed via git archive|ssh tar)")
  { [ "$cache_ok" = 1 ] || [ "$src_ok" = 0 ]; } || gaps+=("(warm-up) $label:cold-cache(advisory — first build slow, NOT blocking M-S)")
done < "$NODES_FILE"

echo
echo "M-S gate needs ≥2 PRIMARY nodes READY (backup nodes like M1 optional)."
echo "primary readiness: $prim_ready/$prim_total READY"
if [ "${#gaps[@]}" -gt 0 ]; then echo "gaps:"; printf '  - %s\n' "${gaps[@]}"; fi

smoke_rc=0
if [ "$SMOKE" = 1 ]; then
  echo; echo "=== smoke: backlog plumbing (list + spec-gate; NOT a full claim — that's driven by the skill) ==="
  if bash "$HERE/backlog.sh" list >/dev/null 2>&1; then echo "  ✅ backlog.sh list works (shared backlog reachable over git)";
  else echo "  ❌ backlog.sh list failed — check git remote/network"; smoke_rc=1; fi
  sid="goal-smoke-$$"; tmp="$(mktemp -d)"; spec="$tmp/$sid.toml"
  printf '[spec]\ncapability = "sense"\ncomponent = "goal-smoke"\nacceptance = ["readiness probe — never claimed for real"]\nscope_allow = ["docs/_smoke-never"]\ncaps = ["windows"]\n' > "$spec"
  if bash "$HERE/spec-gate.sh" validate "$spec" >/dev/null 2>&1; then echo "  ✅ spec-gate accepts a well-formed spec";
  else echo "  ❌ spec-gate rejected a valid spec — backlog plumbing broken"; smoke_rc=1; fi
  rm -rf "$tmp"
  echo "  (full claim→work→gate across ≥2 real nodes is driven by the /goal Setup skill.)"
fi

# M-S contract: ≥2 PRIMARY nodes READY (+ smoke clean). Per-node gaps are listed
# above as advisory — a not-yet-ready primary does NOT block the gate as long as
# ≥2 others are ready (matches the stated contract; fixed per codex+agy review).
{ [ "$prim_ready" -ge 2 ] && [ "$smoke_rc" = 0 ]; } && { echo; echo "M-S: ✅ fleet ready ($prim_ready/$prim_total primary nodes READY, ≥2 required)"; exit 0; }
echo; echo "M-S: ❌ not ready — need ≥2 PRIMARY nodes READY (have $prim_ready); close the gaps above"; exit 1
