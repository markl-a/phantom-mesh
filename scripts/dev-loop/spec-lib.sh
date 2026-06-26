#!/usr/bin/env bash
# spec-lib.sh — shared, section-anchored parser for the [spec] envelope (sourced by
# both spec-gate.sh and deviation-handler.sh so there is ONE parser, no drift).
#
# Robust to: double OR single quotes, trailing `# comment` (even one containing quotes),
# and a same-named key in ANOTHER section (lookups are anchored to [spec] only).
# Limitation (fail-closed, by design): arrays must be single-line. A multi-line array
# yields 0 entries → spec-gate REJECTs it (you get a clear error, never a silent
# empty-scope that quietly blocks everything).

# spec_section <file> -> the body lines of the [spec] section (until the next [header]/EOF)
spec_section() {
  awk '
    /^[[:space:]]*\[/ { s=$0; sub(/^[[:space:]]*\[/,"",s); sub(/\].*/,"",s); insec=(s=="spec"); next }
    insec { print }
  ' "$1" 2>/dev/null
}

# spec_val <section-text> <key> -> scalar value (surrounding quotes + trailing comment stripped).
# Whether the value is quoted is decided by the FIRST non-space char after '=' — so a BARE
# value with a trailing comment that itself contains quotes (e.g. `max_files = 3  # "cap"`)
# is read as `3`, not as the comment text, and a quoted value may contain a literal '#'.
spec_val() {
  local line v; line="$(printf '%s\n' "$1" | grep -E "^[[:space:]]*$2[[:space:]]*=" | head -1)"
  [ -n "$line" ] || return 0
  v="${line#*=}"; v="${v#"${v%%[![:space:]]*}"}"          # value, left-trimmed
  case "$v" in
    \"*) v="${v#\"}"; v="${v%%\"*}";;                      # starts with " → double-quoted
    \'*) v="${v#\'}"; v="${v%%\'*}";;                      # starts with ' → single-quoted
    *)   v="${v%%#*}"; v="${v%"${v##*[![:space:]]}"}";;    # bare → drop comment, right-trim
  esac
  printf '%s' "$v"
}

# spec_list <section-text> <key> -> one entry per line (quotes stripped); only the content
# BETWEEN [ ] is scanned, so a trailing `# "comment"` after the ] is ignored.
spec_list() {
  local line content; line="$(printf '%s\n' "$1" | grep -E "^[[:space:]]*$2[[:space:]]*=" | head -1)"
  [ -n "$line" ] || return 0
  case "$line" in *\[*\]*) content="${line#*[}"; content="${content%]*}";; *) content="${line#*=}";; esac
  printf '%s' "$content" | grep -oE "\"[^\"]*\"|'[^']*'" | sed "s/^[\"']//; s/[\"']\$//"
}
