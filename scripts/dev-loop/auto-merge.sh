#!/usr/bin/env bash
# auto-merge.sh <id> — GUARDED auto-merge of dev/<id> into BASE (§0.1, Stage-2).
#
# This is the guarded opening of the §0.1 autonomy gate (owner 2026-06-10, choice
# "有護欄地開"): the unattended loop may merge its OWN work into the dev base branch
# only when EVERY safety condition holds — otherwise it stays branches-only (the
# default everywhere else). AUTONOMY-GOVERNANCE.md R2/R5 still apply; this never
# touches `main`, only the dev integration base (BACKLOG_BASE).
#
# Conditions to merge (ALL required; any miss → branches-only, never main):
#   * enabled    : $STATE_DIR/auto-merge-enabled exists AND its first line NAMES the
#                  exact dev base authorized (allowlist); $BASE must equal it, else
#                  refuse. Arm with:  echo '<dev-base>' > ~/.phantom-mesh/auto-merge-enabled
#                  (an empty file, or any other branch incl. main/master, is refused.)
#   * not stopped: kill-switch file absent     ($STATE_DIR/auto-merge-stop)
#   * not dry-run: observation file absent      ($STATE_DIR/auto-merge-dryrun)
#                  (dry-run = log "WOULD merge" + notify, but DON'T → branches-only)
#   * all-green  : caller's --verify-exit / --review-exit / --deviation are all 0
#                  (review.sh R5 consensus = both reviewers APPROVE, already run)
#   * no R2 zone : the diff touches NO forbidden zone and deletes NO file
#                  (.github/CI, secrets/.env, schema/migrations, wrangler/deploy)
#                  — defense-in-depth on top of deviation-handler's own R2 check
#   * clean merge: dev/<id> merges --no-ff into a FRESH origin/BASE with no conflict
#
# Every decision is appended to $STATE_DIR/auto-merge.log and (best-effort) sent to
# the fleet inbox, so there is always an audit trail of what merged and what didn't.
#
# Usage (called by runner-loop after `backlog.sh done`):
#   auto-merge.sh <id> --verify-exit N --review-exit N --deviation N
# Exit: 0 merged+pushed · 10 declined (disabled/stop/dry-run) · 11 not-all-green
#       · 20 REFUSED (R2 forbidden zone / deletion) · 30 merge conflict · 31 push raced
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT" || exit 3

ID="${1:?usage: auto-merge.sh <id> --verify-exit N --review-exit N --deviation N}"; shift || true
BASE="${BACKLOG_BASE:-step3-coach-install-schedule}"
STATE_DIR="${PHANTOM_STATE_DIR:-$HOME/.phantom-mesh}"
NODE="${BACKLOG_NODE:-$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown)}"
ENABLED="$STATE_DIR/auto-merge-enabled"
STOP="$STATE_DIR/auto-merge-stop"
DRYRUN="$STATE_DIR/auto-merge-dryrun"
LOG="$STATE_DIR/auto-merge.log"

VEXIT=1; REXIT=1; DEV=1
while [ $# -gt 0 ]; do case "$1" in
  --verify-exit) VEXIT="${2:?}"; shift;;
  --review-exit) REXIT="${2:?}"; shift;;
  --deviation)   DEV="${2:?}"; shift;;
  *) echo "auto-merge: unknown arg '$1'" >&2; exit 3;;
esac; shift; done

mkdir -p "$STATE_DIR" 2>/dev/null || true
say() { local m; m="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)  [$NODE] $*"; printf '%s\n' "$m"; printf '%s\n' "$m" >> "$LOG" 2>/dev/null || true; }
# best-effort owner notification (never fatal): use the repo's newest phantom.
notify() {
  local p; p="$( { [ -x "$ROOT/core/target/debug/phantom" ] && echo "$ROOT/core/target/debug/phantom"; } || command -v phantom 2>/dev/null || true)"
  [ -n "$p" ] || return 0
  ( "$p" inbox send all "$1" >/dev/null 2>&1 ) & local c=$!
  ( sleep 10; kill "$c" 2>/dev/null ) & local w=$!
  wait "$c" 2>/dev/null || true
  kill "$w" 2>/dev/null; wait "$w" 2>/dev/null || true   # reap the watchdog so it can't signal a recycled PID (review: agy)
}

# ── kill-switch / enable / dry-run ──────────────────────────────────────────
if [ -f "$STOP" ];     then say "STOP file present → branches-only ($ID)"; exit 10; fi
if [ ! -f "$ENABLED" ]; then say "not enabled (no auto-merge-enabled) → branches-only ($ID)"; exit 10; fi

# HARD GUARD — ALLOWLIST, not blacklist (review: codex): the enable file must NAME
# the exact dev integration base authorized for auto-merge. $BASE must equal it, or
# we refuse. This makes "only ever the one dev base" an enforced invariant — any
# other branch (main, master, production, stable, release/*, …) is refused unless
# explicitly named — and a residual main/master deny blocks even a mis-authorized file.
case "$BASE" in
  main|master|trunk) say "REFUSED $ID: BASE='$BASE' is the public main line — never auto-merge there"; exit 12;;
esac
ALLOWED_BASE="$(head -n1 "$ENABLED" 2>/dev/null | tr -d '[:space:]')"
if [ -z "$ALLOWED_BASE" ]; then
  say "REFUSED $ID: $ENABLED is empty — it must NAME the dev base authorized for auto-merge (allowlist). Run: echo '$BASE' > $ENABLED"; exit 12
fi
if [ "$BASE" != "$ALLOWED_BASE" ]; then
  say "REFUSED $ID: BASE='$BASE' != authorized base '$ALLOWED_BASE' (allowlist) — auto-merge only targets the one named dev base"; exit 12
fi

# ── re-assert the gates (defense-in-depth; the caller already ran them) ──────
if [ "$VEXIT" != 0 ] || [ "$REXIT" != 0 ] || [ "$DEV" != 0 ]; then
  say "gates NOT all-green (verify=$VEXIT review=$REXIT deviation=$DEV) → branches-only ($ID)"; exit 11
fi
git rev-parse -q --verify "refs/heads/dev/$ID" >/dev/null 2>&1 || { say "no local branch dev/$ID → branches-only"; exit 11; }

# fresh BASE for an honest merge-base + race-free push.
git fetch -q origin "+refs/heads/$BASE:refs/remotes/origin/$BASE" 2>/dev/null \
  || { say "cannot fetch origin/$BASE → branches-only ($ID)"; exit 11; }
mb="$(git merge-base "origin/$BASE" "dev/$ID" 2>/dev/null)" || { say "no merge-base with origin/$BASE → branches-only ($ID)"; exit 11; }

# ── R2 forbidden-zone + deletion scan (belt-and-suspenders over deviation-handler) ──
# The retire commit legitimately MOVES backlog/<id>.toml → backlog/done/, so a
# deletion UNDER backlog/ is expected and excluded; any OTHER deletion is refused.
changed="$(git diff --name-only "$mb" "dev/$ID" 2>/dev/null)"
deleted="$(git diff --diff-filter=D --name-only "$mb" "dev/$ID" 2>/dev/null | grep -v '^backlog/' || true)"
forbidden="$(printf '%s\n' "$changed" | grep -Ei '(^|/)\.github/|(^|/)\.gitlab|(^|/)\.circleci/|(^|/)secrets?(/|\.|$)|(^|/)\.env($|\.)|(^|/)migrations?/|(^|/)wrangler\.toml$|(^|/)\.deploy|(^|/)id_rsa|\.pem$' || true)"
if [ -n "$deleted" ] || [ -n "$forbidden" ]; then
  say "REFUSED $ID — R2 forbidden zone/deletion in diff (deleted:[$(printf '%s' "$deleted" | tr '\n' ' ')] forbidden:[$(printf '%s' "$forbidden" | tr '\n' ' ')]) → branches-only, needs human"
  notify "[auto-merge] REFUSED dev/$ID on $NODE — R2 zone/deletion in diff; left for human review"
  exit 20
fi

# ── dry-run observation mode: announce intent, do NOT merge ─────────────────
if [ -f "$DRYRUN" ]; then
  say "DRY-RUN: WOULD auto-merge dev/$ID into $BASE (all green, no R2 zone). Remove $DRYRUN to arm real merges."
  notify "[auto-merge DRY-RUN] WOULD merge dev/$ID into $BASE on $NODE (all gates green) — observation mode, not merged"
  exit 10
fi

# ── REAL guarded merge (one retry on a benign push race) ────────────────────
attempt=0
while [ "$attempt" -lt 2 ]; do
  attempt=$((attempt+1))
  git fetch -q origin "+refs/heads/$BASE:refs/remotes/origin/$BASE" 2>/dev/null || true
  git checkout -q -B "$BASE" "origin/$BASE" 2>/dev/null || { say "cannot checkout fresh $BASE → branches-only ($ID)"; exit 11; }
  if ! git merge --no-ff -m "auto-merge dev/$ID into $BASE (§0.1 guarded: R5 consensus + dev_verify + clean scope + no R2 zone)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" "dev/$ID" >/dev/null 2>&1; then
    git merge --abort 2>/dev/null || true
    say "CONFLICT merging dev/$ID into $BASE → branches-only, needs human ($ID)"
    notify "[auto-merge] CONFLICT dev/$ID into $BASE on $NODE — left for human merge"
    git checkout -q "dev/$ID" 2>/dev/null || true
    exit 30
  fi
  if git push -q origin "$BASE" 2>/dev/null; then
    git push -q origin ":refs/heads/claim/$ID" 2>/dev/null || true   # release the lock; work is in BASE
    say "✓ AUTO-MERGED dev/$ID into $BASE (§0.1 guarded, attempt $attempt)"
    notify "[auto-merge] ✅ dev/$ID merged into $BASE on $NODE (§0.1 guarded: all gates green)"
    git checkout -q "dev/$ID" 2>/dev/null || true
    exit 0
  fi
  say "push of $BASE rejected (raced) — refetch + retry ($ID, attempt $attempt)"
done
say "push kept losing the race after $attempt attempts → branches-only ($ID); dev/$ID is pushed, retry next time"
notify "[auto-merge] dev/$ID could not push $BASE (raced) on $NODE — branches-only, will retry"
git checkout -q "dev/$ID" 2>/dev/null || true
exit 31
