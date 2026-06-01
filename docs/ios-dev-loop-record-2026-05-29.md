# iOS phantom-mesh dev — /goal + /loop record (2026-05-28 → 2026-05-29)

Archived before cancelling /goal + /loop. Reusable to restart the same
autonomous dev loop later. Branch: `wip/mac/ios-work`.

---

## The /goal objective (this version)

持續地把 iOS 版 phantom-mesh AI terminal app 開發到 production-ready，
DESIGN-DRIVEN（從 BIG-GOAL → SPEC-30/31/32 iOS specs → wireframe/mockup 找
design 要求但 app 缺/stub 的功能，端到端實作，no fakery）。

Operator role: reviewer-director（不 real-time typist），mobile + Tailscale
primary access。核心要求："別做假" — 每個 stage 在真實 sim (Appium) 驗證，
誠實說明無法自動驗證的部分。

---

## The /loop prompts (2 recurring cron jobs, every 15 min, session-only)

### Loop A — design-driven dev (cron id 717c97b3)

```
In worktree `/Users/marklight/Documents/GitHub/hailmary/phantom-mesh-ios` on
branch `wip/mac/ios-work`, continuously develop the iOS phantom-mesh app toward
production-ready — DESIGN-DRIVEN. Each iteration: (1) git sync:
`git checkout -- app/src-tauri/Cargo.lock`; fetch; merge origin/main (or rebase
if non-ff). (2) Consult the design chain — `docs/superpowers/BIG-GOAL.md` ->
specs under `docs/superpowers/specs/` (SPEC-30/31/32 iOS, SPEC-21 focus, mobile
specs) -> wireframe/mockup/prototype docs — and identify ONE concrete iOS
feature/screen the design calls for but the app does NOT implement (or only
stubs). If its design is missing/thin, write/extend that design doc first
(CLAUDE.md reader-threshold + 繁中 inline rules), then implement. (3) Implement
end-to-end — NO fakery, a real working feature not a stub/claim: app/src UI +
core/src Rust / Tauri commands + ts-rs bindings; cargo check (from core/) +
`npm run build` (app/) green. (4) When a dev stage is complete: dispatch
MULTIPLE AI tools (codex + opencode via scripts/ai/dispatch.sh, + a Claude
subagent) to independently review+test the diff, collect their HONEST findings,
fix, self-check, re-review until MOST tools find nothing. Verify what's
verifiable (browser iPhone-viewport, native sim screenshot+OSLog, wire-level)
and state honestly what can't be auto-tested. (5) Commit + push origin
wip/mac/ios-work — NEVER push main. Respect CLAUDE.md staged-dispatch. If
genuinely blocked (build error, secrets, decision required), write
`.ai-shared/queue/mac/ios-blocker.md` and stop the loop.
```

### Loop B — watchdog health check (cron id f5384f92)

```
WATCHDOG CHECK for the iOS phantom-mesh app (worktree
/Users/marklight/Documents/GitHub/hailmary/phantom-mesh-ios, branch
wip/mac/ios-work). This is a health check, NOT a forced new iteration. Decide in
this order: (1) If there is in-progress dev work (an unfinished
feature/fix/build/review/commit), CONTINUE and finish that — do NOT start
anything new, do NOT interrupt it. (2) Else if the app is not running in the
simulator, get it running again (open Simulator, boot iPhone 15 ACE233BC iOS 17,
install dist/phantom-mesh-ios-sim.app, launch ai.phantommesh.app). (3) Else (no
in-progress work AND app running) start the NEXT design-driven gap: git sync;
consult BIG-GOAL -> SPEC-30/31/32 iOS specs -> wireframe/mockup, pick ONE
concrete iOS feature/screen the design calls for but app lacks or stubs,
implement it end-to-end (no fakery; cargo check from core/ + npm run build
green), rebuild the sim app so the change is VISIBLE, then multi-AI review
(codex+opencode via scripts/ai/dispatch.sh + a Claude subagent), fix until most
tools find nothing, commit + push (never main). Always make changes visible by
rebuilding+reinstalling the sim. If blocked, write
.ai-shared/queue/mac/ios-blocker.md and stop.
```

---

## Session record — what the loop accomplished

### Real bugs fixed (all committed + multi-AI reviewed)
- 登入完全壞 — onboarding FSM `enteredAtMs` u64→BigInt → JSON.stringify 炸,
  三模式都進不了 app。根治在 invoke bridge (`safeInvoke` coerceBigInts) +
  onboardingFsm explicit。 (commit `7d886165`)
- 集群 tab crash 炸全 app — PeerBadge `badgeStyleFor` 無 default, 後端回小寫
  status (online/unhealthy) vs TS PascalCase → undefined.text crash。修
  case-insensitive + default + ErrorBoundary 隔離每個 tab。 (commit `a2f77082`)
- onlineCount「0/6」假數字 — strict `=== 'Online'` 比小寫 wire → store 層
  normalizePeerStatus。 (同 `a2f77082`)
- focus 結束空白 — complete_session 回 summary:"" (等 unimplemented LLM analyze)
  → deterministic takeaway 基於真實 metrics (時長/完成度/中斷)。
  (commits `4f6e0771`, `2203f69d`)
- 對話無 key error CLI jargon — describeError 偵測 no-key → 友善引導去設定。
  (commits `a0eb0869`, `75bb78d5`, `90164a4c` — codex 抓 "all providers failed"
  太寬, 改用 "No provider had a usable key" 精確 signal)
- demo agents.toml `minimax-m2.5-free` 開箱對話 404 — OpenCode free tier 死 +
  agent.model 覆蓋 failover provider default_model。改 providers failover chain
  (groq-first)。 (commits `3d9d889f`, `2f05eb1b`)
- cluster 派送 401 X-Cluster-Auth 技術字串 → 友善「secret 不符, 去設定檢查」。
  (commit `16ac712f`)

### Features added
- coach review mobile screen + 「教練」nav tab (複用 desktop CoachReviewReader,
  接真實 daily_review_load)。 (commits `4b809f1c`, `1ae902d5` scroll wrapper,
  `43b26274` touch-target ≥44pt)
- Reduced Motion HIG (SPEC-31 §3.2 D) — CSS prefers-reduced-motion + JS
  scrollIntoView gating。 (commits `3085b47f`, `e62a2e2a`)
- nav tab a11y aria-label + landmark (VoiceOver)。 (commit `e6067b84`)

### Key-driven verification (operator provided ~/Downloads/llm-keys.env)
Injected 9 provider keys into sim container env (secure, never printed):
- 對話 happy path: 單輪「1+1」→「2」+ 多輪 context (記 42 → 問+1 → 答 43)。
- cluster 派送 wire: 對 secret (= coordinator agents.toml cluster_secret "from
  ~/.phantom-mesh/agents.toml [core]") → HMAC 驗證 + coordinator 回覆。
- cluster 401: 錯 secret → 友善 error 顯示 (Appium 驗證)。
- focus → coach review: pipeline 端到端 — focus 結束寫 Life Node event (via
  life_node::focus_session) → coach review 顯示「focus (1)」event。

### HIG / a11y audit (SPEC-31 §3.2 D/E) — mostly already compliant
- Reduced Motion: 唯一真實 gap, 已做。
- VoiceOver: icon-only buttons (send/nav/header) 已有 aria-label; text buttons
  有 visible text。**Lesson**: 不要給有 visible text 的 button 加不含該 text
  的 aria-label → 違反 WCAG 2.5.3 (FocusPage 試過, revert)。
- Dynamic Type: padding-based 高度, 無死高文字容器, 已合規。
- Touch target / safe-area: coach review 已修。
- Haptic: 未做 (需 tauri-plugin-haptics native plugin + 真機驗證)。

### Honest negatives / reverts
- FocusPage 控制 button aria-label → WCAG 2.5.3 違反 (覆蓋 visible text), revert。
- capture_focus_wire `emit_event_pseudo` 寫 Life Node 改 → redundant (life_node
  已寫 focus event), 沒生效 (中文 event 從沒寫), revert。修正誤判: focus→coach
  本來就 work。

### Blocked — needs operator (see .ai-shared/queue/mac/ios-blocker.md)
1. **nav 重構** — bottom nav 7 tab 違反 SPEC-31 §3.2(f) cap 3 (chat/coach/
   settings + 其他 stack-push)。解鎖 backend-ready 的 **habit screen** (SPEC-22,
   4 commands wired: habit_create/checkin/list/streak)。design 決策。
2. **UC5 歷史** — MobileHistory = broker squad dispatch, 需 phantommesh.io OAuth
   登入 (無法自動化)。
3. **vault / skill 後端** — broker_vault_wire 是 crypto primitives (無 inventory
   list); skill_wire embedding_search (L874) + INSERT (L1119) unimplemented,
   無 Tauri command。需 core de-stub + wire 才能 no-fakery mobile screen。
4. **Haptic** — 需 native plugin + 真機。

### Testing infrastructure built (reusable)
- **Appium** + appium-xcuitest-driver + WebDriverAgent — 真實 sim UI 自動化。
  WKWebView 無 WEBVIEW context (NATIVE_APP only), DOM 經 native a11y tree, 用
  React aria-label 的 `ACCESSIBILITY_ID` 或 `IOS_PREDICATE "label == '...'"`。
- E2E journey scripts (`/tmp/*.py`): onboarding demo → 6+ tabs, screenshot +
  scan a11y page source for error markers ([ERR]/undefined/TypeError/失敗)。
- localStorage seed (sqlite ItemTable, **UTF-16LE**) for cluster config —
  container UUID 變動需 dynamic 解析 (`simctl get_app_container` + glob)。
- env key inject: cp 到 `{container}/Library/Application Support/
  ai.phantommesh.app/.phantom-mesh/env`, restart app (lib.rs setup() loads)。
- minified crash triage: package-ios `tauri ios build` 覆蓋 sourcemap build →
  讀 minified bundle 該 column + grep minified fn 名 (e.g. `M0`) 定義 + 字串
  literal 辨識 component (M0 = PeerBadge)。

### Systemic gotcha記錄到 memory (reference_ios_sim_testing)
- ts-rs u64 → JS BigInt hard-crashes Tauri invoke (JSON.stringify throws);
  safeInvoke coerceBigInts 根治, 但直接 `@tauri-apps/api/core` invoke caller
  (captureFocus.ts/localKeys.ts/brokerLogin.ts) 要手動 coerce。
- agent provider:model precedence: `agent.model` 覆蓋 `providers.X.default_model`;
  failover chain 用 `providers = ["groq:llama-3.3-70b-versatile", ...]` 各帶
  自己 valid model。
- cluster_secret 來源: coordinator 讀 `~/.phantom-mesh/agents.toml [core]
  cluster_secret`, 非 env CLUSTER_SECRET。
