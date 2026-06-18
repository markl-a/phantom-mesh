# Roadmap

_Status snapshot — 2026-06-19._

This is the single source of truth for project status. It is intentionally
honest: "Shipped" means there is real, working code (most of it covered by
tests); "In progress" means partially built; "Planned" means not started yet.

Current public release line: **v0.6.0-rc.1** (release candidate). The project
is exploratory and not yet considered stable — see the README warning before
relying on it.

For a detailed, feature-by-feature honesty matrix, see
[`docs/FEATURE-MATRIX.md`](docs/FEATURE-MATRIX.md). For the historical change
log, see [`CHANGELOG.md`](CHANGELOG.md) and
[`RELEASE-NOTES.md`](RELEASE-NOTES.md).

---

## Shipped

Working today, exercised on the maintainer's machines:

### Agent runtime
- Conversational REPL (`phantom`) with streaming output, inline tool calls,
  multi-line input, slash commands, `@file` / `@image` inlining, and a planning
  mode that gates tools until you confirm.
- Multi-provider LLM routing with automatic failover across the supported
  providers, token-aware context compaction, and live cost tracking.
- A tool set exposed over the agent loop and over MCP (read/write/shell/grep/
  fetch and mesh operations), with stall detection in the agentic loop.

### Interfaces
- `phantom mcp` — an MCP stdio server usable as a drop-in subagent for any MCP
  client (Claude Code, Cursor, etc.).
- `phantom serve` — a WebSocket JSON-RPC daemon plus an embedded web dashboard
  at `http://localhost:7878` (status bar, terminal, tasks, settings).
- `phantom onboarding` — a browser-based first-run setup wizard, plus a
  terminal onboarding wizard when no agent config is found.
- A Tauri desktop app (macOS, Linux, Windows) and mobile shells (Android, iOS).

### Mesh / cluster
- A peer-to-peer compute mesh over HTTP with HMAC-SHA256 authentication.
- An async job queue for cross-node task dispatch, with capability-aware peer
  scoring and a tested rule-based task decomposition and result-integration
  brain.
- `phantom cluster status` / `phantom cluster peers` for inspecting the mesh.
- Local-network peer discovery groundwork (mDNS service advertise/browse).
- Persistent per-conversation history.

### Life track (capture → coach)
- A capture pipeline for food / focus / habit events, with an optional attached
  image analyzed through a multimodal provider on the daemon path.
- A focus-session lifecycle (disk-backed timer).
- A daily-review aggregator and coach engine with a deterministic markdown
  report, a shame-free / medical-disclaimer lint, and a stats-only degraded
  path when no LLM is available.
- Cross-platform scheduler generation (launchd / systemd / scheduled tasks).

### Skills (Hermes)
- A local skill bank backed by SQLite with full-text (FTS5) keyword recall;
  judge → extract → store steps are real and tested.

### Encryption & storage
- Client-side crypto primitives: age v1 file encryption, ed25519 device
  identity, and HKDF-derived per-purpose subkeys (well covered by tests).
- An event store (SQLite + FTS5) with at-rest encrypted blobs.
- A broker vault design for holding third-party OAuth tokens, with client-side
  seal/unseal primitives.
- Linux Secret Service keystore integration for secrets at rest.

### Security hardening
- Authenticated cluster RPC by default (no empty-secret fallback in normal
  operation; a single-release migration escape hatch is logged loudly when used).
- Same-origin CORS on the dashboard by default.
- SSRF protections on the fetch tools (loopback / private-range URLs blocked
  unless explicitly allowed).
- Workspace-bound filesystem paths, a shell-command blocklist, path-traversal
  protection, and constant-time HMAC comparison.

### Integrations & delivery
- A Telegram bot channel with a user allowlist.
- `PHANTOM.md` / `AGENTS.md` project-context files auto-loaded from the working
  directory.
- Cross-platform pre-built binaries and GitHub Actions CI / release workflows.

---

## In progress

Real code exists but it is partial, stubbed on one path, or not yet covered by
behavioural tests:

- **Multimodal capture on mobile.** The daemon image path works; mobile camera /
  audio / widget capture UIs are not built yet, and audio capture / ASR is
  absent.
- **The full skill loop.** Keyword recall ships; embedding-based semantic recall,
  the measure/feedback step, and cross-node skill sync are not finished.
- **At-rest keystore coverage.** Crypto primitives are strong, but native OS
  keystore binding is only complete on Linux; macOS / Windows / Android / iOS
  fall back to encrypted-file storage.
- **Cluster dispatch surfaces.** The dispatch brain is tested, but it has no
  end-user CLI / app surface, no cost tracking, and request idempotency is not
  yet implemented.
- **LLM-based task decomposition.** DAG validation and prompt building are real
  and tested; the end-to-end LLM decompose flow is not yet wired to a live
  provider on every path.
- **Onboarding flow internals.** The state-machine table is real; the
  advance / rollback transitions and a 30-second-to-first-result budget are not
  finished.
- **Integration test coverage.** Vault, event-encryption round-trip, and
  multimodal pipeline have integration tests; cluster discovery, peer
  registration, daily review, and onboarding are still being covered.

---

## Planned

Not started, or deferred to a later release:

- Native OS keystore binding on macOS, Windows, Android, and iOS (secure-enclave
  backed where available).
- Additional LLM providers beyond the three primary ones tested today.
- Embedding / semantic recall for the skill bank.
- Release automation for the mobile app stores (currently manual upload /
  sideload).
- A browser dashboard aimed at non-CLI users.
- A multi-user / household shared mesh with per-user encryption boundaries.
- Watch-surface and third-party extension capture interfaces.

---

> Found something that disagrees with this page? The honest per-feature
> breakdown in [`docs/FEATURE-MATRIX.md`](docs/FEATURE-MATRIX.md) is the
> detailed reference; please open an issue if code and this page diverge.
