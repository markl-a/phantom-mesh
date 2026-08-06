#!/usr/bin/env bash
# spec-gate.sh — ACCEL-SPEC-GATE (autonomy governance 支柱 1: the spec envelope).
#
# AUTONOMY-GOVERNANCE.md §1 pillar 1: every self-igniting task MUST be bound to a
# spec, co-defined by human + AI in advance. NO SPEC -> DON'T DO IT. This is the
# Stage-1 shell form of the future native `spectyn dev spec validate` /
# `core/src/dev_loop/spec_gate.rs` (which the design doc lists as "behind the gate",
# i.e. a later native port — so we emulate it in shell now, exactly as M2/M3 were).
#
# A spec is a small TOML file. It MUST declare (else this gate REJECTS the task):
#   [spec]
#   capability  = one of: sense | learn | nudge | dispatch   (the 4 product capabilities
#                 ①看見生活+程式 / ②越用越懂 / ③提示進步 / ④替你做事; ①②③④/1-4 accepted)
#   component   = which MVP component (non-empty)
#   acceptance  = how we know this spec is done (non-empty)
#   scope_allow = files/dirs that MAY change (non-empty list)
#   # optional: scope_forbid (extra), max_files (soft cap, default 3), max_hours
#
# R2 forbidden zones (schema migration / CI / secret / external API) are ALWAYS
# forbidden regardless of scope_forbid — the deviation-handler enforces that.
#
# Usage:  spec-gate.sh validate <spec-file.toml>
# Exit:   0 = valid (task may enter the queue); 2 = invalid/incomplete (REJECT);
#         3 = usage / file-not-found.

set -uo pipefail

usage() { sed -n '2,30p' "$0"; }

[ "${1:-}" = "validate" ] || { echo "spec-gate: first arg must be 'validate'" >&2; usage >&2; exit 3; }
f="${2:-}"
[ -n "$f" ] && [ -f "$f" ] || { echo "spec-gate: spec file not found: '${f:-<none>}'" >&2; exit 3; }

# Shared, section-anchored [spec] parser (one parser for spec-gate + deviation-handler,
# robust to single/double quotes, trailing comments, and same-named keys in other sections).
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/spec-lib.sh"
SECTION="$(spec_section "$f")"

cap_raw="$(spec_val "$SECTION" capability)"
component="$(spec_val "$SECTION" component)"
acceptance="$(spec_val "$SECTION" acceptance)"
allow_n="$(spec_list "$SECTION" scope_allow | grep -c . || true)"
max_files="$(spec_val "$SECTION" max_files)"; max_files="${max_files:-3}"

# Normalise capability to one of the 4 canonical slugs.
cap=""
case "$(printf '%s' "$cap_raw" | tr '[:upper:]' '[:lower:]')" in
  sense|see|life|①|1)            cap="sense";;
  learn|understand|skill|②|2)    cap="learn";;
  nudge|remind|③|3)              cap="nudge";;
  dispatch|do|act|④|4)           cap="dispatch";;
  *) cap="";;
esac

errs=0
note() { echo "  ✗ $1"; errs=$((errs+1)); }
echo "=== spec-gate: $f ==="
[ -n "$cap" ]        || note "capability missing/unknown ('${cap_raw:-<empty>}') — must be one of: sense|learn|nudge|dispatch (①②③④/1-4)"
[ -n "$component" ]  || note "component missing — declare which MVP component this serves"
[ -n "$acceptance" ] || note "acceptance missing — declare how we know this spec is done"
[ "${allow_n:-0}" -ge 1 ] || note "scope_allow missing/empty — declare which files/dirs may change (bounded scope)"
# soft sanity on the cap
case "$max_files" in (*[!0-9]*|'') note "max_files must be an integer (got '${max_files}')";; esac

if [ "$errs" -gt 0 ]; then
  echo "  RESULT: ❌ REJECT — $errs missing/invalid field(s). No spec (or incomplete spec) → don't do the task (governance 支柱1)."
  exit 2
fi
echo "  capability : $cap"
echo "  component  : $component"
echo "  acceptance : $acceptance"
echo "  scope_allow: $allow_n path(s);  max_files: $max_files"
echo "  RESULT: ✅ VALID — spec envelope complete; task may enter the bounded queue."
exit 0
