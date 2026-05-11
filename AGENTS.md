# Phantom Mesh — Agent Guide

**Last updated:** 2026-04-27
**Status:** active, single source of truth for cross-tool development

If you are an automated coding assistant (Claude Code, Codex, Cursor, OpenCode, Gemini, etc.) picking up work in this repo, **read this file first**. It is intentionally tool-neutral.

---

## 1. Source of Truth

Read in this order:

1. **`_planning-audit/MASTER-PLAN.md`** — strategic plan: goals, scope, 4-day sprint, post-release roadmap. **Only modify this file** for strategic changes.
2. **`SESSION_RESUME.md`** — tactical state: cluster status, today's progress, next concrete steps. **Update this file** every session.
3. **`docs/ARCHITECTURE.md`** — high-level architecture (still valid, predates planning consolidation).
4. **`PHANTOM.md`** — quick architecture sketch.
5. This file (`AGENTS.md`) — cross-tool conventions.

> **Do not** read `_planning-audit/archived/` unless you are doing historical research. It contains 130+ legacy planning docs that have been superseded.

---

## 2. Product Definition (locked 2026-04-27)

Phantom Mesh v0.1.0-alpha is:

> An open-source AI agent runtime that:
> 1. Runs **standalone** as a Claude Code-style REPL (`phantom`)
> 2. Acts as a **subagent** for Claude Code / Codex via MCP (`phantom mcp`) and WebSocket (`phantom serve`)
> 3. Forms a **mesh** across 8 user devices (Mac / 3× Win / 2× Android / 2× iOS)
> 4. Provides a **shared web frontend** (HTML+JS) embedded by `phantom serve` and wrapped by Tauri on mobile

**Open-source target:** 2026-05-01.

---

## 3. Repo Boundaries

### `core/` — main Rust crate
Owns: runtime spine, providers, tools, MCP, mesh, serve, agent loop, REPL.
- `core/src/bin/phantom.rs` — CLI entry (REPL + subcommands)
- `core/src/agent.rs` — agent execution loop with `AgentEvent` streaming
- `core/src/serve.rs` — HTTP/WebSocket server
- `core/src/mcp.rs` — MCP stdio server (40 tools)
- `core/src/tools/` — capability implementations
- `core/src/providers/` — LLM backends (Anthropic, OpenAI-compat, Gemini)

### `crates/pm-types/` — shared type contracts
Used by `core/` and `app/src-tauri/`.

### `app/src-tauri/` — Tauri shell (desktop + mobile)
Wraps the shared web frontend. Owns OS integration, command bridge, sidecar binaries (in `binaries/`, populated by `build.rs`).

### `app/src/` — web frontend (TypeScript + React + Tailwind)
Operator UI. Same codebase loaded by Tauri on iOS/Android.

### `configs/` — agent configuration templates
Per-device starting points (`agents.coordinator.toml`, `agents.worker.toml`, etc.).

### `scripts/` — public tooling
See `scripts/README.md`. `scripts/dev/` is for internal maintenance.

### Do not build new product paths on
- `apple-oauth-relay/`
- `app/src/pages/legacy/`
- `src/clawtex/`

Mobile target = `app/src-tauri/` only. Earlier Expo prototype is archived under `_planning-audit/archived/legacy-expo-mobile/`.

---

## 4. Architecture Principles

1. **Contract first** — define types in `crates/pm-types/` before implementing.
2. **Replaceable capabilities** — providers, tools, channels are interchangeable families with registries.
3. **Thin runtime spine** — `core/` orchestrates; surfaces (CLI, web, Tauri) consume the same contracts.
4. **Surface-neutral behavior** — no logic that only works in browser fallback mode.
5. **Subagent-first UX** — Claude Code / Codex are first-class consumers. Match their interaction patterns where reasonable.

---

## 5. Current Sprint Focus (2026-04-27 → 2026-05-01)

See `_planning-audit/MASTER-PLAN.md` §5 for the day-by-day plan. Summary:

- **Day 2 (4/28)**: Upgrade `phantom` REPL to Claude Code style — multi-line input via rustyline `Validator`, streaming via `run_with_callbacks`, visible inline tool calls.
- **Day 3 (4/29)**: Build single-page web frontend (`core/web/`) with dashboard + xterm.js terminal panel; refresh Tauri Android/iOS apps to load it.
- **Day 4 (4/30)**: Company Mac dogfooding (software + data science tasks), API key rotation, `git filter-repo`, cross-platform binaries, CI green.
- **Day 5 (5/1)**: Tag `v0.1.0-alpha`, push public.

---

## 6. Handoff Discipline

Before ending a session:

1. Update `SESSION_RESUME.md` with: what changed, what's blocked, next concrete step.
2. If strategy changed, update `_planning-audit/MASTER-PLAN.md` (do **not** create new sprint/TODO/freeze docs).
3. Run `cargo check` in `core/` if Rust code changed.
4. Do not commit unless the user explicitly asks.

---

## 7. Guardrails

- Do not create new top-level `*.md` planning files at repo root. All planning lives in `_planning-audit/`.
- Do not re-introduce archived FREEZE / SLICE / SPRINT / TODO docs. They were retired 2026-04-27.
- Do not commit secrets. `agents.toml` and `.env*` are gitignored. New API keys go in `~/.phantom-mesh/env`.
- Do not push to `main` without an explicit user instruction.
- Do not bypass `git filter-repo` plan for the 5/1 release. The current history contains API keys (commits `3abf406`, `0d5c714`) that must be removed before the repo goes public.

---

## 8. Parallel Work / Multi-Session Discipline

Two coding sessions on the same feature space will collide. Use these rules.

### Filesystem isolation (hard requirement)

- **Never run two assistant sessions in the same working directory.** They share `.git/index`, `target/`, `node_modules/`, and lockfiles — the second writer silently overwrites the first. This is data loss without warning.
- For parallel work, use `git worktree` — one worktree per feature/platform. Worktrees share `.git/objects` (cheap) but get independent index, build artifacts, and checkout.

### Worktree convention

- Worktree directory: `.worktrees/` (gitignored). One subdirectory per branch.
- Branch naming: `feat/<topic>` (e.g. `feat/windows`, `feat/android`).
- Setup: `git worktree add .worktrees/<topic> -b feat/<topic> phase1-r1-foundations`
- Per-worktree first run: `npm install` in `app/`, then `cargo build` in the relevant crate. Each worktree has its own `target/` and `node_modules/`.
- Cleanup when done: `git worktree remove .worktrees/<topic>` (after PR merged).

**Windows-only gotcha:** Cargo's `target/` inside `.worktrees/<topic>/` is frequently locked by Windows Defender / real-time AV scanning newly-emitted `build-script-build.exe` files, surfacing as `存取被拒 / Access Denied (os error 5)` during `cargo check`. Workaround: point `CARGO_TARGET_DIR` outside `.worktrees/`, e.g. `CARGO_TARGET_DIR=D:/tmp/phantom-windows-target cargo check`. Permanent fix: add `D:\tmp\` (or your chosen target dir) to Defender exclusions.

### Hot files (expect merge conflicts)

Two parallel sessions editing these will require manual merge resolution at PR time:

- `core/src/bin/phantom.rs` — `#[cfg(target_os = "...")]` blocks scattered through one file
- `core/src/platform/mod.rs` — single source of truth for platform branching
- `core/Cargo.toml`, `app/src-tauri/Cargo.toml` — target-specific deps
- `app/src-tauri/tauri.conf.json` — desktop + mobile bundle config
- `app/src-tauri/capabilities/*.json` — platform ACL
- `Cargo.lock` (× 2: `core/Cargo.lock`, `app/src-tauri/Cargo.lock`)
- `app/package.json` / `app/package-lock.json`
- `.github/workflows/*.yml` — CI matrix

If a session must touch one of these files, finish + merge before the other session starts on it. If both sessions need it, coordinate manually.

### Integration flow

1. Each worktree branch ships its own PR back to `phase1-r1-foundations`.
2. Resolve hot-file conflicts at merge time — do not share state across live sessions.
3. After both branches merged, delete the worktrees (`git worktree remove`) and the remote feature branches.

### Destructive ops require explicit user confirmation

Never run these without the user explicitly asking, even in auto mode:

- `git push --force` / `--force-with-lease` on any shared branch
- `git tag -d <tag>` + `git push origin :refs/tags/<tag>` (rewriting public tags)
- `git reset --hard` on commits already pushed
- `git stash drop` / `git stash clear` (may belong to another session)
- `git worktree remove --force` while another session has uncommitted work in it

---

## 9. One-Line Summary

Phantom Mesh is an open-source AI agent runtime — Claude Code-style REPL + MCP/WS subagent + Tauri-wrapped web frontend — running as a mesh across 8 user devices. Ship 2026-05-01.
