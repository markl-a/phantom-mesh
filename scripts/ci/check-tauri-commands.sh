#!/usr/bin/env bash
# SPEC-17 tauri-cmd-lint (T-WL-06 / G1, NG4).
#
# Enforces two compile-time-drift guards over the Tauri command surface:
#   1. REGISTRATION: every `#[tauri::command]` fn MUST appear in the central
#      `tauri::generate_handler![...]` list in app/src-tauri/src/lib.rs. An
#      unregistered command silently never reaches the frontend (a real drift
#      footgun); NG4 also forbids runtime registration, so the static list is
#      the single source of truth.
#   2. NAMING (G1): command names must be snake_case (lower-case, digits,
#      underscores) — no mixedCase/PascalCase — so the wire stays uniform.
#
# Cheap, dependency-free (pure bash + grep/awk), runs as an ubuntu ci-fast gate.
# Exit 0 = clean; exit 1 = violations (printed). Run from repo root or anywhere.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
src_dir="$repo_root/app/src-tauri/src"
lib_rs="$src_dir/lib.rs"

if [[ ! -f "$lib_rs" ]]; then
  echo "check-tauri-commands: $lib_rs not found" >&2
  exit 2
fi

# 1. Declared commands: the fn name on the first `fn NAME(` line at or after
#    each `#[tauri::command]` attribute (handles `pub fn` / `pub async fn`).
#    Portable awk (sub-based, no gawk-only 3-arg match); files via find (no
#    globstar dependency).
declared="$(find "$src_dir" -name '*.rs' -type f -exec cat {} + | awk '
  { gsub(/\r$/, "") }                                   # tolerate CRLF checkouts
  # Match the attribute ONLY when it starts a line (excludes prose that merely
  # mentions "#[tauri::command]" inside a // line), and accept the parenthesised
  # form `#[tauri::command(rename_all=...)]` as well as the bare one.
  # KNOWN LIMITATION (accepted): a `#[tauri::command]` written at column 0
  # INSIDE a /* ... */ block comment (a doc code-example) would be picked up.
  # That is vanishingly rare, and the worst case is a spurious "declared" name
  # → a false-FAIL (safe direction), never letting a real unregistered command
  # slip through. Full /* */ tracking was tried and rejected: Rust allows
  # nested block comments + inner `/*! */` doc blocks + header banners, which an
  # awk state machine over-swallows (it ate ~140 real commands). Not worth it.
  /^[[:space:]]*#\[tauri::command/ { want=1; next }
  want {
    # Rust grammar guarantees only further attributes, comments (///, //,
    # /* ... */ in any layout), and blank lines sit between an outer attribute
    # and the fn it annotates — never arbitrary code. So the command fn is
    # simply the NEXT fn-definition line; scan to it without trying to enumerate
    # (and reset on) every skippable line type. This handles all comment styles,
    # incl. multi-line block doc comments, for free. (Worst case — a
    # #[tauri::command] with no following fn — only occurs in code that does not
    # compile, where a spurious grab would at most false-FAIL, never false-pass.)
    if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?(pub\([a-z]+\)[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+[a-zA-Z0-9_]+/) {
      line=$0; sub(/.*fn[[:space:]]+/, "", line); sub(/[^a-zA-Z0-9_].*/, "", line);
      if (line != "") print line; want=0
    }
  }
' | sort -u)"

# 2. Registered commands: last `::` segment of each entry inside the
#    generate_handler![ ... ] block (strips path prefix + trailing comma).
registered="$(awk '
  { gsub(/\r$/, "") }
  /generate_handler!\[/ { inblock=1; next }
  inblock && /\]\)/ { inblock=0 }
  inblock { print }
' "$lib_rs" | sed -E 's://.*$::; s/.*:://; s/[, ]//g' | grep -E '^[a-zA-Z0-9_]+$' | sort -u)"

decl_n=$(printf '%s\n' "$declared" | grep -c . || true)
reg_n=$(printf '%s\n' "$registered" | grep -c . || true)
echo "check-tauri-commands: $decl_n declared #[tauri::command] fns, $reg_n registered in generate_handler!"

fail=0

# 1. Declared-but-not-registered → hard error, MINUS a documented baseline
#    allowlist (existing known-unregistered commands; see the .txt). New
#    unregistered commands still fail — that is the drift-gate.
allowlist_file="$repo_root/scripts/ci/tauri-cmd-unregistered-allowlist.txt"
allow="$( { [[ -f "$allowlist_file" ]] && grep -vE '^[[:space:]]*(#|$)' "$allowlist_file"; } | sort -u || true)"
unregistered="$(comm -23 <(printf '%s\n' "$declared") <(printf '%s\n' "$registered"))"
new_unregistered="$(comm -23 <(printf '%s\n' "$unregistered" | grep . | sort -u) <(printf '%s\n' "$allow"))"
allow_n=$(printf '%s\n' "$allow" | grep -c . || true)
if [[ -n "$new_unregistered" ]]; then
  echo "::error:: $(printf '%s\n' "$new_unregistered" | grep -c .) NEW command(s) are #[tauri::command] but NOT in generate_handler! (unreachable from frontend). Register them in lib.rs, or add to scripts/ci/tauri-cmd-unregistered-allowlist.txt with a reason:" >&2
  printf '   - %s\n' $new_unregistered >&2
  fail=1
else
  echo "check-tauri-commands: registration OK ($allow_n known-unregistered baselined in allowlist)."
fi
# Hygiene: an allowlisted name that is now registered (or no longer a command) is
# stale — warn so the baseline shrinks over time (non-fatal).
stale_allow="$(comm -12 <(printf '%s\n' "$allow") <(printf '%s\n' "$registered"))"
if [[ -n "$stale_allow" ]]; then
  echo "::warning:: allowlist entries now registered — remove from tauri-cmd-unregistered-allowlist.txt:" >&2
  printf '   - %s\n' $stale_allow >&2
fi

# 2. Naming (G1): snake_case only.
bad_names="$(printf '%s\n' "$declared" | grep -vE '^[a-z][a-z0-9_]*$' || true)"
if [[ -n "$bad_names" ]]; then
  echo "::error:: command name(s) violate G1 snake_case (lower_snake_case required):" >&2
  printf '   - %s\n' $bad_names >&2
  fail=1
fi

if [[ "$fail" -eq 0 ]]; then
  echo "check-tauri-commands: OK — all commands registered + snake_case."
fi
exit "$fail"
