#!/usr/bin/env bash
# seed-marker.sh — record which commit THIS node's spectyn binary was built from.
#
# self-update.sh (S7) decides "am I behind?" by comparing origin/<base> to a
# marker file = the commit the installed binary was built from. That marker is
# only knowable at BUILD time (the binary doesn't reliably self-report it:
# core_sha comes from a build-time SPECTYN_GIT_HASH that the default `cargo
# build` doesn't set). So call this RIGHT AFTER a successful build, while the
# checkout is still at the built commit — the arm / dev-node flow does this so a
# fresh node's first self-update is reliable (closes the no-marker edge: review
# codex r5).
#
# Usage: seed-marker.sh            # records HEAD as the built commit
#        seed-marker.sh <commit>   # records an explicit commit
# Idempotent; prints what it recorded.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# ${HOME:-} so a minimal env with HOME unset doesn't crash under `set -u`
# before we can give a clear error (review: agy).
STATE_DIR="${SPECTYN_STATE_DIR:-${HOME:-}/.spectyn-mesh}"
MARKER="${SPECTYN_BUILT_COMMIT:-$STATE_DIR/built-commit}"

cd "$ROOT" 2>/dev/null || { echo "seed-marker: not in a repo ($ROOT)" >&2; exit 1; }
sha="${1:-$(git rev-parse HEAD 2>/dev/null)}"
[ -n "$sha" ] || { echo "seed-marker: cannot resolve a commit" >&2; exit 1; }
git rev-parse --verify -q "${sha}^{commit}" >/dev/null 2>&1 \
  || { echo "seed-marker: '$sha' is not a commit in this repo" >&2; exit 1; }
# Peel to the COMMIT sha — `^{commit}` so an annotated tag resolves to the
# commit it points at, not the tag object's own sha (which would never match
# self-update's commit comparisons; review: agy r2).
sha="$(git rev-parse "${sha}^{commit}")"

# CHECK the write path end-to-end — a failed mkdir/write must NOT report success
# and leave the node unseeded (review: codex+agy). mkdir the MARKER's own parent
# (it can be overridden to live outside STATE_DIR), then verify the file landed.
mkdir -p "$(dirname "$MARKER")" \
  || { echo "seed-marker: cannot create $(dirname "$MARKER")" >&2; exit 1; }
printf '%s\n' "$sha" > "$MARKER" \
  || { echo "seed-marker: cannot write $MARKER" >&2; exit 1; }
[ "$(cat "$MARKER" 2>/dev/null)" = "$sha" ] \
  || { echo "seed-marker: marker readback mismatch at $MARKER" >&2; exit 1; }
echo "seed-marker: recorded built commit $(git rev-parse --short "$sha") → $MARKER"
