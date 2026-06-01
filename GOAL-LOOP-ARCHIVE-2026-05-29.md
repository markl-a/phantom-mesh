# Goal-Loop Archive — Mac app build loop (2026-05-29)

> Snapshot of the autonomous `/loop` + `/goal` operating procedure as run on the
> **mac** host (Claude Code) during the v0.6.0 Life-Track app build-out, plus the
> session work record. Saved for later reuse / handoff. The loop + goal are being
> **paused** after this snapshot (per operator request).

---

## 1. The `/loop` directive (verbatim)

```
每十分鐘檢查 mac 版本 phantom-mesh AI terminal 是否運作，沒運作就同步最新版本；
這次要先依 BIG-GOAL→spec→wireframe→mockup→prototype 設計鏈設計其中提到的功能，
再把 mac 版本所有功能/介面/能做的都做好並 commit + push
```

Standing operator mandates layered on top (from this + prior sessions):
- **「反正你就不斷地補完缺口跟功能 拜託了」** — continuously, autonomously fill
  gaps + build features without asking for direction each iteration.
- **「要從登入到使用的每個狀況跟場景都完整的測試跟開發」** + **「TUI跟app也都要測試一遍」**
  — test/develop every login→usage scenario across CLI + TUI + app.
- **「你需要跑實際的cli跟app去測試」** — verify by running the real CLI/app, not
  just web mode.
- **Dispatch**: distribute dev/test work across antigravity / gemini / codex /
  opencode / claude-subagent, then Claude re-reviews + fixes.
- **Language**: Traditional Chinese by default; code / commit messages in English.

## 2. The `/goal` anchor (summary)

Full definition lives in `.claude/commands/goal.md`. Essence: 7 phantom-* projects
public + résumé-ready (gate met 7/7), then a 5-platform mesh (macOS/iOS/Windows/
Linux/Android) with any-node prompt fan-out (proven). Operational principles that
**always** apply: **shame-free · consent-gated · reversible**. Handoff discipline:
every meaningful change updates HANDOFF docs + memory, commit + push immediately.

## 3. "This version of the goal loop" — operating methodology

The cadence I converged on, one iteration per ~10-min tick:

1. **Sync** — `git fetch` → `git pull --rebase`. This Mac (`mac`) and `z13`
   (claude-code-z13-wsl) both push to shared `main`; z13 commits rapidly, so each
   push usually needs a rebase-then-push retry. No real file overlap in practice
   (z13 = CLI/TUI/core; mac = app/iOS), so rebases stay clean.
2. **Health** — `phantom selftest` (binary works) as the "AI terminal 運作" check.
3. **zh-TW canary** — when z13 touches TUI/i18n, run
   `cargo test --lib tui_render_tests` on this LANG=zh_TW Mac. It catches
   locale-fragile assertions z13's English CI passes silently (CJK cells get
   space-padded: `追 蹤 2` ≠ `追蹤 2`; fix with
   `txt.split_whitespace().collect::<String>()` + no-space forms).
4. **Pick the next gap** via the design chain (BIG-GOAL → SPEC → wireframe). Prefer
   a self-contained, verifiable, low-risk unit (≤ one feature/iteration).
5. **Build** — app-side preferred (Tauri command + ts-rs + React surface). Reuse
   tested core (e.g. `EventStore`, `daily_review`) over re-implementing.
6. **Verify (real-execution)** — `cargo check` (core + app-tauri), `tsc`, a unit
   test for new commands, a CLI run when a shared core fn is involved, and a
   web-mode Playwright render (0 genuine JS errors; web CORS noise is benign).
7. **Commit + push** — split logical commits; **push to main is gated** (CLAUDE.md
   forbids unconfirmed main pushes) → confirm per push, rebase, retry.

Build-env gotchas learned: the MCP server contends on `target/debug/.cargo-lock`,
so `cargo test` in the app crate can stall for minutes; the crate lives in `core/`
(no root Cargo.toml) so cargo must run from `core/` or `app/src-tauri/`.

## 4. Session work record (2026-05-29, mac)

App Life-Track (P2) surface brought to comprehensive parity with the CLI/TUI:

| Commit | What |
|---|---|
| `208275e7` | coord: flag z13 `read_event` broken on real events (task-2026052901) |
| `779c8ff4` | feat(app): event-detail modal — tap a timeline event to see its analysis |
| `aac9f2b9` | feat(app): delete a single event from the timeline (reversibility) |
| `d1f9703b` | fix(app): CoachReviewReader loading skeleton + error retry (§10.13) |
| `7e80100b` | fix(app): complete SPEC-41 S1 menu-bar dropdown (dead item + rows) |
| `9eb49622` | feat(app): coach review generation — Tomorrow's one action |
| `85f798b8` | fix(app): habit check-in creates the chip first (fresh users) |
| `d0e4914b` | feat(app): quick note capture on the timeline |
| `283ee820`/`e078910c`/`7d17ab2f` | feat(app): data export (JSON/MD) + filters + folder + clean summaries |
| `4e586a7b` | feat(app): Dashboard life-log stats card |
| `e5fda87a` | feat(app): Recall (回想) content-search |
| `0ccd453f` | fix(tui): locale-robust /habits assertions (zh-TW canary) |
| `9d06d2fe` | fix(providers): infer provider from model-name prefix |
| `a235adfb`/`b136a302` | feat(cli): phantom food / habit shortcuts |
| `630c4e63`/`b22d84a7`/`e8e97359` | feat(app): Identity & Privacy tab + cross-surface focus unification |
| `3c50a0a8` | feat(app): macOS menu-bar Life-Track quick actions |

**App Life-Track surfaces now complete**: capture (food/focus/habit/note) ·
timeline + event-detail modal · daily review + coach-generation · recall · stats ·
export · per-event delete · identity/privacy · menu-bar quick actions.

**Final health review (2026-05-29)**: `phantom selftest` → **71 pass / 0 fail**;
zh-TW TUI canary → **110/0**. App + CLI verified healthy at loop pause.

## 5. Open items / next gaps (for whoever resumes)

- **`read_event` bug (z13's queue, task-2026052901)** — core `read_event` reads
  `body.age`, which the real capture path (`EventStore::write_event`) never writes
  (0/192 real events have it). `phantom event show` + TUI `/event` fail on real
  data. Fix: read via `EventStore::read_meta`/`read_analysis` (key-aware), exactly
  like `app/src-tauri/src/commands/event_detail.rs::event_show`. Brief at
  `.ai-shared/queue/z13/task-2026052901.brief.md`.
- **SPEC-41 S1 live-health header** — tray header is a static version string;
  the live "N peer alive" / "Today: N events" lines need a dynamic menu rebuild.
- **SPEC-41 S12** — vault setup + mesh peer-add wizard (mDNS scan → invite, QR
  fallback) is unbuilt; large + security-sensitive (cluster bootstrap, SPEC-15).
- **Work-Track app surfaces** — z13 is building many TUI Work-Track commands
  (`/diff`, `/log`, `/branch`, `/event`, `/recall`); the desktop app has no
  equivalents yet (lower priority — dev-workflow, not Life Track).
