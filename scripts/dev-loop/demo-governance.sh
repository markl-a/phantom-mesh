#!/usr/bin/env bash
# demo-governance.sh — the AUTONOMY-GOVERNANCE.md §4 demonstrable acceptance.
#
# Runs the locked §4 scenario end-to-end in a HERMETIC throwaway git repo + a temp
# state dir, exercising every R1–R5 path, and asserts the two safety invariants:
#   • main is never touched (无害化: branch-only, no merge to main);
#   • the REAL moat ledger (~/.phantom-mesh/partner-signals.jsonl) is byte-identical
#     before and after (防污染牆: machine traffic never pollutes partner-signals).
#
# This is both the governance "可示範" acceptance and the dogfood test for spec-gate
# + deviation-handler. Exit 0 = all assertions pass.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEC_GATE="${HERE}/spec-gate.sh"
HANDLER="${HERE}/deviation-handler.sh"

pass=0; fail=0
expect() { # label actual expected
  if [ "$2" = "$3" ]; then echo "  ✓ $1 (exit $2)"; pass=$((pass+1));
  else echo "  ✗ $1 — got exit $2, expected $3"; fail=$((fail+1)); fi
}
assert() { # label condition(0/1-as-exit)
  if [ "$2" = 0 ]; then echo "  ✓ $1"; pass=$((pass+1)); else echo "  ✗ $1"; fail=$((fail+1)); fi
}

# ── pollution-wall snapshot of the REAL moat ledger ──────────────────────────
REAL_MOAT="${HOME}/.phantom-mesh/partner-signals.jsonl"
moat_sig() { [ -f "$REAL_MOAT" ] && cksum "$REAL_MOAT" 2>/dev/null || echo "absent"; }
MOAT_BEFORE="$(moat_sig)"

# ── hermetic sandbox ─────────────────────────────────────────────────────────
SBX="$(mktemp -d)"; STATE="$(mktemp -d)"
cleanup() { rm -rf "$SBX" "$STATE"; }
trap cleanup EXIT
export PHANTOM_STATE_DIR="$STATE"
export DEVIATION_MAX_ROUNDS=2

cd "$SBX"
git init -q
git config user.email demo@phantom.local; git config user.name "phantom-demo"
git config commit.gpgsign false; git config tag.gpgsign false   # don't hang on a global signing config
git config diff.renames true                                    # so renames show as R### (test the R path)
git symbolic-ref HEAD refs/heads/main 2>/dev/null || true
mkdir -p src
cat > spec.toml <<'EOF'
[spec]
capability  = "dispatch"
component   = "demo widget"
acceptance  = "widget returns ok"
scope_allow = ["src/"]
max_files   = 3
EOF
echo "seed" > src/seed.txt
git add -A && git commit -qm "seed"
git checkout -q -b dev   # all autonomous work happens on a dev branch, never main

echo "=== §4 governance acceptance demo (hermetic) ==="
echo

echo "[0] spec-gate rejects an unbounded task (no spec / incomplete)"
: > "$STATE/empty.toml"   # outside the repo so it can't leak into a commit
"$SPEC_GATE" validate "$STATE/empty.toml" >/dev/null 2>&1; expect "empty/incomplete spec → REJECT" "$?" 2
"$SPEC_GATE" validate missing-file.toml >/dev/null 2>&1; expect "absent spec → setup error" "$?" 3
"$SPEC_GATE" validate spec.toml >/dev/null 2>&1; expect "valid spec → ACCEPT" "$?" 0
echo

echo "[1] CLEAN: in-scope change, verify green, review APPROVE → PASS (land)"
echo "a" > src/a.txt; git add -A; git commit -qm "in-scope a"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "conforming change → PASS" "$?" 0
echo

echo "[2] SCOPE-EXCEED: change outside scope_allow → RETRY then ESCALATE (R1-i, R3, R4)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
mkdir -p other; echo "x" > other/oops.txt; git add -A; git commit -qm "out of scope"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "1st deviation → RETRY" "$?" 10
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "2nd consecutive → ESCALATE needs-human" "$?" 20
[ -s "$STATE/deviation-proposals.jsonl" ]; assert "escalation wrote a needs-human proposal" "$?"
grep -q "needs-human proposal" "$STATE/notifications.log" 2>/dev/null; assert "owner was notified" "$?"
echo

echo "[3] FORBIDDEN ZONE: diff touches CI (.github/) → CONTAINED immediately (R2, no retry)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
mkdir -p .github/workflows; echo "on: push" > .github/workflows/ci.yml; git add -A; git commit -qm "touch CI"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "R2 forbidden zone → CONTAINED" "$?" 30
echo

echo "[4] DELETION: removing a tracked file → CONTAINED (R2 'delete files')"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
git rm -q src/seed.txt; git commit -qm "delete a file"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "deletion → CONTAINED" "$?" 30
echo

echo "[5] dev_verify RED: in-scope but build/test failed → RETRY (R1-ii)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
echo "b" > src/b.txt; git add -A; git commit -qm "in-scope but breaks build"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 1 --review-exit 0 >/dev/null 2>&1
expect "verify-red → RETRY" "$?" 10
echo

echo "[6] REVIEW REQUEST_CHANGES: in-scope, verify green, but reviewer blocked → RETRY (R1-iii/R5)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
echo "c" > src/c.txt; git add -A; git commit -qm "in-scope but reviewer rejects"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 1 >/dev/null 2>&1
expect "review-not-approved → RETRY" "$?" 10
echo

echo "[7] R2: refuses to operate on main"
git checkout -q main
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD >/dev/null 2>&1
expect "on main → refused (setup error)" "$?" 3
git checkout -q dev
echo

echo "[8a] benign in-scope rename (src/a → src/renamed) → PASS (not over-contained)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
git mv src/a.txt src/renamed.txt; git commit -qm "in-scope rename"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "in-scope rename → PASS" "$?" 0

echo "[8b] rename to a FORBIDDEN dest (src → .github/) → CONTAINED (bypass closed, R2)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
git mv src/renamed.txt .github/evil.yml; git commit -qm "rename into CI"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "rename to forbidden dest → CONTAINED" "$?" 30

echo "[8c] rename to an OUT-OF-SCOPE dest (src/b → root) → RETRY (scope-exceeded, not silent PASS)"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
git mv src/b.txt bb.txt; git commit -qm "rename out of scope"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "out-of-scope rename → RETRY" "$?" 10
echo

echo "[9] invalid range is NOT silently PASS (must be a setup error)"
"$HANDLER" --spec spec.toml --range "no-such-ref..HEAD" --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "bad range → setup error (not PASS)" "$?" 3
echo

echo "[10] benign source whose NAME contains 'secret' is NOT a forbidden zone → PASS"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
echo q > src/SecretQuestion.txt; git add -A; git commit -qm "in-scope file named like a secret"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "src/SecretQuestion.txt (in scope) → PASS (not over-contained)" "$?" 0

echo "[11] missing --verify-exit/--review-exit is a setup error (can't bypass R1/R5)"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --review-exit 0 >/dev/null 2>&1
expect "omitting verify-exit → setup error" "$?" 3
echo

echo "[12] REPO-ROOT forbidden dir (migrations/) is caught, not just nested */migrations/* → CONTAINED"
"$HANDLER" --spec spec.toml --reset >/dev/null 2>&1
mkdir -p migrations; echo "ALTER TABLE" > migrations/001_init.sql; git add -A; git commit -qm "root migration"
"$HANDLER" --spec spec.toml --range HEAD~1..HEAD --verify-exit 0 --review-exit 0 >/dev/null 2>&1
expect "root-level migrations/ → CONTAINED" "$?" 30
echo

echo "=== safety invariants ==="
# main must hold ONLY the seed commit (autonomous work never reached it)
main_commits="$(git rev-list --count main 2>/dev/null || echo 0)"
[ "$main_commits" = "1" ]; assert "main untouched — only the seed commit (no autonomous merge)" "$?"
# the real moat ledger must be byte-identical
[ "$(moat_sig)" = "$MOAT_BEFORE" ]; assert "real partner-signals.jsonl byte-identical (污染牆 held)" "$?"
# ledger captured outcomes; never wrote near partner-signals
grep -q '"outcome":"pass"' "$STATE/dev-loop-log.jsonl" && grep -q '"outcome":"contained"' "$STATE/dev-loop-log.jsonl"
assert "dev-loop-log.jsonl recorded pass + contained outcomes" "$?"

echo
echo "=== RESULT: ${pass} passed, ${fail} failed ==="
[ "$fail" -eq 0 ] && { echo "✅ governance Stage-1 §4 acceptance: PASS"; exit 0; }
echo "❌ governance demo FAILED"; exit 1
