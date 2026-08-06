#!/usr/bin/env bash
# deviation-handler.sh — autonomy governance 支柱 2 (detect → contain → normalize → notify).
#
# AUTONOMY-GOVERNANCE.md §1 pillar 2 + R1–R5 (owner-signed 2026-06-08). After the
# loop attempts a spec-bound work item, this evaluates the result and decides what
# happens — implementing the LOCKED rules exactly:
#
#   R1 deviation = ANY of: (i) change outside the spec's scope_allow;
#       (ii) dev_verify red (--verify-exit != 0); (iii) >=1 reviewer REQUEST_CHANGES
#       (--review-exit != 0); (iv) over the bounded cap (> max_files, or an R2 zone);
#       (v) same task failed >=2 times in a row.
#   R2 contain (hard-prohibited, structural): this handler NEVER merges, deletes,
#       force-pushes, touches CI/secret/schema, or runs irreversible shell — it only
#       READS the diff and WRITES a ledger / proposal / notification. A diff that
#       itself touches an R2 zone or deletes files = CONTAINED immediately (no retry).
#       It also refuses to operate on main/master ("絕不碰主線").
#   R3 normalize = auto-correct <= MAX_ROUNDS (default 2); still failing -> downgrade
#       to a `needs-human` proposal (diff + reason, NOT merged, isolated) + notify.
#   R4 stuck / repeated fail -> STOP + notify owner (no skill-synthesis to force-patch).
#   R5 review consensus is consumed via --review-exit (review.sh: 0 APPROVE / else block).
#
# Pollution wall (支柱 3): writes ONLY to the dev-loop ledger, NEVER to partner-signals
# (the moat ledger). This script contains zero references to partner-signals by design.
#
# Usage:
#   deviation-handler.sh --spec <file> [--range R | --staged] [--verify-exit N]
#                        [--review-exit N] [--max-rounds N]
#   deviation-handler.sh --spec <file> --reset      # clear this branch's retry counter
#
# Exit: 0 PASS (land it) · 10 RETRY (normalize, under cap) · 20 ESCALATE (needs-human)
#       · 30 CONTAINED (R2 forbidden/destructive, hard stop) · 3 setup error.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "${HERE}/spec-lib.sh"   # shared, section-anchored [spec] parser (no drift with spec-gate)
STATE_DIR="${SPECTYN_STATE_DIR:-${HOME}/.spectyn-mesh}"
LEDGER="${STATE_DIR}/dev-loop-log.jsonl"             # audit trail (NOT partner-signals)
PROPOSALS="${STATE_DIR}/deviation-proposals.jsonl"   # needs-human escalations
NOTIFY_LOG="${STATE_DIR}/notifications.log"
MAX_ROUNDS="${DEVIATION_MAX_ROUNDS:-2}"

SPEC=""; RANGE="HEAD~1..HEAD"; VERIFY_EXIT=""; REVIEW_EXIT=""; DO_RESET=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --spec)        SPEC="${2:-}"; shift;;
    --range)       RANGE="${2:-}"; shift;;
    --staged)      RANGE="--staged";;
    --verify-exit) VERIFY_EXIT="${2:-}"; shift;;
    --review-exit) REVIEW_EXIT="${2:-}"; shift;;
    --max-rounds)  MAX_ROUNDS="${2:-}"; shift;;
    --reset)       DO_RESET=1;;
    -h|--help)     sed -n '2,34p' "$0"; exit 0;;
    *) echo "deviation: unknown arg '$1'" >&2; exit 3;;
  esac
  shift
done

mkdir -p "$STATE_DIR"
branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)"
bkey="$(printf '%s' "$branch" | tr '/ ' '__')"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo unknown)"
# The retry counter (R1-v "same TASK fails >=2x") is keyed by (branch, spec) so two
# UNRELATED deviations on the same branch don't escalate each other. A task = a spec.
# Key on the FULL (absolutised) spec path so specs that merely share a basename
# (a/spec.toml vs b/spec.toml) get distinct counters.
speckey() { local p="${1:-task}"; case "$p" in /*) :;; *) p="$PWD/$p";; esac; printf '%s' "$p" | tr '/ .:' '____'; }

if [ "$DO_RESET" = 1 ]; then
  if [ -n "$SPEC" ]; then
    rm -f "${STATE_DIR}/deviation-rounds-${bkey}-$(speckey "$SPEC").count"
    echo "deviation: retry counter reset for task '$(basename "$SPEC")' on '$branch'"
  else
    rm -f "${STATE_DIR}/deviation-rounds-${bkey}-"*.count 2>/dev/null
    echo "deviation: retry counters reset for all tasks on '$branch'"
  fi
  exit 0
fi

# ── R2: never operate on main/master ─────────────────────────────────────────
case "$branch" in
  main|master) echo "deviation: refusing to operate on '$branch' — work must be on a dev branch (R2 絕不碰主線)." >&2; exit 3;;
esac

# ── spec must exist + be valid (no spec → don't do; 支柱1) ────────────────────
[ -n "$SPEC" ] && [ -f "$SPEC" ] || { echo "deviation: --spec <file> required and must exist (got '${SPEC:-<none>}')." >&2; exit 3; }
if ! "${HERE}/spec-gate.sh" validate "$SPEC" >/dev/null 2>&1; then
  echo "deviation: spec '$SPEC' fails spec-gate — cannot evaluate an unbounded task (支柱1)." >&2; exit 3
fi
counter="${STATE_DIR}/deviation-rounds-${bkey}-$(speckey "$SPEC").count"

# R1-ii/R1-iii/R5 must actually be evaluated. Both signals are REQUIRED — without them an
# in-scope diff would fall through to PASS and silently bypass dev_verify + review (R5).
[ -n "$VERIFY_EXIT" ] && [ -n "$REVIEW_EXIT" ] || {
  echo "deviation: --verify-exit and --review-exit are required (pass dev_verify's and review.sh's real exit codes; omitting them would bypass R1-ii/R1-iii/R5)." >&2; exit 3; }

json_escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
json_arr() { local out="" x; for x in "$@"; do out="${out:+$out,}\"$(json_escape "$x")\""; done; printf '[%s]' "$out"; }

# ── parse spec via the shared parser: scope_allow globs + max_files ───────────
SPEC_SECTION="$(spec_section "$SPEC")"
ALLOW=()
while IFS= read -r q; do [ -n "$q" ] && ALLOW+=("$q"); done < <(spec_list "$SPEC_SECTION" scope_allow)
max_files="$(spec_val "$SPEC_SECTION" max_files)"; max_files="${max_files:-3}"
case "$max_files" in *[!0-9]*|'') max_files=3;; esac

in_scope() {
  local file="$1" a
  for a in "${ALLOW[@]:-}"; do
    [ -n "$a" ] || continue
    case "$a" in
      */) [ "${file#"$a"}" != "$file" ] && return 0;;            # directory prefix
      *)  [ "$file" = "$a" ] && return 0; case "$file" in $a) return 0;; esac;;
    esac
  done
  return 1
}
is_forbidden_path() {   # R2 zones: CI / key-material / secret-store / schema-migration.
  # Narrow, file-shaped patterns — NOT substrings — so benign source like
  # `SecretQuestion.tsx` or `credentialService.ts` is NOT flagged.
  case "$1" in
    .github/*|*/.github/*|.circleci/*|*/.circleci/*|Jenkinsfile|*/Jenkinsfile|.gitlab-ci.yml|*/.gitlab-ci.yml) return 0;;  # CI
    .env|*/.env|*.env.*|*.key|*.pem|*.p8|*.pfx|*.jks|*.keystore) return 0;;                                                # key material
    secrets/*|*/secrets/*|.secrets/*|*/.secrets/*|*.secret|secrets.*|*.secrets.yml|*.secrets.yaml|*.secrets.json|*credentials.json|credentials/*|*/credentials/*) return 0;;  # secret store (incl. repo-root dirs)
    migrations/*|*/migrations/*|migration/*|*/migration/*|*.up.sql|*.down.sql) return 0;;                                  # schema migration (incl. repo-root)
    *) return 1;;
  esac
}

# ── gather the change (name-status). Rename = `R### old new`, copy = `C### old new`,
#    delete = `D path`. An invalid range must NOT silently look "clean". ───────────
# core.quotePath=false → non-ASCII / spaced paths come back literal (not "\\343..."), so
# in_scope/forbidden matching compares the real path, not a quoted form.
if [ "$RANGE" = "--staged" ]; then status="$(git -c core.quotePath=false diff --staged --name-status 2>/dev/null)"; gerc=$?; rlabel="(staged)"
else status="$(git -c core.quotePath=false diff --name-status "$RANGE" 2>/dev/null)"; gerc=$?; rlabel="$RANGE"; fi
[ "$gerc" -eq 0 ] || { echo "deviation: invalid/ambiguous git range '$RANGE' (git exit $gerc) — cannot evaluate, not PASS." >&2; exit 3; }

REASONS=(); FORBIDDEN=(); NFILES=0; out_of_scope=0
while IFS=$'\t' read -r st p1 p2; do
  [ -n "${st:-}" ] || continue
  case "$st" in
    R*)  # rename: p1=old (source), p2=new (dest). NOT auto-contained — a benign in-scope
         # rename (src/a -> src/b) is a normal change. But the bypass is closed three ways:
         # moving OUT of a protected zone is tampering (zone-src), the dest must be in-scope,
         # and a forbidden dest is contained.
         [ -n "${p1:-}" ] || continue; NFILES=$((NFILES+1))
         is_forbidden_path "$p1" && FORBIDDEN+=("zone-src:$p1")
         if [ -n "${p2:-}" ]; then is_forbidden_path "$p2" && FORBIDDEN+=("zone:$p2"); in_scope "$p2" || out_of_scope=$((out_of_scope+1)); fi ;;
    C*)  # copy: source kept, new dest created. Check BOTH — copying a secret/key OUT of a
         # forbidden zone into an allowed path is exfiltration (R2), and the dest must be in scope.
         [ -n "${p2:-}" ] || { [ -n "${p1:-}" ] && { NFILES=$((NFILES+1)); is_forbidden_path "$p1" && FORBIDDEN+=("zone-src:$p1"); in_scope "$p1" || out_of_scope=$((out_of_scope+1)); }; continue; }
         NFILES=$((NFILES+1)); is_forbidden_path "$p1" && FORBIDDEN+=("zone-src:$p1")
         is_forbidden_path "$p2" && FORBIDDEN+=("zone:$p2"); in_scope "$p2" || out_of_scope=$((out_of_scope+1)) ;;
    D*)  [ -n "${p1:-}" ] || continue; NFILES=$((NFILES+1)); FORBIDDEN+=("delete:$p1") ;;
    *)   [ -n "${p1:-}" ] || continue; NFILES=$((NFILES+1))
         is_forbidden_path "$p1" && FORBIDDEN+=("zone:$p1"); in_scope "$p1" || out_of_scope=$((out_of_scope+1)) ;;
  esac
done <<EOF
$status
EOF

# An empty diff means there is nothing to evaluate — do NOT report PASS (that would
# green-light unevaluated work). The loop must hand us a real change.
[ "$NFILES" -gt 0 ] || { echo "deviation: empty diff for '${rlabel}' — nothing to evaluate (not PASS)." >&2; exit 3; }

# ── R1 classification ────────────────────────────────────────────────────────
hard=0
[ "${#FORBIDDEN[@]}" -gt 0 ] && hard=1                                    # R2 zone / deletion
[ "$out_of_scope" -gt 0 ] && REASONS+=("scope-exceeded:${out_of_scope} file(s) outside scope_allow (R1-i)")
[ "$NFILES" -gt "$max_files" ] && REASONS+=("file-cap:${NFILES} > max_files ${max_files} (R1-iv)")
if [ -n "$VERIFY_EXIT" ] && [ "$VERIFY_EXIT" != "0" ]; then REASONS+=("dev_verify-red:exit ${VERIFY_EXIT} (R1-ii)"); fi
if [ -n "$REVIEW_EXIT" ] && [ "$REVIEW_EXIT" != "0" ]; then REASONS+=("review-not-approved:exit ${REVIEW_EXIT} (R1-iii/R5)"); fi
[ "${#FORBIDDEN[@]}" -gt 0 ] && REASONS+=("forbidden-zone/destructive: ${FORBIDDEN[*]} (R2)")

ndev="${#REASONS[@]}"
echo "=== deviation-handler: ${rlabel} on '${branch}' ==="
echo "  spec       : $SPEC"
echo "  changed    : ${NFILES} file(s) (cap ${max_files});  out-of-scope: ${out_of_scope};  forbidden/destructive: ${#FORBIDDEN[@]}"
echo "  verify-exit: ${VERIFY_EXIT:-<n/a>}   review-exit: ${REVIEW_EXIT:-<n/a>}"

reasons_json="[]"; [ "$ndev" -gt 0 ] && reasons_json="$(json_arr "${REASONS[@]}")"
forbidden_json="[]"; [ "${#FORBIDDEN[@]}" -gt 0 ] && forbidden_json="$(json_arr "${FORBIDDEN[@]}")"
ledger() {  # outcome round
  printf '{"ts":"%s","branch":"%s","spec":"%s","outcome":"%s","round":%s,"files_changed":%s,"reasons":%s,"forbidden":%s}\n' \
    "$TS" "$(json_escape "$branch")" "$(json_escape "$SPEC")" "$1" "$2" "$NFILES" "$reasons_json" "$forbidden_json" >> "$LEDGER"
}
escalate_proposal() {  # outcome round
  local diffstat; diffstat="$(git -c core.quotePath=false diff --stat "$( [ "$RANGE" = "--staged" ] && echo --staged || echo "$RANGE" )" 2>/dev/null | tail -1 | sed 's/^[[:space:]]*//')"
  printf '{"ts":"%s","branch":"%s","spec":"%s","outcome":"%s","round":%s,"needs_human":true,"reasons":%s,"forbidden":%s,"diffstat":"%s"}\n' \
    "$TS" "$(json_escape "$branch")" "$(json_escape "$SPEC")" "$1" "$2" "$reasons_json" "$forbidden_json" "$(json_escape "${diffstat:-}")" >> "$PROPOSALS"
  local msg="[$TS] $1 branch=${branch} reasons=[${REASONS[*]:-}] → needs-human proposal in $(basename "$PROPOSALS"). NOT merged, isolated."
  printf '%s\n' "$msg" >> "$NOTIFY_LOG"
  echo
  echo "  🔔 OWNER NOTIFIED (R3/R4): $msg"
}

# ── decide outcome ───────────────────────────────────────────────────────────
# CLEAN: no deviations at all → conforms + verified + approved → land it.
if [ "$ndev" -eq 0 ]; then
  rm -f "$counter"; ledger "pass" 0
  echo "  RESULT: ✅ PASS — conforms to spec, verified, approved. Safe to land (on branch, --no-ff)."
  exit 0
fi

# HARD CONTAIN (R2): a forbidden-zone touch or a deletion is never auto-retried —
# contain immediately, escalate to human, notify. (无害化 first, then 交還給人.)
if [ "$hard" -eq 1 ]; then
  ledger "contained" 0; escalate_proposal "contained" 0
  echo "  RESULT: 🛑 CONTAINED (R2) — diff touches a forbidden zone / deletes files."
  echo "          No merge, no destructive action taken; downgraded to needs-human. Do NOT land."
  exit 30
fi

# RETRYABLE (R3 normalize): scope-exceed / file-cap / verify-red / review-changes.
round=$(( $( [ -f "$counter" ] && cat "$counter" 2>/dev/null || echo 0 ) + 1 ))
printf '%s' "$round" > "$counter"
echo "  reasons    : ${REASONS[*]}"
if [ "$round" -ge "$MAX_ROUNDS" ]; then
  ledger "escalate" "$round"; escalate_proposal "escalate" "$round"
  echo "  RESULT: 🚩 ESCALATE needs-human — ${round}/${MAX_ROUNDS} consecutive deviations (R1-v + R4 stuck→halt)."
  echo "          Auto-correct exhausted; downgraded to proposal, isolated, owner notified. Do NOT land."
  exit 20
else
  ledger "retry" "$round"
  echo "  RESULT: ⚠️ RETRY (round ${round}/${MAX_ROUNDS}) — normalize: auto-correct toward spec and re-run (R3). Do NOT land yet."
  exit 10
fi
