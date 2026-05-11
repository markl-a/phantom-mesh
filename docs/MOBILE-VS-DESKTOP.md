# Mobile vs Desktop — what each can do in the mesh

**Status**: 🟡 DESIGN — captures user 5/2 decision; targets v0.1.0 launch
**Companion**: `docs/CONTRIBUTOR-FUNNEL.md` (recipe upstream flow), `docs/CO-EVOLUTION.md` (3-tier model)
**Authority**: this doc decides which mesh roles each platform can play; SPEC-FREEZE-V1 §11 enforces.

The question: when the OSS launches and arbitrary users run phantom on
their devices, what can each platform actually DO inside the mesh? The
answer is asymmetric — desktop has full play, mobile has hard
constraints baked into the OS, and a third "Termux escape hatch" runs
on Android only.

---

## 0. The user's framing (5/2)

> "Android / iOS production versions probably can't do this; but
> desktop versions should be able to, right? Or should we make
> Android/iOS into a terminal-style package as well, with traditional
> UI and roughly traditional functionality?"

→ Yes, desktop can. Mobile cannot self-modify code. **UI style and
mesh-participation capability are independent decisions.** This doc
spells out both.

---

## 1. Hard constraints per platform

### 1.1 Desktop (macOS / Windows / Linux) — full play

✅ Can:
- `cargo build` / `cargo test` / `cargo check` (toolchain available)
- `shell` / `fork` / `exec` (OS allows)
- Rewrite own binary on disk + redeploy via `phantom upgrade`
- Run autoevolve loop that modifies `core/*.rs`
- Receive a recipe and apply its `git format-patch` + rebuild
- Participate in **Mesh 即 CI** dev hot-reload loop

❌ Cannot:
- (Nothing fundamental — only practical limits like Apple gatekeeper
  on macOS, but those have ad-hoc codesign workarounds — see
  `commit 1f27127`)

### 1.2 iOS (App Store + sandboxed sideload, both same)

❌ Hard sandbox limits — these are OS-level, not policy:
- No `fork()` / `posix_spawn()` (kernel forbids for sandbox apps)
- No JIT (W^X enforced strictly)
- No reading or modifying app bundle at runtime
- No long-running background process (~30s after backgrounded)

❌ App Store policy adds:
- No "dynamic code download and execution"
- App must ship with all code; no rebuild from source post-install
- No `cargo`, `git`, `make` — anything that would be a separate process

→ **iOS phantom can never run autoevolve / cargo / Mesh 即 CI**.
This is a hard wall, not a current limitation we'll fix later.

What iOS phantom CAN do:
- Run any in-process Rust code (linked statically into the app)
- Call HTTPS APIs (LLM providers, mesh peers via Tailscale)
- Read/write files within app container (`Documents/`, `Library/`)
- Run a local TCP listener on `127.0.0.1` while foregrounded
- Render a TerminalShell (Tauri WebView)

### 1.3 Android Tauri APK (Play Store + sideload)

❌ Sandbox limits (same root cause as iOS but slightly looser):
- No `cargo` / `git` / external toolchain by default
- App container is the writable scope
- Foreground service possible (background daemon-ish, but with
  persistent notification)

❌ Play Store policy:
- Same "no dynamic code download" rule as App Store
- Cannot self-modify APK contents

→ **Android Tauri APK = same role as iOS** for our purposes.

### 1.4 Android Termux (escape hatch, sideload only)

✅ Termux is a Linux userspace running on the Android kernel:
- Has shell / fork / exec / `cargo` (with package install)
- Can build phantom from source on the device
- Can run as a long-lived background process (with WAKE_LOCK)
- Already shipped: `phantom-aarch64-linux-android` raw binary
  (`feat/android` commit `963c3fe`)

❌ But:
- NOT App Store / Play Store distributable
- Power-user only — most users won't install Termux
- Apple has no equivalent — there is no "Termux for iOS"

→ Android Termux = **same capabilities as Linux desktop**, accessed via
sideload. It's the escape hatch for power users who want Mesh 即 CI on
their phone.

---

## 2. UI style vs mesh-participation capability are independent

A common confusion: "if mobile can't auto-modify code, what's the point
of putting TerminalShell on it?" The two are orthogonal:

| | UI style | Can self-modify code |
|---|---|---|
| Desktop TerminalShell on Mac/Win/Linux | terminal style | ✅ |
| Desktop traditional GUI (Tauri default-ish) | tap/click | ✅ (UI doesn't affect mesh) |
| Mobile TerminalShell (5/9 demo's path) | terminal style | ❌ sandbox blocks |
| Mobile traditional iOS/Android UI | tap/swipe native | ❌ sandbox blocks |

→ Choosing TerminalShell on mobile is a UX choice (geek-friendly +
visually consistent across 5 platforms), NOT a capability choice. We
get the same iOS sandbox limits regardless of UI.

→ The decision is: **what does the mobile UI look like to the user?**
Functionally the mobile app is whatever subset of mesh functions the
sandbox allows.

---

## 3. Three roles mobile can play in the mesh (without self-modifying)

Even though mobile can't auto-modify its own code, it can still
participate in the mesh in three meaningful ways:

### Role 1 — Sandbox worker (SPEC-FREEZE-V1 §11.2 / §11.5 already specs this)

Mac coordinator dispatches a task → mobile peer accepts (via
`/rpc/squad/dispatch`) → runs the agent locally inside the app's
process → returns result.

```
worker_caps on iOS / Android Tauri:
  ["file_in_container", "memory", "web", "subagent", "llm_local"]
```

Tasks fitting these caps:
- Run an LLM completion via HTTPS API (groq / openai / anthropic / gemini)
- Read/write files inside `Documents/` (sandbox-prefixed)
- Search local memory (sled-backed)
- Fetch a URL via `web_fetch`
- Spawn an in-process subagent (tokio task, not OS process)

Tasks that get auto-routed AROUND mobile (Squad Pipeline dispatcher
filters):
- `shell` (no fork)
- `git_*` (no exec)
- `cargo_*` (no toolchain)
- `xcode_simctl` / `spotlight_search` (macOS-only)

### Role 2 — Read-only mesh observer / dashboard

Mobile's TerminalShell connects to Mac coordinator and renders mesh
state in real time:

- Header: 5 peer dots showing online/offline/warn
- Scrollback: dispatched tasks + per-peer streamed output
- Log of cross-peer dispatch events
- Build/test status from Mesh 即 CI runs (when desktop dev mode active)

Use case: developer codes on Mac while watching iPad show the live test
matrix. iPad becomes a **mesh dashboard**, not a worker.

This role doesn't require any extra capabilities — read-only HTTPS to
Mac coordinator's `/rpc/peers` + `/api/dispatch/log` (latter not yet
implemented; v0.2). Privacy-friendly: nothing leaves Mac except mesh
state.

### Role 3 — Tier 1 contribution generator

Per `docs/CONTRIBUTOR-FUNNEL.md` §3, Tier 1 = sandbox extensions
(`~/.phantom-mesh/extensions/{prompts,skills,hooks}/`). Mobile's
autoevolve CAN write to extensions (its own personal customization
inside its own data container). It just can't write to `core/*.rs`.

So mobile users can:
- Customize their phantom's prompts (Tier 1)
- Build personal skills (Tier 1)
- Publish those as recipes (Tier 1 only — no patch attached)
- Get credit in CONTRIBUTORS.md when their recipe is adopted by others

→ Mobile = **first-class participant in CONTRIBUTOR-FUNNEL Tier 1**.
They just can't reach Tier 2/3 (that requires touching core code
which mobile can't do — but their contribution is real and
attributable).

---

## 4. The v0.1.0 launch decision: one mobile build, TerminalShell

For 5/15 OSS launch, **ship ONE iOS / Android Tauri build**:
- UI: TerminalShell (same React component as desktop `/term`)
- Capabilities: Sandbox worker (Role 1) + read-only observer (Role 2)
- Distribution: sideloaded IPA / APK (not App Store / Play Store yet —
  paid Apple Developer Program / Play Console + review take 1-3
  weeks; out of v0.1.0 scope)

→ Reason: TerminalShell is geek-friendly, visually unified across all
5 platforms, and doesn't require us to build TWO products
(traditional + terminal) for v0.1.0. Premature optimization.

5/9 demo selling point unchanged: "same binary, same UI, 5 platforms
all in the mesh."

---

## 5. v0.2+ optional traditional UI shell

If real users (post-launch) report "TerminalShell is hard to use on a
4" iPhone screen for non-technical tasks", v0.3+ adds a traditional
mobile UI **as a wrapping shell** around the same daemon:

```
Tab-based mobile app (Tauri):
  ├─ Tab 1: Mesh status
  │     Live peer dots, current dispatched tasks,
  │     "tap to approve incoming task"
  ├─ Tab 2: Settings
  │     Provider keys (groq / anthropic / etc.)
  │     Tailscale onboarding + cluster_secret
  │     worker_caps toggle (accept dispatch on/off)
  ├─ Tab 3: Notifications
  │     Push when peer dispatches a task to this phone
  │     Approval flow (auto-approve known agents, prompt for unknown)
  ├─ Tab 4: Power user → switch to TerminalShell
  │     Same component as desktop /term
```

Implementation cost: ~1 week (one engineer) on top of v0.1.0.
**Not in scope for 5/15.** Would target v0.3 sprint (5/22 → 5/29 or
later).

→ Underlying engine stays the same Tauri runtime + same phantom
in-process daemon. Only the UI layer diverges. Code reuse > 90% with
desktop.

---

## 6. Three Termux-on-Android escape hatch

For Android power users who already have Termux:

```
$ pkg install rust git
$ cargo install --git https://github.com/markl-a/phantom-mesh-private \
    --bin phantom
$ phantom serve
```

This installs the FULL desktop-equivalent phantom on Android. It can:
- Run autoevolve (modifies its own copy of `core/*.rs`)
- Participate in **Mesh 即 CI** dev hot-reload
- Reach Tier 2 / Tier 3 contributions
- Run as a foreground service via Termux's `termux-wake-lock`

This escape hatch:
- Already has shipped binary (commit `963c3fe` on `feat/android`)
- Documented separately as v0.2 path (not in 5/15 default install)
- Marketing line: "Android phone = portable Linux full worker"

iOS has no equivalent. Apple's sandbox is OS-level; no Termux for iOS
exists or can exist without jailbreaking. iOS users wanting full
mesh-CI participation MUST use a desktop.

---

## 7. Decision matrix per role

| Capability | Desktop | iOS / Android Tauri (sandbox) | Android Termux |
|---|---|---|---|
| Run autoevolve to modify own `core/*.rs` | ✅ | ❌ (sandbox) | ✅ |
| `phantom upgrade` swap binary | ✅ | ⚠ requires re-sideload | ✅ |
| Receive `/rpc/squad/dispatch` (Role 1) | ✅ | ✅ (sandbox-cap-filtered) | ✅ |
| TerminalShell read-only mesh observer (Role 2) | ✅ | ✅ | ✅ |
| Customize prompts/skills/hooks (Tier 1) | ✅ | ✅ | ✅ |
| Publish Tier 1 recipe to broker | ✅ | ✅ | ✅ |
| Publish Tier 2 recipe (touches `scripts/`, `tests/`) | ✅ | ❌ no patch generation | ✅ |
| Trigger Tier 3 PR (touches `core/*.rs`) | ✅ | ❌ no patch generation | ✅ |
| Mesh 即 CI hot-reload participation | ✅ | ❌ no rebuild | ✅ |
| Co-Authored-By in CONTRIBUTORS.md | ✅ | ✅ (Tier 1 only) | ✅ |

→ Mobile is "first-class on Role 1 + Role 2 + Tier 1; absent on Tier
2/3 and dev-CI." Termux is "as if it's a Linux desktop on a phone."

---

## 8. Sprint impact

What this decision settles for sprint planning:

### v0.1.0 (5/1 → 5/15) — minimal mobile

- ✅ ship single Tauri iOS IPA + Android APK
- ✅ TerminalShell UI (already built — `9548273`)
- ✅ Role 1 (sandbox worker) wired (Squad-a worker_caps + Squad-b
  `/rpc/squad/dispatch` already shipped)
- ⏳ Role 2 (read-only observer) needs `/api/dispatch/log` endpoint
  (not started; small — ~2h)
- ❌ Termux path documented but not on the install-path default
- ❌ traditional mobile UI: deferred

### v0.2 (5/15 → 5/22) — Termux + Tier 1 publishing

- Termux install script as documented secondary path
- `phantom evolve publish` + ed25519 sign (CONTRIBUTOR-FUNNEL §5)
- Mobile users can now publish Tier 1 recipes

### v0.3+ — Mobile traditional UI (if demanded)

- Tab-based mobile shell wrapping TerminalShell
- Push notifications for dispatch approval
- Worker accept/reject UI

→ The "should mobile be traditional UI?" decision is **deferred until
real user feedback post-launch**. We don't speculate — we ship one
product, see what users say, then split if needed.

---

## 9. Implications for the demo (5/9)

**5/9 talk track stays unchanged**:

> "Phantom-mesh runs on five platforms, one binary. Each platform
> participates in the mesh as a peer. iOS and Android run a sandbox
> subset — they can't shell out (Apple's rule, not ours), but they
> handle LLM-side work like file analysis, web fetching, prompt
> classification. Heavy work like git operations or shell commands
> get auto-routed to a Mac, Windows, or Linux peer. The dispatcher
> agent on Mac figures this out by looking at each peer's
> `worker_caps` field on `/rpc/ping`. So when I say 'analyze this
> codebase' here, recon goes to a Windows desktop, log enrichment
> runs on a Linux cloud node, severity classification runs on this
> iPhone, and a synthesizer agent on Mac merges the results."

→ Sandbox-on-mobile becomes a **feature not a limitation**. "Mobile
runs the LLM-thinky work because that's what mobile can do safely."

---

## 10. Anti-pattern: don't try to bypass

Things we explicitly DON'T do (because they'd violate Apple/Google
policy or compromise security):

- ❌ Use App-Bound Domains tricks to do dynamic loading on iOS
- ❌ Ship interpreters (Lua / JS) that effectively let extensions
  modify behavior beyond Tier 1 scope
- ❌ JailbreakMe-style workarounds for jailbroken iOS
- ❌ Smuggle binary patches through "asset downloads"

These would either get the app rejected from store distribution
(when we eventually pursue that) OR violate the trust model.

---

## 11. Summary in one paragraph

**Desktop = full play (Mesh 即 CI + autoevolve + all 3 contribution
tiers). Mobile sandbox (iOS / Android Tauri) = role 1 sandbox worker
+ role 2 observer + role 3 Tier 1 contributor; cannot self-modify
code, ever, by OS design. Android Termux = escape hatch with full
desktop-equivalent capabilities for power users only. v0.1.0 launches
ONE mobile product (TerminalShell on Tauri); v0.3+ may add a
traditional UI wrapper if user demand justifies.**

---

## References

- `docs/SPEC-FREEZE-V1.md` §11.2 — iOS sandbox worker spec
- `docs/SPEC-FREEZE-V1.md` §11.5 — Android Tauri sandbox spec
- `docs/SPEC-FREEZE-V1.md` §3 — sandbox sub-contract
- `docs/CONTRIBUTOR-FUNNEL.md` §1-§3 — recipe / broker / PR flow
- `docs/CO-EVOLUTION.md` §38-69 — 3-tier model
- `core/src/mesh.rs::PeerStatus.worker_caps` — runtime cap declaration
- `feat/android` commit `963c3fe` — Termux raw binary already shipped
- App Store Review Guideline 3.2.2 — "no dynamic code download" rule
