# Runbook: Live Mesh Test (single-node + cross-peer + Squad Pipeline)

**Created**: 2026-05-01 (after first end-to-end live test of `/rpc/squad/dispatch`)
**Audience**: this session + Z13 / Oracle / iOS / Android sessions
**Companion to**: SPEC-FREEZE-V1 §9.2 pre-flight, SESSION-ONBOARDING §2 daily verification, MULTI-AGENT-QA workflow

This runbook is the canonical "how do I test the mesh end-to-end"
sequence. **Phase 1** (single-node) is what's testable today on the
Mac M1 alone. **Phase 2** (cross-peer) needs ≥1 other peer online.
**Phase 3** (Squad Pipeline live) needs ≥2 other peers online and
exercises the 5/9 demo's headline feature against real RPC fan-out.

## 0. Prerequisites

```bash
PHANTOM_BIN="$HOME/.cargo/bin/phantom"
DAEMON_URL="http://127.0.0.1:7878"

# Confirm binary + daemon
"$PHANTOM_BIN" --version
# expect: phantom 0.4.0 (<commit-hash>+, macos-aarch64, built <date>)

curl -sm5 "$DAEMON_URL/healthz"
# expect: ok

# cluster_secret must be set (HMAC requires it)
SECRET=$(grep -E "^\s*cluster_secret\s*=" $HOME/.phantom-mesh/agents.toml \
  | sed 's/.*= *"//; s/"$//')
[ -n "$SECRET" ] && echo "✓ cluster_secret configured (${#SECRET} chars)" \
                 || { echo "✗ no cluster_secret"; exit 1; }
```

If daemon isn't running:
```bash
launchctl kickstart -k "gui/$UID/ai.phantommesh.serve"
sleep 3
```

If binary is stale (commit drift):
```bash
cd ~/Documents/workspace/hailmary/phantom-mesh
./scripts/build-mac.sh
# then redeploy:
DIST=dist/phantom-aarch64-apple-darwin
cp "$DIST" "$HOME/.cargo/bin/phantom"
codesign --force --sign - "$HOME/.cargo/bin/phantom"
cp "$DIST" "$HOME/Library/Application Support/phantom-mesh/bin/phantom"
codesign --force --sign - "$HOME/Library/Application Support/phantom-mesh/bin/phantom"
launchctl kickstart -k "gui/$UID/ai.phantommesh.serve"
sleep 3
```

## 1. HMAC token helper

Almost every cluster RPC requires `X-Cluster-Auth: hex(HMAC-SHA256(key=cluster_secret, msg=body))`.
Save this helper:

```bash
hmac_token() {
  local body="$1"
  python3 -c "
import hmac, hashlib, sys
print(hmac.new('$SECRET'.encode(), '''$body'''.encode(), hashlib.sha256).hexdigest()
)
"
}
```

NOTE: `cluster_secret || body` (string concat then SHA256) does NOT work — daemon uses real HMAC-SHA256 from the `hmac` crate. SPEC-FREEZE-V1 §2.1 was corrected on 2026-05-01 in commit `55e5565` after this exact test failure mode surfaced.

## 2. Phase 1 — single-node tests (run today, Mac M1 alone)

### 2.1 /rpc/ping carries new fields

```bash
curl -s -m 3 "$DAEMON_URL/rpc/ping" | python3 -m json.tool
```

**Expected** (post-Squad-a, post-`352486c`):
```json
{
    "active_tasks": 0,
    "agents": ["coder", "local", "master", "researcher", "reviewer"],
    "capabilities": [],
    "core_sha": "<short-sha>",
    "name": "<node_name>",
    "online": true,
    "phantom_version": "0.4.0",
    "uptime_secs": <N>,
    "version": "0.4.0",
    "wire_version": 1,
    "worker_caps": []
}
```

If `agents` or `worker_caps` keys missing → daemon is on a pre-Squad-a binary; rebuild + redeploy per §0 prerequisites.

### 2.2 /rpc/squad/dispatch happy path

```bash
BODY='{"agent":"master","prompt":"output the literal text MESH_TEST_OK and nothing else","wire_version":1}'
TOKEN=$(hmac_token "$BODY")

curl -s -m 60 -X POST "$DAEMON_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: $TOKEN" \
  -d "$BODY"
```

**Expected**:
```json
{"agent":"master","elapsed_ms":<3000-10000>,"node":"<node_name>","output":"MESH_TEST_OK","wire_version":1}
```

Wallclock ~3-10s depending on provider latency. If output isn't literally `MESH_TEST_OK`, the master agent's prompt was over-interpreted — annoying but not a bug.

### 2.3 /rpc/squad/dispatch error paths

Three deterministic 400s + one 401:

```bash
# unknown agent → 400 + available_agents list
BODY='{"agent":"nonexistent","prompt":"hi","wire_version":1}'
curl -s -X POST "$DAEMON_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: $(hmac_token "$BODY")" \
  -d "$BODY" -w "\nHTTP %{http_code}\n"
# expect: HTTP 400, error: agent `nonexistent` not configured ..., available_agents: [...]

# wire_version too high → 400 + phantom upgrade hint
BODY='{"agent":"master","prompt":"hi","wire_version":99}'
curl -s -X POST "$DAEMON_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: $(hmac_token "$BODY")" \
  -d "$BODY" -w "\nHTTP %{http_code}\n"
# expect: HTTP 400, error: peer is wire v99, this binary is v1, run `phantom upgrade`

# missing fields → 400 + received_keys
BODY='{"wire_version":1}'
curl -s -X POST "$DAEMON_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: $(hmac_token "$BODY")" \
  -d "$BODY" -w "\nHTTP %{http_code}\n"
# expect: HTTP 400, error: both `agent` and `prompt` fields required, received_keys: ["wire_version"]

# bad HMAC → 401
BODY='{"agent":"master","prompt":"hi","wire_version":1}'
curl -s -X POST "$DAEMON_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: BOGUS_TOKEN_DEADBEEF" \
  -d "$BODY" -w "\nHTTP %{http_code}\n"
# expect: HTTP 401, error: unauthorized — bad X-Cluster-Auth
```

### 2.4 phantom peer list completes ≤8s

```bash
time timeout 8 "$PHANTOM_BIN" peer list
```

**Expected**: returns within ~5s even when peers are offline (Bug #23 fix wraps each ping in 5s deadline). If it hangs >8s → daemon is on a pre-Bug-#23 binary; rebuild.

### 2.5 /api endpoints

```bash
curl -s "$DAEMON_URL/api/version" | python3 -m json.tool
# expect: commit != "unknown" (Bug #13 fix); should be a real short SHA

curl -s "$DAEMON_URL/api/dashboard/status" | python3 -m json.tool
# expect: tools_enabled + tools_available BOTH present (Bug #14)

curl -s "$DAEMON_URL/api/sessions" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(f'{len(d)} sessions')"
# expect: ≥1 session
```

### 2.6 /api/chat streaming (optional)

```bash
curl -sN -X POST "$DAEMON_URL/api/chat" \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"reply with the single word OK"}' | head -10
```

**Expected**: chunked NDJSON / SSE-ish stream; first frame should be `{"type":"token","content":"O"}`-ish. If stream hangs → API key for the master agent's provider isn't set; check `/api/providers/health`.

### 2.7 broken-pipe panic regression check

Bug #24 fix should be verified:

```bash
COUNT_BEFORE=$(ls $HOME/.phantom-mesh/crashes/ 2>/dev/null | wc -l)
"$PHANTOM_BIN" doctor | head -1 > /dev/null
"$PHANTOM_BIN" --help | head -2 > /dev/null
sleep 0.5
COUNT_AFTER=$(ls $HOME/.phantom-mesh/crashes/ 2>/dev/null | wc -l)
[ "$COUNT_BEFORE" = "$COUNT_AFTER" ] \
  && echo "✓ no new broken-pipe crash" \
  || echo "✗ Bug #24 regression — $((COUNT_AFTER-COUNT_BEFORE)) new crash log(s)"
```

## 3. Phase 2 — cross-peer tests (need ≥1 other peer online)

Prereqs: at least one of Z13 / Oracle / iOS / Android peer is up on a binary that speaks `wire_version=1` and has the same `cluster_secret`.

### 3.1 Find online peers

```bash
"$PHANTOM_BIN" peer list 2>&1 | grep online
```

**Expected**: ≥1 row with status `online`. If none, rebuild + redeploy per other-session's `SESSION-ONBOARDING.md` §3.X.

Pick one online peer's URL (call it `$PEER_URL`).

### 3.2 Cross-peer /rpc/ping

```bash
curl -s -m 5 "$PEER_URL/rpc/ping" | python3 -m json.tool
```

**Expected**:
```
"core_sha": "<should match Mac's>"
"wire_version": 1
"agents": [...]    # if peer is on post-Squad-a binary
"worker_caps": [...]   # iOS/Android sandbox: 5 caps; Mac/Win/Linux: empty
```

Drift verdict:
- Same `core_sha` as Mac → peers are byte-identical builds, perfect
- Same `wire_version=1` but different `core_sha` → semantically compatible but the build hash differs (a redeploy is overdue but mesh works)
- `wire_version != 1` → STOP. Run `phantom upgrade` on the older peer.

### 3.3 Cross-peer dispatch

```bash
BODY='{"agent":"master","prompt":"reply with PEER_REACHED","wire_version":1}'
TOKEN=$(hmac_token "$BODY")

curl -s -m 30 -X POST "$PEER_URL/rpc/squad/dispatch" \
  -H 'Content-Type: application/json' \
  -H "X-Cluster-Auth: $TOKEN" \
  -d "$BODY"
```

**Expected**: HTTP 200 with `output: "PEER_REACHED"`. If 401 → cluster_secret drift between Mac and that peer. If 400 (unknown agent) → that peer's agents.toml doesn't have `master` (rare; default config does).

### 3.4 Mac peer list shows new fields

After redeploying any peer to Squad-a binary, Mac's cached PeerInfo gets the new fields on next ping cycle (~30s):

```bash
sleep 35
curl -s "$DAEMON_URL/rpc/peers" | python3 -m json.tool | head -40
```

Look for entries where `agents` is non-empty (i.e. the peer reported back its inventory).

## 4. Phase 3 — full Squad Pipeline live (need ≥2 other peers online)

The 5/9 demo's headline feature: Mac dispatches to N peers in parallel, collects, synthesises.

### 4.1 Pre-flight peer inventory

```bash
"$PHANTOM_BIN" peer list
# Confirm: ≥2 peers online, all with same core_sha, all with non-empty agents lists
```

### 4.2 Dispatcher dry-run from CLI

(Once `phantom dispatch --dry-run "..."` lands as a CLI command per Squad-e remaining work; for now, exercise via TerminalShell's `/dispatch`.)

```bash
# Ensure /term route is reachable in browser (TerminalShell React build mounted)
curl -s -m 3 "$DAEMON_URL/term" -o /dev/null -w "HTTP %{http_code}\n"
# expect: 200 (if HTTP 404, daemon doesn't yet serve the React build —
# v0.2 pipeline integration; use Tauri Desktop or `pnpm dev` instead)
```

### 4.3 Browser-driven /dispatch

In a browser opened to `$DAEMON_URL/term` (or via `pnpm dev` against a vite preview):

1. Header should show: `phantom <version> · <node_name> · ●●●○○` (peer dots)
2. Type: `/dispatch  對 codebase 做安全分析`
3. SquadGrid appears with N panels (one per peer in dispatcher's plan)
4. Each panel transitions: pending → streaming → done
5. After ~30s wallclock, scrollback gets per-peer summary
6. (Once /api/dispatch/synthesize lands) unified markdown report

### 4.4 Verify by curl (no browser)

For each peer in your dispatch plan, fire a `/rpc/squad/dispatch` in parallel and time the slowest:

```bash
PEERS=("http://100.X:7878" "http://100.Y:7878" "http://100.Z:7878")
PROMPTS=("recon: list 3 things" "enrich: analyse output" "review: comment")

start=$(date +%s)
for i in "${!PEERS[@]}"; do
  BODY="{\"agent\":\"master\",\"prompt\":\"${PROMPTS[i]}\",\"wire_version\":1}"
  TOKEN=$(hmac_token "$BODY")
  curl -s -X POST "${PEERS[i]}/rpc/squad/dispatch" \
    -H 'Content-Type: application/json' \
    -H "X-Cluster-Auth: $TOKEN" \
    -d "$BODY" > "/tmp/dispatch-$i.json" &
done
wait
elapsed=$(( $(date +%s) - start ))
echo "all peers responded in ${elapsed}s"

for i in "${!PEERS[@]}"; do
  echo "=== peer $i ==="
  cat /tmp/dispatch-$i.json | python3 -m json.tool | head -10
done
```

**Expected**: total `elapsed` ≈ slowest peer's elapsed_ms (parallel, not serial).

## 5. Failure modes + fixes

| Symptom | Likely cause | Fix |
|---|---|---|
| `/rpc/ping` lacks `agents`/`worker_caps` keys | daemon on pre-Squad-a binary | rebuild + redeploy per §0 |
| `/rpc/squad/dispatch` returns 401 with valid HMAC token | cluster_secret drift OR token computed as concat-then-SHA256 | re-read SPEC-FREEZE-V1 §2.1 + re-pull `cluster_secret` from 1Password OOB |
| `phantom peer list` >8s | pre-Bug-#23 binary | rebuild |
| `core_sha` mismatch across peers | one peer didn't redeploy after a tagged release | rebuild + redeploy that peer |
| `wire_version` mismatch | peer on too-old or too-new binary | run `phantom upgrade` on the offending peer (or rebuild manually) |
| New crash logs after broken-pipe pipe | pre-Bug-#24 binary | rebuild |
| `/term` returns 404 | TerminalShell React build not wired into daemon static serving (v0.2 integration item) | use Tauri Desktop OR `pnpm dev` proxy until integrated |
| `phantom_swarm` returns "agent not configured" on a peer | dispatcher routed to a peer whose `agents` list lacks the role | dispatcher should consult `/rpc/peers[].agents` before plan emission; if it doesn't, that's a Squad-c prompt-engineering bug |

## 6. Cleanup

After live test:
```bash
# Reset crash log directory if Phase 1 §2.7 left junk
rm -f $HOME/.phantom-mesh/crashes/crash-*broken-pipe*

# Reset test transcripts so /api/sessions doesn't bloat
ls $HOME/.phantom-mesh/conversations/test-*.jsonl 2>/dev/null && \
  rm -f $HOME/.phantom-mesh/conversations/test-*.jsonl

# Daemon stays running (don't kill — cross-platform peers depend on it)
"$PHANTOM_BIN" peer list   # final sanity check
```

## 7. Schedule

- **Today (5/1)** — Phase 1 done (this runbook is the artefact)
- **5/2-5/4** — other sessions deploy → Phase 2 testable
- **5/5-5/7** — Phase 3 (≥2 peers online for Squad Pipeline)
- **5/8** — full dress rehearsal: all 9 peers up, run §3.1 + §4 end-to-end
- **5/9** — interview demo runs §4.3 via browser TerminalShell

### 7.1 Live-fire evidence log

| Date  | Phase | Coordinator | Peers online | Result | Notes |
|-------|-------|-------------|--------------|--------|-------|
| 5/3 r66 | 3 | Mac M1 `3715e17df9` | 2 Win (`1ac6a5c7d2+`) + local | ✓ all 3 nodes returned, synthesizer ran | peer `100.107.205.98` emitted raw `<shell={...}>` instead of rendered tool-call output — Windows binary tool-call renderer regression; non-blocking for v0.1.0 demo, file as Win-side bug |
| 5/3 r70 | 3 | Mac M1 `3715e17df9` | Win `1ac6a5c7d2+` (`100.107.205.98`) + Win `bb0f5c9091+` (`100.106.176.125`) + local | ⚠ both Win peers errored, only local responded | dispatch path unstable on Win — same coordinator/binary as r66, same peer A; second Win peer is new binary `bb0f5c9091+`. Likely transient or model/key issue on Win side. Re-run when Z13 next online. |
| 5/3 r72 | 3 | Mac M1 `3715e17df9` | 2 Win `f40f03f6c6+` + local | ✓ peer A (`100.107.205.98`) clean `ok`; ✗ peer B (`100.106.176.125`) errored | **r66's raw `<shell={...}>` rendering bug is fixed in `f40f03f6c6+`** ✓. peer B errors persist across two binaries (`bb0f5c9091+` r70 + `f40f03f6c6+` r72) → not a binary bug, likely provider-key / model misconfig on that specific machine. Action item for that session: run `phantom doctor` and confirm provider keys + working model. |

Update as further runs land. The point is to have a one-glance audit trail
of which peer combinations have actually exchanged signed dispatches.

## 8. What this runbook does NOT cover

- Tauri Desktop / iOS Tauri / Android Tauri APK app launch (covered in
  per-session §3.X of SESSION-ONBOARDING.md)
- Streaming SSE per-peer panels in SquadGrid (lands v0.2; v0.1.0 ships
  synchronous request/response)
- /api/dispatch/synthesize endpoint that runs `[agent.synthesizer]`
  on coordinator (not yet shipped; SPEC-FREEZE-V1 §11.1 mentions; v0.2)
- HMAC rotation / cluster_secret migration (out of v0.1.0 scope; SPEC-
  FREEZE-V1 §12.7)
- Browser-side HMAC computation (browser can't have cluster_secret;
  awaits /api/dispatch proxy in v0.2)

## 9. References

- `docs/SPEC-FREEZE-V1.md` §2.1 — wire protocol contract
- `docs/SPEC-FREEZE-V1.md` §9.2 — 5/9 morning pre-flight (8 steps)
- `docs/SPEC-FREEZE-V1.md` §12 — mesh lifecycle (handshake, dispatch, recovery)
- `docs/MULTI-AGENT-QA.md` §3 — PR review workflow
- `docs/SESSION-ONBOARDING.md` §2 — daily verification per session
- `docs/runbooks/secret-cleanup-5-8.md` — pre-launch git history sanitisation
- `core/src/serve.rs::rpc_squad_dispatch` — implementation
- `core/src/mesh.rs::make_auth_token_bytes` — HMAC computation truth
