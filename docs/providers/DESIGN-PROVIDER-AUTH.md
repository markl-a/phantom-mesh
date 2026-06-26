# DESIGN — Provider subscription auth (Claude / ChatGPT / Gemini)

Status: **proposal / evaluation** (2026-06-15). Not yet implemented.
Scope: improve how the CLI signs in to LLM **subscriptions**, borrowing OpenClaw's
sanctioned approach where it's genuinely better. Account login (phantom's own
Google/Apple/email via the broker) is OUT of scope — it already works.

## 1. Current state (what phantom does today)

All subscription sign-in is **detection-only** — phantom reuses tokens the
official CLIs cached; it does NOT run its own OAuth and does NOT refresh:

| provider | what phantom reads | file | requires |
|---|---|---|---|
| Claude | `~/.claude/credentials.json` → `accessToken`/`access_token`/`sk-ant-*` | `core/src/providers/claude_cli.rs` | user ran `claude` login first |
| ChatGPT (Codex) | codex auth.json → `tokens.{access_token,refresh_token,account_id}` | `core/src/providers/codex_cli.rs` (`CodexAuth`) | user ran `codex` login first |
| Gemini | `GEMINI_API_KEY` env | `core/src/providers/credential_scanner.rs` | user set the env var |

Onboarding Step 2/4 (`run_first_time_onboarding`) only *detects* these; if none
are present it falls to the free plugin / paste-key / Ollama.

## 2. OpenClaw reference (verified 2026-06-15)

- **OpenAI/ChatGPT/Codex**: OpenClaw runs its OWN OAuth — PKCE + state → browser
  `auth.openai.com/oauth/authorize` → loopback `http://127.0.0.1:1455/auth/callback`
  (or paste the redirect URL) → exchange at `auth.openai.com/oauth/token` → store
  `{access_token, refresh_token, account_id, expires}`; **auto-refresh** under a
  file lock. **OpenAI explicitly sanctions** subscription OAuth in external tools
  (so this is NOT impersonation). `openclaw models auth login --provider openai`.
- **Claude**: two paths — (a) paste a setup-token, (b) reuse the local Claude CLI
  login (== phantom today). Anthropic stance: "allowed again", conditional.
- **Gemini**: OpenClaw does not implement subscription OAuth → no reference.

Sources: docs.openclaw.ai/concepts/oauth, /providers/openai.

## 3. What phantom already has to reuse

- `core/src/oauth.rs` — **core-side PKCE OAuth**: `gen_random_b64` + SHA-256 S256
  challenge, `google_start_url(daemon_port)` / `apple_start_url(...)` (build the
  authorize URL), `handle_callback(code, state)` (token exchange with
  `code_verifier`), `get_result()`. Callback is served by the running daemon
  (`phantom serve`) at its port. → directly adaptable to OpenAI endpoints.
- `core/src/providers/codex_cli.rs::CodexAuth` — already models
  `{access_token, refresh_token, account_id}` (just reads them today).
- `core/src/keys.rs::set_api_key` — escaped, never-string-built secret persistence.

So #1 below is mostly: add OpenAI start/exchange URLs + a callback capture +
write the minted tokens where `codex_cli` already reads them.

## 4. Proposed changes

### #1 — ChatGPT/Codex: add sanctioned in-process OAuth  (highest value)
**Why**: removes the "you must install + sign into the official `codex` CLI
first" requirement; OpenAI sanctions it.

- Add to `oauth.rs` (or a new `oauth_openai.rs`): `openai_start_url()` (PKCE,
  `auth.openai.com/oauth/authorize`, `redirect_uri=http://127.0.0.1:<port>/auth/callback`)
  and `openai_exchange(code, verifier)` → `auth.openai.com/oauth/token` → parse
  `{access_token, refresh_token, id_token}`; extract `account_id` from the access
  token (same as `codex_cli` does).
- Callback capture: reuse the daemon callback path (preferred — serve is already
  spawned in onboarding Step 1) OR a one-shot loopback listener on a fixed port
  with a **paste-the-redirect-URL fallback** (OpenClaw offers both; the paste
  fallback covers headless / no-browser).
- Persist tokens where `codex_cli::find_codex_auth` reads them (so the runtime
  path is unchanged) — i.e. write a phantom-owned `codex`-shaped auth.json, or a
  new `~/.phantom-mesh/openai_oauth.json` that `codex_cli` also checks.
- Onboarding Step 2/4: add a "Sign in with ChatGPT" option next to detect/paste.

### #2 — Claude: add a paste-setup-token path  (low effort)
- Besides detection, let the user paste a Claude setup-token; store it in the
  same `credentials.json` shape `claude_cli::extract_claude_token` already parses.
- Detection stays the default when the official CLI login is present.

### #3 — Subscription token auto-refresh  (correctness)
- Today phantom reads a cached token and never refreshes → long sessions can 401
  mid-run. Add a refresh helper: on use, if `expires` is past, POST the
  `refresh_token` to the provider token endpoint under a file lock, rewrite the
  stored creds. `CodexAuth` already carries `refresh_token`.

## 5. Where it plugs in
- Onboarding Step 2/4 (`run_first_time_onboarding`, talk-first) gains, per
  provider: **detect → [Sign in (OAuth)] → paste token/key** options.
- Runtime: unchanged — `claude_cli`/`codex_cli` keep reading the same files; the
  refresh helper sits in the read path.

## 6. Risks / non-goals
- **Claude own-OAuth: NOT proposed.** Anthropic's stance is only "CLI reuse
  allowed"; minting our own Claude subscription OAuth (official client_id) is the
  ToS-gray/fragile path. Stick to detection + paste-token for Claude.
- **OpenAI OAuth client_id**: confirm which client_id is sanctioned for
  third-party use (OpenClaw uses one; we must use the sanctioned public flow, not
  scrape the official codex binary's secret).
- **Gemini**: no subscription OAuth (none sanctioned/standard); keep env/API-key
  + the free plugin. A separate Google-OAuth-for-Gemini-API is its own item.
- Secrets only ever via `keys::set_api_key` / 0600 files, never logged.

## 7. Effort + sequencing (rough)
1. **#3 refresh** (S, isolated, immediate correctness win) — refresh helper +
   wire into `codex_cli`/`claude_cli` read path.
2. **#2 Claude paste-token** (S) — one onboarding option + reuse the parser.
3. **#1 ChatGPT OAuth** (M) — OpenAI start/exchange + callback capture + persist
   + onboarding option. Biggest UX win; do after #3 so refresh covers it.

## 8. Open questions for the owner
- Confirm the **sanctioned OpenAI OAuth client_id** to use (vs reading codex's).
- Callback capture preference: **daemon callback** (already running) vs a
  **fixed loopback port + paste fallback** (more like OpenClaw, headless-safe)?
- Do we want #1 in **CLI only**, or also wire the GUI/Tauri onboarding (which
  already has its own loopback OAuth for the account)?
