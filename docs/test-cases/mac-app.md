# macOS Desktop App (Tauri 2) — 全測試用例庫 v1 (2026-06-12)

> **Surface**: `mac desktop app`（`app/` React + `app/src-tauri` Rust，Tauri 2，productName "Phantom Mesh"）。本檔是 Charter Wave 2 為「桌面兩面」補的對等 mac.md 級逐條 case DB（解 BRK-10 / INV-15 K2）。
>
> **覆蓋範圍**: 桌面 daily-loop 主要 CUJ（onboarding → 每日 capture/you-talk/review → 5 能力面 → 跨面 supervision → error/empty/offline）× macOS 桌面行為（NSStatusItem / 全域捷徑 / Keychain / Gatekeeper / TCC）+ 跨 SPEC P4/SPEC-31 invariant。
>
> **權威來源**: 流程 = `docs/superpowers/specs/2026-06-12-platform-flows-design/surface-mac-app.md`（已逐面驗證、附 file/line cite）；參考 = `plans/desktop-app-references-2026-06-12.md`。Code 真相裁決一律以 `app/src/*` + `app/src-tauri/src/*` + `core/src/*` as-built 為準（SSOT 規則 A.2.4）。
>
> **docs-only 鐵則**: 本檔只記驗收條件，不改任何 code。所有已知 code 問題（IP 洩漏、MOCK 假稽核、entitlement、同意門禁抹除、死碼未掛載）一律記成「可重複測試的驗收條件 + 標 [現況 FAIL / code-backlog]」，FAIL case 要求改的是 **code**（非本文件），並指向流程文件 / G-gap 編號的追蹤點。
>
> **編號規約**: `MACAPP-{CUJ|ABIL|PLAT|INV|NOFAKE}-{feat}-{nnn}`，ID 永不重用。每條對齊 INV-15（流程 + 驗收 + 測試三件套）：`Verifies` 欄回扣流程文件章節 + SPEC/能力。

---

## §0. Schema legend（對齊 mac.md §0，CLI runner / Maestro / Playwright 可直讀）

共用 token/schema 字典見 [`README.md`](README.md)；本檔只列 macOS app 端補充。

每條 case 欄位：

| 欄 | 意義 |
|---|---|
| **ID** | 唯一識別（永不重用） |
| **Type** | `unit`（cargo/vitest 內部）/ `integ`（Tauri command 整合）/ `e2e`（Playwright/WebDriver/tauri-driver 驅動）/ `manual`（人眼 / 人手）/ `static`（grep/AST 斷言原始碼，用於 NO-FAKING 守護）/ `monitor`（synthetic canary cron） |
| **Auto** | `✅` 完全自動 / `⚠` 需 env 或 fixture / `❌` manual only / `⏰` cron |
| **Setup** | 跑前準備（TempDir / env var / mock daemon / fresh localStorage） |
| **cmd** | 實際命令或步驟（runner 直接抄、shell/JS 可執行） |
| **expected** | 通過條件（exit code / DOM 斷言 / grep match / 截圖比對） |
| **Verifies** | 對應流程文件章節 + SPEC G[X] / 能力編號 |
| **last_run** | 最後驗證日時 + runner |
| **狀態** | `✅` 過 / `🟡` partial / `🔴` 重要缺 / `🟥` 現況 FAIL（code 缺陷已知，等修碼）/ `⬜` 未做 |

> **狀態欄 `🟥` = 現況 FAIL / code-backlog**：該 case 一旦能跑就會 FAIL，因為對應 code 缺陷已被流程文件核對為真（非測試問題）。修法在 **code**，本檔僅凍結驗收條件。對映 `plans/desktop-app-references-2026-06-12.md` 的 G1–G10 gap 編號。

---

## §1. CUJ-01: First-run / onboarding（login + 選 + 接後端大腦，18 條）

> 流程權威: surface-mac-app.md §1。FSM source of truth = `core/src/onboarding_wire.rs` FORWARD_ORDER：`fresh_install → created_identity → joined_cluster → set_provider → first_reply_received`。Gate = `localStorage[ONBOARDED_KEY]`（`App.tsx`）。

### 1.A onboarding gate + FSM 推進（8 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ01-ONB-001 | e2e | ⚠ fresh | 清 localStorage（`ONBOARDED_KEY` 不存在）| 冷啟 .app | 渲染 `<OnboardingHello>`（pages/onboarding-hello.tsx）非 sidebar shell | §1 gate / App.tsx | ⬜ | ⬜ |
| MACAPP-CUJ01-ONB-002 | e2e | ⚠ fresh | onboarded=true | 冷啟 .app | 跳過 OnboardingHello → 進 StartupCheck → main shell | §1 gate | ⬜ | ⬜ |
| MACAPP-CUJ01-ONB-003 | unit | ✅ | - | `cargo test --lib onboarding_wire::tests`（FORWARD_ORDER 順序）| 5 step 順序 == `fresh_install,created_identity,joined_cluster,set_provider,first_reply_received` | §1 FSM SSOT | ⬜ | ✅ existing |
| MACAPP-CUJ01-ONB-004 | integ | ⚠ broker mock | `BROKER_URL=mock` | `onboarding_advance(fresh_install)` → broker OAuth loopback | exit 0、session 建立、進 created_identity | §1 D1 login-first | ⬜ | 🟡 |
| MACAPP-CUJ01-ONB-005 | integ | ✅ | TempDir HOME | `onboarding_advance(created_identity)` | ed25519 mint + 寫入 macOS Keychain（SPEC-40 §7.5）| §1 created_identity / SPEC-12 | ⬜ | 🟡 |
| MACAPP-CUJ01-ONB-006 | integ | ✅ | TempDir | `onboarding_advance(joined_cluster)` | detached `phantom serve` 起 + mDNS advertise（**SINGLE-NODE**）| §1 joined_cluster | ⬜ | 🟡 |
| MACAPP-CUJ01-ONB-007 | integ | ⚠ mock provider | mock Claude CLI/Codex/Ollama | `onboarding_advance(set_provider)` → detect + drag-rank | 偵測到 ≥1 provider、可排序、Ollama always-on fallback | §1 set_provider / D5 | ⬜ | 🟡 |
| MACAPP-CUJ01-ONB-008 | e2e | ⚠ fresh | full flow | 跑完 5 step | `localStorage[ONBOARDED_KEY]=="true"` + onComplete() | §1 first_reply_received | ⬜ | ⬜ |

### 1.B 後端大腦 fork（apex §2 — 最大 onboarding 缺口，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ01-FORK-001 | static | ✅ | - | `grep -rE "cloud.*sandbox\|linux.*sandbox" app/src/` | ≥1 命中（cloud-Linux-sandbox 後端選項存在於 GUI）| §1 CRITICAL GAP / BIG-GOAL §2 | ⬜ | 🟥 FAIL [G-fork/code-backlog]：現況 grep = **0 hits**。onboarding 無「我沒桌面 → 開雲端 Linux sandbox」分支，joined_cluster 隱性假設「本機=後端大腦」。修法=onboarding FSM 加 backend-brain step（spec-level，非 UI hack）。流程 surface-mac-app.md §1「the single biggest onboarding hole」 |
| MACAPP-CUJ01-FORK-002 | e2e | ⚠ fresh | joined_cluster step | 讀 step 文案 | 出現「This Mac is now your backend brain」級心智模型句（own-desktop-mesh / pair-later / cloud-sandbox 三選一或至少 default 說明）| §1 onboarding states to design | ⬜ | 🟥 FAIL [G8/code-backlog]：現況 default path 缺心智模型句（surface-mac-app.md findings medium#2）。預期 = 加一句框架文案，無新畫面 |
| MACAPP-CUJ01-FORK-003 | manual | ❌ | onboarded | Settings → Cluster | 有 peer-join（Stage 2）入口（MeshPeerAddWizard 可達）| §1 peer-join deferred | ⬜ | 🟡 wizard 存在但埋 /settings/cluster/add-peer 3 層深（findings low#9）|
| MACAPP-CUJ01-FORK-004 | e2e | ⚠ fresh | demo-relay path | 觸發 cloud-sandbox handoff | 非 no-op：回真實 handoff 或明示「coming soon」而非靜默落 null | §1 cloud sandbox / SPEC-52 | ⬜ | 🟥 FAIL [code-backlog]：cloud-sandbox 非 first-run 選項；SPEC-52 demo-relay SPEC-ONLY |

### 1.C StartupCheck 自檢（每次啟動的阻斷面，6 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ01-SCK-001 | integ | ✅ | onboarded、runtime healthy | 冷啟 → StartupCheck | runtime/providers/LLM 三檢 healthy → 800ms 後 auto-advance 進 main | §1 self-check | ⬜ | 🟡 |
| MACAPP-CUJ01-SCK-002 | e2e | ⚠ degraded | provider rate-limited / Ollama 未跑 | 冷啟 | **非阻斷**：直接進 /conversation + 可關 banner（不是全螢幕 gate）| §5 / findings high#4 | ⬜ | 🟥 FAIL [code-backlog]：StartupCheck 每次啟動都跑且阻斷（App.tsx:117-127 selfCheckPassed 每次 mount reset false）；degraded 時卡牆 + 「跳過，強制進入主介面」。修法=非阻斷化 + 移除 force-enter 按鈕（surface-mac-app.md findings high#4）|
| MACAPP-CUJ01-SCK-003 | e2e | ⚠ degraded | 同上 | 觀察按鈕 | **不存在**「跳過，強制進入」escape hatch | findings high#4 | ⬜ | 🟥 FAIL [code-backlog]：現況存在 scary force-enter 按鈕 |
| MACAPP-CUJ01-SCK-004 | manual | ❌ | 連續 5 次冷啟 healthy | 計每次到主介面 step 數 | 0 額外 step（app 直接開到內容，mac 慣例）| findings high#4 | ⬜ | 🟥 FAIL [code-backlog]：現況 2-3 step/每次 |
| MACAPP-CUJ01-SCK-005 | integ | ✅ | unhealthy（runtime 真壞）| 冷啟 | 才 hard-gate（unhealthy 才擋，degraded 不擋）| findings high#4 fix | ⬜ | ⬜ |
| MACAPP-CUJ01-SCK-006 | e2e | ✅ | onboarded | 持久化檢查 | 有「last check passed」flag、不每次 reset | findings high#4 fix | ⬜ | 🟥 FAIL [code-backlog]：現況每次 mount reset false |

---

## §2. CUJ-02: Daily-use core loop（capture → you-talk → review，16 條）

> 流程權威: surface-mac-app.md §2。真正全域捷徑只有兩個（`lib.rs:308-313`）：`Cmd+Shift+H`→`shortcut://chip`、`Cmd+Shift+F`→`shortcut://focus`。`review`/`settings` 是 **tray menu 項**（`lib.rs:729-730`）非熱鍵。`App.tsx` 監聽事件 + `navigate()`。

### 2.A 全域捷徑 + 事件路由（6 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ02-SHC-001 | integ | ✅ | app 起 | 觸發 global-shortcut `Cmd+Shift+H` | Rust emit `shortcut://chip`（lib.rs:309）| §2 / lib.rs:308-313 | ⬜ | 🟡 |
| MACAPP-CUJ02-SHC-002 | integ | ✅ | app 起 | 觸發 `Cmd+Shift+F` | emit `shortcut://focus` | §2 | ⬜ | 🟡 |
| MACAPP-CUJ02-SHC-003 | e2e | ⚠ | App.tsx 監聽 | emit `shortcut://chip` | `navigate("/habit")` → HabitPage(ChipPopover) | §2 sequence | ⬜ | 🟡 |
| MACAPP-CUJ02-SHC-004 | integ | ✅ | tray menu | click 「今日回顧」 | emit `shortcut://review` → navigate("/review")（tray 項非熱鍵）| §2 / lib.rs:729-730 | ⬜ | ⬜ |
| MACAPP-CUJ02-SHC-005 | manual | ❌ | 在 Safari 打 email 中途 | 按 `Cmd+Shift+H` 記水 250 | **不奪焦**：Safari 文字游標不丟、popover 自關（SPEC-41 G7 no focus steal）| §2 / G7 / G2 | ⬜ | 🟥 FAIL [G2/code-backlog]：現況 `shortcut://chip`→`navigate("/habit")` 是主視窗整頁 route swap，奪走 Safari 焦點與 context（~5 step + 失 context）。真 NSPopover 未建。修法=tray webview window + always-on-top transient popover（surface-mac-app.md findings high#2）|
| MACAPP-CUJ02-SHC-006 | e2e | ⚠ | event name 對齊 | 監聽 emit 名 | shipped 名 `shortcut://chip`/`tray://settings`/`deep-link://navigate`/`agent_event`（非 SPEC-17 `<domain>:<verb>`）| §0 naming reality | ⬜ | 🟡 divergent（design to shipped names）|

### 2.B you-talk 對話 dispatch（4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ02-CONV-001 | e2e | ⚠ provider | sidebar → 對話（/）| 送一則 message | ConversationView 渲染、agent run 起 | §2 / §3① | ⬜ | 🟡 |
| MACAPP-CUJ02-CONV-002 | integ | ⚠ mock LLM | agent run | 觀察 emit | 串 `agent_event` deltas（commands/agent.rs:84）| §2 sequence | ⬜ | 🟡 |
| MACAPP-CUJ02-CONV-003 | e2e | ✅ | 全新 user、空記憶 | 落地 /conversation（default route）| 有複利感空狀態：starter prompts + 「越用越複利 / 今天還不認識你」框架句 | §5 cold-start / findings low#8 | ⬜ | 🟥 FAIL [G9/code-backlog]：現況空 chat 無 onboarding-to-value 橋、無「讀你 git/files 開機」CTA（BIG-GOAL §3②C）。修法=純文案 + 1 card（surface-mac-app.md findings low#8）|
| MACAPP-CUJ02-CONV-004 | manual | ❌ | onboarding(英)→shell | 比對語言 | 單一 session 單語言（不該英文 onboarding → 中文 shell 鞭笞）| §2 / cross_surface#1 | ⬜ | 🟥 FAIL [code-backlog]：現況 onboarding 全英、shell 全中（PRIMARY_NAV 對話/儀表板/…中文）。修法=i18n 或單語收斂（surface-mac-app.md findings medium#5）|

### 2.C menu-bar hub 掛載（daily-loop 前提，6 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ02-MBD-001 | static | ✅ | - | `grep -rn "MenuBarDropdown" app/src/` | 有 mounting component import（非僅 self + types.ts）| §2 / §5 / G2 | ⬜ | 🟥 FAIL [G2/code-backlog]：現況 `MenuBarDropdown.tsx` 只被自己與 types.ts 引用、從未掛載。§2 整個 daily-loop 「always-on hub」建立在沒裝上的元件上。修法=掛進 Tauri tray webview window（refs §3.5 nspanel）。流程 surface-mac-app.md §2/findings high#2 |
| MACAPP-CUJ02-MBD-002 | manual | ❌ | daemon stopped | 看 menu-bar header dot | grey dot + "daemon stopped" + "Restart daemon"（DaemonHealth Grey）| §5 / SPEC-40 DaemonHealth | ⬜ | 🟥 FAIL [code-backlog]：MenuBarDropdown 未掛載 → header-dot 降級今天觀察不到（surface-mac-app.md §5 daemon offline 第二點）|
| MACAPP-CUJ02-MBD-003 | manual | ❌ | coach pending | menu-bar | orange dot + "Coach review ready"（visibility only_if_coach_pending）| §2 morning loop | ⬜ | 🟥 FAIL [code-backlog]：同上未掛載 |
| MACAPP-CUJ02-MBD-004 | e2e | ⚠ | popover 真掛後 | `Cmd+Shift+H` | transient always-on-top popover（Esc 關、不導主視窗）| §2 / G2 fix | ⬜ | ⬜ |
| MACAPP-CUJ02-MBD-005 | manual | ❌ | tray | 觀察 Dock | LSUIElement / activationPolicy=accessory（不佔 Dock/Cmd+Tab）| refs §3.5 menubar | ⬜ | ⬜ |
| MACAPP-CUJ02-MBD-006 | e2e | ⚠ | popover 開、開 file dialog | 失焦 | child window（dialog）不誤觸隱藏（需 flag 抑制）| refs §3.5 edge case | ⬜ | ⬜ |

---

## §3. 五能力面（ABIL，14 條）

> 流程權威: surface-mac-app.md §3。每能力對齊 BIG-GOAL §3①-⑤。

### 3.A ① see life+code（P2，3 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-ABIL1-CAP-001 | integ | ✅ | TempDir + EventKey | `/habit` ChipPopover → captureHabit → `habit_append`（capture_habit_wire）| event 加密寫入 | §3① / SPEC-22 | ⬜ | ✅（core 已驗，見 mac.md FH-002）|
| MACAPP-ABIL1-CAP-002 | integ | ⚠ focus | `/focus` FocusStartSheet → focus session | event 寫入（as-built focus timer）；audio/ASR path 不作 v0.6.0 驗收 | §3① / SPEC-21 P1-7 | ⬜ | 🟡 [v0.7.0+ deferred / SPEC-21 P1-7 DRIFT-blocked] |
| MACAPP-ABIL1-CAP-003 | integ | ⚠ Gemini mock | `/food` FoodCapturePanel → capture_food_wire（Gemini delegate）| event 寫入 | §3① / SPEC-20 / SPEC-00 §4.1 | ⬜ | 🟡 |

### 3.B ② compounding owned memory + skills（P3 #1，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-ABIL2-MEM-001 | integ | ✅ | seeded events | `/recall` RecallSearch（lib/recall.ts，FTS over 加密 Life Node events）| 回傳 well-formed 結果（讀側 wired，SPEC-16）| §3② read side | ⬜ | 🟡 |
| MACAPP-ABIL2-MEM-002 | e2e | ✅ | onboarded | 從 sidebar 導航到 skill-bank | 可達（有 Route + PRIMARY_NAV/LABS_NAV entry）| §3② / G1 | ⬜ | 🟥 FAIL [G1/code-backlog]（**blocker**）：`pages/skill-bank.tsx` 存在但 App.tsx 無 `<Route path="/skills">`、無 PRIMARY_NAV/LABS_NAV entry；唯一參照是 useDeepLinkRouter.ts 一個孤兒 '/skills' allowlist string → apex #1 表面 UI 不可達（∞ step）。修法=+1 LABS_NAV + +1 Route（15min quick-win）。流程 surface-mac-app.md findings blocker#1 |
| MACAPP-ABIL2-MEM-003 | static | ✅ | - | 讀 `pages/skill-bank.tsx` 的 `BACKEND_WIRED` | 標 false + 渲染誠實「尚未實作 Not yet wired」（非假已實作）| §3② / SPEC-31 | ⬜ | 🟡 honest-empty 正確（誠實債），待 G1 掛路由後可見 |
| MACAPP-ABIL2-MEM-004 | unit | ✅ | - | `cargo test --lib skill_wire`（embedding_search/skill_store 落地後）| 非 `unimplemented!()` | §3② / BRK-1 | ⬜ | 🟥 FAIL [BRK-1/code-backlog]：`skill_wire.rs:882/1127 unimplemented!()` 全 7 面 panic。能力 #1 後端 0 實作（BIG-GOAL §3②）。Wave 3 R1 落地 |

### 3.C ③ reactive review（DEFERRED，2 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-ABIL3-REV-001 | integ | ⚠ LLM mock | `/review` CoachReviewReader | 渲染 daily review（daily_review.rs + coach_scheduler_daemon.rs 寫 pending review）| §3③ reactive | ⬜ | 🟡 |
| MACAPP-ABIL3-REV-002 | static | ✅ | - | `grep -rE "GPS\|sensor.*nudge\|real.?time.*proactive" app/src/` | 0 命中（③ real-time proactive 不得進 v0.6.0，INV-8）| §3③ / INV-8 | ⬜ | ✅ DEFERRED 守恆（只 reactive 正確）|

### 3.D ④ safe unattended（governor + flight-recorder + escalation，3 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-ABIL4-GOV-001 | e2e | ⚠ daemon | 一個 awaiting_approval task | `/dashboard` TasksPanel 觀察該 task 狀態 | 顯示為 distinct「需要核准」高可見列 + inline Approve/Redirect/Stop | §3④ / G3 / BIG-GOAL §3④ | ⬜ | 🟥 FAIL [G3/code-backlog]（**high**）：`TasksPanel.tsx:22` DAEMON_STATUS_MAP 把 `awaiting_approval` → `pending`（待處理 chip）。同意門禁（apex 稱「產品本身」）被靜默抹除成被動 chip，user 無從 act → 把 bounded-consent 模型悄悄變成 §8/§12 明排除的 fire-and-forget。修法=別 collapse、渲染 amber 列 + 三鈕 + nav badge（surface-mac-app.md findings high#3）|
| MACAPP-ABIL4-GOV-002 | manual | ❌ | 長跑 task | 找 governor 設定 UI | 有 owner hard-brake config（wall-clock/token-budget/battery/stuck 門檻）| §3④ / BIG-GOAL §3④ | ⬜ | 🟥 FAIL [code-backlog]：現況無 governor config UI、無簽章 append-only flight-recorder viewer、無 escalation 面（differentiator largely planned）|
| MACAPP-ABIL4-GOV-003 | integ | ✅ | DispatchPlanner | `cluster_dispatch_wire::plan_dispatch`（SPEC-26 capability-match preview）| deterministic preview 渲染 | §3④ exists 面 | ⬜ | 🟡 |

### 3.E ⑤ life×work synthesis（P2 + dual-track，2 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-ABIL5-SYN-001 | e2e | ⚠ data | `/dashboard` | LifeStatsPanel + TasksPanel + CostPanel 同屏 | 三 panel 渲染（生活+工作 metrics 一屏）| §3⑤ partial | ⬜ | 🟡 |
| MACAPP-ABIL5-SYN-002 | integ | ⚠ LLM | Conversation 問跨域（如「低專注下午排 code review」）| reactive 推理可跨 life+work | §3⑤ reactive-only | ⬜ | 🟡 無專屬 surface（騎 coach+you-talk）|

---

## §4. 跨面 supervision handoff（phone ↔ backend，5 條）

> 流程權威: surface-mac-app.md §4。`phantom://` deep-link 在 Rust on_open_url 解析/allowlist → re-emit `deep-link://navigate`；`lib/deepLink.ts` 映射 hosts→routes（chat→/、mesh→/cluster、settings→/settings/<section>）。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ04-DL-001 | integ | ✅ | app 起 | 傳 `phantom://chat/<id>` | on_open_url validate → emit `deep-link://navigate` → 導 / | §4 / SPEC-17 §11.2 | ⬜ | 🟡 |
| MACAPP-CUJ04-DL-002 | integ | ✅ | app 起 | 傳 `phantom://settings/agents` | 導 /settings/agents | §4 deepLink.ts | ⬜ | 🟡 |
| MACAPP-CUJ04-DL-003 | integ | ✅ | app 起 | 傳 `phantom://coach/review?date=...`（SPEC-41 §10.6 cold-launch landing）| 導 /review（不落 null）| §4 gap | ⬜ | 🟥 FAIL [code-backlog]：`deepLink.ts` 無 `coach` host → phone push 點進 mac review 落 null（無 nav）。修法=加 coach/supervise host + Rust allowlist（surface-mac-app.md §4 To build a）|
| MACAPP-CUJ04-DL-004 | integ | ✅ | app 起 | 傳 `phantom://supervise/<task>` | 解析為 approve/redirect/stop channel | §4 gap | ⬜ | 🟥 FAIL [code-backlog]：無 supervise deep-link、無 tool-call-boundary 核准 channel（單向 nav plumbing only）|
| MACAPP-CUJ04-DL-005 | integ | ✅ | 非 allowlist URL | 傳惡意 `phantom://evil` | drop + 不 nav（defense-in-depth、僅 log code 不 log raw URL）| §4 / §13 privacy | ⬜ | 🟡 |

---

## §5. error / empty / offline / cold-start（7 條）

> 流程權威: surface-mac-app.md §5。誠實空/未接線狀態是 codebase 紀律（SPEC-31 NO FAKING）。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-CUJ05-ERR-001 | e2e | ✅ | daemon stop | 看 CostPanel/TasksPanel | 「無法連接 Daemon … [重試]」（TasksPanel.tsx:106）| §5 offline panels | ⬜ | ✅（已驗可達）|
| MACAPP-CUJ05-ERR-002 | e2e | ✅ | 單機無 peer | ClusterStatusDashboard | 「孤立模式（單機跑）」+ Add-peer hint（line:67）；DispatchPlanner disable + 「尚無連線的 peer」 | §5 empty cluster | ⬜ | ✅ |
| MACAPP-CUJ05-ERR-003 | e2e | ✅ | 空 vault | /recall 空態 | 「尚無事件 — 用專注/習慣/飲食頁記錄後會出現在這裡」（誠實空態）| §5 empty memory | ⬜ | 🟡（誠實但無 cold-start backfill affordance）|
| MACAPP-CUJ05-ERR-004 | e2e | ✅ | backend 未接該 step | onboarding soft-note | 「State advanced locally; backend not yet wired」(soft) vs 「Could not complete this step」(hard) 區分 thrown/deferred | §5 backend-not-wired | ⬜ | ✅ |
| MACAPP-CUJ05-ERR-005 | e2e | ✅ | web/browser（無 Tauri IPC）| 開 web 版 | App.tsx listen() 無害 reject；safeInvoke httpFallback | §5 no-Tauri | ⬜ | ✅ |
| MACAPP-CUJ05-ERR-006 | e2e | ✅ | day-0 全新 user | /conversation default | cold-start 有 backfill「讀你既有 git/files」affordance（BIG-GOAL §3②C） | §5 cold-start | ⬜ | 🟥 FAIL [G9/code-backlog]：現況只顯示空，無 retroactive activation。同 CONV-003 |
| MACAPP-CUJ05-ERR-007 | manual | ❌ | provider key 失效 | set_provider | 友善 validate 失敗（SPEC-41 §11 MACOS_PROVIDER_VALIDATE_FAIL planned）| §5 provider invalid | ⬜ | 🟡 best-effort try/catch only |

---

## §6. SPEC-31 NO-FAKING 假稽核守護（NOFAKE，4 條）

> 流程權威: surface-mac-app.md §3④「⚠️ 一個假的 audit log 已出貨且可達」。`SecurityPanel.tsx`（路由 `/settings/security`）render 硬編 `MOCK_EVENTS`（AUD-001..010，含「審核中」狀態）= 可達假 audit/flight-recorder viewer，違反 SPEC-31 NO-FAKING。對映 refs G4。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-NOFAKE-001 | static | ✅ | - | `grep -rn "MOCK_EVENTS" app/src/screens/macos/SecurityPanel.tsx` | **0 命中**（無硬編假稽核事件）| §3④ / SPEC-31 / G4 | ⬜ | 🟥 FAIL [G4/code-backlog]：現況 `SecurityPanel.tsx:36+` 起硬編 `MOCK_EVENTS`（AUD-001..010），render 出可達的假 audit/flight-recorder viewer。違反 SPEC-31 NO-FAKING + Charter §E.3 T5。修法=移除 MOCK_EVENTS 或明標 `[demo 模式]`（refs §五 quick-win，1hr）|
| MACAPP-NOFAKE-002 | e2e | ⚠ daemon off | /settings/security、daemon 不可達 | 看畫面 | 誠實空態 / 「無資料」而非渲染假 AUD-* 列 | §3④ / SPEC-31 | ⬜ | 🟥 FAIL [G4/code-backlog]：現況 offline → 退化成 mock 假畫面 |
| MACAPP-NOFAKE-003 | static | ✅ | - | `grep -rEn "AUD-0[0-9]+" app/src/` | 0 命中（無假審計 ID 種子）| SPEC-31 | ⬜ | 🟥 FAIL [G4/code-backlog]：MOCK_EVENTS 含 AUD-001..010 假 ID |
| MACAPP-NOFAKE-004 | e2e | ⚠ real daemon | /settings/security、daemon 有真 audit | 看畫面 | 顯示真 `get_audit_log` 資料（非 mock fallback）；「審核中」為**真** live consent gate 非 display-only | §3④ / G3 | ⬜ | 🟥 FAIL [code-backlog]：現況 audit 「審核中/approve」outcome 為 display-only 非 live gate（與 G3 同根）|

---

## §7. macOS 平台專屬（PLAT，6 條）

> 流程權威: surface-mac-app.md §1（Keychain）+ mac.md §6 桌面對照。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MACAPP-PLAT-KEY-001 | integ | ✅ | TempDir | created_identity 後 | ed25519 私鑰存 macOS Keychain（SPEC-40 §7.5）非明文檔 | §1 / SPEC-12 P4 | ⬜ | 🟡 |
| MACAPP-PLAT-GK-001 | manual | ❌ | 簽章 .app | 雙擊開 .app | Gatekeeper 不擋（已 codesign）| SPEC-63 codesign | ⬜ | ⬜ |
| MACAPP-PLAT-GK-002 | manual | ❌ | 未簽 .app | 開 | macOS 印「無法驗證開發者」 | SPEC-63 | ⬜ | ⬜ |
| MACAPP-PLAT-TCC-001 | manual | ❌ | TCC 鎖 | 截圖/麥克風請求 | 印權限請求對話框（不寫垃圾）| macOS TCC | ⬜ | ⬜ |
| MACAPP-PLAT-NOT-001 | manual | ❌ | first coach delivery | coach 推 notification | 印 NSUNotificationCenter 權限請求 | macOS notification / SPEC-24 | ⬜ | ⬜ |
| MACAPP-PLAT-REV-001 | e2e | ✅ | onboarded | Settings → Security 找「Delete all my data」 | 有 GUI 資料刪除 affordance（含 confirm）| §reversibility / findings medium#7 | ⬜ | 🟥 FAIL [code-backlog]：現況無 GUI 'delete all'；`phantom data delete --all` kill-switch CLI-only；sidebar 「登出/重新設定」只清 localStorage 不刪 key/token。違反 INV-12 可逆性。修法=Settings 加 delete-all action（surface-mac-app.md findings medium#7）|

---

## §8. 統計

```
Total: 70 條（MACAPP-*）

By §:
  §1 onboarding:        18
  §2 daily loop:        16
  §3 五能力:            14
  §4 supervision:        5
  §5 error/empty:        7
  §6 NO-FAKING:          4
  §7 平台專屬:           6

By 狀態（2026-06-12 建檔，依審查者欄 emoji 機械重數）:
  ✅ 過 / 已驗:          7
  🟡 partial:           26
  🟥 現況 FAIL (code-backlog): 25
  ⬜ 未做:               12
```

### §8.1 「現況 FAIL / code-backlog」清單（25 條 → 對映 refs G-gap）

| FAIL case | 根因 code 缺陷 | gap 編號 | 修法落點 |
|---|---|---|---|
| MACAPP-CUJ01-FORK-001/002/004 | 無 backend-brain fork / 無心智模型句 / cloud-sandbox 非選項 | G-fork/G8 | onboarding FSM 擴充 + 文案 |
| MACAPP-CUJ01-SCK-002/003/004/006 | StartupCheck 每次啟動阻斷 + force-enter 按鈕 | G6 | 非阻斷化 + 移除按鈕 |
| MACAPP-CUJ02-SHC-005 | shortcut→整頁 route swap 奪焦（無真 NSPopover）| G2/G7 | tray webview popover |
| MACAPP-CUJ02-CONV-003 | 冷啟空 chat 無複利框架 | G9 | starter prompts + card |
| MACAPP-CUJ02-CONV-004 | 英語 onboarding → 中文 shell 鞭笞 | — | i18n / 單語收斂 |
| MACAPP-CUJ02-MBD-001/002/003 | MenuBarDropdown 死碼未掛載 | G2 | 掛 tray webview window |
| MACAPP-ABIL2-MEM-002 | skill-bank 無 Route 不可達（blocker）| G1 | +1 NAV + +1 Route（15min）|
| MACAPP-ABIL2-MEM-004 | skill_wire unimplemented!() | BRK-1 | Wave 3 R1 schema |
| MACAPP-ABIL4-GOV-001/002 | awaiting_approval→pending 抹除 + 無 governor UI | G3 | 別 collapse + amber 列 + 三鈕 |
| MACAPP-CUJ04-DL-003/004 | deepLink 無 coach/supervise host | — | 加 host + allowlist |
| MACAPP-CUJ05-ERR-006 | cold-start 無 retroactive backfill | G9 | 同 CONV-003 |
| MACAPP-NOFAKE-001/002/003/004 | SecurityPanel MOCK_EVENTS 假稽核 | G4 | 移除 MOCK 或標 demo |
| MACAPP-PLAT-REV-001 | 無 GUI delete-all（可逆性 CLI-only）| — | Settings 加 action |

> **紀律重申**：上表每條的「修法落點」改的都是 **code**（`app/src/*` / `app/src-tauri/src/*` / `core/src/*`），**不是本檔**。本檔僅凍結驗收條件，使 code 修好後可用同一條 case 機械驗證回綠。

---

## §9. CLI runner / 自動化 hook 範例

```bash
#!/bin/bash
# scripts/test-runner-mac-app.sh — 跑所有 Auto=✅ 的 static/integ 守護
set -e

# §6 NO-FAKING 假稽核守護（這幾條現況預期 FAIL，修碼後才回綠）
! grep -rn "MOCK_EVENTS" app/src/screens/macos/SecurityPanel.tsx   # NOFAKE-001
! grep -rEn "AUD-0[0-9]+" app/src/                                  # NOFAKE-003

# §3② G1 skill-bank 可達守護
grep -rn "path=[\"']/skills" app/src/App.tsx                        # MEM-002（修碼後）

# §2C G2 MenuBarDropdown 掛載守護
test "$(grep -rln 'import.*MenuBarDropdown' app/src/ | grep -v types.ts | wc -l)" -gt 0  # MBD-001

# §1B fork 守護
grep -rE "cloud.*sandbox|linux.*sandbox" app/src/                   # FORK-001

# §3③ INV-8 proactive 守恆（應 0 命中 = PASS）
! grep -rE "GPS|real.?time.*proactive" app/src/                     # REV-002

echo "（現況：FAIL 守護多為紅，待 code-backlog 修碼回綠）"
```

> Playwright / tauri-driver e2e（onboarding FSM、shortcut 路由、dashboard 狀態列）為 Phase 2，對齊 mac.md §14 自動化 roadmap。
