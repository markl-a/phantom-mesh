#!/usr/bin/env bash
# self-update.sh — a node auto-catches-up to the latest framework version (S7).
#
# When `origin/<base>` moves ahead of the commit this node last BUILT, this
# rebuilds the spectyn binary, swaps the running serve, and records the new
# built commit — so a framework change pushed to the base branch propagates to
# the whole fleet within a tick, no manual rebuild per machine.
#
# SAFETY (the whole point — never break a working node):
#  * build-BEFORE-swap: a failed `cargo build` leaves the OLD binary serving;
#    the marker is NOT advanced, so it retries next tick (honest, self-healing).
#  * skip-when-busy: if the checkout is on a `dev/*` work branch (mid-task), do
#    nothing — never yank the tree out from under an in-flight task.
#  * skip-when-dirty: tracked-file changes present → skip (don't clobber edits).
#  * single build: one `cargo build` (parallel release compiles get SIGKILL'd
#    on memory pressure — the fleet's 137 constraint).
#  * branches-only posture: this only ever moves the node TO origin/<base>
#    (the integration branch the owner controls); it never pushes anything.
#
# Override hooks (defaults do the real thing; tests inject fakes):
#   SELF_UPDATE_BUILD_CMD   build step       (default: cargo build --release --bin spectyn in core/)
#   SELF_UPDATE_INSTALL_CMD install the built binary to the serve's path
#   SELF_UPDATE_RESTART_CMD restart the serve with the new binary
#   SPECTYN_BUILT_COMMIT    marker file      (default: ~/.spectyn-mesh/built-commit)
#
# Usage: self-update.sh [--base <branch>]
# Exit:  0 up-to-date · 1 updated+serve-healthy · 2 build/install failed (tree
#        restored, old binary kept) · 3 skipped (busy/dirty) · 4 setup error
#        (fetch/checkout) · 5 updated but serve restart UNVERIFIED (needs human)
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BASE="${BACKLOG_BASE:-step3-coach-install-schedule}"
STATE_DIR="${SPECTYN_STATE_DIR:-$HOME/.spectyn-mesh}"
MARKER="${SPECTYN_BUILT_COMMIT:-$STATE_DIR/built-commit}"
while [ $# -gt 0 ]; do case "$1" in
  --base) BASE="${2:?--base needs a branch}"; shift;;
  -h|--help) sed -n '2,22p' "$0"; exit 0;;
  *) echo "self-update: unknown arg '$1'" >&2; exit 4;;
esac; shift; done

cd "$ROOT" || { echo "self-update: no repo at $ROOT" >&2; exit 4; }
mkdir -p "$STATE_DIR"

# ── default real steps (overridable) ────────────────────────────────────────
default_build() { ( cd "$ROOT/core" && cargo build --release --bin spectyn ); }

default_install() {
  # Copy the fresh build to the path the serve actually launches from, if that
  # differs from the build output. macOS: a freshly-copied binary must be
  # re-signed or launchd SIGKILLs it on exec (codesigning) — fold that in.
  local built="$ROOT/core/target/release/spectyn" dest="$HOME/.local/bin/spectyn"
  [ -x "$built" ] || { echo "self-update: build output missing: $built" >&2; return 1; }
  # SAME-FILE guard (not an unchanged-content optimization): if the serve runs
  # directly from the build output, `built` and `dest` are the literal same
  # inode → skip the copy (cp'ing a file onto itself would truncate it). When
  # dest is a separate copy this is correctly false and we always install the
  # fresh build below (review: agy r5 — clarifying the intent, the copy SHOULD
  # run every update).
  if [ -e "$dest" ] && [ "$built" -ef "$dest" ]; then return 0; fi
  # mkdir the install dir if missing — a non-existent ~/.local/bin used to make
  # the `[ -d ]` guard skip the copy AND return its falsy status, failing the
  # whole update on a fresh box (review: agy r3).
  mkdir -p "$(dirname "$dest")" || { echo "self-update: cannot create $(dirname "$dest")" >&2; return 1; }
  cp "$built" "$dest.tmp" && mv "$dest.tmp" "$dest" || return 1
  case "$(uname -s)" in Darwin) codesign -f -s - "$dest" >/dev/null 2>&1 || true;; esac
  return 0
}

# Resolve the serve port: SPECTYN_PORT wins, else [core].port / port in
# agents.toml, else 7878 — so a node on a custom port isn't falsely judged
# unhealthy and aborted (review: agy r3).
resolve_port() {
  if [ -n "${SPECTYN_PORT:-}" ]; then printf '%s' "$SPECTYN_PORT"; return; fi
  local p
  p="$(sed -n 's/^[[:space:]]*port[[:space:]]*=[[:space:]]*\([0-9]\{2,5\}\).*/\1/p' \
        "$STATE_DIR/agents.toml" 2>/dev/null | head -1)"
  printf '%s' "${p:-7878}"
}

_healthz() { curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$1/healthz" 2>/dev/null; }

default_restart() {
  # Swap the running serve to the new binary and PROVE the new one took over.
  #
  # A plain "is :PORT answering 200?" is a FALSE-POSITIVE trap (review: codex +
  # agy r4): if the old process ignores the kill, or the new one fails to bind
  # (EADDRINUSE because the port wasn't released yet), the OLD serve answers 200
  # and we'd wrongly report success. So the non-launchd path requires a
  # DOWN→UP transition: first confirm the port goes quiet (old gone), THEN
  # confirm it answers again (new up). launchd `kickstart -k` does the kill+start
  # atomically, but we still verify UP afterwards.
  local uid port i; uid="$(id -u)"; port="$(resolve_port)"
  if launchctl print "gui/$uid/ai.spectynmesh.serve" >/dev/null 2>&1; then
    launchctl kickstart -k "gui/$uid/ai.spectynmesh.serve" >/dev/null 2>&1
  else
    local bin="$HOME/.local/bin/spectyn"; [ -x "$bin" ] || bin="$ROOT/core/target/release/spectyn"
    pkill -f "spectyn serve" 2>/dev/null || true
    # Wait for the OLD serve to actually let go of the port (≤10s): healthz must
    # stop returning 200. If it never goes quiet, the kill failed — abort the
    # swap rather than launch into an occupied port and trust a stale 200.
    for i in $(seq 1 10); do
      [ "$(_healthz "$port")" != "200" ] && break
      sleep 1
    done
    if [ "$(_healthz "$port")" = "200" ]; then
      echo "self-update: WARNING — old serve still holding :$port after kill; not starting new (would bind-fail / read stale 200)" >&2
      return 1
    fi
    nohup "$bin" serve > "$STATE_DIR/serve.log" 2>&1 &
    disown 2>/dev/null || true
  fi
  # Now confirm the NEW serve is UP (≤15s). After the down→up transition this 200
  # can only be the new process.
  for i in $(seq 1 15); do
    [ "$(_healthz "$port")" = "200" ] && return 0
    sleep 1
  done
  echo "self-update: WARNING — serve did not answer healthz on :$port after restart" >&2
  return 1
}

BUILD_CMD="${SELF_UPDATE_BUILD_CMD:-default_build}"
INSTALL_CMD="${SELF_UPDATE_INSTALL_CMD:-default_install}"
RESTART_CMD="${SELF_UPDATE_RESTART_CMD:-default_restart}"

# ── guards ──────────────────────────────────────────────────────────────────
cur="$(git symbolic-ref -q --short HEAD || echo DETACHED)"
case "$cur" in
  dev/*|feat/*) echo "self-update: on work branch $cur (mid-task) — skipping"; exit 3;;
esac
if [ -n "$(git status --porcelain | grep -v '^??' || true)" ]; then
  echo "self-update: tracked changes present — skipping (won't clobber edits)"; exit 3
fi

# ── detect ──────────────────────────────────────────────────────────────────
git fetch origin "+refs/heads/$BASE:refs/remotes/origin/$BASE" 2>/dev/null \
  || { echo "self-update: cannot fetch origin/$BASE" >&2; exit 4; }
target="$(git rev-parse "origin/$BASE" 2>/dev/null)" || { echo "self-update: no origin/$BASE" >&2; exit 4; }
# The marker records the commit this node last BUILT. It is SEEDED at bootstrap
# (arm/dev-node writes it right after the first build) — that seeding is what
# makes self-update reliable. With no marker we fall back to HEAD, which is only
# correct if the running binary matches the checkout; a node whose checkout was
# moved without a build AND without a marker is the one unhandled edge (review:
# codex r5) — bootstrap must seed the marker to avoid it.
built="$(cat "$MARKER" 2>/dev/null || git rev-parse HEAD)"

if [ "$target" = "$built" ]; then
  echo "self-update: up-to-date ($(git rev-parse --short "$target"))"
  exit 0
fi
# Only update when origin/<base> is genuinely AHEAD of what we built — i.e.
# `built` is an ancestor of `target`. A plain inequality would treat a local
# checkout that is ahead of / diverged from origin as "behind" and DOWNGRADE it
# (review: agy r5). If built isn't an ancestor (diverged or ahead), do nothing.
if git rev-parse --verify -q "$built^{commit}" >/dev/null 2>&1 \
   && ! git merge-base --is-ancestor "$built" "$target" 2>/dev/null; then
  echo "self-update: built $(git rev-parse --short "$built" 2>/dev/null || echo "$built") is not an ancestor of origin/$BASE ($(git rev-parse --short "$target")) — diverged/ahead, NOT downgrading"
  exit 0
fi

echo "self-update: origin/$BASE at $(git rev-parse --short "$target") is ahead of built $(git rev-parse --short "$built" 2>/dev/null || echo "$built") — updating"

# Remember exactly where the tree was, so ANY failure restores it — the old
# binary is still serving from this commit's source, and the runner must never
# be left on a broken/partly-updated tree (review: codex+agy). Preserve the
# ORIGINAL ref form: restore to the branch if we were on one, else the commit —
# detaching a node that started on a branch would break the busy model (codex r3).
orig_ref="$(git symbolic-ref -q --short HEAD || git rev-parse HEAD)"
restore_tree() { git checkout -q "$orig_ref" 2>/dev/null || true; }
# CONTRACT: a successful update leaves HEAD DETACHED at origin/<base>. That is
# the explicit, intended end-state — it matches how nodes are armed (`git
# checkout --detach FETCH_HEAD`), the busy-guard only cares about dev/*/feat/*
# work branches, and backlog.sh always branches dev/<id> off origin/<base>
# regardless of attachment. We deliberately do NOT re-attach/fast-forward a
# branch on success: `git checkout -B <branch> <target>` would force-reset that
# branch and could silently discard unpushed commits on e.g. `main` (review:
# agy r5 vs codex r4 — the two requirements conflict, so the safe resolution is
# the explicit detached contract, not touching any branch).

# ── move source to the new base (single-branch-clone safe: detach to commit) ─
git checkout -q --detach "$target" 2>/dev/null || { echo "self-update: cannot checkout $target" >&2; exit 4; }

# ── build (single) ──────────────────────────────────────────────────────────
if ! $BUILD_CMD; then
  echo "self-update: BUILD FAILED at $(git rev-parse --short "$target") — restoring old tree, keeping old serve, will retry next tick"
  restore_tree                       # tree back to the commit the old binary serves
  exit 2                             # marker NOT advanced → retried next tick
fi

# ── install the built binary (only after a green build) ──────────────────────
if ! $INSTALL_CMD; then
  echo "self-update: install failed — restoring old tree, old serve still running, marker not advanced" >&2
  restore_tree
  exit 2
fi

# ── swap the running serve, THEN record the new built commit ─────────────────
# Advance the marker ONLY after the restart is verified healthy (review: agy r3).
# If we marked first and the restart failed, the next run would see "up-to-date"
# and never retry the restart → a permanently dead serve. With the marker held
# back, a failed restart leaves the node "still behind", so the next run rebuilds
# (a cargo no-op since the binary is already built) and retries the restart —
# self-healing.
if $RESTART_CMD; then
  printf '%s\n' "$target" > "$MARKER"
  echo "self-update: ✅ updated to $(git rev-parse --short "$target") — serve restarted and healthy (detached at origin/$BASE, per contract)"
  exit 1
fi
# Built+installed but serve didn't verify healthy. RESTORE the tree to where it
# was (review: agy r4): marker is held AND the tree goes back to the old ref, so
# the next run re-detects "behind" and retries the FULL update incl. restart.
# Without the restore, a fresh node (no marker) would fall back to built=HEAD —
# now pointing at $target — and falsely judge itself up-to-date, deadlocking a
# degraded serve forever. The binary on disk is the new one; the old serve (if
# still up) keeps answering until the retry swaps it. Distinct exit so the
# caller surfaces a degraded node instead of reporting a clean update.
restore_tree
echo "self-update: ⚠ updated binary to $(git rev-parse --short "$target") but serve restart UNVERIFIED — tree restored, will retry next run; needs attention" >&2
exit 5
