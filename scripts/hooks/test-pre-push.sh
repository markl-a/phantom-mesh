#!/usr/bin/env bash
# Throwaway-repo verification for scripts/hooks/pre-push (ACCEL-FRAMEWORK §⑥).
# Proves BOTH polarities: the cases that must be blocked are blocked, and the
# cases that must pass (feature branches, merge commits to main) really pass.
# No cargo, no network. Drives the hook by feeding ref lines on stdin.
set -uo pipefail

HOOK="$(cd "$(dirname "$0")" && pwd)/pre-push"
[ -x "$HOOK" ] || { echo "FATAL: hook not executable at $HOOK"; exit 2; }

ZERO="0000000000000000000000000000000000000000"
PASS=0
FAIL=0

ok()   { echo "  ✓ $1"; PASS=$((PASS+1)); }
bad()  { echo "  ✗ $1"; FAIL=$((FAIL+1)); }

# Run the hook in $REPO with HEAD set as desired, feeding one ref line on stdin.
# Args: <expect: block|allow> <desc> <local_ref> <local_sha> <remote_ref> <remote_sha>
run_case() {
  local expect="$1" desc="$2" lref="$3" lsha="$4" rref="$5" rsha="$6"
  local rc
  ( cd "$REPO" && printf '%s %s %s %s\n' "$lref" "$lsha" "$rref" "$rsha" | "$HOOK" origin "file://$REPO" ) >/dev/null 2>&1
  rc=$?
  if [ "$expect" = "block" ]; then
    if [ "$rc" -ne 0 ]; then ok "BLOCK: $desc (rc=$rc)"; else bad "BLOCK expected but passed: $desc"; fi
  else
    if [ "$rc" -eq 0 ]; then ok "ALLOW: $desc"; else bad "ALLOW expected but blocked: $desc (rc=$rc)"; fi
  fi
}

REPO="$(mktemp -d)"
trap 'rm -rf "$REPO"' EXIT
git init -q "$REPO"
cd "$REPO"
git config user.email t@t && git config user.name t

# main: c0 (root) -> c1 (non-merge)
git commit -q --allow-empty -m c0
# Normalize the initial branch name to `main` regardless of git's init default
# (older git defaults to `master`).
git branch -M main 2>/dev/null || git checkout -q -B main
C0="$(git rev-parse HEAD)"
git commit -q --allow-empty -m c1
C1="$(git rev-parse HEAD)"

# feature branch off c0 with its own commit
git checkout -q -b step3-coach-install-schedule "$C0"
git commit -q --allow-empty -m feat1
FEAT="$(git rev-parse HEAD)"

# a real merge commit on main (merge feature into main)
git checkout -q main
git merge -q --no-ff -m "merge feat" step3-coach-install-schedule
MERGE="$(git rev-parse HEAD)"
[ "$(git rev-list --parents -n1 "$MERGE" | wc -w)" -ge 3 ] || { echo "setup: MERGE is not a merge commit"; exit 2; }

echo "pre-push hook polarity tests:"

# ── NEGATIVE (must be blocked), HEAD == main ─────────────────────────────────
git checkout -q main
run_case block "non-merge commit (c1) pushed to main while HEAD==main" \
  "refs/heads/main" "$C1" "refs/heads/main" "$ZERO"

# force / non-FF to main: remote tip MERGE is NOT an ancestor of older C1.
run_case block "force/non-FF push to main (rewind to older C1)" \
  "refs/heads/main" "$C1" "refs/heads/main" "$MERGE"

# deleting main (git sends "(delete)" as the local ref name)
run_case block "delete main" \
  "(delete)" "$ZERO" "refs/heads/main" "$MERGE"

# ── POSITIVE (must pass) ─────────────────────────────────────────────────────
# merge commit to main, HEAD==main, fast-forward (new ref) → allowed.
run_case allow "merge commit pushed to main (HEAD==main)" \
  "refs/heads/main" "$MERGE" "refs/heads/main" "$ZERO"

# feature-branch push (the daily step3 push) → always allowed, even non-merge.
git checkout -q step3-coach-install-schedule
run_case allow "feature branch push (non-merge commit, HEAD==feature)" \
  "refs/heads/step3-coach-install-schedule" "$FEAT" \
  "refs/heads/step3-coach-install-schedule" "$ZERO"

# feature branch FORCE push → allowed (we only guard main).
run_case allow "feature branch force-push (only main is guarded)" \
  "refs/heads/step3-coach-install-schedule" "$FEAT" \
  "refs/heads/step3-coach-install-schedule" "$C1"

# non-merge commit pushed to main BUT HEAD is a feature branch → allowed
# (guard 2 only fires when HEAD==main; this is e.g. ff-only fast-forward).
git checkout -q step3-coach-install-schedule
run_case allow "fast-forward of main while HEAD==feature (merge tip, FF)" \
  "refs/heads/main" "$MERGE" "refs/heads/main" "$C1"

echo
echo "RESULT: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
