#!/usr/bin/env bash
# epic-acceptance-score.sh — count acceptance-criterion checkboxes across E001-E006
# (and optionally E007) epic specs, emit a Markdown table + total %, and exit
# non-zero if the total is below the freeze-week threshold.
#
# Spec:    docs/superpowers/features/F600-freeze-week-protocol-runbook.md
# Runbook: docs/superpowers/runbooks/E007-freeze-week.md
#
# POSIX bash (target: Linux, macOS bash 3.2, Git Bash on Windows).
# No external tools beyond awk, grep, sed (all POSIX).
#
# Env vars (the contract):
#   PHANTOM_E007_MIN_PERCENT   integer 0-100, default 80
#                              total must be >= this to exit 0 (SHIP gate)
#   PHANTOM_E007_SPECS_DIR     path, default docs/superpowers/specs/_current
#                              directory containing E00[1-6]-*.md files
#   PHANTOM_E007_INCLUDE_E007  "1" to include E007 in the total (default off —
#                              the gate is about E001-E006 per F600 spec)
#
# Exit codes:
#   0   total >= threshold → SHIP (S1)
#   1   total <  threshold → SLIP_TO=2026-06-17 (S2)
#   2   spec format drift (missing acceptance-criteria H2 — either
#       "## Acceptance criteria" or "## 驗收標準" — or no checkboxes
#       in some epic) — DO NOT cut on a broken scoreboard
#   64  usage error (bad flag, missing dir)
#
# Flags:
#   --help                Print this header and exit 0.
#   --strict              Fail with exit 2 if any E00[1-6]-*.md lacks an
#                         acceptance-criteria H2 ("## Acceptance criteria" or
#                         "## 驗收標準", per F600 risk-register).
#   --include-e007        Same as PHANTOM_E007_INCLUDE_E007=1.
#   --threshold N         Same as PHANTOM_E007_MIN_PERCENT=N.
#   --specs-dir PATH      Same as PHANTOM_E007_SPECS_DIR=PATH.
#
# Output: a Markdown table on stdout; diagnostics on stderr.

set -u

# ---------- defaults ----------
THRESHOLD="${PHANTOM_E007_MIN_PERCENT:-80}"
SPECS_DIR="${PHANTOM_E007_SPECS_DIR:-docs/superpowers/specs/_current}"
INCLUDE_E007="${PHANTOM_E007_INCLUDE_E007:-0}"
STRICT=0

# ---------- arg parsing ----------
print_help() {
    # Print the header comment block (lines 2..N starting with '#').
    awk '
        NR == 1 { next }
        /^#/    { sub(/^# ?/, ""); print; next }
        { exit }
    ' "$0"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h)
            print_help
            exit 0
            ;;
        --strict)
            STRICT=1
            shift
            ;;
        --include-e007)
            INCLUDE_E007=1
            shift
            ;;
        --threshold)
            shift
            if [ "$#" -eq 0 ]; then
                echo "error: --threshold requires a value" >&2
                exit 64
            fi
            THRESHOLD="$1"
            shift
            ;;
        --specs-dir)
            shift
            if [ "$#" -eq 0 ]; then
                echo "error: --specs-dir requires a value" >&2
                exit 64
            fi
            SPECS_DIR="$1"
            shift
            ;;
        *)
            echo "error: unknown argument: $1 (try --help)" >&2
            exit 64
            ;;
    esac
done

# ---------- validate inputs ----------
case "$THRESHOLD" in
    ''|*[!0-9]*)
        echo "error: threshold must be a non-negative integer, got: $THRESHOLD" >&2
        exit 64
        ;;
esac

if [ ! -d "$SPECS_DIR" ]; then
    echo "error: specs dir does not exist: $SPECS_DIR" >&2
    exit 64
fi

# ---------- which epics to score ----------
if [ "$INCLUDE_E007" = "1" ]; then
    EPIC_GLOB_PATTERN='E00[1-7]-*.md'
    GATE_SCOPE_LABEL="E001-E007"
else
    EPIC_GLOB_PATTERN='E00[1-6]-*.md'
    GATE_SCOPE_LABEL="E001-E006"
fi

# ---------- per-spec scorer (awk) ----------
# Reads file on stdin, prints "done total" on stdout.
# Logic: enter scoring mode at the acceptance-criteria H2 — either the English
# "^## Acceptance criteria" or the Chinese "^## 驗收標準" (canonical specs in
# docs/superpowers/specs/_current use the Chinese heading) — exit at next "^## ".
# Within scoring mode, count "- [x]" / "- [X]" as done, "- [ ]" as pending.
# The Chinese literal is matched as its UTF-8 byte sequence, which is
# locale-independent in POSIX awk (gawk, mawk, BSD awk).
score_one_spec() {
    awk '
        BEGIN { in_section = 0; done = 0; total = 0; have_section = 0 }
        /^## (Acceptance criteria|驗收標準)/ {
            in_section = 1
            have_section = 1
            next
        }
        /^## / {
            in_section = 0
            next
        }
        in_section && /^- \[[xX]\]/ {
            done++
            total++
            next
        }
        in_section && /^- \[ \]/ {
            total++
            next
        }
        END {
            # Exit 3 from awk = no Acceptance criteria H2 found
            # Exit 4 from awk = section exists but no checkboxes
            if (!have_section) { exit 3 }
            if (total == 0)    { exit 4 }
            print done, total
        }
    '
}

# ---------- collect epic files (sorted, deterministic) ----------
# Use a portable for-loop instead of mapfile (bash 3.2 on macOS lacks it).
EPIC_FILES=""
for f in "$SPECS_DIR"/$EPIC_GLOB_PATTERN; do
    [ -e "$f" ] || continue
    EPIC_FILES="$EPIC_FILES $f"
done

if [ -z "$EPIC_FILES" ]; then
    echo "error: no epic specs matched ${SPECS_DIR}/${EPIC_GLOB_PATTERN}" >&2
    exit 64
fi

# Sort for deterministic output.
EPIC_FILES_SORTED=$(printf '%s\n' $EPIC_FILES | sort)

# ---------- emit table ----------
TOTAL_DONE=0
TOTAL_TOTAL=0
DRIFT=0

printf '| Epic | Done | Total | %%   | Status |\n'
printf '|------|------|-------|-----|--------|\n'

for f in $EPIC_FILES_SORTED; do
    base=$(basename "$f")
    # Epic ID is the first 4 chars (E001, E002, …).
    epic_id=$(printf '%s' "$base" | cut -c1-4)

    result=$(score_one_spec < "$f")
    rc=$?

    if [ "$rc" -eq 3 ]; then
        DRIFT=1
        printf '| %s | -    | -     | -   | DRIFT  |\n' "$epic_id"
        echo "warning: $base has no acceptance-criteria H2 ('## Acceptance criteria' / '## 驗收標準') (strict-mode failure)" >&2
        continue
    fi
    if [ "$rc" -eq 4 ]; then
        DRIFT=1
        printf '| %s | 0    | 0     | -   | DRIFT  |\n' "$epic_id"
        echo "warning: $base has an acceptance-criteria H2 but no checkboxes" >&2
        continue
    fi
    if [ "$rc" -ne 0 ]; then
        echo "error: scoring $base failed with awk rc=$rc" >&2
        exit 2
    fi

    done=$(printf '%s' "$result" | awk '{print $1}')
    total=$(printf '%s' "$result" | awk '{print $2}')

    if [ "$total" -eq 0 ]; then
        pct=0
    else
        pct=$(( done * 100 / total ))
    fi

    if [ "$pct" -ge "$THRESHOLD" ]; then
        status="GREEN"
    elif [ "$pct" -gt 0 ]; then
        status="AMBER"
    else
        status="RED"
    fi

    printf '| %s | %-4d | %-5d | %-3d | %-6s |\n' "$epic_id" "$done" "$total" "$pct" "$status"

    TOTAL_DONE=$(( TOTAL_DONE + done ))
    TOTAL_TOTAL=$(( TOTAL_TOTAL + total ))
done

# ---------- total row ----------
if [ "$TOTAL_TOTAL" -eq 0 ]; then
    TOTAL_PCT=0
else
    TOTAL_PCT=$(( TOTAL_DONE * 100 / TOTAL_TOTAL ))
fi

if [ "$TOTAL_PCT" -ge "$THRESHOLD" ]; then
    TOTAL_STATUS="GREEN"
elif [ "$TOTAL_PCT" -gt 0 ]; then
    TOTAL_STATUS="AMBER"
else
    TOTAL_STATUS="RED"
fi

printf '|------|------|-------|-----|--------|\n'
printf '| TOTAL| %-4d | %-5d | %-3d | %-6s |\n' "$TOTAL_DONE" "$TOTAL_TOTAL" "$TOTAL_PCT" "$TOTAL_STATUS"

# ---------- gate output ----------
printf '\n'
printf 'Scope:     %s (set PHANTOM_E007_INCLUDE_E007=1 to include E007)\n' "$GATE_SCOPE_LABEL"
printf 'Threshold: %d%% (set via PHANTOM_E007_MIN_PERCENT or --threshold)\n' "$THRESHOLD"
printf 'Result:    TOTAL >= %d%% ? ' "$THRESHOLD"

if [ "$STRICT" -eq 1 ] && [ "$DRIFT" -eq 1 ]; then
    printf 'STRICT-FAIL (spec format drift)\n'
    printf 'Action:    Fix the spec(s) flagged above; re-run.\n'
    exit 2
fi

if [ "$TOTAL_PCT" -ge "$THRESHOLD" ]; then
    printf 'YES → SHIP (S1, tag v0.6.0)\n'
    printf 'Action:    Proceed with §6 tag-and-release in runbook.\n'
    exit 0
else
    printf 'NO  → SLIP_TO=2026-06-17 (S2)\n'
    printf 'Action:    Post §4.2 slip announcement; enter §4.3 first-24h-of-S2 plan.\n'
    exit 1
fi
