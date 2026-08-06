#!/usr/bin/env bash
# pollution-wall-check.sh — the moat-pollution guard (`dev_loop_never_writes_partner_signals`).
#
# WHY THIS EXISTS: an earlier unattended loop polluted the product's moat metric — of 95 partner
# interactions, 43 (45%) were machine-injected, not real use. That made "do I genuinely use this
# every day?" unmeasurable. The rule (EXECUTION-PLAN / AUTONOMY-GOVERNANCE): the dev-loop chain
# must write ONLY to its own ledgers (dev-loop-log.jsonl, deviation-proposals.jsonl, reports) and
# NEVER touch the partner-signals moat ledger. This check is the PRECONDITION that must pass before
# (re-)arming ANY loop (commute-loop, coordinator-driven, future self-ignition).
#
# Two layers, BOTH run by default:
#   1. STATIC (airtight vs. indirection) — NO script in the RUNTIME dev-loop chain may even NAME a
#      partner-signals path in executable (non-comment) code, case-insensitively. The chain writes
#      only its own ledgers, so it has zero legitimate reason to reference the moat ledger at all —
#      read OR write. Forbidding any mention (not just a literal same-line redirect) closes the
#      variable-indirection bypass, e.g.  LEDGER="$STATE_DIR/partner-signals.jsonl"; >> "$LEDGER".
#      Reported with TRUE original line numbers; a future legitimate READ fails ON PURPOSE so the
#      exception is a deliberate, reviewed decision.
#   2. RUNTIME (self-contained hermetic proof) — seed a known partner-signals.jsonl inside a TEMP
#      SPECTYN_STATE_DIR, run a REAL deviation-handler cycle against a throwaway git repo, then
#      assert that ledger is byte-identical AND that no partner-signals file was created anywhere in
#      the state dir. This tests the EXACT production invariant (a write into the CONFIGURED state
#      dir's ledger) and is hermetic — it does NOT read $HOME, so ambient spectyn activity cannot
#      make it flake, and a write to $SPECTYN_STATE_DIR/partner-signals cannot slip past it. It also
#      catches a constructed-string write the static text scan cannot see.
#
# Usage:
#   pollution-wall-check.sh                static guard + the hermetic runtime proof
#   pollution-wall-check.sh --static-only  skip the runtime proof (e.g. no git available)
# Exit: 0 = wall intact / 1 = pollution risk (a chain script names partner-signals, or the runtime
#       proof saw the ledger change) / 2 = setup error (a chain script / deviation-handler / git
#       missing — the wall could not be fully proven).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
RUN_PROOF=1; [ "${1:-}" = "--static-only" ] && RUN_PROOF=0

# The RUNTIME dev-loop chain — every script that actually executes during a loop. Test/check
# harnesses (demo-governance.sh, *-check.sh, *selftest*) are intentionally NOT here: they may create
# a partner-signals fixture in their own temp dir to PROVE non-pollution.
CHAIN=(
  "$HERE/spec-gate.sh" "$HERE/spec-lib.sh" "$HERE/deviation-handler.sh"
  "$HERE/commute-loop.sh" "$HERE/status.sh"
  "$ROOT/scripts/local-ai/review.sh" "$ROOT/.claude/skills/local-ai/ask.sh"
  "$ROOT/scripts/dev-cluster/run-task.sh" "$ROOT/scripts/dev-cluster/coordinator.sh"
  "$ROOT/scripts/dev-cluster/lease.sh" "$ROOT/scripts/dev-cluster/hosts.sh"
)
DEVIATION="$HERE/deviation-handler.sh"

fail=0; setup=0; pass=0
say_fail()  { echo "  ✗ $1"; fail=$((fail+1)); }
say_setup() { echo "  ⚠ $1"; setup=$((setup+1)); }
say_pass()  { echo "  ✓ $1"; pass=$((pass+1)); }

echo "=== pollution wall: dev_loop_never_writes_partner_signals ==="
echo "--- layer 1: static (no runtime-chain script names partner-signals in code) ---"
for f in "${CHAIN[@]}"; do
  rel="${f#$ROOT/}"
  [ -f "$f" ] || { say_setup "$rel — MISSING from the chain (setup error)"; continue; }
  # awk: per line, strip a full-line OR trailing ' #...' comment, then match the token
  # case-insensitively on what remains. Reports the TRUE original line number (NR) and original
  # text. Any executable mention — literal or a variable assigned the path — is flagged, so
  # indirection (write through a variable) is caught too.
  hits="$(awk '
    { s=$0
      sub(/^[ \t]*#.*/, "", s)        # full-line comment -> gone
      sub(/[ \t]#.*/, "", s)          # trailing comment  -> gone
      if (tolower(s) ~ /partner[-_]signals/) printf "%d: %s\n", NR, $0 }
  ' "$f" 2>/dev/null || true)"
  if [ -n "$hits" ]; then
    say_fail "$rel names partner-signals in executable code (the chain must never touch the moat ledger):"
    printf '%s\n' "$hits" | sed 's/^/        /'
  else
    say_pass "$rel — no partner-signals reference in code"
  fi
done

echo "--- layer 2: runtime proof (real deviation cycle leaves a seeded partner-signals untouched) ---"
if [ "$RUN_PROOF" = 0 ]; then
  echo "  · skipped (--static-only)"
elif ! command -v git >/dev/null 2>&1; then
  say_setup "git not available — cannot run the hermetic runtime proof"
elif [ ! -x "$DEVIATION" ] && [ ! -f "$DEVIATION" ]; then
  say_setup "deviation-handler.sh missing — cannot run the runtime proof"
else
  work="$(mktemp -d "${TMPDIR:-/tmp}/pollution-proof.XXXXXX")" || work=""
  if [ -z "$work" ]; then say_setup "mktemp failed — cannot run the runtime proof"; else
    trap 'rm -rf "$work"' EXIT
    state="$work/state"; repo="$work/repo"; mkdir -p "$state" "$repo/src"
    seed="$state/partner-signals.jsonl"
    printf 'SEED real-use signal — the dev loop must NOT touch this.\n' > "$seed"
    sum() { (cksum "$1" 2>/dev/null || md5 -q "$1" 2>/dev/null || shasum "$1" 2>/dev/null) | awk '{print $1}'; }
    before="$(sum "$seed")"
    # hermetic repo: base commit on the default branch, then an IN-SCOPE change on a dev branch so a
    # real (passing) deviation cycle runs and exercises the ledger-writing path.
    ( cd "$repo"
      git init -q; git config user.email proof@spectyn.local; git config user.name proof
      git config commit.gpgsign false
      printf 'one\n' > src/widget.txt; git add -A; git commit -q -m base
      git checkout -q -b dev/pollution-proof
      printf 'two\n' >> src/widget.txt; git add -A; git commit -q -m change
      cat > spec.toml <<'EOF'
[spec]
capability  = "dispatch"
component   = "pollution proof widget"
acceptance  = "widget returns ok"
scope_allow = ["src/"]
EOF
      SPECTYN_STATE_DIR="$state" bash "$DEVIATION" --spec spec.toml --range HEAD~1..HEAD \
        --verify-exit 0 --review-exit 0 >/dev/null 2>&1
      echo $? > "$work/devrc"
    )
    devrc="$(cat "$work/devrc" 2>/dev/null || echo 127)"
    after="$(sum "$seed")"
    # any partner-signals file created anywhere under the state dir (besides our seed) = pollution
    stray="$(find "$state" -name '*partner?signals*' ! -path "$seed" 2>/dev/null | head -1)"
    # PROVE the cycle actually ran before trusting "untouched": the handler must return a governance
    # verdict (0 pass / 10 retry / 20 escalate / 30 contained) AND have written its own ledger. A
    # setup error (3), "couldn't run" (126/127), or a missing ledger means it never did any work —
    # so an unchanged seed proves nothing. That guards against a vacuous green (codex r3).
    case "$devrc" in 0|10|20|30) ran=1;; *) ran=0;; esac
    [ -f "$state/dev-loop-log.jsonl" ] || ran=0
    if [ "$ran" = 0 ]; then
      say_setup "runtime proof INCONCLUSIVE — the deviation cycle did not actually run (exit=$devrc, ledger $([ -f "$state/dev-loop-log.jsonl" ] && echo written || echo missing)); cannot prove non-pollution"
    elif [ "$before" = "$after" ] && [ -z "$stray" ]; then
      say_pass "a real deviation cycle ran (exit=$devrc, ledger written) and left \$SPECTYN_STATE_DIR/partner-signals.jsonl byte-identical; no partner-signals ledger created"
    else
      [ "$before" != "$after" ] && say_fail "runtime proof: the seeded partner-signals.jsonl CHANGED during a real cycle (moat pollution)"
      [ -n "$stray" ] && say_fail "runtime proof: the cycle created a partner-signals file in the state dir: $stray"
    fi
  fi
fi

echo "=== ${pass} pass, ${setup} setup-issue, ${fail} fail ==="
if [ "$fail" -gt 0 ]; then
  echo "🛑 pollution wall BREACHED — do NOT arm any loop until fixed."; exit 1
fi
if [ "$setup" -gt 0 ]; then
  echo "⚠ setup incomplete — the wall could not be fully proven (missing chain script / git / demo). Resolve before arming."; exit 2
fi
echo "✅ pollution wall intact — the dev-loop chain cannot write the moat ledger."
echo "   (precondition met: a loop may be armed, subject to its own gates / §0.1.)"
exit 0
