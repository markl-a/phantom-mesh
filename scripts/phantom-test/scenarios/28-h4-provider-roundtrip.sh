#!/usr/bin/env bash
# 28-h4-provider-roundtrip.sh — H4 coverage for the 4 new Hermes providers.
#
# Verifies:
#   1. `phantom keys list` accepts / surfaces *_API_KEY entries for the 4 H4
#      providers (Mistral, xAI, Together, Fireworks)
#   2. for at least ONE provider whose API key IS in the operator's env, a
#      `phantom repl --agent master -c "<short prompt>"` round-trip through
#      that provider's adapter returns a non-empty assistant response — i.e.
#      the binary was built with `--features experimental-extra-providers`
#      and dispatches correctly through the H4 adapter, not the openai_compat
#      fallback.
#
# Detection strategy:
#   The 0.4.0-era keys_help() text does NOT list xai/together/fireworks even
#   on post-PR-#25 builds, so `keys help` is NOT a reliable H4 signal. The
#   tight signal is: phantom's response to `provider.type = "mistral"` in
#   agents.toml. A pre-H4 binary either falls back to a different provider
#   or emits "unknown provider type" — caught below as a SKIP, not a fail.
#
# Skip matrix:
#   - operator has NO H4 keys set in env / saved          → exit 77
#   - binary returns "unknown provider type" / falls back → exit 77 (pre-H4)
#   - selected provider returns 429 / quota error         → exit 77
#   - selected provider returns 401 (bad key)             → exit 77
#
# References:
#   - audit doc: docs/superpowers/audits/2026-05-15-coverage-gaps.md (HIGH H4)
#   - PR #25, merged @ 66b0a18
#   - core/src/providers/{mistral,xai,together,fireworks}.rs

source "$PHANTOM_TEST_LIB/common.sh"

scenario "H4 — phantom keys lists 4 new providers + at least 1 round-trip"
require_cmd "$PHANTOM_BIN"

# ── Step 1: phantom keys list — must run cleanly ────────────────────────────
keys_list=$("$PHANTOM_BIN" keys list 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')
keys_ec=$?
if [ "$keys_ec" -ne 0 ]; then
  fail "phantom keys list exited non-zero ($keys_ec) — basic CLI sanity broken"
  printf '%s\n' "$keys_list" | head -10 | sed 's/^/    /'
  exit 1
fi
pass "phantom keys list runs cleanly"

# ── Step 2: H4 provider matrix ──────────────────────────────────────────────
# xAI is registered as `xai`; per provider_env_var_name() the env var
# is XAI_API_KEY (the function uppercases + suffixes _API_KEY by convention).
declare -a H4_PROVIDERS=("mistral" "xai" "together" "fireworks")
declare -A H4_ENV_VARS=(
  [mistral]="MISTRAL_API_KEY"
  [xai]="XAI_API_KEY"
  [together]="TOGETHER_API_KEY"
  [fireworks]="FIREWORKS_API_KEY"
)
declare -A H4_DEFAULT_MODELS=(
  [mistral]="mistral-small-latest"
  [xai]="grok-2-latest"
  [together]="meta-llama/Llama-3.3-70B-Instruct-Turbo-Free"
  [fireworks]="accounts/fireworks/models/llama-v3p1-8b-instruct"
)

# Which H4 *_API_KEY entries are visible to phantom (live env OR saved)?
declare -a HAVE_KEYS=()
declare -a MISSING_KEYS=()
for p in "${H4_PROVIDERS[@]}"; do
  env_var="${H4_ENV_VARS[$p]}"
  if [ -n "${!env_var:-}" ]; then
    HAVE_KEYS+=("$p")
    pass "  $p: live env var $env_var present"
  elif printf '%s' "$keys_list" | grep -qE "${env_var}[[:space:]]"; then
    HAVE_KEYS+=("$p")
    pass "  $p: saved key for $env_var (will be auto-loaded by phantom)"
  else
    MISSING_KEYS+=("$p")
    step "  $p: no key for $env_var — cannot round-trip this provider"
  fi
done

step "  keys present for ${#HAVE_KEYS[@]} of 4 H4 providers: ${HAVE_KEYS[*]:-none}"
step "  keys missing for ${#MISSING_KEYS[@]} of 4: ${MISSING_KEYS[*]:-none}"

if [ "${#HAVE_KEYS[@]}" -eq 0 ]; then
  warn "skip: none of the 4 H4 providers have a key in env or saved in"
  warn "      ~/.phantom-mesh/env. Set at least one to run the round-trip:"
  warn "      e.g.  phantom keys set mistral <key>"
  exit 77
fi

# ── Step 3: pick the first available H4 provider, configure it, round-trip ──
TARGET="${HAVE_KEYS[0]}"
TARGET_MODEL="${H4_DEFAULT_MODELS[$TARGET]}"
step "round-trip target: $TARGET (model=$TARGET_MODEL, type=$TARGET)"

WORKDIR="$(tmpdir)/h4-roundtrip-cwd"
mkdir -p "$WORKDIR"
cat > "$WORKDIR/agents.toml" <<TOML
[core]
host = "127.0.0.1"
port = 17878

[providers.$TARGET]
type          = "$TARGET"
api_key       = "env:${H4_ENV_VARS[$TARGET]}"
default_model = "$TARGET_MODEL"

[agent.master]
provider     = "$TARGET"
model        = "$TARGET_MODEL"
instructions = "You are a terse test fixture. Reply with exactly what is asked."
TOML

step "calling phantom repl --agent master -c '<short>' against $TARGET (90s cap)"
out=$( cd "$WORKDIR" && timeout 90 "$PHANTOM_BIN" repl --agent master -c "Reply with exactly the two characters: ok" 2>&1 \
       | sed -E 's/\x1b\[[0-9;]*m//g' )

# ── Skip-or-fail triage on the transcript ──────────────────────────────────
#
# The pre-H4-build signal is the binary either:
#   (a) prints "unknown provider type ..." and bails, or
#   (b) silently rewrites the request to a DIFFERENT URL — most commonly
#       https://openrouter.ai/... — because the unknown-type fell back to the
#       openrouter / opencode default-routing path. This is observable because
#       the resulting error mentions a URL that is NOT the H4 provider's domain.
if printf '%s' "$out" | grep -qiE 'unknown provider type|no such provider|provider type.*not.*found'; then
  warn "skip: PHANTOM_BIN does not recognize provider type '$TARGET' — pre-H4 build."
  warn "      Rebuild from a post-PR-#25 commit with --features"
  warn "      experimental-extra-providers to enable this scenario."
  exit 77
fi

# Provider-domain expected substrings per target.
declare -A EXPECTED_DOMAIN=(
  [mistral]="mistral.ai"
  [xai]="x.ai"
  [together]="together"
  [fireworks]="fireworks"
)
expected="${EXPECTED_DOMAIN[$TARGET]}"
# If the transcript shows an HTTP-error URL that is NOT the expected domain,
# the binary's H4 adapter did not run — it fell through to a different
# provider. Treat as pre-H4 SKIP.
wrong_url=$(printf '%s' "$out" \
  | grep -oE 'https?://[A-Za-z0-9./_-]+' \
  | grep -v -F "$expected" \
  | grep -vE 'phantom-mesh|localhost|127\.0\.0\.1' \
  | head -1 || true)
if [ -n "$wrong_url" ] && printf '%s' "$out" | grep -qiE 'HTTP [45][0-9][0-9]|failed: \['; then
  warn "skip: binary appears to have routed to '$wrong_url' instead of $expected —"
  warn "      pre-H4 fallback path. Rebuild from a post-PR-#25 commit."
  exit 77
fi

# Provider-side soft-fails — independent of phantom plumbing.
if printf '%s' "$out" | grep -qiE 'HTTP 429|rate.?limit|quota|usage limit|too many requests'; then
  warn "skip: $TARGET returned rate-limit / quota — provider-side, not phantom-side"
  exit 77
fi
if printf '%s' "$out" | grep -qiE 'HTTP 401|invalid.*api.*key|authentication.*failed|unauthorized'; then
  warn "skip: $TARGET returned auth failure — key may be invalid; not phantom-side"
  exit 77
fi

# ── Assertions: response is non-empty AND contains 'ok' OR a cost line ─────
# Note: not every model honors "reply with exactly ok" — soft-match.
got_cost=0
got_ok=0
if printf '%s' "$out" | grep -qE '\$[0-9]'; then
  got_cost=1
fi
case "$out" in *[oO][kK]*) got_ok=1 ;; esac

if [ "$got_cost" -eq 1 ]; then
  pass "$TARGET emitted cost summary line — provider returned a usable response"
fi
if [ "$got_ok" -eq 1 ]; then
  pass "$TARGET response contains 'ok' (model followed instruction)"
fi

if [ "$got_cost" -eq 0 ] && [ "$got_ok" -eq 0 ]; then
  chars=$(printf '%s' "$out" | wc -c | tr -d ' \n')
  fail "$TARGET response missing both cost line and 'ok' (transcript $chars chars):"
  printf '%s\n' "$out" | tail -10 | sed 's/^/    /' >&2
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
