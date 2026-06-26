# Windows Desktop App (Tauri 2) — 全測試用例庫 v1 (2026-06-12)

> **Surface**: `win desktop app` = Tauri 2 GUI（`app/src-tauri`，productName "Phantom Mesh"，identifier `ai.phantommesh.app`，v0.6.0）。與 mac/linux/mobile **同一份 `app/` React+Rust codebase**；本檔記 Windows deltas。其 headless 雙生子 = SPEC-46 治理的 `phantom.exe` CLI（GUI 把它 bundle 成 daemon binary），故 CLI invariant I1–I13 滲入 app 的 process model。
>
> **覆蓋範圍**: 桌面 daily-loop 主要 CUJ × Windows 桌面行為（WebView2 / in-proc runtime :7878 / DPAPI + Credential Manager / 全域捷徑 Modifiers / Defender Firewall / SmartScreen / 註冊表 deep-link）+ 跨 SPEC P4/SPEC-31 invariant。
>
> **權威來源**: 流程 = `docs/superpowers/specs/2026-06-12-platform-flows-design/surface-win-app.md`（已逐面驗證、附 file/line cite）；參考 = `plans/desktop-app-references-2026-06-12.md`、SPEC-46（Windows CLI behavior）。Code 真相裁決一律以 `app/src/*` + `app/src-tauri/src/*` + `core/src/*` as-built 為準（SSOT 規則 A.2.4）。
>
> **docs-only 鐵則**: 本檔只記驗收條件，不改 code。已知 code 問題（Modifiers::SUPER 在 Windows 被攔、asset-protocol enable:false、uninstall taskkill /F I9 violation、MOCK_EVENTS 假稽核、awaiting_approval→pending、firewall prompt、StartupCheck race、stale phantom.exe shadowing、demo-relay no-op）一律「可重複測試的驗收條件 + 標 [現況 FAIL / code-backlog]」，FAIL case 要求改的是 **code**。
>
> **編號規約**: `WINAPP-{CUJ|ABIL|PLAT|INV|NOFAKE}-{feat}-{nnn}`，ID 永不重用。每條對齊 INV-15（流程 + 驗收 + 測試三件套）。與 `mac-app.md` MACAPP-* 對等格式。

---

## §0. Schema legend（對齊 mac-app.md / mac.md，CLI runner / WebDriver 可直讀）

共用 token/schema 字典見 [`README.md`](README.md)；本檔只列 Windows app 端補充。

| 欄 | 意義 |
|---|---|
| **ID** | 唯一識別（永不重用） |
| **Type** | `unit` / `integ`（Tauri command）/ `e2e`（WebDriver/tauri-driver/Playwright）/ `manual` / `static`（grep/AST 守護）/ `monitor`（canary cron） |
| **Auto** | `✅` 全自動 / `⚠` 需 env/fixture / `❌` manual only / `⏰` cron |
| **Setup** | 跑前準備 |
| **cmd** | 實際命令或步驟（PowerShell / JS / shell） |
| **expected** | 通過條件 |
| **Verifies** | 流程文件章節 + SPEC G[X] / I[X] / 能力編號 |
| **last_run** | 最後驗證日時 + runner |
| **狀態** | `✅` 過 / `🟡` partial / `🔴` 重要缺 / `🟥` 現況 FAIL（code-backlog）/ `⬜` 未做 |

> **GROUNDING LEGEND**（沿用 surface-win-app.md）：[EXISTS]=repo 今有；[PARTIAL]=wired but stubbed/mocked；[PLANNED]=spec-only；[SPEC-ONLY]=spec 在 file 缺。
>
> **`🟥` = 現況 FAIL / code-backlog**：對映 refs G1–G10 + SPEC-46 I-violation。修法在 **code**，本檔只凍結驗收條件。

---

## §1. CUJ-01: First-run / onboarding（login + 選 + 接後端大腦，16 條）

> 流程權威: surface-win-app.md §1。FSM = `core/src/onboarding_wire.rs`：`fresh_install → created_identity → joined_cluster → set_provider → first_reply_received`。Gate = `App.tsx:113-115` `localStorage[ONBOARDED_KEY]`。D1–D5 baked in。

### 1.A onboarding gate + FSM + DPAPI 身分（7 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ01-ONB-001 | e2e | ⚠ fresh | 清 localStorage | 冷啟 Phantom Mesh.exe | `localStorage[ONBOARDED_KEY] !== "true"` → render `<OnboardingHello>`（App.tsx:113-115）| §1 gate | ⬜ | ⬜ |
| WINAPP-CUJ01-ONB-002 | unit | ✅ | - | `cargo test --lib onboarding_wire::tests` | FORWARD_ORDER 5 step 順序正確 | §1 FSM SSOT | ⬜ | ✅ existing |
| WINAPP-CUJ01-ONB-003 | integ | ⚠ broker mock | `BROKER_URL=mock` | `broker_login_start` → 開系統瀏覽器（tauri-plugin-opener）→ `phantom://oauth/callback?p=<b64>` | OS 路由回 on_open_url → validate_oauth_callback_url → emit `deep-link://oauth-callback` → broker_login_finish | §1 identity/login / lib.rs:375-413 | ⬜ | 🟡 |
| WINAPP-CUJ01-ONB-004 | integ | ✅ | clean HKCU | install 後查註冊表 | `HKCU\Software\Classes\phantom` deep-link handler 已寫（Tauri-managed）| §1 Windows delta / tauri.conf deep-link.desktop.schemes | ⬜ | ⬜ |
| WINAPP-CUJ01-ONB-005 | integ | ✅ | TempDir HOME | `onboarding_advance(created_identity)` | ed25519 master seed 以 **DPAPI** per-user wrap + 存 **Windows Credential Manager**（`KeystoreBackend::WindowsCredentialManager` → name `windows-credman`）| §1 / identity_wire.rs:202,475,1062+ / SPEC-12 | ⬜ | 🟡（SPEC-12 標 Win Keychain v0.7.0 partial）|
| WINAPP-CUJ01-ONB-006 | integ | ✅ | TempDir | `onboarding_advance(joined_cluster)` | single-node `serve` + mDNS advertise（本機=backend）| §1 joined_cluster / onboarding-hello.tsx:96-103 | ⬜ | 🟡 |
| WINAPP-CUJ01-ONB-007 | integ | ⚠ mock | mock claude/codex token + Ollama | `set_provider`：`read_claude_cli_token`/`read_codex_token`/`detect_local_servers`（讀 Windows-path token stores，credential_scanner.rs）| 偵測 + drag-rank + Ollama fallback | §1 set_provider / D5 / Windows delta | ⬜ | 🟡 |

### 1.B 後端大腦 fork（apex §2，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ01-FORK-001 | e2e | ⚠ fresh | joined_cluster step | 讀文案 | 出現「This machine is now your backend brain; your phone will connect to it」級心智模型句 | §1 / findings medium#8 | ⬜ | 🟥 FAIL [G8/code-backlog]：現況 joined_cluster 只說「Starting this machine as a mesh node so it is discoverable」，從不講 backend-brain 角色 + 兩選項 fork 被靜默 collapse 成選項 1。修法=改文案 + QR/pairing hint（surface-win-app.md findings medium#8）|
| WINAPP-CUJ01-FORK-002 | integ | ⚠ | demo-relay handoff | `onboarding_start_demo_relay_handoff` | 非 no-op：真 handoff 或明示「coming soon」 | §1 cloud sandbox | ⬜ | 🟥 FAIL [code-backlog]：現況 handoff 走 `run_or_unimplemented(start_demo_relay_handoff)` 回 `onboarding.not_yet_wired` Err（onboarding_wire.rs:80-82,160-163 自測斷言）= PARTIAL-stub。SPEC-52 demo-relay SPEC-ONLY。cloud-sandbox picker [PLANNED] |
| WINAPP-CUJ01-FORK-003 | manual | ❌ | onboarded | Settings → Cluster | MeshPeerAddWizard 可達（peer-join Stage 2）| §1 own-desktop peer-join | ⬜ | 🟡 wizard EXISTS、deferred 誠實 |
| WINAPP-CUJ01-FORK-004 | e2e | ⚠ fresh | joined_cluster | 找配對 hint | 有 phone-app QR/pairing 引導（連到 MVP 另一半）| §1 / findings medium#8 fix | ⬜ | 🟥 FAIL [G8/code-backlog]：現況 0 引導到 phone |

### 1.C StartupCheck 自檢 race（Windows 冷啟 false-failure，5 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ01-SCK-001 | integ | ✅ | onboarded、runtime ready | 冷啟 → StartupCheck | runtime/providers/LLM healthy → 800ms auto-advance | §1 self-check / StartupCheck.tsx | ⬜ | 🟡 |
| WINAPP-CUJ01-SCK-002 | e2e | ⚠ cold | 冷啟（in-proc runtime 需 ~15-18s init）| 啟動瞬間觀察 StartupCheck | **非** false unhealthy：有界輪詢「Starting your node…」直到 ready，逾時才 fail | §1 / findings high#2 | ⬜ | 🟥 FAIL [G6/code-backlog]（**high**）：`useSystemHealth.runCheck()` 單射探 localhost:7878/api/status（useSystemHealth.ts:52-69），但 in-proc PhantomMeshRuntime 需 ~15-18s init（lib.rs:613-619）。冷啟探針早於 runtime bind → 顯示 `overallStatus='unhealthy'` → 「系統無法正常運作」+ retry/skip，但其實只是在開機。違反「冷啟不該感覺壞掉」。修法=有界輪詢 ~1s×25s 或 Rust emit `runtime://ready` 事件 gate（surface-win-app.md findings high#2）|
| WINAPP-CUJ01-SCK-003 | manual | ❌ | 連續 5 次 first-of-session 冷啟 | 計每次「重新檢查」click 數 | 0 click（auto-advance on ready）| findings high#2 | ⬜ | 🟥 FAIL [G6/code-backlog]：現況每次 cold launch = 1 false-failure 屏 + ≥1 手動 click |
| WINAPP-CUJ01-SCK-004 | e2e | ⚠ fail | runtime 真壞 | StartupCheck | per-check fail icon + 重新檢查/重新設定/跳過強制進入（StartupCheck.tsx:89-114）| §5 self-check fail | ⬜ | 🟡（fail 面正確，但 race 造成 false-fail）|
| WINAPP-CUJ01-SCK-005 | integ | ✅ | - | 監聽 runtime init | 有 `runtime://ready` 事件可 gate（取代 HTTP poll 自己擁有的 port）| findings high#2 fix | ⬜ | 🟥 FAIL [G6/code-backlog]：現況無 ready 事件 |

---

## §2. CUJ-02: Daily-use core loop（desktop shell，14 條）

> 流程權威: surface-win-app.md §2。`useIsMobile()` 在 Windows 桌面為 false → render sidebar shell（非 mobile AppTemplate，App.tsx:148-281）。tray 常駐 runtime；關窗不殺 node。

### 2.A 全域捷徑 Modifiers（Windows 攔截，5 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ02-SHC-001 | static | ✅ | - | 讀 `app/src-tauri/src/lib.rs:760` global-shortcut 註冊 | Windows 用 `Modifiers::CONTROL\|ALT`（Ctrl+Alt+F/H，Windows-safe）非 `Modifiers::SUPER` | §2 / cross_surface#B / findings high#1 | ⬜ | 🟥 FAIL [G5/code-backlog]（**high**）：現況 `lib.rs:760` 用 `Modifiers::SUPER`（= Windows 鍵）。Win+Shift+F 開 Windows Feedback Hub、Win+Shift+<key> 多被 shell 攔 → focus/habit 熱鍵在 Windows 既標錯（標籤仍寫 Cmd+Shift）又常失效。CommandPalette.tsx:36 已用 `metaKey\|\|ctrlKey` 證明可分支。修法=OS 分支 Modifiers（surface-win-app.md findings high#1）|
| WINAPP-CUJ02-SHC-002 | manual | ❌ | app 起 | 按標籤上的 `Win+Shift+F` | 真的觸發 focus capture（非開 Feedback Hub / 無反應）| §2 / findings high#1 | ⬜ | 🟥 FAIL [G5/code-backlog]：現況常被 Windows shell 吞 |
| WINAPP-CUJ02-SHC-003 | static | ✅ | - | grep tray label `lib.rs:694-695` + App.tsx 註解 | 標籤渲染自實際註冊組合（Windows 顯「Ctrl+Alt+F」非硬編「Cmd+Shift+F」）| §2 / findings high#1 | ⬜ | 🟥 FAIL [G5/code-backlog]：現況硬編 Cmd+Shift 字串，Windows 顯示錯誤 |
| WINAPP-CUJ02-SHC-004 | integ | ✅ | shortcut 修好後 | 觸發已註冊熱鍵 | emit `shortcut://focus`/`shortcut://chip` → navigate /focus/habit | §2 daily loop | ⬜ | ⬜ |
| WINAPP-CUJ02-SHC-005 | integ | ✅ | tray | click 開始專注/記錄習慣/今日回顧 | emit `shortcut://focus\|chip\|review` → route /focus /habit /review | §2 / lib.rs tray | ⬜ | 🟡 |

### 2.B you-talk + command palette（5 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ02-CONV-001 | e2e | ⚠ provider | / Conversation（default route）| 送 message → `run_agent`/`send_message` | 串 `agent_event` stream（task-started/tool-call/delta/done/error，agent.rs:84,148）| §2 / §3① | ⬜ | 🟡 |
| WINAPP-CUJ02-CONV-002 | e2e | ✅ | 全新 user 空記憶 | 落地 / Conversation | cold-start welcome grid 有複利框架 + retro-ingest CTA | §5 / findings medium#6 | ⬜ | 🟥 FAIL [G9/code-backlog]：現況 ConversationView welcome grid（學業/事業/健康，line:29-66）+ 空 /recall「尚無事件」，無 retro-ingest（讀 git/files/logs，BIG-GOAL §3②C）→ 首跑感覺壞掉。修法=空態教複利 loop（surface-win-app.md findings medium#6）|
| WINAPP-CUJ02-CONV-003 | manual | ❌ | onboarding(英)→shell | 比對語言 | 單一 session 單語言 | §2 / cross_surface#A | ⬜ | 🟥 FAIL [code-backlog]：現況 onboarding 硬英（D2 無語言選）→ shell 全中（sidebar App.tsx:47-56、StartupCheck 系統自檢、welcome grid ConversationView.tsx:29-66）。修法=shell localize 對齊英 / i18n toggle（surface-win-app.md findings medium#5）|
| WINAPP-CUJ02-CONV-004 | e2e | ⚠ | 主 shell（App.tsx）| 按 `Ctrl+K` | 主 shell 有 command palette → react-router navigate 各 PRIMARY_NAV/LABS_NAV route + quick actions | §2 / findings medium#7 | ⬜ | 🟥 FAIL [code-backlog]：現況 CommandPalette.tsx 只接 pageStore/Labs 區（setArea tasks/devices/settings），主 React-Router 10-item sidebar 無 palette → 13 surface 只能滑鼠點。修法=主 shell 掛 palette（surface-win-app.md findings medium#7）|
| WINAPP-CUJ02-CONV-005 | static | ✅ | - | 讀 CommandPalette.tsx:36 | 用 `metaKey\|\|ctrlKey`（跨 OS 正確分支樣板）| §2 cross_surface | ⬜ | ✅（已正確，為 SHC-001 fix 範本）|

### 2.C 桌面 shell 路由 + onboarding back-nav（4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ02-NAV-001 | e2e | ✅ | onboarded | 渲染主 shell | sidebar 10 項（非 mobile AppTemplate；useIsMobile=false）| §2 / App.tsx:148-281 | ⬜ | 🟡 |
| WINAPP-CUJ02-NAV-002 | e2e | ⚠ | onboarding set_provider step | 找 Back 按鈕 | 非初始 step 皆可 Back（map 到 FSM rollback edge）| §2 / findings low#9 | ⬜ | 🟥 FAIL [code-backlog]：現況 rollback 只在 joined_cluster→created_identity（onboarding-hello.tsx:225 canGoBack）；set_provider/first_reply 無 Back，違反可逆性（BIG-GOAL §6）。修法=每非初始 step 允許 Back（surface-win-app.md findings low#9）|
| WINAPP-CUJ02-NAV-003 | e2e | ⚠ | set_provider 空態 | 看「Sign in to a CLI / start Ollama」提示 | 有 Re-scan 按鈕 + inline 貼 API key path（不必離開 app）| §5 / findings low#10 | ⬜ | 🟥 FAIL [code-backlog]：現況提示「come back」但無 in-app 重掃/貼 key 路徑（onboarding-hello.tsx:495-498）→ 須離 app 開 terminal auth CLI 再回（5+ step）。修法=加 Re-scan + inline key（surface-win-app.md findings low#10）|
| WINAPP-CUJ02-NAV-004 | manual | ❌ | tray | 關主視窗 | runtime/node 不死（tray 常駐）| §2 tray resident | ⬜ | 🟡 |

---

## §3. 五能力面（ABIL，16 條）

> 流程權威: surface-win-app.md §3。

### 3.A ① see life+code（P2，含 asset-protocol，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-ABIL1-CAP-001 | integ | ✅ | TempDir + EventKey | `/focus` `/habit` `/food` `/timeline`（screens/macos/* 在 Windows 復用）→ focus_*/habit_*/food_analyze/events_query | event 加密寫入 | §3① / lib.rs:869-884 | ⬜ | 🟡 |
| WINAPP-ABIL1-CAP-002 | integ | ✅ | Windows | 截圖落 `$HOME/.phantom-mesh/screenshots/**` | 截圖檔寫入 | §3① Windows delta | ⬜ | 🟡 |
| WINAPP-ABIL1-CAP-003 | static | ✅ | - | 讀 `app/src-tauri/tauri.conf.json:28` `assetProtocol.enable` | `enable: true`（截圖可經 `asset:` in-app 渲染）| §3① / P2-14 / G(asset) | ⬜ | 🟥 FAIL [P2-14/code-backlog]：現況 `assetProtocol.enable: false`（tauri.conf.json:28），雖 `assetProtocol.scope.allow` 已配（line:31），但 protocol disabled → 截圖**無法**經 `asset:` in-app 渲染（scope 是半真）。須翻 `enable: true` 才能渲染。標 [PARTIAL]。對映 Charter §C P2-14（enable:false 勝，標記改 PARTIAL）|
| WINAPP-ABIL1-CAP-004 | integ | ✅ | Windows | 硬體探測 `scripts/detect_hardware.ps1`（bundled resource）| PowerShell 探測跑通（Windows-specific）| §3① / tauri.conf.json:71 | ⬜ | 🟡 |

### 3.B ② compounding owned memory + skills（P3 #1，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-ABIL2-MEM-001 | integ | ✅ | seeded events | `/recall` RecallSearch（`recall_search`，recall_wire.rs）| 回 well-formed 結果（讀側 EXISTS）| §3② / SPEC-16 | ⬜ | 🟡（thin but wired）|
| WINAPP-ABIL2-MEM-002 | integ | ✅ | - | Settings → Memory（`get_memory_observations`/`get_memory_stats`/`search_memory`，MemoryPanel.tsx）| panel 渲染 | §3② surfaces EXISTS | ⬜ | 🟡 |
| WINAPP-ABIL2-MEM-003 | unit | ✅ | - | `cargo test --lib skill_wire`（embedding_search/skill_store 落地後）| 非 `unimplemented!()` | §3② / BRK-1 | ⬜ | 🟥 FAIL [BRK-1/code-backlog]：`skill_wire.rs:881-882,1119-1127` literally `unimplemented!("Stage 4: ...")`。「turn work into self-running skills」有 UI affordance 但無 compounding 後端。Wave 3 R1 落地 |
| WINAPP-ABIL2-MEM-004 | static | ✅ | - | grep `memory.db`/`conversations/` 加密 | events age 加密（E004 delivered）；memory.db/conversations 標 plaintext v0.7.0（誠實，INV-10）| §3② / BIG-GOAL §4 table | ⬜ | 🟡 honest scope（v0.7.0 明文已誠實標）|

### 3.C ③ reactive review（DEFERRED，2 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-ABIL3-REV-001 | integ | ⚠ LLM mock | `/review` CoachReviewReader（`daily_review_load`/`daily_review_generate`）+ `/reflection`（`partner_latest_reflection`）| 渲染 daily review | §3③ reactive / lib.rs:885-898 | ⬜ | 🟡 |
| WINAPP-ABIL3-REV-002 | static | ✅ | - | `grep -rE "GPS\|sensor.*push\|real.?time.*proactive" app/src/` | 0 命中（無 GPS/sensor push；只 coach-review-ready desktop notification，INV-8）| §3③ / INV-8 | ⬜ | ✅ DEFERRED 守恆 |

### 3.D ④ safe unattended（governor + flight-recorder + escalation，差異化最少建，4 條）

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-ABIL4-GOV-001 | integ | ✅ | - | 觀察 daemon **watchdog**（daemon.rs:285-366，max 5 auto-restart / 120s cooldown / owner 可關）| watchdog 為真 owner-side governor primitive | §3④ EXISTS | ⬜ | 🟡（唯一已建 governor primitive）|
| WINAPP-ABIL4-GOV-002 | e2e | ⚠ daemon | awaiting_approval task | `/dashboard` 或 Security 看該 task | distinct「需要核准」高可見 + inline Approve/Reject/Stop（非折成 pending）| §3④ / G3 / BIG-GOAL §3④ | ⬜ | 🟥 FAIL [G3/code-backlog]（**high**）：`TasksPanel.tsx:21-29` DAEMON_STATUS_MAP 把 `awaiting_approval`→`pending`；audit「審核中/approve」outcome display-only 非 live consent gate。同意門禁被抹除→悄變 fire-and-forget（§12 anti-goal）。修法=別 collapse + amber 列 + 三鈕 + nav badge |
| WINAPP-ABIL4-GOV-003 | e2e | ⚠ | desktop shell | 找 Unattended/Long-run 面 | 有 governor 四 hard-brake（wall-clock/token-budget/battery/stuck）config + 唯讀 flight-recorder viewer + 「escalations go to <phone>」| §3④ / findings high#4 | ⬜ | 🟥 FAIL [G(gov)/code-backlog]（**high**）：現況 desktop shell 無 governor/budget/battery 設定、無 audit/flight-recorder viewer、無「長跑會升級到手機」affordance（App.tsx nav 無此 route）。桌面正是 overnight brain 跑處，缺此面 = 讀作 fire-and-forget（BIG-GOAL §12）。`get_estop_status`/`get_audit_log` command EXISTS 但無 UI 串。修法=加 Unattended/Long-run sidebar+Settings 面（surface-win-app.md findings high#4）|
| WINAPP-ABIL4-GOV-004 | integ | ✅ | unattended 跑、Win lock/sleep | runtime 維持 :7878 listen | 鎖屏/睡眠下 phone 仍可達；battery/wall-clock brake 讀 Win32 power state | §3④ Windows delta | ⬜ | 🟥 FAIL [code-backlog]：listen 部分 EXISTS，但 battery/wall-clock brake 讀 Win32 power state **not yet wired** |

### 3.E ⑤ life×work synthesis（P2，2 條）— 略同 mac-app §3.E

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-ABIL5-SYN-001 | e2e | ⚠ data | `/dashboard` | LifeStatsPanel + CostPanel + NodeInfoPanel 同屏（`life_stats`/`get_costs`/cluster status，lib.rs:792-794,833,893）| 三 panel 渲染 | §3⑤ / EXISTS | ⬜ | 🟡 |
| WINAPP-ABIL5-SYN-002 | integ | ⚠ LLM | Conversation 問「依今日專注排工作」 | reactive synthesis（非 auto-schedule，那會是 ③）| §3⑤ reactive | ⬜ | 🟡 |

---

## §4. 跨面 supervision handoff（phone ↔ Windows backend brain，6 條）

> 流程權威: surface-win-app.md §4。Windows app = backend brain；phone = supervision remote。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ04-DL-001 | integ | ✅ | app 起 | broker auth + `broker_sync_from_vault`/`broker_list_cached_peers` | 兩面同 broker 認證、拉 peer | §4 / lib.rs:780-786 | ⬜ | 🟡 |
| WINAPP-CUJ04-DL-002 | integ | ✅ | app 起 | `broker_register_self_peer` | Windows node 註冊使 phone 可發現 | §4 reachability | ⬜ | 🟡 |
| WINAPP-CUJ04-DL-003 | integ | ✅ | app 起、phone 連 | phone 打 axum `:7878 /rpc/*`（同網 mDNS / 跨 NAT broker/Tailnet `*.ts.net` CSP-allowed）| /rpc/* 回應 | §4 / lib.rs:632-664 / tauri.conf.json:26 | ⬜ | 🟡 |
| WINAPP-CUJ04-DL-004 | integ | ✅ | app 起 | 傳 `phantom://chat/:id`/`phantom://settings/*`/`phantom://mesh/:peer` | on_open_url→dispatch_deep_link→emit `deep-link://navigate`（僅 chat/settings/mesh host forward）| §4 / lib.rs:425-451 / SPEC-17 §11.2 | ⬜ | 🟡 |
| WINAPP-CUJ04-DL-005 | integ | ✅ | 非 allowlist | 傳 `phantom://evil` | drop + 僅 log length/code（不 log raw URL，§13 privacy）| §4 / lib.rs:452-466 | ⬜ | 🟡 |
| WINAPP-CUJ04-DL-006 | e2e | ⚠ daemon | 長跑碰 budget/stuck/high-risk 邊界 | 觀察 escalation transport（tool-call-boundary → push phone → consent token back）| phone 被喚 + approve/redirect/stop async bounded | §4 §3④ loop | ⬜ | 🟥 FAIL [G(escalate)/code-backlog]：headline differentiator [PLANNED]。現況長跑只 stream `agent_event` 給在看的 surface，無 async consent gate；PushNotification/RemoteTrigger plumbing 不在 app。修法=接 escalation transport（surface-win-app.md §4 core loop）|

---

## §5. error / empty / offline / cold-start + Windows-specific 失效（10 條）

> 流程權威: surface-win-app.md §5。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-CUJ05-ERR-001 | e2e | ✅ | backend 未接該 step | onboarding 錯誤 | 誠實 inline：`backendReady=false`→「Backend for this step is still in progress」；reject→「Could not complete this step: <err>」；soft→「State advanced locally」（onboarding-hello.tsx:550-587）| §5 onboarding errors | ⬜ | ✅ |
| WINAPP-CUJ05-ERR-002 | e2e | ✅ | runtime down | panels | 降級「(離線模式)」+ retry | §5 offline | ⬜ | 🟡（但見 NOFAKE：SecurityPanel 離線退化成 mock）|
| WINAPP-CUJ05-ERR-003 | e2e | ✅ | 空記憶 day-0 | /recall | 「尚無事件 — 用專注/習慣/飲食頁記錄後會出現在這裡」（RecallSearch.tsx:89）| §5 empty memory | ⬜ | 🟡（誠實空態，但無 retro-ingest，見 CONV-002）|
| WINAPP-CUJ05-ERR-004 | e2e | ✅ | 無 provider | set_provider 空態 | 「No signed-in providers or local Ollama detected yet…」（onboarding-hello.tsx:494-498）| §5 provider-empty | ⬜ | 🟡 |
| WINAPP-CUJ05-ERR-005 | integ | ✅ | 非 allowlist deep-link | 傳惡意 URL | drop + log（no raw URL，defense-in-depth，no user error）| §5 / §13 | ⬜ | 🟡 |
| WINAPP-CUJ05-WIN-001 | e2e | ⚠ Windows | 冷啟、bind `0.0.0.0:7878`（lib.rs:659-661）| 觀察 Defender Firewall 對話框 | bind 前先 in-app 一行卡片解釋「Windows 會問是否允許…這讓手機連到後端，請允許 Private network」；預設 bind 127.0.0.1，opt-in 才 0.0.0.0 | §5 / findings high#3 | ⬜ | 🟥 FAIL [G(fw)/code-backlog]（**high**）：現況 bind 0.0.0.0:7878 觸發 Defender Firewall「Allow Phantom Mesh?」對話框 onboarding 中途彈出、無 in-app 解釋。User 慣性 Cancel → 靜默斷掉 mesh-peer/phone-supervises 路徑（整個 MVP 依賴）。修法=bind 前說明卡 + 預設 loopback + 可選 netsh advfirewall 規則（surface-win-app.md findings high#3）|
| WINAPP-CUJ05-WIN-002 | e2e | ⚠ Windows | PATH 上有 stale `phantom.exe` shadow | `selftest --json` / `focus` | 非 0 bytes、focus 非 stub（GUI bundle 的 daemon binary 不被舊 exe 遮蔽）| §5 / SPEC-46 §3 I-violation | ⬜ | 🟥 FAIL [SPEC-46/code-backlog]：現況 stale phantom.exe shadowing on PATH → `selftest --json` 0 bytes + `focus` stub（SPEC-46 §3 EXISTS-bug）|
| WINAPP-CUJ05-WIN-003 | manual | ❌ | Defender 持 handle | build/link | 非 `os error 5`（用 CARGO_TARGET_DIR 移出 worktree mitigate）| §5 / windows-msi-build.md §7 | ⬜ | 🟡 known mitigation（MEMORY: Windows Defender locks cargo target）|
| WINAPP-CUJ05-WIN-004 | manual | ❌ | release-signed installer | 跑 .msi | 非 SmartScreen 警告（EV cert 簽，非 dev self-signed）| §5 / windows-msi-build.md §7 | ⬜ | 🟥 FAIL [code-backlog]：現況 dev self-signed → SmartScreen 警告（EV cert = release-prep）|
| WINAPP-CUJ05-WIN-005 | static | ✅ | - | 讀 `service uninstall` 實作 | **非**無條件 `taskkill /F /IM phantom.exe`（I9：不得殺正在跑的 unattended run）| §3④ Windows delta / SPEC-46 g2/§3 / I9 | ⬜ | 🟥 FAIL [SPEC-46-I9/code-backlog]（**high**）：現況 `service uninstall` 無條件 `taskkill /F /IM phantom.exe`（EXISTS-bug），危及 live unattended run，違反 SPEC-46 g2/§3 I9。修法=uninstall 前檢查/優雅停 live run，不無腦 /F kill |

---

## §6. SPEC-31 NO-FAKING 假稽核守護（NOFAKE，4 條）

> 流程權威: surface-win-app.md §3④。`SecurityPanel` render audit log + risk level + block/approve outcome，但 `SecurityPanel.tsx:36-145,217-225` fallback 到 `MOCK_EVENTS`、`get_audit_log` best-effort（offline→mock）= EXISTS-as-mock，違反 SPEC-31 NO-FAKING（refs G4）。與 mac-app NOFAKE-* 對等。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-NOFAKE-001 | static | ✅ | - | `grep -rn "MOCK_EVENTS" app/src/screens/macos/SecurityPanel.tsx` | **0 命中**（無硬編假稽核）| §3④ / SPEC-31 / G4 | ⬜ | 🟥 FAIL [G4/code-backlog]：現況 `SecurityPanel.tsx:36-145,217-225` MOCK_EVENTS + offline 退化 mock，render 可達假 audit/flight-recorder viewer。違反 SPEC-31 NO-FAKING + Charter §E.3 T5。修法=移除 MOCK 或明標 `[demo 模式]`（refs §五 quick-win 1hr）|
| WINAPP-NOFAKE-002 | e2e | ⚠ daemon off | /settings/security、daemon 不可達 | 看畫面 | 誠實「(離線模式)」空態 banner，**不** render 假 AUD-* 列當真資料 | §3④ / SPEC-31 | ⬜ | 🟥 FAIL [G4/code-backlog]：現況 isOffline → mock 假畫面 |
| WINAPP-NOFAKE-003 | static | ✅ | - | `grep -rEn "AUD-0[0-9]+" app/src/` | 0 命中（無假審計 ID 種子）| SPEC-31 | ⬜ | 🟥 FAIL [G4/code-backlog]：MOCK_EVENTS 含 AUD-* 假 ID |
| WINAPP-NOFAKE-004 | e2e | ⚠ real daemon | /settings/security、真 audit | 看畫面 | 顯示真 `get_audit_log`；「審核中/approve」為真 live consent gate 非 display-only | §3④ / G3 | ⬜ | 🟥 FAIL [code-backlog]：現況 outcome display-only（與 G3 同根）|

---

## §7. Windows 平台專屬（PLAT，6 條）

> 流程權威: surface-win-app.md §1（DPAPI/Credential Manager）+ §5 Windows failure modes。

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| WINAPP-PLAT-DPAPI-001 | integ | ✅ | TempDir | created_identity 後 | seed DPAPI per-user wrap + 存 Windows Credential Manager（`windows-credman`），非明文檔 | §1 / identity_wire.rs / SPEC-12 P4 | ⬜ | 🟡（SPEC-12 標 v0.7.0 partial）|
| WINAPP-PLAT-DPAPI-002 | static | ✅ | - | 讀 lib.rs:496-497 comment | DPAPI key wrap non-portable by design；cross-device key reconciliation 標 flagged follow-up（誠實）| §3② Windows portability | ⬜ | 🟡 honest flagged |
| WINAPP-PLAT-REG-001 | integ | ✅ | install | 查 `HKCU\Software\Classes\phantom` | deep-link scheme handler 已註冊（Tauri-managed at install）| §1 Windows delta | ⬜ | ⬜ |
| WINAPP-PLAT-INPROC-001 | integ | ✅ | desktop run() | `run()` in lib.rs:597-671 | 起 in-proc `PhantomMeshRuntime`（RuntimeState::init ~15-18s）+ axum :7878；前端走 `tauri::invoke` 非 HTTP | §0 architecture | ⬜ | 🟡 |
| WINAPP-PLAT-SIDECAR-001 | integ | ✅ | tray「重新啟動精靈」 | `daemon.rs` spawn `phantom-mesh-x86_64-pc-windows-msvc.exe` + watchdog（5 restart/120s）| sidecar 起 + watchdog 守 | §0 / daemon.rs | ⬜ | 🟡 |
| WINAPP-PLAT-REV-001 | e2e | ✅ | onboarded | Settings → Security 找「Delete all my data」 | 有 GUI 資料刪除 affordance（含 confirm）| §reversibility / INV-12 | ⬜ | 🟥 FAIL [code-backlog]：現況無 GUI delete-all（`phantom data delete --all` CLI-only），違反 INV-12 可逆性。修法=Settings 加 delete-all（同 mac-app PLAT-REV-001）|

---

## §8. 統計

```
Total: 72 條（WINAPP-*）

By §:
  §1 onboarding:        16
  §2 daily loop:        14
  §3 五能力:            16
  §4 supervision:        6
  §5 error/empty/win:   10
  §6 NO-FAKING:          4
  §7 平台專屬:           6

By 狀態（2026-06-12 建檔，依審查者欄 emoji 機械重數）:
  ✅ 過 / 已驗:          4
  🟡 partial:           35
  🟥 現況 FAIL (code-backlog): 29
  ⬜ 未做:               4
```

### §8.1 「現況 FAIL / code-backlog」清單（29 條 → 對映 refs G-gap / SPEC-46 I）

| FAIL case | 根因 code 缺陷 | gap/I 編號 | 修法落點 |
|---|---|---|---|
| WINAPP-CUJ02-SHC-001/002/003 | `lib.rs:760` Modifiers::SUPER（Win 鍵）被 Windows shell 攔 + 標籤錯 | G5 | OS 分支 Modifiers（Win=Ctrl+Alt）+ 動態標籤 |
| WINAPP-CUJ01-SCK-002/003/005 | StartupCheck 單射探針 race in-proc runtime 15-18s init | G6 | 有界輪詢 / `runtime://ready` 事件 |
| WINAPP-CUJ01-FORK-001/004 | 無 backend-brain 心智模型句 + 無 phone 配對引導 | G8 | 文案 + QR/pairing |
| WINAPP-CUJ01-FORK-002 | demo-relay handoff no-op stub | — | 真 handoff 或誠實 coming-soon |
| WINAPP-CUJ02-CONV-002 | 冷啟空記憶無 retro-ingest 框架 | G9 | 空態教複利 loop |
| WINAPP-CUJ02-CONV-003 | 英 onboarding → 中文 shell | — | i18n / 單語 |
| WINAPP-CUJ02-CONV-004 | 主 shell 無 command palette | — | 主 shell 掛 palette |
| WINAPP-CUJ02-NAV-002/003 | onboarding 無 Back + 無 Re-scan/inline key | — | rollback edge + re-scan |
| WINAPP-ABIL1-CAP-003 | asset-protocol enable:false（截圖不可 in-app 渲染）| P2-14 | tauri.conf.json:28 翻 true |
| WINAPP-ABIL2-MEM-003 | skill_wire unimplemented!() | BRK-1 | Wave 3 R1 schema |
| WINAPP-ABIL4-GOV-002 | awaiting_approval→pending 抹除 + audit display-only | G3 | 別 collapse + 三鈕 |
| WINAPP-ABIL4-GOV-003/004 | 無 governor UI / flight-recorder viewer + Win32 power brake 未接 | G(gov) | 加 Unattended/Long-run 面 |
| WINAPP-CUJ04-DL-006 | escalation transport 未接（tool-call→phone consent）| G(escalate) | 接 push/consent channel |
| WINAPP-CUJ05-WIN-001 | bind 0.0.0.0 觸發 Firewall prompt 無解釋 + 預設非 loopback | G(fw) | 說明卡 + 預設 127.0.0.1 |
| WINAPP-CUJ05-WIN-002 | stale phantom.exe shadowing → selftest 0 bytes/focus stub | SPEC-46 §3 | PATH 解析優先自 bundle binary |
| WINAPP-CUJ05-WIN-004 | dev self-signed → SmartScreen 警告 | — | EV cert（release-prep）|
| WINAPP-CUJ05-WIN-005 | `service uninstall` 無條件 taskkill /F /IM 殺 live run | SPEC-46 I9 | uninstall 前檢查/優雅停 |
| WINAPP-NOFAKE-001/002/003/004 | SecurityPanel MOCK_EVENTS 假稽核 | G4 | 移除 MOCK 或標 demo |
| WINAPP-PLAT-REV-001 | 無 GUI delete-all（可逆性 CLI-only）| INV-12 | Settings 加 action |

> **紀律重申**：上表每條「修法落點」改的都是 **code**（`app/src-tauri/src/lib.rs`、`tauri.conf.json`、`app/src/*`、`core/src/*`），**不是本檔**。本檔僅凍結驗收條件，使 code 修好後可用同一條 case 機械驗證回綠。

---

## §9. CLI runner / 自動化 hook 範例（PowerShell + grep）

```powershell
# scripts/test-runner-win-app.ps1 — 跑 Auto=✅ 的 static 守護（現況多預期 FAIL，修碼後回綠）

# §6 NO-FAKING（應 0 命中）
if (Select-String -Path app/src/screens/macos/SecurityPanel.tsx -Pattern "MOCK_EVENTS" -Quiet) { "FAIL NOFAKE-001" }
if (Get-ChildItem app/src -Recurse | Select-String -Pattern "AUD-0\d+" -Quiet)            { "FAIL NOFAKE-003" }

# §2A G5 Windows 快捷鍵：lib.rs:760 不該用 Modifiers::SUPER（Windows）
if (Select-String -Path app/src-tauri/src/lib.rs -Pattern "Modifiers::SUPER" -Quiet)       { "FAIL SHC-001 (Windows 用 SUPER 被攔)" }

# §3A P2-14 asset-protocol：tauri.conf.json:28 應 enable:true
if (Select-String -Path app/src-tauri/tauri.conf.json -Pattern '"enable"\s*:\s*false' -Quiet) { "FAIL CAP-003 (asset enable:false)" }

# §5 SPEC-46 I9：service uninstall 不該無條件 taskkill /F
if (Get-ChildItem app/src-tauri/src -Recurse | Select-String -Pattern "taskkill /F /IM phantom" -Quiet) { "FAIL WIN-005 (I9 violation)" }

# §3③ INV-8 proactive 守恆（應 0 命中 = PASS）
if (-not (Get-ChildItem app/src -Recurse | Select-String -Pattern "GPS|real.?time.*proactive" -Quiet)) { "PASS REV-002" }

Write-Output "（現況：FAIL 守護多為紅，待 code-backlog 修碼回綠）"
```

> WebDriver / tauri-driver e2e（onboarding FSM、shortcut 路由、Firewall prompt、dashboard 狀態列）為 Phase 2，對齊 mac.md §14 自動化 roadmap。Windows e2e 需 `tauri-driver` + Edge WebDriver（msedgedriver）。
