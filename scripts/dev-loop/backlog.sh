#!/usr/bin/env bash
# backlog.sh — SHARED task backlog in the repo + atomic claim via git (S3).
#
# The backlog is a directory of spec-gated *.toml files committed to the repo
# (backlog/ at repo root). Any machine posts specs; any machine claims one and
# works it on a branch; m1 being offline changes nothing because the backlog
# travels with git, not with a coordinator process.
#
# CLAIM ATOMICITY: claiming <id> = pushing a marker commit to refs/heads/claim/<id>.
# Two nodes racing both try to create the same remote ref with DIFFERENT commits;
# the git server accepts exactly one and rejects the other (non-fast-forward) —
# that rejection IS the "someone else got it" signal. No lease daemon needed.
#
# BASELINE-SYNC (closes the A4 gap): claim always branches from origin/<base>
# fetched seconds ago — never from a stale local checkout. Works on single-branch
# clones (a remote unix node / z13) because we fetch the base ref explicitly.
#
# Usage:
#   backlog.sh list                       specs + claim state + caps (fetches first)
#   backlog.sh next                       print first OPEN spec THIS node may claim (caps-matched)
#   backlog.sh post <spec.toml>           validate (spec-gate) -> commit to backlog/ -> push <base>
#   backlog.sh claim <id>                 atomic claim + create work branch dev/<id> off origin/<base>
#   backlog.sh done <id>                  retire spec (backlog/done/) + push work branch
#   backlog.sh release <id>               un-claim (delete remote claim ref) — abandoned tasks
#
# PLATFORM ROUTING: a spec may declare `caps = ["macos","ios"]` in its [spec]; a
# node only claims tasks whose caps it covers (so each machine develops for its
# own platforms). Node caps come from PHANTOM_NODE_CAPS / ~/.phantom-mesh/caps /
# OS fallback. A spec with no `caps` is claimable by any node (back-compat).
#
# Env: BACKLOG_BASE (default: step3-coach-install-schedule), BACKLOG_NODE (default: hostname),
#      BACKLOG_DIR (default: backlog), PHANTOM_NODE_CAPS (comma-sep platform caps)
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
SPEC_GATE="$HERE/spec-gate.sh"

BASE="${BACKLOG_BASE:-step3-coach-install-schedule}"
NODE="${BACKLOG_NODE:-$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown)}"
BDIR="${BACKLOG_DIR:-backlog}"

cd "$ROOT" || exit 3
sub="${1:-list}"

die() { echo "backlog: $*" >&2; exit 2; }

fetch_base() {
  # Single-branch-clone safe: fetch the base ref explicitly.
  git fetch origin "+refs/heads/$BASE:refs/remotes/origin/$BASE" 2>/dev/null \
    || die "cannot fetch origin/$BASE — check network/remote"
  # Claim refs need --prune so RELEASED claims disappear locally. If this
  # fetch fails we DROP the local copies instead of keeping them (review:
  # codex) — the push CAS is the real arbiter, so a missing local ref only
  # costs one rejected push, while a stale one wrongly blocks open tasks.
  if ! git fetch origin "+refs/heads/claim/*:refs/remotes/origin/claim/*" --prune 2>/dev/null; then
    git for-each-ref --format='%(refname)' 'refs/remotes/origin/claim/*' 2>/dev/null \
      | while read -r r; do git update-ref -d "$r" 2>/dev/null || true; done
  fi
}

claimed_ref() { git rev-parse -q --verify "refs/remotes/origin/claim/$1" 2>/dev/null; }

claim_owner() {
  # marker commit subject: "claim <id> by <node> at <ts>"
  git log -1 --format=%s "refs/remotes/origin/claim/$1" 2>/dev/null \
    | sed -n 's/^claim [^ ]* by \([^ ]*\) at .*/\1/p'
}

# ── platform capability routing (each machine develops for its platforms) ────
. "$HERE/spec-lib.sh"   # spec_section / spec_list — to read a spec's [spec] caps

# node_caps — the platforms THIS machine can develop for. Source order:
#   PHANTOM_NODE_CAPS env  →  ~/.phantom-mesh/caps file  →  OS-derived fallback.
# One per line. Set it per the fleet plan, e.g. z13=windows,linux ·
# ayaneo=windows,gui · acer=windows,android · m1(mac)=macos,ios.
node_caps() {
  # TRIM each item (leading/trailing space) rather than strip ALL whitespace —
  # so "mac os" stays "mac os" (won't silently become a different real cap), and
  # the bash + PowerShell ports agree on what a cap is (review: codex).
  local trim='s/^[[:space:]]*//;s/[[:space:]]*$//'
  local out=""
  if [ -n "${PHANTOM_NODE_CAPS:-}" ]; then
    out="$(printf '%s\n' "$PHANTOM_NODE_CAPS" | tr ',' '\n' | sed "$trim" | grep -v '^$')"
  else
    local f="${PHANTOM_STATE_DIR:-$HOME/.phantom-mesh}/caps"
    [ -f "$f" ] && out="$(tr ',' '\n' < "$f" | sed "$trim" | grep -v '^$')"
  fi
  if [ -n "$out" ]; then printf '%s\n' "$out"; return; fi
  # Empty/missing caps → fall back to OS so a node ALWAYS has at least its own OS
  # cap (an empty caps file must not silently make the node claim nothing forever).
  case "$(uname -s 2>/dev/null)" in
    Darwin) echo macos;;
    Linux)  echo linux;;
    *MINGW*|*MSYS*|*CYGWIN*) echo windows;;
    *) echo any;;
  esac
}

# dump a backlog spec's content (from origin/<base>) to a temp file; echo the path.
# FAILS (rc 1, no path) if the spec can't be read — so a missing/typo'd id can't
# masquerade as an empty (=any-caps) spec and slip through the caps gate (review:
# codex). Callers must check the return code.
spec_dump() {
  local t; t="$(mktemp)"
  if git show "origin/$BASE:$BDIR/$1.toml" > "$t" 2>/dev/null && [ -s "$t" ]; then
    printf '%s' "$t"; return 0
  fi
  rm -f "$t"; return 1
}

# caps_ok <spec-file> — 0 if THIS node satisfies EVERY cap the spec requires.
# A spec with no `caps` is claimable by any node (back-compat). A node whose
# caps don't cover the spec's caps is NOT a valid claimer (routes the task away).
caps_ok() {
  local sc nc need; sc="$(spec_list "$(spec_section "$1")" caps)"
  [ -z "$sc" ] && return 0                      # no caps required → any node
  nc="$(node_caps)"
  while IFS= read -r need; do
    [ -z "$need" ] && continue
    printf '%s\n' "$nc" | grep -qxF "$need" || return 1
  done <<EOF
$sc
EOF
  return 0
}

# spec_caps_str <spec-file> — the required caps as a space-joined string (display)
spec_caps_str() { spec_list "$(spec_section "$1")" caps | tr '\n' ' ' | sed 's/[[:space:]]*$//'; }

case "$sub" in
  list)
    fetch_base
    echo "backlog on origin/$BASE ($BDIR/)   [this node caps: $(node_caps | tr '\n' ' ')]"
    found=0
    for f in $(git ls-tree --name-only "origin/$BASE" -- "$BDIR/" 2>/dev/null | grep '\.toml$'); do
      found=1
      id="$(basename "$f" .toml)"
      if t="$(spec_dump "$id")"; then
        caps="$(spec_caps_str "$t")"
        capnote=""; [ -n "$caps" ] && capnote="  caps:[$caps]"
        mine=""; caps_ok "$t" || mine="  (not this machine)"
        rm -f "$t"
      else
        capnote="  (spec unreadable)"; mine=""
      fi
      if claimed_ref "$id" >/dev/null; then
        echo "  $id  [claimed by $(claim_owner "$id" || echo '?')]$capnote"
      else
        echo "  $id  [open]$capnote$mine"
      fi
    done
    [ "$found" = 1 ] || echo "  (empty — post one with: backlog.sh post <spec.toml>)"
    ;;

  caps)
    # Print THIS node's platform capabilities (what backlog routing matches on).
    node_caps | tr '\n' ' '; echo
    ;;

  next)
    # Print the first OPEN spec THIS node is allowed to claim (caps-matched),
    # for the routine to feed `claim`. Empty output = nothing for this machine.
    fetch_base
    for f in $(git ls-tree --name-only "origin/$BASE" -- "$BDIR/" 2>/dev/null | grep '\.toml$'); do
      id="$(basename "$f" .toml)"
      claimed_ref "$id" >/dev/null && continue          # skip already-claimed
      t="$(spec_dump "$id")" || continue                # unreadable spec → skip
      if caps_ok "$t"; then rm -f "$t"; echo "$id"; exit 0; fi
      rm -f "$t"
    done
    exit 0
    ;;

  post)
    f="${2:?usage: backlog.sh post <spec.toml>}"
    [ -f "$f" ] || die "spec file not found: $f"
    "$SPEC_GATE" validate "$f" || die "spec-gate REJECTED $f — fix the [spec] envelope first"
    id="$(basename "$f" .toml)"
    fetch_base
    # symbolic-ref for a branch, raw SHA when detached — restoring a detached
    # HEAD via the literal string "HEAD" would silently strand the caller on
    # $BASE (review: agy r3)
    cur_branch="$(git symbolic-ref -q --short HEAD || git rev-parse HEAD)"
    dirty="$(git status --porcelain | grep -v '^??' || true)"
    [ -z "$dirty" ] || die "working tree has uncommitted changes — commit/stash before posting"
    # Snapshot the spec BEFORE switching branches: if $f is tracked on the
    # current branch but absent on $BASE, the checkout would delete it from
    # the working tree and the copy below would read nothing (review: agy).
    tmp_spec="$(mktemp)"
    cp "$f" "$tmp_spec"
    # restore the caller's branch on EVERY exit from here on (review: agy —
    # a push failure must not strand the caller on $BASE)
    restore() { [ "$cur_branch" != "$BASE" ] && git checkout -q "$cur_branch" 2>/dev/null; rm -f "$tmp_spec"; }
    git checkout -q "$BASE" 2>/dev/null || git checkout -qb "$BASE" "origin/$BASE" \
      || { rm -f "$tmp_spec"; die "cannot checkout $BASE"; }
    git merge -q --ff-only "origin/$BASE" 2>/dev/null \
      || { restore; die "local $BASE diverged from origin — reconcile first"; }
    mkdir -p "$BDIR"
    cp "$tmp_spec" "$BDIR/$id.toml"
    git add "$BDIR/$id.toml"
    # pathspec-limited: never sweep unrelated staged files into this commit
    if ! git commit -q -m "backlog: post spec $id

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" -- "$BDIR/$id.toml"; then
      restore; die "nothing to commit (already posted?)"
    fi
    if ! git push -q origin "$BASE"; then
      restore; die "push failed — fetch + retry (local $BASE has the post commit)"
    fi
    echo "✓ posted $id to backlog on $BASE"
    restore || true
    ;;

  claim)
    id="${2:?usage: backlog.sh claim <id>}"
    fetch_base
    git ls-tree --name-only "origin/$BASE" -- "$BDIR/$id.toml" | grep -q . \
      || die "no spec $BDIR/$id.toml on origin/$BASE — backlog.sh list"
    if claimed_ref "$id" >/dev/null; then
      die "$id already claimed by $(claim_owner "$id" || echo '?')"
    fi
    # Platform routing: refuse a task whose required caps this node can't satisfy,
    # so a Windows box can't grab a macOS/iOS task (and vice-versa).
    _ct="$(spec_dump "$id")" || die "cannot read spec $BDIR/$id.toml from origin/$BASE"
    if ! caps_ok "$_ct"; then
      _need="$(spec_caps_str "$_ct")"; rm -f "$_ct"
      die "$id needs caps [$_need] but node '$NODE' has [$(node_caps | tr '\n' ' ')] — not for this machine (try a node with those caps)"
    fi
    rm -f "$_ct"
    # LOCAL preconditions BEFORE touching the remote (review: codex+agy — a
    # claim ref pushed and then a failed local checkout would orphan-lock the
    # task for everyone). Anything that could make the checkout fail dies here.
    dirty="$(git status --porcelain | grep -v '^??' || true)"
    [ -z "$dirty" ] || die "working tree has uncommitted changes — commit/stash before claiming"
    git rev-parse -q --verify "refs/heads/dev/$id" >/dev/null 2>&1 \
      && die "local branch dev/$id already exists — finish/delete it first (previous attempt?)"
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
    # nonce makes the marker commit unique even for two runners with the same
    # NODE name in the same second — identical marker SHAs would make the CAS
    # push a no-op "success" for BOTH racers (review: codex).
    nonce="$$-${RANDOM:-0}${RANDOM:-0}"
    marker="$(git commit-tree "origin/$BASE^{tree}" -p "origin/$BASE" -m "claim $id by $NODE at $ts nonce $nonce" 2>/dev/null)" \
      || die "cannot create claim marker commit"
    # THE atomic step: first push creating refs/heads/claim/<id> wins; a racer's
    # different marker commit is rejected by the server's ref CAS.
    if ! git push -q origin "$marker:refs/heads/claim/$id" 2>/dev/null; then
      fetch_base
      die "$id was claimed concurrently by $(claim_owner "$id" || echo '?') — pick another"
    fi
    # work branch off the fresh base (baseline-sync), single-branch-clone safe.
    # Preconditions above make failure unexpected — but if it still happens,
    # ROLL THE CLAIM BACK so the task is not locked for other nodes.
    if ! git checkout -qb "dev/$id" "origin/$BASE"; then
      git push -q origin ":refs/heads/claim/$id" 2>/dev/null \
        && die "could not create work branch dev/$id — claim rolled back" \
        || die "could not create work branch dev/$id AND claim rollback failed — run: backlog.sh release $id"
    fi
    echo "✓ claimed $id (marker pushed) — now on branch dev/$id off origin/$BASE"
    echo "  when done: backlog.sh done $id"
    ;;

  done)
    id="${2:?usage: backlog.sh done <id>}"
    cur="$(git rev-parse --abbrev-ref HEAD)"
    [ "$cur" = "dev/$id" ] || die "run from the work branch dev/$id (you are on $cur)"
    mkdir -p "$BDIR/done"
    if [ -f "$BDIR/$id.toml" ]; then
      git mv "$BDIR/$id.toml" "$BDIR/done/$id.toml" || die "cannot retire spec (git mv failed)"
      # pathspec-limited: unrelated staged work must never ride this commit.
      # A failed retire commit must NOT fall through to push-and-claim-success
      # (review: codex r3 — silent wrong-contents completion).
      git commit -q -m "backlog: retire spec $id (work complete on dev/$id)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" -- "$BDIR/$id.toml" "$BDIR/done/$id.toml" \
        || die "retire commit failed (hooks?) — resolve and re-run: backlog.sh done $id"
    fi
    git push -q -u origin "dev/$id" || die "push of dev/$id failed"
    echo "✓ dev/$id pushed (spec retired to $BDIR/done/) — review gate + owner review next"
    ;;

  release)
    id="${2:?usage: backlog.sh release <id>}"
    git push -q origin ":refs/heads/claim/$id" || die "cannot delete claim ref (not claimed?)"
    echo "✓ released claim on $id"
    ;;

  -h|--help|help) sed -n '2,30p' "$0";;
  *) die "unknown subcommand '$sub' — list|next|post|claim|done|release";;
esac
