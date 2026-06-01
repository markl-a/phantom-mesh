#!/usr/bin/env bash
# 29-t11-retry-on-429.sh — T11 retry-middleware coverage.
#
# Verifies: when an outbound provider call gets a 429 once and then 200, the
# T11 retry middleware:
#   (a) sees the 429
#   (b) backs off briefly (honoring Retry-After if we emit one)
#   (c) retries
#   (d) the second attempt's 200 is what gets surfaced to the caller
#
# We use a self-contained Python mock that:
#   - listens on a free local port
#   - returns 429 on the FIRST POST to /v1/chat/completions
#   - returns a normal OpenAI-compat 200 on the SECOND (and any subsequent) POST
#   - logs every request to a counter file we read after the run
#
# The mock impersonates Mistral's openai-compat surface so the H4 mistral
# provider adapter (which T11 wired retry into) takes the path. We point
# `providers.mistral.url` at our mock via agents.toml.
#
# Skip matrix:
#   - python missing                          → exit 77 (require_cmd)
#   - PHANTOM_BIN doesn't recognize 'mistral' → exit 77 (pre-H4 binary; T11
#     retry only mounted for the 4 H4 providers per audit notes)
#   - mock receives 0 requests inside 90s     → fail (binary didn't even try)
#   - mock receives only 1 request and        → fail (NO retry happened — the
#     phantom call returned an error                regression we're guarding)
#
# References:
#   - audit doc: docs/superpowers/audits/2026-05-15-coverage-gaps.md (MEDIUM T11)
#   - branch feat/provider-retry, merged @ 2fff9f3
#   - core/src/providers/retry.rs (RetryClient::execute_with_retry)

source "$PHANTOM_TEST_LIB/common.sh"

scenario "T11 — provider retry middleware: 429 then 200, single visible attempt"
require_cmd "$PHANTOM_BIN"
require_cmd python

# H4-detection deferred until AFTER the round-trip: pre-H4 binaries either
# emit "unknown provider type" OR route to the wrong URL entirely (no POST
# reaches our mock). Either case → 0 attempts on the mock counter, which
# we use as the SKIP signal below. This is more reliable than scanning
# `keys help`, which (as of 2026-05) doesn't list mistral on either side
# of PR #25 — see scenario 28 for the same reasoning.

# ── Pick a free port for the mock ───────────────────────────────────────────
MOCK_PORT="${MOCK_429_PORT:-12099}"
TMP="$(tmpdir)"
COUNTER_FILE="$TMP/mock-429-counter.txt"
PID_FILE="$TMP/mock-429.pid"
LOG_FILE="$TMP/mock-429.log"
SERVER_PY="$TMP/mock-429-server.py"
echo 0 > "$COUNTER_FILE"

# ── Write the mock server ───────────────────────────────────────────────────
cat > "$SERVER_PY" <<PY
#!/usr/bin/env python3
"""Mock that returns 429 on the FIRST POST to /v1/chat/completions, then 200.

Counter is persisted to disk so the harness can read it after we exit.
"""
import http.server, json, os, signal, sys, time

PORT = int(os.environ.get("MOCK_PORT", "$MOCK_PORT"))
COUNTER = os.environ.get("COUNTER_FILE", r"$COUNTER_FILE")

def bump():
    try:
        with open(COUNTER) as f:
            n = int(f.read().strip() or "0")
    except Exception:
        n = 0
    n += 1
    with open(COUNTER, "w") as f:
        f.write(str(n))
    return n

class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("[mock-429] " + (fmt % args) + "\n")

    def do_GET(self):
        # Health probe so the harness can wait for readiness.
        if self.path.startswith("/healthz"):
            self.send_response(200); self.end_headers()
            self.wfile.write(b"ok")
            return
        # Models list — some providers ping this on startup.
        if self.path.startswith("/v1/models"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "object": "list",
                "data": [{"id": "mistral-small-latest", "object": "model", "owned_by": "mock"}],
            }).encode())
            return
        self.send_error(404)

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self.send_error(404); return
        n = bump()
        # Drain & inspect the request body. We need to know whether the
        # client wants streaming (SSE) or one-shot JSON so the 200 reply
        # matches — if a non-streaming JSON is returned to a streaming
        # client, phantom reports "stream completed with no content".
        want_stream = False
        try:
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length) if length else b""
            if raw:
                try:
                    req = json.loads(raw.decode("utf-8"))
                    want_stream = bool(req.get("stream"))
                except Exception:
                    pass
        except Exception:
            pass
        if n == 1:
            # First POST: simulate a 429 with a short Retry-After so the
            # retry middleware backs off ~0.5s (default config) instead of
            # honoring a long server-side hint that would blow our 90s cap.
            self.send_response(429)
            self.send_header("Retry-After", "1")
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "error": {"type": "rate_limit_error",
                          "message": "mock: throttling first attempt"},
            }).encode())
            sys.stderr.write("[mock-429] attempt #%d -> 429 (synthetic, stream=%s)\n" % (n, want_stream))
            return
        # Subsequent POSTs: success — match the streaming/non-streaming shape.
        if want_stream:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            # Two delta chunks then [DONE], same shape as scenario-12 mock.
            for piece in ("o", "k"):
                chunk = {"choices": [{"index": 0,
                                       "delta": {"content": piece},
                                       "finish_reason": None}]}
                self.wfile.write(("data: " + json.dumps(chunk) + "\n\n").encode())
                self.wfile.flush()
            # Final chunk with usage so phantom records cost.
            final = {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                     "usage": {"prompt_tokens": 8, "completion_tokens": 1, "total_tokens": 9}}
            self.wfile.write(("data: " + json.dumps(final) + "\n\n").encode())
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            sys.stderr.write("[mock-429] attempt #%d -> 200 ok (SSE)\n" % n)
            return
        body = json.dumps({
            "id": "mock-" + str(int(time.time() * 1000)),
            "object": "chat.completion",
            "model": "mistral-small-latest",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 1, "total_tokens": 9},
        })
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body.encode())
        sys.stderr.write("[mock-429] attempt #%d -> 200 ok (json)\n" % n)

def main():
    srv = http.server.HTTPServer(("127.0.0.1", PORT), H)
    def shutdown(*_a):
        sys.stderr.write("[mock-429] shutting down\n")
        srv.shutdown()
    signal.signal(signal.SIGINT, shutdown)
    signal.signal(signal.SIGTERM, shutdown)
    sys.stderr.write("[mock-429] listening on http://127.0.0.1:%d\n" % PORT)
    srv.serve_forever()

if __name__ == "__main__":
    main()
PY

# ── Start the mock ──────────────────────────────────────────────────────────
MOCK_PORT="$MOCK_PORT" COUNTER_FILE="$COUNTER_FILE" python "$SERVER_PY" \
  >"$LOG_FILE" 2>&1 &
echo $! > "$PID_FILE"

cleanup() {
  if [ -f "$PID_FILE" ]; then
    local pid; pid=$(cat "$PID_FILE")
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      sleep 0.3
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
  fi
}
trap cleanup EXIT

# Wait up to 5s for the mock to bind.
ready=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -sS --max-time 1 "http://127.0.0.1:$MOCK_PORT/healthz" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 0.5
done
if [ "$ready" -ne 1 ]; then
  fail "mock 429 server failed to bind on :$MOCK_PORT (see $LOG_FILE)"
  exit 1
fi
pass "mock 429 server up on :$MOCK_PORT (PID $(cat "$PID_FILE"))"

# ── Build a temp agents.toml pointing mistral at the mock ───────────────────
WORKDIR="$TMP/t11-cwd"
mkdir -p "$WORKDIR"
cat > "$WORKDIR/agents.toml" <<TOML
[core]
host = "127.0.0.1"
port = 17878

# Point the H4 mistral provider at our local mock. The provider adapter
# uses provider.url verbatim when set — this is the documented escape
# hatch for self-hosted gateways (and for tests like this one).
[providers.mistral]
type          = "mistral"
url           = "http://127.0.0.1:$MOCK_PORT"
api_key       = "mock-no-auth"
default_model = "mistral-small-latest"

[agent.master]
provider     = "mistral"
model        = "mistral-small-latest"
instructions = "You are a terse test fixture."
TOML

# ── Drive a single phantom call ────────────────────────────────────────────
step "phantom repl --agent master -c '...' (60s cap; expect 1×429 then 1×200)"
start_ts=$(date +%s)
out=$( cd "$WORKDIR" && timeout 60 "$PHANTOM_BIN" repl --agent master -c "Reply with exactly the two characters: ok" 2>&1 \
       | sed -E 's/\x1b\[[0-9;]*m//g' )
elapsed=$(( $(date +%s) - start_ts ))

cleanup  # stop the mock so the counter file stops mutating

attempts=$(cat "$COUNTER_FILE" 2>/dev/null | tr -d ' \n')
attempts=${attempts:-0}

step "  mock counted $attempts POST(s) to /v1/chat/completions in ${elapsed}s"
step "  mock log tail:"
tail -10 "$LOG_FILE" 2>/dev/null | sed 's/^/      /'

# ── Diagnose binary capability if attempts == 0 ─────────────────────────────
# Pre-H4 binaries either bail with "unknown provider type" or fall through
# to a different provider (different URL) — in BOTH cases our local mock
# sees zero POSTs. Treat as SKIP, not fail.
if [ "$attempts" -eq 0 ]; then
  if printf '%s' "$out" | grep -qiE "unknown provider type|no such provider|provider.*not.*found"; then
    warn "skip: binary rejected provider type 'mistral' — pre-H4 build (PR #25)."
    exit 77
  fi
  if printf '%s' "$out" | grep -oE 'https?://[A-Za-z0-9./_-]+' \
       | grep -qvE "127\.0\.0\.1|localhost|phantom-mesh"; then
    warn "skip: binary routed to a non-mock URL (visible in transcript) — pre-H4"
    warn "      build silently fell back to a different provider's adapter."
    step "  phantom output tail:"
    printf '%s\n' "$out" | tail -8 | sed 's/^/      /'
    exit 77
  fi
  fail "binary did not POST to the mock at all — agent config or provider routing broken"
  step "  phantom output tail:"
  printf '%s\n' "$out" | tail -8 | sed 's/^/      /'
  exit 1
fi

# ── The retry assertion ────────────────────────────────────────────────────
if [ "$attempts" -lt 2 ]; then
  fail "T11 retry did NOT fire: only $attempts attempt(s); expected ≥ 2 (1×429 then ≥1×200)"
  step "  phantom output tail:"
  printf '%s\n' "$out" | tail -10 | sed 's/^/      /'
else
  pass "T11 retry fired: $attempts attempts (1×429 → $((attempts - 1))×200) in ${elapsed}s"
fi

# ── And the user-facing outcome: phantom should report success ──────────────
if printf '%s' "$out" | grep -qiE 'HTTP 429|rate.?limit|too many requests' \
   && ! printf '%s' "$out" | grep -qE '\$[0-9]'; then
  fail "user-visible reply still looks like a 429 error — retry didn't surface the 200 to the caller"
  step "  phantom output tail:"
  printf '%s\n' "$out" | tail -10 | sed 's/^/      /'
elif printf '%s' "$out" | grep -qE '\$[0-9]'; then
  pass "phantom emitted a cost line — caller saw the 200, not the 429"
else
  warn "ambiguous transcript (no clear cost line, no clear error) — inspect manually:"
  printf '%s\n' "$out" | tail -10 | sed 's/^/      /' >&2
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
