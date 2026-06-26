# FINAL — mesh-references development plan

> **What this project is:** "mesh-references" is the work of turning the three gap-aware
> reference reports (`plans/cli-references-2026-06-12.md`, `…/desktop-app-references-…`,
> `…/mobile-app-references-…`) into a *shippable, prioritized backlog* for phantom-mesh.
> The reports already did the research; this plan picks the items that are (a) specific to
> THIS codebase, (b) verified against current code, and (c) worth doing now. Generic "adopt
> a CLI library" wishlist items are deferred or cut.
>
> **Core thesis (carried from the reports, re-confirmed):** phantom does **not** lack
> features — it lacks *honest UI surfaces for the apex abilities* and *finished plumbing*.
> Highest ROI is single-file/single-function correctness fixes + wiring already-built
> backends to the front, **not** big rewrites.

---

## Prioritized backlog

### P1 — do now (verified-real, single-file/low-risk, unblocks honesty + correctness)

| ID | Item | Surface | Evidence (verified) |
|----|------|---------|----------|
| **P1-1** | Fix `NO_COLOR` spec violation **and** add TTY fallback in `is_colored()` | CLI | `core/src/util/term.rs:18` = `var("NO_COLOR").is_err()` → empty `NO_COLOR=` wrongly disables color; **no `IsTerminal` gate** so `phantom … \| cat` emits raw ANSI. Reuse existing `atty_stdout()` (phantom.rs:10701). |
| **P1-2** | Route `help`/`--help` to **stdout** (currently all `eprintln!`) | CLI | `phantom help \| less` gets nothing today; mechanical `eprintln!→println!` swap. |
| **P1-3** | Make **skill-bank reachable**: +1 `PRIMARY_NAV` entry + 1 `Route` | Desktop | `app/src/pages/skill-bank.tsx` exists with honest-empty state but is unrouted (App.tsx PRIMARY_NAV 46–57) → apex #1 "compounding memory" is psychologically absent. ~15 min. |
| **P1-4** | Render `awaiting_approval` as a **first-class consent chip** (stop mapping → `pending`) + inline Approve/Reject/Stop | Desktop | `TasksPanel.tsx:21–29 DAEMON_STATUS_MAP` erases apex §3④ bounded-consent — the OSS-gap differentiator. |
| **P1-5** | Remove fake audit data / strip hardcoded private IPs | Desktop + Mobile | `SecurityPanel.tsx:36 MOCK_EVENTS` violates SPEC-31 NO-FAKING; `AppTemplate.tsx` hardcodes Mac IP + 5 Tailnet node IPs (info leak in shipped APK). |
| **P1-6** | iOS ATS one-line fix: `NSAllowsArbitraryLoads=true` → `NSAllowsLocalNetworking=true` | Mobile | `Info.plist:38`; App-Store rejection risk, zero functional loss (phantom only talks Tailnet/LAN). |
| **P1-7** | Windows global-shortcut OS branch + dynamic label | Desktop | `lib.rs:760` uses `Modifiers::SUPER` (Win key) → Win+Shift+F swallowed by Windows shell; label still says "Cmd". Branch to Ctrl+Alt on Windows. |

### P2 — next (medium, builds on existing `phantom.db` / proven patterns; one PR each)

| ID | Item | Surface | Why |
|----|------|---------|-----|
| **P2-1** | `phantom status` unified entry (identity / serve up? / paired peers + direct-vs-relay + relative handshake age; `--json`) | CLI | Info is scattered across `doctor --mesh` / `peer list` / `cluster`. Model on Tailscale/fly `status`. |
| **P2-2** | `phantom logs` (DB-backed) + `recall -c/--cid` continuation + `--model`/`--tool` filters + `--json` | CLI | `phantom.db` already stores task/dispatch; smallest interface gap, highest payoff (simonw/llm pattern). |
| **P2-3** | `phantom netcheck` (or `doctor --net`) + `peer ping <id>` (UDP/HNS port reach, NAT, RTT, direct-or-relay) | CLI | Directly surfaces the logged HNS-port-range + stale-address (`:17878→:7878`) silent-drop traps. |
| **P2-4** | Snapshot-before-mutate + `phantom restore/undo` (CLI) **and** desktop real NSPopover via `tauri-nspanel` mounting existing `MenuBarDropdown.tsx` | CLI + Desktop | Feeds the "safe unattended runs" pillar (gemini `--checkpointing`/aider `/undo`); `MenuBarDropdown.tsx` is built-but-never-mounted (SPEC-41 daily-loop hub). |
| **P2-5** | Standardize `doctor` checks to `{name, status: pass\|warn\|fail, hint}` + semantic exit code; add `indicatif` progress to `self-update`/`evolve`/`swarm`/`cluster upgrade`; `comfy-table` for cluster/peer/sessions tables | CLI | All build on already-leading `doctor --json` + tabular commands; warn-only, low risk. |
| **P2-6** | Push→approval closed loop: APNs/FCM interactive notification → deep-link with `taskId` → **approval decision over encrypted reconnect** (payload is only a trigger) | Mobile | The ④ differentiator; both iOS (`aps-environment`/`UIBackgroundModes` missing) and Android (no FCM/`POST_NOTIFICATIONS`) lack it. Model on Home Assistant actionable notifications. |
| **P2-7** | Cross-device pairing: `phantom pair/invite` short-lived token + `phantom join <token>` + QR on phone | CLI + Mobile | Kills the logged stale-peer-address silent-drop trap; replaces manual IP entry. Tailscale auth-key model. |

### P3 — architectural / roadmap (decide-before-coding; phased + double-gate)

| ID | Item | Note |
|----|------|------|
| **P3-1** | **owned-memory backend (apex #1)**: ✅ `skill_store()` + recall path DONE; remaining = hierarchical-markdown context load (global→project→subdir) + semantic `ort` leg (deferred) | `skill_wire.rs:1836 skill_store()` **now persists the extract hand-off** (M1 done; tests :3304/:3360), recall wired into the agent loop (`agent.rs:730`). No longer a stub. |
| **P3-2** | **Self-update signature verification** (cosign/minisign) on CLI; Tauri `updater` plugin (minisign + GitHub Release static JSON) on desktop | self-update pulls/swaps/restarts with **no verification** today; high priority, large surface. Replaces the AppData-lock `self-update.ps1` hack. |
| **P3-3** | **clap migration** (phased: wrap `phantom completions <shell>` + `phantom man` first, then incrementally replace the ~17k-line hand-rolled parser) | Common root cause of missing completions/man/`--quiet`/`--verbose`/consistent `--json`. Phased + double-gate regression only. |
| **P3-4** | Foundational data decisions (lock before writing): plaintext/Markdown = source of truth, SQLite = index only, always-exportable; Stronghold(secrets) vs Store(prefs) split; per-platform capability files + `freezePrototype:true`; mobile local-first + **CRDT** conflict resolution | Cheap to decide now, very expensive to retrofit (Logseq cautionary tale). Not code yet — a locked decision record. |

---

## Cut / deferred from the reports (over-scoped, duplicate, or unsafe-now)

- **24/7 environment capture (screenpipe/Rewind-style)** — premature + privacy hazard for a daily-use personal tool; not in the apex MVP. Defer past v0.6.
- **iOS continuous background execution / CRDT offline sync as a near-term deliverable** — iOS background is ~30s-capped; commit to the **APNs-triggered supervisory remote-control** model instead (keep CRDT as a P3-4 *decision*, not a build item).
- **`phantom explain` / `phantom alias` / Khoj proactive newsletters / serve `/metrics`** — nice-to-have surface area, not load-bearing for apex; backlog only.
- **Full `clap` rewrite as a single effort** — kept but explicitly **phased** (P3-3); a big-bang rewrite would break the just-landed P1 quick-wins.

---

## Task breakdown — top 3

### P1-1 — NO_COLOR + TTY fix (`core/src/util/term.rs`)
- Change `is_colored()` to: color **off** iff `NO_COLOR` is *present and non-empty* (spec-correct), **and** `stdout().is_terminal()` is true — additionally honor `CLICOLOR_FORCE`/`FORCE_COLOR`.
- Reuse the existing `atty_stdout()` helper (phantom.rs:10701) rather than re-implementing TTY detection.
- Add `crate::env_lock` tests: `NO_COLOR=` (empty) keeps color; `NO_COLOR=1` disables; piped stdout disables.
- Verify `phantom doctor` + `phantom doctor --json` output is unchanged in CI/pipe runs (the comment at term.rs:11 flags this dependency).

### P1-4 — `awaiting_approval` consent chip (`app/src/screens/.../TasksPanel.tsx`)
- Remove the `awaiting_approval → pending` collapse in `DAEMON_STATUS_MAP` (line 21–29); give it a distinct high-visibility state.
- Add inline Approve / Reject / Stop actions wired to the existing governor RPC + a Dashboard badge count.
- Snapshot/visual-check that no other status consumer assumed the old `pending` mapping.

### P1-3 — skill-bank reachable (`app/src/App.tsx`)
- Add one `PRIMARY_NAV` entry (PRIMARY_NAV 46–57) and one `<Route>` pointing at the existing `pages/skill-bank.tsx`.
- No backend work — the page already ships an honest-empty state; this only restores apex #1's *visibility* (full backend = P3-1).
- Confirm the route renders the honest-empty state (not a crash) and the nav item is keyboard-reachable.

---

## Changes from draft

- **Draft was unusable.** `PLAN-mesh-references.md` contains only a captured planner error
  (`Not inside a trusted directory and --skip-git-repo-check was not specified`) — the codex
  planner aborted before producing any plan. Nothing to incorporate.
- **agy's review WAS usable — it carried the project.** agy ignored the broken draft, located
  the three real reference reports in `plans/`, and produced substantive, code-grounded
  feedback. This FINAL is built on those reports as filtered by agy's five points.
- **Adopted from agy:** flagging the draft as an infra failure; the clap-sequencing risk (→
  P3-3 made explicitly phased *after* the P1 quick-wins); cutting screenpipe-style capture and
  near-term iOS background/CRDT as over-scoped; surfacing the unmounted `MenuBarDropdown.tsx` +
  Tauri `deep-link`/`single-instance`; and adding the missing **secure pairing protocol**
  (→ P2-7) and APNs/cert provisioning prerequisite (→ P2-6).
- **Corrected against code (independent verification, refreshed 2026-06-21):** the mobile report's
  `skill_store() = unimplemented!()` claim is **stale** — `skill_store` (`skill_wire.rs:1836`) now
  **persists the extracted hand-off** (drains the hand-off queue → `store_skill_with_embedding`;
  tests `:3304`/`:3360`), returning a typed `SkillError::StoreFailed` only when no hand-off was queued —
  never panics, not a stub. The remaining owned-memory gap is the semantic `ort` embedding leg (deferred,
  → P3-1), not the store path. The `term.rs:18` NO_COLOR bug
  and no-TTY-gate were verified true.
- **Trimmed:** collapsed three separate report backlogs into one P1/P2/P3 list; dropped generic
  CLI-UX nice-to-haves (`explain`/`alias`/`/metrics`/Khoj newsletters) to backlog-only.
