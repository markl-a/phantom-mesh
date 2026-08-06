#!/usr/bin/env bash
# verify-m2-m3.sh — one-shot, owner-runnable M2 + M3 acceptance check (two-party verify).
#
# Runs the REAL local path on THIS machine (no skip): arm + drive local AIs (M2), the
# local >=2-AI review gate + governance §4 (M3). Prints PASS/FAIL per check so you can
# judge each milestone with your own hands, on EACH machine.
#
# Works on macOS bash AND Windows git-bash — on Windows you MUST run it from Git Bash
# (NOT WSL: WSL can't see the Windows npm-installed AI CLIs; NOT cmd/PowerShell).
#
# Usage (on each machine):  cd <your spectyn-mesh clone> && bash scripts/dev-loop/verify-m2-m3.sh
#   QUICK=1   skip the live >=2-AI review.sh call (the slow/costly part); still checks the
#             reviewers are available + runs the hermetic governance demo.
#
# Note: a full run calls the real local AIs a handful of times (a few minutes, small cost).
# Exit 0 = everything that CAN run here passed.

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." 2>/dev/null && pwd)"
cd "$ROOT" 2>/dev/null || { echo "verify: cannot locate repo root from $0" >&2; exit 1; }
OS="$(uname -s 2>/dev/null || echo unknown)"
MACHINE="${SPECTYN_NODE:-$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo this)}"
T="/tmp/pv.$$"

pass=0; fail=0; skip=0; warn=0
ok(){ printf '  [PASS]  %s\n' "$1"; pass=$((pass+1)); }
no(){ printf '  [FAIL]  %s\n' "$1"; fail=$((fail+1)); }
sk(){ printf '  [skip]  %s\n' "$1"; skip=$((skip+1)); }
wn(){ printf '  [warn]  %s\n' "$1"; warn=$((warn+1)); }   # noted, does NOT fail the run
have(){ for e in "" .cmd .exe; do command -v "$1$e" >/dev/null 2>&1 && return 0; done; return 1; }

echo "================================================================"
echo " spectyn M2/M3 verify — ${MACHINE} (${OS})"
echo " repo: $ROOT"
case "$OS" in MINGW*|MSYS*|CYGWIN*) echo " shell: git-bash ✓ (correct on Windows — not WSL)";; esac
echo "================================================================"

# ───────────────────────── M2 本機武裝 ─────────────────────────
echo
echo "[M2] 本機武裝 — local Claude Code drives THIS machine's own AIs (no SSH)"

# M2.1 — arm + enrol (live-detects which AIs work here, writes the node descriptor)
scripts/dev-cluster/node-setup.sh "$MACHINE" --role dev >"${T}.node" 2>&1 || true
if [ -f "$HOME/.spectyn-mesh/node.json" ]; then
  wtools=""
  for t in claude codex opencode agy; do
    grep -oE "\"$t\":\{[^}]*\"working\":true" "$HOME/.spectyn-mesh/node.json" >/dev/null 2>&1 && wtools="${wtools} ${t}"
  done
  ok "M2.1 arm + enrol — node.json written; working AIs here:${wtools:- none} (claude = the driver itself)"
else
  no "M2.1 arm + enrol — node.json NOT written (no AI answered? see ${T}.node)"
fi

# M2.2 — drive each present local AI through the local-ai skill (the M2 essence)
ASK="$ROOT/.claude/skills/local-ai/ask.sh"
drove=0
for t in codex opencode agy; do
  if have "$t"; then
    out="$("$ASK" "$t" "Reply with exactly one word: PONG" 2>/dev/null || true)"
    if printf '%s' "$out" | grep -q "PONG"; then ok "M2.2 drive ${t} via ask.sh — answered PONG"; drove=$((drove+1))
    else wn "M2.2 drive ${t} via ask.sh — present but no PONG (flaky/login? — not fatal if another AI works)"; fi
  else
    sk "M2.2 drive ${t} — not installed on this machine"
  fi
done
[ "$drove" -ge 1 ] || no "M2.2 — NO local AI answered here (M2 needs >=1 working AI)"

# M2.3 — the auto-arm surface (option B SessionStart hook detector). Capture to a var
# first: piping straight into `grep -q` makes grep close the pipe on the first match,
# SIGPIPE-killing the still-printing hook and (under pipefail) looking like a failure.
hint_out="$(CLAUDE_PROJECT_DIR="$ROOT" bash "$ROOT/.claude/hooks/dev-node-hint.sh" 2>/dev/null || true)"
if printf '%s' "$hint_out" | grep -q "spectyn-mesh dev node"; then
  ok "M2.3 auto-arm surface — SessionStart hook emits the dev-node hint"
else
  no "M2.3 auto-arm surface — hook produced no hint"
fi

# ───────────────────────── M3 互審進核心 ─────────────────────────
echo
echo "[M3] 互審進核心 — local >=2-AI review gate (consensus before landing)"

revs=0; rlist=""
for t in codex opencode agy; do have "$t" && { revs=$((revs+1)); rlist="${rlist} ${t}"; }; done

if [ "$revs" -lt 2 ]; then
  no "M3.1 review gate — only ${revs} local reviewer AI here (${rlist:-none}); M3 needs >=2. Install/login another (codex/opencode/agy)."
elif [ "${QUICK:-0}" = 1 ]; then
  sk "M3.1 review gate — QUICK mode (skipped live AI review; ${revs} reviewers available:${rlist})"
else
  echo "  … running scripts/local-ai/review.sh HEAD~1..HEAD — calls real AIs (${rlist} ), ~30-90s"
  rc=0; scripts/local-ai/review.sh HEAD~1..HEAD >"${T}.rev" 2>&1 || rc=$?
  case "$rc" in
    0) ok "M3.1 review gate — reached consensus APPROVE (exit 0)";;
    1) ok "M3.1 review gate — ran and BLOCKED (reviewer requested changes, exit 1) — gate works";;
    2) ok "M3.1 review gate — round-cap NEEDS-HUMAN (exit 2) — escalation works";;
    3) no "M3.1 review gate — setup error (exit 3): <2 reviewers returned a verdict (see ${T}.rev)";;
    *) no "M3.1 review gate — unexpected exit ${rc} (see ${T}.rev)";;
  esac
fi

# M3.2 — governance §4 acceptance demo (hermetic, no AI calls, fast)
if scripts/dev-loop/demo-governance.sh >"${T}.gov" 2>&1; then
  ok "M3.2 governance §4 demo — $(grep -oE '[0-9]+ passed, [0-9]+ failed' "${T}.gov" | tail -1)"
else
  no "M3.2 governance §4 demo — FAILED (see ${T}.gov)"
fi

# ───────────────────────── verdict ─────────────────────────
echo
echo "================================================================"
echo " RESULT on ${MACHINE}: ${pass} pass, ${fail} fail, ${warn} warn, ${skip} skip"
if [ "$fail" -eq 0 ]; then
  echo " ✅ M2 + M3 verified on ${MACHINE}"
  echo "================================================================"; exit 0
fi
echo " ❌ ${fail} check(s) failed on ${MACHINE} — logs: ${T}.*"
echo "================================================================"; exit 1
