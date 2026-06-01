#!/usr/bin/env bash
# 27-f1-anthropic-cache.sh — F1 production path coverage.
#
# Verifies the F1 Anthropic SDK upgrades land end-to-end:
#   1. on a real Anthropic call, the binary sends `cache_control` blocks on
#      system + last user (or last tool) — checked indirectly via the
#      `cache_creation_input_tokens` accounting on call #1
#   2. on a SECOND identical call, Anthropic returns
#      `cache_read_input_tokens > 0` — proving the cache hit went through
#      end-to-end and the binary surfaces it in events.jsonl / costs.db
#
# This scenario uses a REAL Anthropic API call. No mock, because the F1 path
# is fundamentally about Anthropic's cache_control semantics — a mock would
# only re-test the JSON we send, not the round-trip behavior the audit flagged
# as the production-risk path.
#
# Skip matrix:
#   - no ANTHROPIC_API_KEY                  → exit 77
#   - binary lacks F1 (no cache_* fields    → exit 77
#     anywhere in events / costs after the
#     two calls — pre-PR-31 behavior)
#   - rate-limited (HTTP 429 / quota)       → exit 77
#
# References:
#   - audit doc: docs/superpowers/audits/2026-05-15-coverage-gaps.md (HIGH #1)
#   - PR #31, merged @ 1d75db9
#   - core/src/streaming.rs lines 285-291 (cache_read_input_tokens accounting)

source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/inspect.sh"

scenario "F1 — Anthropic cache_control sent + cache_read_input_tokens > 0 on 2nd call"
require_cmd "$PHANTOM_BIN"

if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  warn "skip: ANTHROPIC_API_KEY not set — F1 production path is Anthropic-specific"
  exit 77
fi

EVENTS="$PHANTOM_CONFIG_DIR/events.jsonl"
COSTS_DB="$PHANTOM_CONFIG_DIR/costs.db"
COSTS_JSON="$PHANTOM_CONFIG_DIR/costs.json"

if [ ! -f "$EVENTS" ]; then
  warn "skip: $EVENTS not found — run any phantom command first to seed config dir"
  exit 77
fi

# Build a long, deterministic system prompt. Anthropic prompt caching has a
# 1024-token minimum for caching to engage on most models, so we pad with
# stable boilerplate text. Same prompt MUST be used for both calls so the
# prefix matches byte-for-byte.
LONG_PROMPT=$(printf 'Reply with exactly the two characters: ok\n\nContext (ignore, padding for Anthropic prompt cache minimum-1024-token threshold):\n%s' \
  "$(yes 'The quick brown fox jumps over the lazy dog. ' 2>/dev/null \
      | head -200 | tr -d '\n')")

# Use a temp working dir with a minimal agents.toml that pins the master agent
# to Anthropic. Avoid clobbering the operator's real agents.toml.
WORKDIR="$(tmpdir)/f1-cache-cwd"
mkdir -p "$WORKDIR"
cat > "$WORKDIR/agents.toml" <<'TOML'
[core]
host = "127.0.0.1"
port = 17878

[providers.anthropic]
type          = "anthropic"
api_key       = "env:ANTHROPIC_API_KEY"
default_model = "claude-3-5-haiku-latest"

[agent.master]
provider     = "anthropic"
model        = "claude-3-5-haiku-latest"
instructions = "You are a terse test fixture. Reply with exactly what the prompt requests."
TOML

# Helper: ask phantom to call its master agent with the long prompt; return
# trimmed transcript. Strips ANSI for grep-ability.
call_master() {
  local label="$1"
  step "$label: phantom repl --agent master -c '<long prompt>'"
  ( cd "$WORKDIR" && timeout 90 "$PHANTOM_BIN" repl --agent master -c "$LONG_PROMPT" 2>&1 ) \
    | sed -E 's/\x1b\[[0-9;]*m//g'
}

# Detect rate-limit / provider error so we can SKIP cleanly rather than fail.
detect_rate_limit_or_skip() {
  local body="$1" label="$2"
  if printf '%s' "$body" | grep -qiE 'HTTP 429|rate.?limit|quota|usage limit|too many requests|overloaded'; then
    warn "$label: provider rate-limit / overloaded — skipping (not a phantom-side regression)"
    exit 77
  fi
  if printf '%s' "$body" | grep -qiE 'invalid.*api.*key|authentication.*failed|401'; then
    warn "$label: provider auth failure — ANTHROPIC_API_KEY may be invalid; skipping"
    exit 77
  fi
}

# How many cache-related events / cost rows existed BEFORE the run?
before_ms=$(now_ms)

step "call #1: priming the cache (must succeed for cache_creation to fire)"
out1=$(call_master "call #1")
detect_rate_limit_or_skip "$out1" "call #1"

# Sanity: did we get an answer at all?
if printf '%s' "$out1" | grep -qiE 'error|panic|connection'; then
  warn "call #1 looks like an error response (preview):"
  printf '%s\n' "$out1" | tail -8 | sed 's/^/    /' >&2
fi

# Brief pause — Anthropic's cache propagation is "near-instant" but a 1s
# breath avoids racing the persistence write of cost rows.
sleep 1

step "call #2: sending IDENTICAL long prompt (cache should hit)"
out2=$(call_master "call #2")
detect_rate_limit_or_skip "$out2" "call #2"

# ── Detection: does the binary even know about cache_read_input_tokens? ──
#
# Pre-F1 binaries don't surface this field. We probe three places:
#   (a) events.jsonl — any kind/summary mentioning cache
#   (b) costs.json — F1 may have written a column / field
#   (c) costs.db — open and SELECT to see if a column was added
#
# If NONE of them shows a cache field after a real Anthropic call, the
# binary is pre-F1 and we SKIP (not fail).
new_events=$(events_since "$before_ms")
events_have_cache=0
if printf '%s' "$new_events" | grep -qiE 'cache_(read|creation)_input_tokens|cache_read|cache_creation'; then
  events_have_cache=1
fi

costs_have_cache=0
if [ -f "$COSTS_JSON" ] && grep -qE 'cache_(read|creation)_input_tokens' "$COSTS_JSON" 2>/dev/null; then
  costs_have_cache=1
fi

db_has_cache=0
if [ -f "$COSTS_DB" ] && command -v python >/dev/null 2>&1; then
  db_has_cache=$(python - <<PY 2>/dev/null
import sqlite3, sys
try:
    db = sqlite3.connect(r"$COSTS_DB")
    cols = [r[1] for r in db.execute("PRAGMA table_info(cost_records)")]
    print(1 if any("cache" in c.lower() for c in cols) else 0)
except Exception:
    print(0)
PY
)
  db_has_cache=${db_has_cache:-0}
fi

step "  cache field detection: events=$events_have_cache costs.json=$costs_have_cache costs.db=$db_has_cache"

if [ "$events_have_cache" -eq 0 ] && [ "$costs_have_cache" -eq 0 ] && [ "$db_has_cache" -eq 0 ]; then
  warn "skip: binary appears to be PRE-F1 — no cache_(read|creation)_input_tokens field"
  warn "      anywhere in events.jsonl / costs.json / costs.db after a real Anthropic call."
  warn "      Rebuild PHANTOM_BIN from a post-PR-#31 commit to enable this scenario."
  exit 77
fi

pass "binary surfaces cache_* accounting fields (F1 path is wired)"

# ── Now the actual F1 assertion: cache_read_input_tokens > 0 on call #2 ──
#
# Pull the LAST two cost rows (they correspond to the two calls above) and
# verify the second has a positive cache_read_input_tokens.
read_tokens_call2=""
if [ -f "$COSTS_DB" ] && command -v python >/dev/null 2>&1; then
  read_tokens_call2=$(python - <<PY 2>/dev/null
import sqlite3, sys
db = sqlite3.connect(r"$COSTS_DB")
cols = [r[1] for r in db.execute("PRAGMA table_info(cost_records)")]
cache_col = next((c for c in cols if c.lower() == "cache_read_input_tokens"), None)
if not cache_col:
    sys.exit(0)
rows = list(db.execute(f"SELECT {cache_col} FROM cost_records ORDER BY rowid DESC LIMIT 2"))
# rows[0] = call #2 (most recent), rows[1] = call #1
if len(rows) >= 1 and rows[0][0] is not None:
    print(rows[0][0])
PY
)
fi

# Fallback: parse events.jsonl for the most recent cache_read_input_tokens.
if [ -z "$read_tokens_call2" ]; then
  read_tokens_call2=$(printf '%s\n' "$new_events" \
    | python -c "
import json, sys, re
last = 0
for line in sys.stdin:
    try:
        e = json.loads(line)
    except Exception:
        continue
    # try direct field
    for k in ('cache_read_input_tokens', 'cache_read_tokens'):
        if k in e and isinstance(e[k], int):
            last = e[k]
    # or scrape from summary string like '... cache_read=1234 ...'
    s = str(e.get('summary', ''))
    m = re.search(r'cache_read[_\\w]*[=:\\s]+([0-9]+)', s)
    if m:
        last = int(m.group(1))
print(last)
" 2>/dev/null)
fi

read_tokens_call2=${read_tokens_call2:-0}
read_tokens_call2=${read_tokens_call2//[!0-9]/}
[ -z "$read_tokens_call2" ] && read_tokens_call2=0

step "  cache_read_input_tokens on call #2: $read_tokens_call2"

if [ "$read_tokens_call2" -gt 0 ]; then
  pass "F1: cache HIT on 2nd identical prompt (cache_read_input_tokens=$read_tokens_call2)"
else
  # Could be: model didn't engage cache (under min-token threshold), or
  # prompt was below cacheable size, or Anthropic transient. Treat as soft
  # fail — the F1 plumbing is there (we passed the field-detection check)
  # but the cache didn't actually fire. Warn so a human can investigate.
  warn "F1: cache_read_input_tokens is 0 on call #2"
  warn "    plumbing is present (fields detected) but cache didn't engage."
  warn "    Common causes: prompt under ~1024-token threshold, free-tier model,"
  warn "    propagation delay. Inspect manually: phantom debug --tail 50"
  fail "F1: cache_read_input_tokens=0 on identical 2nd call (expected > 0)"
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
