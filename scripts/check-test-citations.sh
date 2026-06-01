#!/usr/bin/env bash
# check-test-citations.sh — anti-fake-green guard (row-aware).
#
# The coverage sweep found docs/test-cases rows marked ✅ (passing) whose cited
# `cargo test` target does NOT exist. `cargo test <substring>` matches 0 tests
# and exits 0 → a green badge that verifies nothing.
#
# RULE: a row whose STATUS column (the last `|`-field) is ✅ MUST cite a test
# that resolves to real code. ⬜ / 🟡 rows may cite not-yet-written tests.
#
# Resolves: `--test <name>` → core/tests/<name>.rs ; `--lib a::b::<fn>` → fn ;
# `--lib <tok>` → module file (any depth) OR exact fn OR fn-substring.
#
# bash 3.2-safe; the row loop runs in the PARENT shell via `< <(...)` process
# substitution (NOT a pipe) so $FNS and $fail are real — an earlier piped-while
# version silently saw empty $FNS and false-flagged real tests.

set -uo pipefail
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 2

DOCS="docs/test-cases/mac.md"
[ -f docs/test-cases/ios.md ] && DOCS="$DOCS docs/test-cases/ios.md"

fail=0
report() { echo "  ✗ $1"; fail=1; }

# Every `fn <name>` in core, and every module .rs basename — written to temp
# FILES, not shell vars. Critical: the lookups grep a FILE (no pipe). A piped
# `printf "$FNS" | grep -q` under `set -o pipefail` returns 141 when grep
# early-exits on a match (SIGPIPE to printf), so has_fn would report FALSE on a
# real MATCH — itself a fake-green in the checker. Grepping a file avoids that.
FNFILE="$(mktemp)"; MODFILE="$(mktemp)"
trap 'rm -f "$FNFILE" "$MODFILE" "${ROWS:-}"' EXIT
grep -rhoE "fn [a-zA-Z0-9_]+" core/src core/tests 2>/dev/null | awk '{print $2}' | sort -u > "$FNFILE"
find core/src -name '*.rs' 2>/dev/null | sed -E 's#.*/##; s#\.rs$##' | sort -u > "$MODFILE"
has_fn()        { grep -qx "$1" "$FNFILE"  2>/dev/null; }
has_fn_substr() { grep -qF "$1" "$FNFILE"  2>/dev/null; }
has_mod()       { grep -qx "$1" "$MODFILE" 2>/dev/null; }

echo "▶ checking ✅-status test citations in: $DOCS"

# A row counts as a passing claim only when its STATUS column (NF-1) STARTS with
# ✅ — not merely contains it. A downgraded row like "⬜ (was ✅; …)" must NOT be
# treated as ✅ (that false-positive is itself a fake-green in the checker).
ROWS="$(mktemp)"
for d in $DOCS; do
  [ -f "$d" ] || continue
  grep -hE "^\|.*cargo test" "$d" 2>/dev/null \
    | awk -F'|' '{ s=$(NF-1); gsub(/^[ \t]+/,"",s); if (s ~ /^✅/) print }' >> "$ROWS"
done

while IFS= read -r row; do
  cite="$(printf '%s' "$row" | grep -oE "cargo test[^\`|]*")"
  [ -z "$cite" ] && continue
  id="$(printf '%s' "$row" | grep -oE 'MAC-[A-Z0-9-]+' | head -1)"

  t="$(printf '%s' "$cite" | grep -oE -- "--test [a-zA-Z0-9_]+" | awk '{print $2}')"
  if [ -n "$t" ]; then
    [ -f "core/tests/$t.rs" ] || report "$id: --test $t → core/tests/$t.rs missing"
    continue
  fi

  libarg="$(printf '%s' "$cite" | sed -nE 's/.*--lib +([a-zA-Z0-9_:]+).*/\1/p')"
  [ -z "$libarg" ] && continue
  case "$libarg" in
    *::*) fn="${libarg##*::}"
          [ "$fn" = "tests" ] && continue
          has_fn "$fn" || report "$id: cites ::$fn but no 'fn $fn' in core/" ;;
    *)    has_mod "$libarg" && continue        # module name at any depth
          has_fn "$libarg" && continue         # exact fn
          has_fn_substr "$libarg" && continue  # substring filter cargo would use
          report "$id: --lib $libarg → no module/fn/fn-substring in core/" ;;
  esac
done < <(cat "$ROWS")

if [ "$fail" -ne 0 ]; then
  echo "✗ fake-green: ✅ row(s) cite a test that does not exist."
  echo "  Fix → write the missing test, OR downgrade the row's 狀態 from ✅."
  exit 1
fi
echo "✓ every ✅ row cites a real test"
exit 0
