# Phantom Mesh — Linux CLI Behavior Spec

> **Authoritative behavior reference for the `phantom` CLI on Linux** (`x86_64`/`aarch64`,
> incl. WSL2). Code-grounded (every entry cites `core/src/...` `file:line`), with **drift
> flags** where the current code diverges from intended behavior. This is the anchor that
> keeps Linux development from wandering — when in doubt, this doc is the spec; if the code
> disagrees, that's a drift item (§9), not a new behavior.
>
> **Scope** (operator-set 2026-05-29): all three use models co-equal — **Life-Track terminal**
> (生活軌道，capture→coach→recall), **mesh/server node** (網狀節點，serve/dispatch/swarm),
> and **dev/agent tool** (開發代理工具，exec/evolve/mcp). Full surface, all groups.
>
> Companion docs: SPEC-44 (Linux foundations) · SPEC-45 (Linux screens/flows) · `--help`.

---

## §0 How to read this

Each command entry has a fixed shape:
- **Behavior** — what it does (with `file:line` of the handler).
- **Args/flags** — every flag + default.
- **I/O** — stdin · stdout · stderr usage.
- **Exit** — exit-code contract (`0` ok / `1` runtime error / `2` usage error, unless noted).
- **Errors** — common failure modes + messages.
- **Linux** — `~/.phantom-mesh` paths, daemon/port dependency, `identity.key` encryption, systemd, display/headless, env.
- **Status** — `✅ works` · `⚠ partial` (works with a caveat/gap) · `❌ drift/stub` (broken or unimplemented on Linux), with the gap + proposed intended behavior.

**Glossary（縮寫/名詞中文）**: daemon（常駐服務）· peer（對等節點）· IKM（input key material，輸入金鑰素材）·
HKDF（雜湊金鑰衍生函數）· HMAC（雜湊訊息驗證碼）· TTY（終端機）· headless（無顯示環境）·
systemd `--user`（使用者級服務管理）· age（檔案加密格式）.

---

## §1 Cross-cutting model (read first — most drift lives here)

### 1.1 Config & data paths (`~/.phantom-mesh/`)
| File/dir | Purpose | Written by |
|---|---|---|
| `agents.toml` (also `./agents.toml`) | provider keys + agents + `[cluster]` + `[workspace]` | onboarding / `providers` / `workspace` / `cluster` |
| `env` | shell-sourcable secrets (auto-loaded at startup, phantom.rs:5184) | manual / onboarding |
| `keys/ed25519.{priv,pub}` | **signing** identity (recipes) — priv mode 0600 | `keys init` |
| `auth.json` | **login** identity (email-hash / OAuth / broker JWT) — mode 0600 | `login` (deleted by `logout`) |
| `identity.key` | **64-byte event-encryption IKM** → HKDF → at-rest EventStore key | ⚠ see §1.3 |
| `events/<id>/{meta,analysis}.json` + modality files | the Life-Track event store (encrypted iff `identity.key`) | `event capture` (daemon) / `note` / `focus stop` |
| `events.jsonl` | daemon **telemetry** log (NOT the event store — see `logs`) | `serve` / argv recorder |
| `conversations/<id>.jsonl` | TUI/REPL session history (plaintext) | `tui`/`repl`/`exec` |
| `evolve-checkpoints/` · `autoevolve.log` · `autoevolve.queue.txt` | self-improvement state | `evolve`/`autoevolve` |
| `peers.json` · `broker.json` · `models-cache.json` · `lang` | cluster peers+caps / broker creds / model cache / UI language | `config pull` / `models` / `lang` |

### 1.2 Three independent identities (a frequent confusion)
1. **`keys/ed25519.*`** — signs CO-EVOLUTION recipes. `keys init`.
2. **`auth.json`** — "who am I logged in as". `login` / `whoami` / `logout`.
3. **`identity.key`** — the at-rest **encryption** root for Life-Track events. *Not* the same as the others.

### 1.3 Encryption at rest — the #1 cross-cutting drift
Every Life-Track read/write path (`event capture`, `note`, `focus stop`, `recall`, `review`,
`coach review`, `data export/stats`, `event show`) encrypts with an age key derived
(HKDF-SHA256) from `~/.phantom-mesh/identity.key` **iff that file exists** — otherwise it
**silently falls back to plaintext**, and encrypted events are **silently skipped** on read
when the key is absent (`life_node/key_derivation.rs`, `storage.rs`).
- **On `main`**: no production code creates `identity.key` (only a test fixture, `data_cli.rs:399`)
  → **every real user stores plaintext** while the product implies encryption. **P4 violation.**
- **Fix status**: `wip/z13-wsl/task-2026052902-eventstore-encryption` adds
  `identity::ensure_root_identity_key()` (64 CSPRNG bytes, 0600, O_EXCL, idempotent) called from
  `keys init` + `serve` boot, verified end-to-end (events become `age-encryption.org/v1`). **Pending merge.**
- **Intended behavior (this spec):** `identity.key` is auto-provisioned at first `keys init` *and*
  first `serve` boot; at-rest encryption is **on by default**; missing-key on a read path is a
  loud warning, not a silent plaintext/skip.

### 1.4 Daemon dependency
Only `event capture` (and the mobile `food`/`focus`/`habit` wire) **POST to a running
`phantom serve`** (default `:7878`). Everything else in Life-Track (`note`, `recall`, `review`,
`coach review`, `event show`, `focus`, `data *`, `logs`) is **fully offline / direct-disk**.

### 1.5 Port resolution
`serve` honors precedence **`--port` > `PHANTOM_PORT` env > config `core.port` (7878)**
(phantom.rs:3569-3596 — verified). **Drift:** `service status` and `self-update`'s health probe
**hardcode `:7878`** (`service/linux.rs:225,273`), so a node served on a custom port reports
"unreachable" / probes the wrong port. WSL2 caveat: `:7879` can be held by a Windows-side
process via mirrored localhost (invisible to Linux `ss`).

### 1.6 Auth / HMAC (cluster)
`dispatch`, `peer assign|send-async|poll`, `git sync`, `cluster upgrade` sign requests with
`X-Cluster-Auth: HMAC-SHA256(cluster_secret, …)` (SPEC-10 canonical + legacy dual-accept).
`cluster status` / `peer ping` use **unauthenticated** `/healthz` / `/rpc/ping` — so "reachable"
≠ "auth-compatible".

### 1.7 Environment variables
`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`/`GROQ_API_KEY`/`GEMINI_API_KEY` (providers) ·
`PHANTOM_PORT` · `PHANTOM_COORD` (self-update/mesh source) · `PHANTOM_MAX_ROUNDS` ·
`PHANTOM_PERM` (allow|ask|diff|deny) · `PHANTOM_PLAN_MODE` · `PHANTOM_AUTH_URL` (broker override) ·
`PHANTOM_LANG` · `PHANTOM_MESH_DIR` (overrides `~/.phantom-mesh`) · `NO_COLOR`.

---

## §2 Interactive

### `phantom` (bare → TUI)
- **Behavior**: launches ratatui TUI via `tui::launch_default()` (phantom.rs:5691). First-run gate (5704): if no config **and stdin is a TTY**, runs `run_first_time_onboarding()` first; applies `[workspace]` cd + pinned agent.
- **Flags**: none (any flag/prompt diverts to one-shot/REPL). Env: `PHANTOM_REPL=1`/`PHANTOM_TUI=0` → force line REPL.
- **I/O**: TTY keystrokes (raw mode) → alt-screen render; stderr = workspace notices.
- **Exit**: 0 clean; **non-zero** if not a TTY (`run_tui` bail "stdin is not a terminal").
- **Linux**: reads `agents.toml`; conversations plaintext; no port bound.
- **Status**: ⚠ partial — non-TTY refusal happens late (inside `run_tui`) and the onboarding wizard is silently skipped headless, so a fresh headless install gets a generic "needs a terminal" error. **Intended:** detect non-TTY early in the bare branch, emit the `repl`/`exec` hint.

### `phantom repl [--agent N] [--session ID] [-c] [PROMPT]`
- **Behavior**: line-mode REPL (phantom.rs:5977), or one-shot with a positional PROMPT. Banner = providers/cluster/agent/session/dir.
- **Flags**: `--agent` (master) · `--session ID` · `-c`/`--continue` (resume last session — note: "continue", not "command") · `--list-sessions` · `--config PATH`.
- **I/O**: line reads (works piped **or** TTY — no TTY gate); stdout = answer; stderr = banner + cost line.
- **Exit**: 0; one-shot propagates agent error non-zero.
- **Linux**: env from `env`; conversations plaintext `conversations/{chat_id}.jsonl`; headless-friendly.
- **Status**: ✅ works.

### `phantom exec [--json|--quiet] [--continue] [--agent N] [PROMPT]`
- **Behavior**: headless single-turn for CI/pipelines (phantom.rs:5465). stdin = prompt when no positional + non-TTY.
- **Flags**: `--json` (one AgentEvent/line) · `--quiet` (final output only) · `-c`/`--continue` · `--agent` (master) · `--session ID` · `--config PATH`.
- **I/O**: stdin (piped prompt) → stdout (stream / json / final); stderr = `[tool]`/`[done]` (suppressed by `--quiet`/`--json`).
- **Exit**: **0** ok · **1** agent error · **2** usage (no prompt+TTY, empty prompt, no `agents.toml`).
- **Linux**: designed headless; ideal for WSL/CI.
- **Status**: ✅ works — clean 0/1/2 contract, proper stdin-pipe detection.

### `phantom tui`
- **Behavior**: explicit TUI entry (phantom.rs:5441) — same app as bare, but no onboarding/workspace logic.
- **Exit**: 0 clean; non-zero non-TTY.
- **Status**: ✅ works (TTY required by design). Minor: `phantom tui --help` launches instead of showing usage.

### `phantom --version [--short]`
- **Behavior**: intercepted before dispatch (phantom.rs:3206). Full provenance line or bare semver with `--short`.
- **Exit**: 0 always.
- **Status**: ✅ works.

---

## §3 Daemon & Service

### `phantom serve [--host H] [--port P] [--openclaw-telegram] [--bot-token-env VAR]`
- **Behavior**: axum HTTP+WebSocket daemon (phantom.rs:3545); boot warnings → coordinator register → heartbeat + mDNS → blocks until Ctrl-C. Serves `/ws /api/* /rpc/* /m /dist/* /scripts/*`.
- **Flags**: `--host` (config host) · `--port` (precedence §1.5) · `--openclaw-telegram` (needs `experimental-openclaw-telegram` feature, else ignored with notice) · `--bot-token-env` (default `TELEGRAM_BOT_API_KEY`; raw token never on CLI).
- **I/O**: banner/warnings → stderr; HTTP is the channel.
- **Exit**: 0 clean shutdown · 1 bind failure (15s `AddrInUse` retry then error).
- **Linux**: `SO_REUSEADDR` + `listen(1024)`; systemd unit `phantom-mesh.service`.
- **Status**: ✅ works. ⚠ **does NOT provision `identity.key` at boot** (§1.3) → `/api/events` capture writes plaintext / 500s on missing key. (Fixed on the encryption branch.) Cosmetic: banner "+2 cluster RPC" double-counts (tool count already includes `cluster_*`).

### `phantom service [install|uninstall|status]`
- **Behavior**: manages systemd `--user` unit `phantom-mesh.service` running `phantom serve` (service/linux.rs:176). Default = `status`.
- **I/O**: install/uninstall → stderr; status → stdout. Shells `systemctl`/`curl`.
- **Exit**: 0 · 1 unknown action / `enable --now` fail / no `$HOME`.
- **Linux**: unit → `~/.config/systemd/user/phantom-mesh.service`; propagates an env allowlist (names logged, values never); prints a **`loginctl enable-linger $USER`** hint (needs sudo) to survive logout.
- **Status**: ✅ works. ⚠ `status` healthz probe hardcodes `:7878` (§1.5) → wrong for custom-port nodes.

### `phantom mcp`
- **Behavior**: MCP stdio JSON-RPC server, protocol `2024-11-05` (mcp.rs:34). stdin = requests, stdout = responses.
- **Exit**: 0 on EOF · 1 on I/O error. Malformed JSON → `-32700` (loop continues).
- **Linux**: no network bind; `tools/list` is platform-conditional (base minus 3 Windows tools + 18 non-iOS subprocess tools).
- **Status**: ⚠ doc-drift — module doc claims "40 tools" but the real Linux list is dynamic and >40. **Intended:** drop the hardcoded "40".

---

## §4 Life Node

> Daemon needed only for `event capture`. All others are offline/direct-disk. Encryption per §1.3.

### `phantom event capture [--kind K] [--image P]… [--audio P]… [--text T] [--tag T] [--coord URL]`
- **Behavior**: POSTs multimodal event to daemon `/api/events`, prints JSON response (phantom.rs:3911; capture.rs:20).
- **Flags**: `--kind` (default `note`) · `--image`/`--audio` (repeatable) · `--text` · `--tag` (repeatable) · `--coord` (default `http://127.0.0.1:$PHANTOM_PORT`).
- **Exit**: 0 · 2 unknown flag · non-zero on no-modality / file-read fail / non-2xx.
- **Linux**: **REQUIRES `serve` :7878** + an LLM key (Gemini, Groq text fallback); 502 if no provider. Encryption happens daemon-side (§1.3).
- **Status**: ✅ works given daemon + key.

### `phantom event show <id>`
- **Behavior**: resolves id/prefix, decrypts + prints meta+analysis (phantom.rs:3849).
- **Exit**: 0 · 2 bad/ambiguous/missing id · 1 meta unreadable (encrypted w/o key).
- **Linux**: offline; reads `events/<id>/`; decrypts via `identity.key` when present.
- **Status**: ✅ works (read-only; degrades cleanly w/o key).

### `phantom coach review --date YYYY-MM-DD [--save]`
- **Behavior**: aggregates a date's events → Markdown brief → shame-free lint → Gemini "tomorrow's one action"; `--save` writes encrypted `reviews/<date>.md` (phantom.rs:4725; daily_review.rs:248).
- **Exit**: 0 · 2 wrong action/flag.
- **Linux**: offline event read; LLM **optional** (absent → "(skipped)"); `--save` encrypts iff `identity.key`.
- **Status**: ✅ works.

### `phantom focus start|status|interrupt|stop|history`
- **Behavior**: disk-backed timer `~/.phantom-mesh/focus-session.json`; `stop` writes a `kind=focus` event (focus_session.rs:83).
- **Flags**: `start [--minutes N] [--task S]` (default 25) · `interrupt ["note"]` · `history [--date]`.
- **Exit**: 0 · 2 already-active / not-active / io / unknown sub.
- **Linux**: fully offline, no daemon/LLM; `stop` encrypts event iff `identity.key`.
- **Status**: ✅ works (single-active invariant). ⚠ stale comment at phantom.rs:4189 still calls it a "Stage-4 stub that panics" — false; **Intended:** delete the comment.

### `phantom note "<text>" [--tag T]…`
- **Behavior**: synchronously writes a `kind=note` event (phantom.rs:4488; note_capture.rs:41). Piped/`-` stdin auto-read.
- **Exit**: 0 · 2 no text / capture fail.
- **Linux**: offline, **no daemon/LLM**; encrypts iff `identity.key`; reports `encrypted|plaintext` in the ✓ line.
- **Status**: ✅ works. (`kind=note` projects to wire `Text` → labelled `text` in recall/show.)

### `phantom recall <query> [--kind K] [--since DATE] [--limit N] [--json]`
- **Behavior**: case-insensitive substring search over summary+tags, newest-first (phantom.rs:4573; recall.rs:67).
- **Exit**: 0 (incl. no matches) · 2 search fail.
- **Linux**: offline; reads the **file store** (not the unwired SPEC-16 `events.sqlite` FTS5); decrypts via key; encrypted events **silently skipped** w/o key.
- **Status**: ✅ works (content-scoped, all dates).

### `phantom review [YYYY-MM-DD] [--json]`
- **Behavior**: deterministic offline daily aggregate Markdown; `--json` emits that day's events (phantom.rs:4650; daily_review.rs:177).
- **Exit**: 0 · 2 invalid date.
- **Linux**: offline, **no LLM** (use `coach review` for AI); decrypts via key.
- **Status**: ✅ works (clean split: `review`=one-day deterministic, `coach review`=AI, `recall`=content search).

### `phantom logs [-n N|--tail N] [--since DUR] [--kind K] [--raw]`
- **Behavior**: tails the daemon telemetry `events.jsonl` (cli_config.rs:433).
- **Exit**: 0 (missing file → friendly note, still 0); non-zero on bad `--since`/unknown flag.
- **Linux**: reads `events.jsonl` — a **SEPARATE file** from the `events/` life-log store.
- **Status**: ⚠ naming overlap — users conflate `logs` (telemetry) with `recall`/`review` (life-log). **Intended:** help text must say "system/serve telemetry, not your life-log".

### `phantom data delete (--all --yes | <id>)`
- **Behavior**: deletes one event dir or wipes `events/` (phantom.rs:4110; data_cli.rs:27).
- **Exit**: 0 · 2 missing `--all`/ambiguous id · 1 single-delete error. `--all` without `--yes` = dry-run error.
- **Linux**: offline; preserves `agents.toml`/`identity.key`/`sessions/`; idempotent.
- **Status**: ✅ works (two-flag guard solid).

### `phantom data export [--format json|md] [--out FILE] [--kind K] [--since DATE]`
- **Behavior**: exports matching events oldest-first (phantom.rs:4028; data_cli.rs:145).
- **Exit**: 0 · 2 unknown format/flag.
- **Linux**: offline; reuses `recall::search_events` (decrypt via key; skips encrypted w/o key).
- **Status**: ✅ works.

### `phantom data stats`
- **Behavior**: whole-store rollup (total · by-kind · span · last-7d) (phantom.rs:4092; data_cli.rs:199).
- **Exit**: 0.
- **Status**: ✅ works.

---

## §5 Self-improvement & Hermes

### `phantom evolve [GOAL] [--max-rounds N] [--agent N] [--rebuild] [--deploy] [--distributed] [--judge …]`
- **Behavior**: autonomous dev loop up to `--max-rounds`; stops on `EVOLVE_DONE`/all-green (phantom.rs:1711). Sandbox ON by default.
- **Flags**: `-n/--max-rounds` (10) · `-a/--agent` (master) · `-r/--rebuild` · `-d/--deploy` · `-D/--distributed` · `--allow-core-evolve` · `--judge`/`--ensemble N`/`--extract-skills`/`--extract-threshold N` (all need `experimental-hermes-curator`, else no-op with ⚠).
- **Exit**: 0 · 1 on any error.
- **Linux**: needs **LLM key** + **git**; checkpoints in `evolve-checkpoints/`.
- **Status**: ⚠ partial — `--extract-skills` is a logged **no-op** (unlanded). **Intended:** wire `hermes::extract::extract_skill`.

### `phantom autoevolve [--once|--watch] [--interval N] [--target check|test] [--distributed]`
- **Behavior**: reactive loop — `cargo <target>` pre-check; red → spawn `evolve`; green+queued → dispatch; auto-commit only if post-check green & tree was dirty (phantom.rs:11838).
- **Flags**: `--once`(default)/`--watch` · `--interval` (300s) · `--max-rounds` (5) · `--target` (check) · `--no-commit` · `-D/--distributed`. Subs: `schedule`, `log`, `digest`.
- **Linux**: needs git + LLM key; state under `~/.phantom-mesh/`.
- **Status**: ✅ works.

### `phantom autoevolve schedule [install|uninstall|status]`
- **Behavior**: registers a periodic `autoevolve --once` job (launchd on mac, schtasks on win).
- **Linux**: ❌ **NOT IMPLEMENTED** — the `cfg(not(macos/windows))` stub prints "macOS+Windows only" + a cron one-liner and returns 0 (phantom.rs:13205). No systemd `.timer`.
- **Status**: ❌ drift/stub. **Intended:** write `~/.config/systemd/user/phantom-autoevolve.{service,timer}` (`OnUnitActiveSec=interval`) + `systemctl --user enable --now`, mirroring mac/win.

### `phantom autoevolve log [--n N]`
- **Behavior**: tails `autoevolve.log` JSONL (phantom.rs:12523).
- **Status**: ⚠ partial — uses `date -r <epoch>` (BSD); on Linux `date -r` = file-mtime, so the timestamp column shows `?`. **Intended:** `date -d @<epoch>` on Linux.

### `phantom swarm <PROMPT> [--agent N] [--throttle …]`
- **Behavior**: fan prompt to all online peers + local, then LLM-synthesize (phantom.rs:2655). 300s timeout.
- **Linux**: needs LLM key (local + synthesis) + ≥1 peer for fan-out.
- **Status**: ✅ works.

### `phantom evolve list|replay|handoff` · `evolve goals next|list|add|mark-done`
- **Behavior**: checkpoint inspect/replay/migrate + a Markdown goal queue (`EVOLVE-GOALS.md`).
- **Exit**: `list`/`goals` 0; `replay`/`handoff` bail→1 on missing id/peer.
- **Status**: ✅ works (pure file I/O for `goals`; `handoff` network-dependent).

### `phantom skill run <path> [--dry-run] [--sandboxed --allow <cmd>…]`
- **Behavior**: executes a Hermes Skill Document (phantom.rs:10024).
- **Exit**: 2 usage · 1 step error · **1 if built without the feature**.
- **Linux**: ❌ requires cargo feature **`experimental-hermes-curator`** — default builds print "✗ built without …" and exit 1.
- **Status**: ⚠ partial (feature-gated). **Intended:** enable the feature in Linux release builds, or de-experimentalize.

---

## §6 Cluster

> Auth/HMAC per §1.6. **Peer-source inconsistency**: `peer`/`cluster status` read `agents.toml [cluster].peers`; `dispatch`/`git` read `~/.phantom-mesh/peers.json` (fallback = RFC5737 test IPs, empty caps). **A real drift** — unify the source.

### `phantom cluster [join|sync|status|leave|upgrade]`
- **Behavior**: manages `[cluster]` block + `status` pings each peer `/healthz` (cli_config.rs:1847). Default = `status`.
- **I/O**: output → stderr (no JSON mode).
- **Exit**: 0 · 1 on config/unknown-sub error.
- **Status**: ✅ works. ⚠ `status` uses unauth `/healthz` only → "reachable" ≠ "auth-compatible".

### `phantom peer [list|discover|ping|assign|send-async|poll]`
- **Behavior**: live peer ops via `ClusterManager` (phantom.rs:2247). `discover` shells `tailscale status`/`dns-sd`.
- **Flags** (assign/send): `-a/--agent` · `--caps CSV` · `-t/--target URL` · `--idempotency-key`.
- **Exit**: 0 · `exit(1)` on ping-fail / no-online-peers / assign-poll error.
- **Errors**: `format_dispatch_error` taxonomy (PeerUnreachable / HMACMismatch / AgentMissing / NoPeerSatisfiesCaps / Timeout).
- **Linux**: peers from `agents.toml`; needs ≥1 peer on `serve`.
- **Status**: ✅ works. ⚠ best-peer `assign` ignores `--caps` (only `--target` wires caps).

### `phantom dispatch [--tag X] [--to Y] [--all] [--agent Z] "task"`
- **Behavior**: capability-routed RPC; picks peer from `peers.json` by tag/name (cli_config.rs:658).
- **Flags**: `-t/--tag` (repeatable, AND) · `--to NAME` · `-a/--agent` · `--async` · `--all`.
- **Exit**: 0 · 1 (no prompt / no peers / no tag-match / HTTP non-2xx / missing job_id).
- **Status**: ✅ works. ⚠ tag routing needs `config pull` to populate caps (fallback topology = empty caps + unreachable test IPs).

### `phantom git sync (--all|--to N) [--branch B] [--cwd P]`
- **Behavior**: fans `git pull && rev-parse` to peers via `/rpc/admin/shell` (cli_config.rs:980).
- **Exit**: **0 even when some peers fail** · 1 on no `--all`/`--to`/peers/secret.
- **Status**: ✅ works. ⚠ **exit always 0 regardless of per-peer failures** — bad for CI. **Intended:** nonzero if `ok < total`. (Note: `/rpc/admin/shell` is RCE-equivalent, gated only by HMAC.)

### `phantom node-capabilities [--json]`
- **Behavior**: detects + prints this node's capability report (phantom.rs:5122). Standalone.
- **Status**: ✅ works.

### `phantom worker-setup [--hub URL] [--name N] [--port P] [--token T]`
- **Behavior**: prints capability report + OS-specific install runbook; Linux branch emits a `phantom-mesh-worker.service` systemd unit + steps (phantom.rs:5149).
- **Status**: ✅ works (generator). ⚠ **purely advisory** — prints steps, never installs/registers.

### `phantom coordinator [--host H] [--port 7900] [--ttl 90]`
- **Behavior**: in-memory peer-registry hub; `POST /register`, `GET /peers?secret_hash=` (phantom.rs:1991).
- **Status**: ⚠ partial — `/register` **unauthenticated**, entries **RAM-only** (lost on restart), `secret_hash` filter not constant-time. **Intended:** proof-of-secret on register + optional persistence.

---

## §7 Identity & Setup

> Headless caveat: `login google`/`broker` + `onboarding` need a **browser/display** (`xdg-open`).

### `phantom keys [init|show|path] [--force]`
- **Behavior**: `init` generates `keys/ed25519.{priv,pub}` (priv 0600) + extensions layout (phantom.rs:4896; identity.rs:87).
- **Exit**: 0 · `show` w/o keypair 1 · unknown sub 2.
- **Status**: ❌ drift (§1.3) — `keys init` does **NOT** create `identity.key`, so Life-Track is plaintext out of the box. **Intended:** also generate `identity.key` (64B) here. (Fixed on encryption branch.)

### `phantom login [email|google|apple|broker]`
- **Behavior**: zero-arg probes broker (`PHANTOM_AUTH_URL`|phantommesh.io); up → broker OAuth, down → local menu (phantom.rs:10478).
- **Status**: ✅ router works.
  - **`login email`** ✅ — local-only; SHA-256×100k password; writes `auth.json` 0600; no display/network.
  - **`login google`** ⚠ — native OAuth+PKCE loopback `:48181`; headless Linux has no `xdg-open`/display → won't open + cross-host callback impossible. **Intended:** detect no-`$DISPLAY` → device-code/manual-paste fallback.
  - **`login broker`** ⚠ — same headless limitation; depends on phantommesh.io.
  - **`login apple`** ❌ stub — always exit 1 ("lands when broker exposes /auth/apple").

### `phantom logout` / `phantom whoami`
- **logout**: deletes `auth.json` only (not keys/identity.key); idempotent; exit 0.
- **whoami**: prints identity to stdout; exit 0 even when not logged in.
- **Status**: ✅ works.

### `phantom config pull|push [--token JWT] [--url U]`
- **Behavior**: `pull` fetches provider keys from broker vault (sealed E2EE default-on), writes `broker.json`; `push` uploads sealed vault (cli_config.rs:2870).
- **Exit**: 0 · bail→1 on no-token / `push` with E2EE disabled.
- **Status**: ✅ works (push gated on E2EE by design).

### `phantom init`
- **Behavior**: generates `./PHANTOM.md` scaffold (phantom.rs:5012). Does **not** touch identity/auth/keys despite the name.
- **Status**: ✅ works (name is a mild footgun).

### `phantom onboarding`
- **Behavior**: local axum on `:7878` + opens browser to `/#settings`, polls `agents.toml` mtime (phantom.rs:9489).
- **Status**: ⚠ partial — **headless Linux: hangs forever** (no browser/display to fill the form). **Intended:** detect no-display → print URL + accept a CLI/TUI config path, or abort with guidance.

### `phantom lang [show|set <en|zh-TW>|reset]`
- **Behavior**: UI language pref in `~/.phantom-mesh/lang` (phantom.rs:5312). Only `en` + `zh-TW` (zh-CN rejected).
- **Exit**: 0 · 2 bad value/unknown action.
- **Status**: ✅ works.

---

## §8 Diagnostics & Agents/Sessions

### `phantom doctor [--json|--mesh]`
- **Behavior**: environment self-diagnostic, colored sections (phantom.rs:13872). `--mesh` = per-peer health table.
- **Exit**: plain + `--json` **always 0** (status in the `.status` field); `--mesh` → **2** any peer offline · **1** version-skew · **0** all green.
- **Status**: ✅ works. ⚠ `--json` never surfaces nonzero exit (consumers must read `.status`).

### `phantom selftest [--json] [--out FILE] [--feature N] [--p0-only] [--list]`
- **Behavior**: shim → `scripts/selftest.sh` runs `scripts/selftest.d/*` features (phantom.rs:14696).
- **Exit**: **0** no P0 failures · **1** ≥1 P0 failure (P1/P2 do NOT fail) · **2** orchestrator/usage error.
- **Linux**: needs repo `scripts/` checkout + `bash` (+ python3 optional).
- **Status**: ✅ works (P0-gated by design — green can hide P1/P2 fails).

### `phantom self-update [--source URL] [--dry-run]`
- **Behavior**: downloads target-triple binary, smoke-tests `--version`, atomic swap, restarts service (phantom.rs:11209).
- **Source precedence**: `--source` → `PHANTOM_COORD` → `agents.toml peers[0]` → `127.0.0.1:<port>`; URL = `<coord>/dist/phantom-<triple>`.
- **Exit**: 0 / dry-run · 1 unsupported triple / download / bad-binary.
- **Linux**: installs into the **running binary's dir** (needs it writable — `/opt` or `/usr/local` fail without privilege); restarts `phantom-mesh.service` if active.
- **Status**: ✅ works. ⚠ system-wide installs need privilege for the rename.

### `phantom debug [--tail N]`
- **Behavior**: single redacted diagnostic bundle → stdout (cli_config.rs:208). Secrets auto-redacted.
- **Status**: ✅ works. (Help example pipes to Windows `Set-Clipboard`; on Linux use `xclip`/`wl-copy`.)

### `phantom providers [list|priority <agent> …]`
- **Behavior**: shows/sets provider failover order in `agents.toml` (cli_config.rs:3333).
- **Status**: ⚠ partial — **all output goes to stderr** (nothing to stdout), so `phantom providers list | …` gets empty — inconsistent with `models`/`debug`. **Intended:** list → stdout.

### `phantom models [status|refresh|test]`
- **Behavior**: model cache status / re-fetch `/v1/models` / probe real tool-call support (cli_config.rs:3593). Cache `models-cache.json`.
- **Status**: ✅ works (same stderr caveat).

### `phantom sessions`
- **Behavior**: queries broker for live TUI sessions across the mesh → stdout (cli_config.rs:1380).
- **Exit**: 0 · non-zero on not-logged-in / no broker token / network.
- **Status**: ✅ works (needs `login` + broker).

### `phantom workspace [show|set <dir> [agent]|clear]`
- **Behavior**: per-machine pin in `agents.toml [workspace]` (cli_config.rs:1412); affects bare `phantom` auto-cd + agent.
- **Status**: ✅ works (output to stderr caveat).

### MAC-ONLY (N/A on Linux)
`phantom snapshot` (APFS/`tmutil`) and `phantom mlx` (Apple MLX) are `cfg(target_os="macos")`; on Linux the `not(macos)` arms print "requires Apple Silicon" / "macOS-only" and `exit(1)`.

---

## §9 Drift & gaps — prioritized fix list

Legend: ✅ fixed (branch noted) · ⏳ DONE-pending-operator-merge · 🔵 DECISION NEEDED (operator) · ⚠️ partial.
All fixes are on `wip/z13-wsl/*` branches, rebased on current main, **not yet merged to main**.

| # | Sev | Command/area | Drift | Status / fix |
|---|---|---|---|---|
| D1 | ❌ P0 | encryption (§1.3) | `identity.key` never provisioned → all Life-Track plaintext | ⏳ **DONE `wip/…task-2026052902`, needs operator merge** (auto-provision at `keys init` + `serve` boot; verified e2e) |
| D2 | ❌ | `autoevolve schedule` (Linux) | stub — no systemd timer | ✅ **DONE `wip/…task-2026053004`** — systemd `--user` `.service`+`.timer` install/uninstall/status |
| D3 | ❌ | `login apple` | stub, exit 1 | 🔵 **DECISION**: (A) implement via broker `/auth/apple` relay [needs the broker endpoint built first] vs (B) drop `apple` from the login menu + document unsupported until broker ships. *Rec: B now, A when broker has the endpoint.* |
| D4 | ⚠ | `skill run` | feature-gated off in default builds | 🔵 **DECISION**: (A) enable `experimental-hermes-curator` in Linux **release** builds so `skill run` ships vs (B) keep gated + make the error point to a build flag. *Rec: A iff Hermes skills are a shipping v0.6 feature; else B. Couples with D16.* |
| D5 | ⚠ | headless OAuth/onboarding | `login google|broker`, `onboarding` hang/fail with no display | ✅ **DONE `wip/…task-2026053003`** — `open_browser` detects no-`$DISPLAY`, prints URL + manual/`login email` guidance; loopback callers surface the headless escape-hatch |
| D6 | ⚠ | port hardcoding | `service status` probe `:7878` ignoring `--port`/`PHANTOM_PORT` | ✅ **DONE `wip/…task-2026053001`** — `resolve_serve_port()` (PHANTOM_PORT > `[core].port` > 7878). (self-update already used `<core.port>`) |
| D7 | ⚠ | `git sync` exit code | always 0 even on per-peer failure | ✅ **DONE `wip/…task-2026053005`** — bails nonzero when `ok_count < peers.len()` |
| D8 | ⚠ | peer-source split | `peer`/`cluster` use `agents.toml`; `dispatch`/`git` use `peers.json` (RFC5737 fallback) | ⚠️ **PARTIAL `wip/…task-2026053006`** — dispatch now warns on the unreachable-placeholder fallback. 🔵 **DECISION** on full unification: (A) one source (peers.json) for all, drop the RFC5737 bootstrap defaults vs (B) keep two sources (live agents.toml + broker-synced peers.json) documented. *Rec: B + the warning (placeholders serve `cluster join` bootstrap).* |
| D9 | ⚠ | `logs` vs `recall`/`review` | naming conflation (telemetry vs life-log) | ✅ **DONE `wip/…task-2026053010`** — help clarifies `logs` = serve telemetry, not the Life-Track log |
| D10 | ⚠ | `providers`/`models`/`workspace` | list output to stderr (unpipeable) | ✅ **DONE `wip/…task-2026053009`** — read-only listings → stdout |
| D11 | ⚠ | `mcp` / `serve` banners | "40 tools" / "+2 cluster RPC" miscounts | ⚠️ **PARTIAL `wip/…task-2026053008`** — dropped hardcoded "50 tools" from `--help`. 🔵 **CONFIRM**: is the serve banner `+2 cluster RPC` a true double-count (cluster_* tools already in `all_tool_names()`) or a legit RPC-endpoint label? If double-count, drop it. |
| D12 | ⚠ | `autoevolve log` | `date -r` (BSD) → `?` timestamps on Linux | ✅ **DONE `wip/…task-2026053007`** — pure chrono `fmt_local_epoch()`, 4 sites |
| D13 | ⚠ | bare `phantom` headless | late generic "needs a terminal" error | ✅ **DONE `wip/…task-2026053002`** — early non-TTY guard → exit 2 + repl/exec/serve hint |
| D14 | ⚠ | `coordinator` | unauth `/register`, RAM-only | 🔵 **DECISION**: (A) require proof-of-secret on `/register` + optional on-disk persistence [harden for production] vs (B) document `coordinator` as dev/bootstrap-only (in-memory, unauth) and steer prod to the broker. *Rec: B unless `coordinator` is meant for production registry use.* |
| D15 | 🧹 | `focus` | stale "Stage-4 stub panics" comment (false) | ✅ **DONE `wip/…task-2026053008`** — comment corrected (focus is implemented) |
| D16 | ⚠ | `extract-skills` | logged no-op | 🔵 **DECISION**: (A) wire `hermes::extract::extract_skill` (needs the `experimental-hermes-curator` feature — couples with D4) vs (B) hide the `--extract-skills` flag until the curator ships. *Rec: tie to D4 — same feature gate.* |
| D17 | ⚠ | desktop `safeInvoke` (task-2026052904) | 6 daemon-proxied commands (`daemon_status`/`start_daemon`/`list_conversations`/`get_conversation_history`/`reset_conversation`/`get_peers`) have NO native Tauri handler; `safeInvoke` (tauri-compat.ts) uses **native** invoke in the packaged desktop app with no HTTP cross-fallback → those reject at runtime (conversation/daemon/peers views break in the .deb). NOT covered by main's `scripts/qa/tauri-cmd-lint.ts` (that lints Rust command *shape*, not invoke-parity). | 🔵 **DECISION (frontend owner)**: (A) `safeInvoke` falls back to `httpFallback` when the native invoke is unregistered [contained, but a cross-platform behavior change — can't verify headlessly here] vs (B) register native proxy handlers for the 6 commands [explicit, more code]. *Rec: A (matches browser-mode behavior); needs a desktop-runtime test to confirm.* |

**Operator decisions outstanding:** D1 (merge), D3, D4, D8-unify, D11-confirm, D14, D16, **D17**. Everything else (D2/D5/D6/D7/D9/D10/D12/D13/D15) is ✅ done on its branch, pending merge.

---

## §10 Staged execution plan (build → debug, one stage per cycle)

> **Status (2026-05-30, autonomous loop):** S1 ⏳ done-pending-merge · **S2 ✅ · S3 ✅ · S4 ✅** (D8 partial) · **S5 ✅** · S6 = decisions written up in §9 (await operator) · **S7 next** (Linux test coverage). 12 fixes on `wip/z13-wsl/*`, rebased on main, unmerged.

Each stage = a `wip/z13-wsl/*` branch, full DEV-QUALITY-LOOP (develop → test ≥2 → verify ≥2 → fix), no main push without confirmation. Ordered by user impact:

- **S1 — Land encryption (D1)**: merge/rebase `task-2026052902` so `identity.key` provisioning + at-rest encryption is real. Acceptance: fresh `keys init`/`serve` → captured events are `age-…` on disk; `event show`/`recall` round-trip.
- **S2 — Headless dignity (D5, D13, D6)**: no-`$DISPLAY` fallbacks for `login google|broker`/`onboarding`; early non-TTY guidance for bare `phantom`; port-aware `service status`/`self-update` probe. Acceptance: every command on a headless WSL box either works or fails with actionable guidance — never hangs.
- **S3 — Linux systemd parity (D2)**: implement `autoevolve schedule` on Linux (`.timer` unit). Acceptance: `schedule install` → `systemctl --user list-timers` shows it; fires `autoevolve --once`.
- **S4 — Cluster correctness (D7, D8)**: `git sync` nonzero-on-failure; unify peer source. Acceptance: CI can gate on `git sync`; `peer` and `dispatch` see the same peers.
- **S5 — UX/consistency polish (D9, D10, D11, D12, D15)**: stdout for listings, help clarifications, banner counts, `date -d`, stale comment. Acceptance: `phantom providers list | jq` works; docs match output.
- **S6 — Feature decisions (D3, D4, D14, D16)**: operator calls on `skill run` feature-gate, `login apple`, coordinator hardening, `extract-skills` — spec'd, then built.
- **S7 — Coverage**: author `core/tests/v5_smoke_linux.rs` (SPEC-60) + extend `v4_e2e_desktop_linux.rs` to exercise the above acceptance checks as real Linux e2e.

> **This doc is the contract.** Future Linux work targets a §9 row or a §10 stage; anything not
> here is out of scope until added. Update §9/§10 as items land (✅) or new drift is found.
</content>
