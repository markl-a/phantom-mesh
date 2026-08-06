# SPEC-80-INFRA · Mode B 3-機協作開發管線（3-machine collaborative dev pipeline）

> **INFRA（infrastructure，基礎設施）spec — 不是 v0.6.0 user-facing contract（使用者面合約）。** 這是 INFRA 系列第一份；它規範 spectyn-mesh 自身**開發流程**所依賴的基礎設施（dev-tooling = 開發工具鏈），不是產品功能。讀者應把它當作「**spectyn-mesh 怎麼被開發出來的契約**」，不是「**spectyn 對使用者承諾什麼**」。
>
> **Status note**：本 spec v0.1 行為已於 2026-05-26 上線（operator 用、3 機跑通）；v0.2 把今天踩到的 5 個 race condition（競爭條件）/ 同步 bug 規則化成 §11，目標 v0.7.0 cycle 完成 codify。

---

## §0 Spec metadata

| Field | Value |
|---|---|
| Spec ID | `SPEC-80-INFRA-mode-b-collab-dev` |
| Title | `Mode B 3-機協作開發管線`（English subtitle: `Mode B — 3-machine interactive collaborative dev pipeline`） |
| Status | `DRAFT (operational since 2026-05-26)` |
| Version | `0.2.0`（v0.1 shipped 2026-05-26 morning；v0.2 = 今天 5 條 lessons codified） |
| Last updated | `2026-05-26` |
| Author | `Mark + Claude Opus 4.7` |
| Reviewer(s) | (待填) |
| Implementation owner | Mark（自己 dogfood，無外部 owner） |
| Target release | `v0.7.0`（codify 完整 v0.2 規則；v0.1 行為已 operational） |
| Pillar(s) served | `cross-pillar（X.infra — dev-tooling）`；supports all 4 pillars indirectly（讓 P1/P2/P3/P4 都被開發出來的腳手架 = scaffolding） |
| Track | `infra`（純內部開發流程，不對應 Life / Work track） |
| Epic | `v0.7.0+ INFRA01 mode-b-collab-dev` |
| BIG-GOAL phrase served | 「Cluster = first-class. Handsets, laptops, tablets are mesh peers — not server/clients.」（[BIG-GOAL.md](../../BIG-GOAL.md) §Four pillars · P1 line 25）— 本檔讓 spectyn-mesh 自己的開發過程就是 cluster-as-first-class（叢集第一公民）的展示：3 台 dev 機器 = 3 個 peer，operator 是叢集擁有者，git + `.ai-shared/` 是替代 mesh transport（中介傳輸層）的 fallback substrate（替補底層） |
| Depends on | `none`（process spec，不引 Cargo dep；只 cite 既有 file infra） |
| Blocks | `v0.8.0+ INFRA02 cluster-dispatch-eat-own-dogfood`（將來 SPEC-26 `cluster_dispatch_wire` 完整 Stage 4 後，本檔的 git-based queue 會被 spectyn 自家 mesh dispatch 取代 — 那份 spec 接手） |
| Template deviation | 這是 **process spec**（流程規格）非 **product spec**（產品規格）；§7 data model = queue file / heartbeat / claim lock 真實 schema（不是 OoS）、§8 invariants 真實、§9 = bash CLI interface（既有檔案 grep 得到），§12-§14 簡化或標 N/A；§11 新增 `v2 lessons learned`（今天踩坑 codify） |

---

## §1 TL;DR

### 1.1 繁中三段

**問題**：v0.6.0 cycle 進入 Phase H frontend wire-up（前端線接 = 把 Rust core 暴露的 wire 模組接到 React UI）後，operator（操作者 = sole maintainer Mark）一個人在一台 mac（MacBook Air，行動筆電）開 claude 一個 task 一個 task 跑太慢，趕不上 6-15 deadline（2026-06-15 v0.6.0 GA = general availability 正式發佈）。同時手邊另兩台機器（node-b = Win+Android dev、node-a = always-on Linux+Win/WSL2，always-on = 24 小時開機）閒置。需要一套**讓 3 台機器同時當開發者**的協作管線（pipeline）。

**方案**：Mode B = interactive（互動式）3-機協作開發管線。每台機器跑一個 `claude` CLI 當 **gateway（閘道協調者）**；operator 用 `/dev-start` slash command（斜線指令 = Claude Code 預先註冊的 ritual = 啟動儀式）觸發；ritual 自動讀 4 個 git-tracked（git 追蹤的）queue file（佇列檔）：`mac.todo.md` / `node-b.todo.md` / `node-a.todo.md` / `SHARED.todo.md`，挑出該機可做的 task，operator 確認後 claude orchestrate 4-stage spec→code dispatch 派給 5 個 subscription tool（訂閱式工具）— opencode / codex / agy / gemini / claude subagent — 透過 fallback chain（後備鏈）逐一試。Task 完成後 commit + push 到 `wip/<host>/<task-uuid>` branch（進行中分支），由 operator 在 mac 上 review + merge to main。

**代價**：明確不做 full automation（完全自動化 = Mode A cron 模式）— 留 v0.8.0；明確不做 spectyn 自家 cluster_dispatch eat-own-dogfood（吃自己狗食 = 自己用自家產品做派工）— 留到 SPEC-26 Stage 4 完成；明確要求 operator 每天 ≤ 30 min review 介入；明確接受 race condition 風險（首版 v0.1 已踩到 H1.3 重複工作 30% 浪費，v0.2 §11 codify 5 條規則防範）；明確不提供跨機 live collaboration（即時協作 = 同檔同時改）。

### 1.2 English abstract

Mode B is the interactive 3-machine collaborative development pipeline for spectyn-mesh, operational since 2026-05-26 evening. Three dev machines (mac / node-b / node-a) each run Claude Code CLI as a gateway orchestrator; operator triggers the `/dev-start` slash command on whichever machine they want to use; the ritual reads four git-tracked queue files (per-host `mac.todo.md` / `node-b.todo.md` / `node-a.todo.md` plus a shared `SHARED.todo.md`), proposes the next task fit for that machine's platform capability, and on operator confirmation dispatches to a fallback chain of five subscription-based AI worker tools (opencode → codex → agy → gemini → claude subagent) via `scripts/ai/dispatch-with-fallback.sh`. Each task is executed on a `wip/<host>/<task-uuid>` branch (never main) and merged back from the mac authority. The architecture treats git as the substrate, the four queue files as the source of truth, and atomic git push as the claim primitive. v0.1 (morning of 2026-05-26) shipped without strict partitioning rules; v0.2 (this spec) codifies five rules learned from a same-day race condition where wave H1.3 (capture_focus_wire) ended up duplicated across mac and node-a queues with roughly 30% wasted re-work. Mode A (cron-unattended overnight) and SPEC-26 mesh-native dispatch are explicitly deferred.

### 1.3 Glossary

> 本表覆蓋本檔用到的核心縮寫 + 英文名詞 + 中文意譯，每條一句話定義。同檔第二次出現後允許只用英文。

> - **gateway（閘道協調者）** — 每台 dev 機器上跑的 claude CLI 本身；它不直接寫 code，而是 dispatch 給其他 AI tool 並彙整結果
> - **fanout（散開派工）** — 一個 task 由 gateway 拆成多個 sub-prompt（子提示）並丟給多個 worker tool 平行跑（本 spec v0.1 序列 fanout；v2 仍序列）
> - **claim atomicity（領取原子性）** — 「同一個 task 不可能被 2 個 host 同時占用」這個保證；本 spec 透過 git push 競賽（push race）+ rejection（拒絕）達成
> - **race condition（競爭條件）** — 兩個 host 在沒看到對方時都領了同一個 task（v0.1 真實踩到 H1.3）
> - **subscription pool（訂閱配額池）** — 每個 AI tool 訂閱方案的 rolling window quota（滾動時間窗額度）；3 機 × 5 工具 = 15 個獨立 pool
> - **heartbeat（心跳）** — per-host 寫到 `.ai-shared/heartbeat/<host>-last.txt` 的 timestamp + status；v0.2 規則用它判定其他 host 是否還活著
> - **queue file（佇列檔）** — `.ai-shared/queue/<host>.todo.md` 或 `SHARED.todo.md` markdown checklist；本 spec 唯一的 task 來源
> - **wip branch（進行中分支）** — `wip/<host>/<task-uuid>` 每 task 獨立 git branch；merge 前過 operator review
> - **fallback chain（後備鏈）** — opencode → codex → agy → gemini → claude subagent 順序；前一個 quota 用完或失敗就試下一個
> - **slash command（斜線指令）** — Claude Code 由 `.claude/commands/<name>.md` 註冊的命名 ritual；本檔指 `/dev-start`
> - **dogfood（自食其力）** — 自己用自家產品；本 spec v0.1 是 mesh 開發者的 dogfood，v2 升級為 spectyn 自己 dispatch 的 dogfood
> - **substrate（底層基板）** — 上層流程靠它運作的底層；本 spec v0.1 substrate = git + GitHub origin，v2 substrate 才是 spectyn mesh 自己
> - **OS-bound vs agnostic（OS 綁定 vs 跨平台）** — task 需特定 OS toolchain（如 iOS build → mac only）= OS-bound；任一 OS 可跑 = agnostic
> - **ritual（啟動儀式）** — `.claude/commands/dev-start.md` 定義的固定 8 步流程，每次 operator 開新 session 跑一次
> - **ship gate（上線門檻）** — `.ai-shared/progress.md` §1 的 V1-V12 12 條檢核項，全綠才能 cut GA tag

---

## §2 Context & Background

### 2.1 為什麼現在做 Mode B

v0.6.0 cycle 進入 Phase H frontend wire-up 之後，剩餘工作分成 4 類：
1. **Apple-bound**（iOS sim build / macOS notarization / Xcode codesign） — 只有 mac 能做
2. **Android-bound**（Android NDK build / APK sign / AVD smoke） — 只有 node-b（裝 Android SDK 的筆電）能做
3. **Linux + Win-bound**（cargo build for `x86_64-unknown-linux-gnu` / Windows MSI codesign / WSL2 整合測試） — node-a 最適合（always-on + Win/WSL2 雙 boot）
4. **Agnostic**（Rust core code / spec writing / docs / TS 前端 component）— 任一機可跑

如果只在 mac 上序列跑，wall-clock（牆鐘時間）= sum of (1-4)；如果 3 機平行 + 任務分到對應平台，wall-clock = max of (1, 2, 3) + agnostic 部分；理想加速比 ≈ 3×。但 2026-05-26 morning 第一次 dogfood 實測 net throughput（淨產出）= 1.4× single-machine baseline，未達 3× — 主因是 race condition（§11 codify）。

### 2.2 v0.1 dogfood 量化數據（2026-05-26 morning → evening）

- **3 機並行時段**：09:00-15:00（mac 早上 09:00 開機；node-a 透過 SSH 從 mac 啟動 around 10:30；node-b 因 SDK 安裝拖延晚到，沒參與本日）
- **完成 task 數**：mac = 4（H1.1 / H1.2 / H1.3 / V4-e2e-macos）、node-a = 2（H1.3 重複工作 + H1.3 衍生 unit test cherry-pick）、node-b = 0
- **race condition 事件**：1 件 — H1.3 capture_focus_wire 在 `mac.todo.md` 標 `task-2026052618` 同時在 `node-a.todo.md` 標 `task-2026052632`，兩機平行做完才發現重複；commit `be9619d test(h1.3): cherry-pick 5 node-a unit tests` 是事後把 node-a 寫的 unit test 搬到 mac 版本 merge 後的補救（loss = 約 1 h node-a 寫的 wire 主體被丟、保留 5 個單元測試）
- **node-a sync issue**：node-a 用 bundle catch-up（用 git bundle 包 commit 從 mac 帶過去）後 local main 沒 fast-forward 到 origin/main，導致 node-a 第一個 commit 看不到 mac 已完成的 H1.1 / H1.2 → 觸發後續判斷錯誤
- **operator 介入時間**：實測 ≈ 45 min（review 6 個 wip branch + merge + 補救 H1.3 race）— 超過 30 min target

結論：v0.1 證明「可跑」，v0.2 必須把 race avoidance（避免重複領） + sync discipline（同步紀律）codify。

### 2.3 在 BIG-GOAL 哪裡

- **BIG-GOAL §Four pillars P1（line 25）**：「Cluster = first-class. Handsets, laptops, tablets are mesh peers — not server/clients.」本 spec 把這句話套到開發過程本身 — 3 台 dev 機器（一台 laptop = mac + 一台 laptop = node-b + 一台 mini-PC = node-a）就是一個 dev-cluster（開發叢集），git + `.ai-shared/` 是 fallback substrate 直到 mesh 自己（SPEC-26 cluster_dispatch_wire）成熟到能取代。
- **BIG-GOAL §3 line 49**：「Spectyn is a peer-to-peer mesh, not server/client.」— 本 spec 的 v2 演化路徑就是把 mac authority 模式（mac 當 merge gatekeeper）逐步淡化到任何機都能 merge（peer-to-peer dev），雖然 v0.2 仍保留 mac 為 merge authority（簡化模型）。

### 2.4 在此之前嘗試過什麼

- **v-0.5 baseline**（2026-05-19 之前）：operator 一台 mac 一個 claude session 序列開發；wall-clock 受限於 mac 在通勤 / 辦公室時段 offline 的時數
- **2026-05-22 ssh fanout experiment**：mac 端 `scripts/ai/remote-dispatch.sh` 透過 SSH 把 prompt 丟到 node-a 的 claude headless 跑 — 證明 transport 可用，但缺少 task 分配機制（operator 要 hand-edit prompt）、缺少 result 回收機制（手動 scp 拉檔）、缺少 race avoidance（agy 在 Windows+SSH broken bug 也踩到，per memory `reference_agy_windows_ssh_broken`）
- **2026-05-26 morning Mode B v0.1**：把 SSH fanout 升級成 git-tracked queue + `/dev-start` ritual + fallback chain；shipped 6 個檔案（見 §6.2）；同日 evening 踩到 §2.2 race，evening 寫本 spec codify v0.2

### 2.5 相關 spec 與檔案

本 spec 是 INFRA 系列第一份，沒有依賴其他 spec。但 cite 既有檔：
- [`scripts/ai/AUTO-DISPATCH-DESIGN.md`](../../../scripts/ai/AUTO-DISPATCH-DESIGN.md) — 原始 design doc，本 spec 是它的 formal codification；Mode A 那部分仍留在 design doc，本檔只規範 Mode B
- [`.claude/commands/dev-start.md`](../../../.claude/commands/dev-start.md) — `/dev-start` slash command 實作；本 spec §10 與它對應
- [`scripts/ai/verify-tools.sh`](../../../scripts/ai/verify-tools.sh) — Step 1 工具檢查腳本；§9.1 規範它的介面
- [`scripts/ai/dispatch-with-fallback.sh`](../../../scripts/ai/dispatch-with-fallback.sh) — fallback chain dispatcher；§9.2 規範它的介面
- [`.ai-shared/tool-policy.md`](../../../.ai-shared/tool-policy.md) — 工具偏好矩陣；本 spec §8 引用其 fallback chain 規則
- [`.ai-shared/queue/*.todo.md`](../../../.ai-shared/queue/) — 4 個 queue file；§7.1 規範 schema
- [`.ai-shared/progress.md`](../../../.ai-shared/progress.md) — ship gate 狀態；§7.4 規範 schema
- [`SPEC-26-SYSTEM-cluster-dispatch.md`](SPEC-26-SYSTEM-cluster-dispatch.md) — 將來取代本 spec git substrate 的 mesh-native dispatcher；blocks 關係

---

## §3 Goals / Non-Goals / Out-of-Scope

### 3.1 Goals

- `[G1]` **Net throughput ≥ 2× single-machine baseline** — 3 機並行協作的 wall-clock 完成總量必須 ≥ 一台 mac 序列跑的 2 倍（理想 3× 不強求，但 1.4× 是 v0.1 失敗 baseline）。`(verifies via: T-mode-b-throughput-measure)`
- `[G2]` **0 duplicate work** — 任何一個 task UUID（unique identifier，唯一識別碼）只能被一台 host 領取一次；H1.3 race 不可再發生。`(verifies via: T-mode-b-no-duplicate-claim)`
- `[G3]` **0 push-to-main race** — `origin/main` branch 只接受 operator 在 mac 上手動 merge 的 commit；任何 claude session 都不直接 push main。`(verifies via: T-mode-b-main-protected)`
- `[G4]` **Operator daily intervention ≤ 30 min** — 包含 morning queue check + 中午 wip branch review + bedtime sync check。`(verifies via: T-mode-b-operator-time-log)`
- `[G5]` **3 機同時 active 可用** — 任一時刻 3 台機都 idle-and-ready 接 task（即使現在沒在跑，3 機都應 ≤ 5 min 可從 idle 進入 running）。`(verifies via: T-mode-b-3-host-warm-up)`
- `[G6]` **Sync correctness invariant** — 任何 claude session commit 前必跑 `git fetch + git merge-base --is-ancestor HEAD origin/main`（或等效 check）確認 local main 是 origin/main 的祖先或同步；不通過則 rebase 再 commit。`(verifies via: T-mode-b-sync-invariant)`

### 3.2 Non-Goals

- `[NG1]` **不做 full automation（cron / unattended）** — 留給 Mode A（SPEC-80 v0.3 或獨立 INFRA02），本 spec 強制 operator-attended（必須有人在）。
- `[NG2]` **不做 spectyn 自家 cluster_dispatch eat-own-dogfood** — 留給 INFRA02（基於 SPEC-26 Stage 4 完成），本 spec 用 git substrate。
- `[NG3]` **不做 live collaborative editing**（即時協作編輯 = 兩機同時改同一行）— file-area ownership 規則（§8.2）保證不會發生；衝突就是 bug。
- `[NG4]` **不做 cross-machine debugger / repl** — 每機自己跑 cargo test / repl，stdout 各自；不嘗試 live mirror。
- `[NG5]` **不做 GPU 分擔** — 每機跑自己的 task；ML 推論（如 ort 模型）只在有對應硬體的 host 跑。
- `[NG6]` **不取代 mac 為 merge authority** — v0.2 仍規定 origin/main 的 merge 從 mac 上由 operator 手動執行（即使 node-a always-on 也不接 merge 權）。簡化模型，避開「兩機都試圖 merge 又互踩」的二級 race。

### 3.3 Out-of-Scope for this version

- `[OoS1]` **Per-host token / cost reporting dashboard** — 訂閱模型下 cost 是 fixed monthly，不值得做 dashboard；如果未來改 pay-per-token 再寫。
- `[OoS2]` **iPad / 手機 operator interface** — operator 目前都從 mac 操作；iPad read-only view 未來可考慮（如 SPEC-70-EXP web dashboard）。
- `[OoS3]` **跨機檔案 diff merge tool** — 衝突由 §8.2 file-area ownership 規則預防；萬一發生靠 `git merge` 手動處理，不寫客製 tool。
- `[OoS4]` **AI 自決 architecture / pillar mapping** — 永遠 operator 決定；本 spec 不開 AI 自決後門。

---

## §4 Job Stories

> Intercom 句型：**When** [情境], **I want to** [動機], **so I can** [結果]。每條映射到至少一個 §3.1 Goal。

- `[J1]` **When** 我（operator）早上開 mac 想開始今天的開發，**I want to** 一個 `/dev-start` 指令就把 verify tools + git pull + 4 queue 讀完 + 提議下個 task 全部跑完，**so I can** 30 秒內知道「現在該做什麼」而不是手動爬 6 個檔案。 (→ G4)
- `[J2]` **When** 我中午想開 node-a 接手 node-b 做不完的 Android 相關工作，**I want to** node-a 上跑 `/dev-start` 就自動知道 node-b 的 queue 是它不能碰的（platform-bound 不可跨機），**so I can** 不會把 node-b 的 task 拉到 node-a 結果中途 toolchain 缺料 fail。 (→ G2, G5)
- `[J3]` **When** mac 跟 node-a 都同時看到 SHARED queue 有個 agnostic task，**I want to** 兩機透過 atomic git push 競賽決定誰拿到（loser 自動跳下一個），**so I can** 不會兩機都做完才發現重複。 (→ G2)
- `[J4]` **When** mac 在 commute（通勤）路上 sleep / offline，**I want to** node-a + node-b 還在跑各自 queue 的 OS-bound task，**so I can** 不會因為 mac 暫離整個 pipeline 停擺。 (→ G5)
- `[J5]` **When** 我晚上想看今天進度，**I want to** 只開一個 `.ai-shared/progress.md` 就看到 V1-V12 ship gate + Phase H Tier 0 + done.md 的 task 列表，**so I can** 不需要 ssh 進 node-a / node-b 抓 log。 (→ G4)
- `[J6]` **When** opencode quota 用完，**I want to** dispatch-with-fallback.sh 自動跳到 codex 再不行跳 agy 再不行跳 claude subagent，**so I can** 任務不會因為單一 tool 滿了就 hard fail。 (→ G1)

---

## §5 Personas

> Mode B 是內部開發工具，只有 1 個真實 persona — operator 自己。但為避免 spec 退化成「Mark 用的」，把 persona 拆 3 種視角，方便未來開源後有第二開發者加入 mesh 仍適用。

### 5.1 Sole-maintainer operator（單人維護者 = 現在的 Mark）

擁有 3-5 台 dev 機器、自己一人開發 spectyn-mesh 趕 GA deadline。期待：自己每天 ≤ 30 min 介入時間，其餘時間 3 機自動推進。

### 5.2 Future contributor（未來協作者 = 開源後加入的人）

spectyn-mesh 開源後第一個 PR contributor。期待：能讀懂 `/dev-start` ritual 就知道自己機器要怎麼接到既有 queue（也許就只 fork + 修自己 queue）。本 spec 對他們的承諾：規則寫死 + 不靠 oral tradition（口傳）。

### 5.3 CI bot persona（自動化代理人 — Mode A 之後）

未來 Mode A cron-unattended 模式 active 時，這個 persona 是 cron daemon（背景常駐程式）自己。本 spec v0.2 不直接服務這個 persona，但 §8 invariants 寫法刻意維持「兩種 actor 都能套用」— bot 跟 human operator 都遵守同一套 atomic claim 規則。

---

## §6 System Architecture

### 6.1 3-tier 架構圖

```mermaid
flowchart TB
    Operator(["操作者 Mark（人）"])
    subgraph Tier1["Tier 1 — 3 機 gateway（Claude Code CLI 作為閘道協調者）"]
        ClaudeMac["claude on mac<br/>（MacBook Air）"]
        ClaudeAcer["claude on node-b<br/>（laptop + Android SDK）"]
        ClaudeZ13["claude on node-a<br/>（always-on Linux + Win/WSL2）"]
    end
    subgraph Tier2["Tier 2 — 訂閱式 AI 工具池（fallback chain）"]
        Opencode["opencode<br/>（免費模型 unlimited）"]
        Codex["codex<br/>（ChatGPT Plus 配額）"]
        Agy["agy<br/>（Google Antigravity）"]
        Gemini["gemini<br/>（Gemini 帳號）"]
        ClaudeSub["claude subagent<br/>（Anthropic Pro 配額）"]
    end
    subgraph Tier3["Tier 3 — 共享狀態（git substrate）"]
        QueueFiles["`.ai-shared/queue/`<br/>4 個 queue file"]
        ClaimedDir["`.ai-shared/claimed/`<br/>原子鎖檔"]
        Progress["`.ai-shared/progress.md`<br/>ship gate 狀態"]
        DoneDir["`.ai-shared/done/`<br/>完成記錄"]
        WipBranches["`wip/<host>/<task-uuid>`<br/>git branches"]
        Origin["origin/main on GitHub"]
    end

    Operator -->|"/dev-start"| ClaudeMac
    Operator -->|"/dev-start"| ClaudeAcer
    Operator -->|"/dev-start"| ClaudeZ13

    ClaudeMac -->|"dispatch-with-fallback.sh"| Opencode
    Opencode -.->|"quota exhausted"| Codex
    Codex -.->|"quota exhausted"| Agy
    Agy -.->|"quota exhausted"| Gemini
    Gemini -.->|"quota exhausted"| ClaudeSub

    ClaudeAcer -->|"dispatch-with-fallback.sh"| Opencode
    ClaudeZ13 -->|"dispatch-with-fallback.sh"| Opencode

    ClaudeMac <-->|"git pull / push"| QueueFiles
    ClaudeAcer <-->|"git pull / push"| QueueFiles
    ClaudeZ13 <-->|"git pull / push"| QueueFiles

    QueueFiles --> ClaimedDir
    QueueFiles --> Progress
    QueueFiles --> DoneDir
    ClaudeMac -->|"push wip/mac/*"| WipBranches
    ClaudeAcer -->|"push wip/node-b/*"| WipBranches
    ClaudeZ13 -->|"push wip/node-a/*"| WipBranches
    WipBranches --> Origin
    Operator -->|"review + merge to main<br/>（only from mac）"| Origin
```

### 6.2 Component breakdown

| 元件 | 程式碼位置 | 職責一句話 | 對外介面（§9） |
|---|---|---|---|
| `/dev-start` slash command | `.claude/commands/dev-start.md` | 定義 8 步 ritual（驗工具 → pull → 讀 4 queue → 提議 task → operator 確認 → atomic claim → dispatch → 結算） | claude CLI 自動 register（Claude Code 啟動時掃 `.claude/commands/`） |
| `verify-tools.sh` | `scripts/ai/verify-tools.sh` | 跑 5 工具的 installed-yes/no + login-state check；OS-aware（macOS / Linux / Win）；輸出 text 或 `--json` | bash CLI，optional `--strict` flag |
| `dispatch-with-fallback.sh` | `scripts/ai/dispatch-with-fallback.sh` | 接 prompt-file，依序試 5 工具，quota / fail 自動跳下一個，log 到 `.ai-shared/quota-alerts.md`，全炸時 Telegram 通知 | bash CLI |
| `dispatch.sh` | `scripts/ai/dispatch.sh` | 單 tool dispatch primitive（fallback chain 內部用） | bash CLI |
| Queue files | `.ai-shared/queue/{mac,node-b,node-a,SHARED}.todo.md` | 4 個 markdown checklist 持有 task 清單 | grep-able text |
| Claim locks | `.ai-shared/claimed/<task-uuid>.<host>.lock` | 每 task 領取時建立的鎖檔；同時 commit + push 才算 claim 成功 | git-tracked file |
| Progress | `.ai-shared/progress.md` | ship gate 狀態（V1-V12）+ Phase H Tier 0 + 完成記錄 | grep-able markdown |
| Done records | `.ai-shared/done/<task-uuid>.md` | 每個完成 task 的 summary + diff stats + 完成時間 | grep-able markdown |
| Wip branches | `wip/<host>/<task-uuid>` on origin | 每 task 獨立 git branch；merge 前過 operator review | git ref |

### 6.3 File-area ownership matrix（§2 race 教訓 → §11 codify 之核心）

| 檔案區 | 唯一可寫 host | Rationale | 違反後果 |
|---|---|---|---|
| `core/src/ios/`、`app/src-tauri/ios/`、`scripts/package-ios.sh` | **mac only** | Xcode + iOS SDK 只在 mac 裝；其他機 commit 無法本機 verify build | iOS build 不可重現 |
| `core/src/android/`、`app/src-tauri/android/`、Android keystore | **node-b only** | Android NDK + JDK + AVD 只在 node-b 設好 | APK sign / build 失敗 |
| `core/src/linux/`、`packaging/linux/`、AppImage + deb 製作 | **node-a only** | Linux toolchain（musl + dpkg）只 node-a 全裝 | Linux release artifact 缺料 |
| `core/src/windows/`、Win MSI codesign、Win 平台測試 | **node-a OR node-b**（看誰先 claim） | 兩機都裝 Win；first-claim wins | 重複工作（v0.1 H1.3 同型 bug） |
| `core/src/<slug>_wire.rs`（18 wire 模組） | **rotating per-spec assignment** | 每 wire 一次只一機改；assignment 寫 task brief | merge conflict 在 lib.rs |
| `docs/superpowers/specs/` | **mac OR node-a**（spec editor — 由 task brief 指定） | spec 寫作不綁 platform；指定一機 | 雙人改同 spec 衝突 |
| `core/tests/` | **任一機**（per-test ownership — 由 task brief 指定） | 測試獨立 file 沒 lib.rs 級衝突 | 較低風險 |
| `.github/workflows/` | **mac only** | CI orchestrator authority；mac merge 權既得 | CI 行為不一致 |
| `.ai-shared/queue/<host>.todo.md` | **該 host only**（mac 不改 node-a queue） | invariant — 詳 §8.1 | 違反 = bug |
| `.ai-shared/queue/SHARED.todo.md` | **任一機（atomic claim required）** | first-push wins | 第 1 個 race-prone 區 — §8.3 重點 |
| `.ai-shared/progress.md` | **任一機 append-only** | union-of-checks 合併規則 | 合併衝突取 union |

### 6.4 Sequence — operator 從 idle 到第一個 task push

```mermaid
sequenceDiagram
    autonumber
    actor U as "Operator"
    participant CM as "claude on mac"
    participant FS as ".ai-shared/ (local)"
    participant Git as "origin/main on GitHub"
    participant DC as "dispatch-with-fallback"
    participant OC as "opencode (worker)"

    U->>CM: "/dev-start"
    CM->>CM: "Step 1: verify-tools.sh"
    CM->>Git: "git pull --rebase --autostash"
    Git-->>CM: "up to date (or N commits ahead)"
    CM->>FS: "read .ai-shared/queue/mac.todo.md"
    CM->>FS: "read .ai-shared/queue/SHARED.todo.md"
    CM->>FS: "read .ai-shared/progress.md (first 50 lines)"
    CM->>U: "Proposed: task-X 'foo'; alts: ..."
    U->>CM: "y (confirm)"

    Note over CM,Git: Step 6 — atomic claim
    CM->>Git: "git pull (再一次 — 抓 race window 內別人 push)"
    CM->>FS: "touch .ai-shared/claimed/task-X.mac.lock"
    CM->>FS: "edit queue: - [ ] → - [~]"
    CM->>Git: "git commit + git push"
    alt push 成功（claim won）
        Git-->>CM: "OK"
        CM->>Git: "git checkout -b wip/mac/task-X"
        CM->>DC: "dispatch prompt to fallback chain"
        DC->>OC: "try opencode first"
        OC-->>DC: "result"
        DC-->>CM: "output"
        CM->>CM: "Edit / Write files per output"
        CM->>Git: "git add + commit + push wip/mac/task-X"
        CM->>FS: "edit queue: - [~] → - [x]"
        CM->>FS: "write .ai-shared/done/task-X.md"
        CM->>Git: "git push origin main (queue + done only)"
        CM->>U: "task X done; wip/mac/task-X pushed; ready for review"
    else push 失敗（race lost — 另一機先 claim）
        Git-->>CM: "rejected (non-fast-forward)"
        CM->>FS: "rm .ai-shared/claimed/task-X.mac.lock"
        CM->>Git: "git pull"
        CM->>U: "race lost; alternative task = task-Y; confirm?"
    end
```

> 註：圖中 step 12（push 成功 → checkout new branch）跟 step 18-19（push 失敗 → release lock + pull）是 §11 v2 rule 1+3 的執行體現。

---

## §7 Data Model

> 本節是 process spec 的 wire-level — 所有 schema 都是 real file format（既有檔 grep 得到），不是 OoS。

### 7.1 Queue file schema（`.ai-shared/queue/<host>.todo.md` 與 `SHARED.todo.md`）

Markdown checklist 格式。每行一個 task，states 4 種，tags 必填。

**Schema BNF（Backus-Naur Form，標記式語法）**：
```
queue_line := "- [" state "] " task_uuid " [" tag_list "] " one_line_brief
state      := " " | "~" | "x" | "!"
              # " " = pending, "~" = in-progress, "x" = done, "!" = blocked
task_uuid  := "task-" YYYYMMDD NN
              # YYYYMMDD = ISO date, NN = 2-digit seq for that day per host
tag_list   := tag ("," tag)*
tag        := "apple" | "android" | "win" | "linux" | "agnostic"
            | "spec" | "docs" | "test" | "ci"
            | "priority-high" | "overnight" | "coordinator"
one_line_brief := /[^\n]{1,140}/   # 一句話 ≤ 140 字元
```

**Header schema**（每 queue file 開頭固定 6 行）：
```markdown
# Queue — <host>

> **Owner**: only <host> claude can claim. Others read but don't claim.
> **Schema**: `- [<state>] <task-uuid> [<tags>] <one-line brief>` (states: ` `=pending, `~`=in-progress, `x`=done, `!`=blocked)
> **Tags**: <comma-list of allowed tags for this host>
> **Briefs**: `.ai-shared/queue/<host>/<task-uuid>.brief.md`
```

**Sections**（固定 3 個）：
```markdown
## Active
- [ ] ...

## Backlog
- [ ] ...

## Done (audit trail — don't delete)
- [x] ...
```

**Per-task brief file**（optional，task 複雜時建）：`.ai-shared/queue/<host>/<task-uuid>.brief.md`，包含 Goal / Context / Acceptance criteria / Constraints / Branch 5 section。範例見 [AUTO-DISPATCH-DESIGN.md §6](../../../scripts/ai/AUTO-DISPATCH-DESIGN.md) line 136-161。

### 7.2 Claim lockfile schema（`.ai-shared/claimed/<task-uuid>.<host>.lock`）

純文字檔，1-3 行：
```
<unix-ts-claimed>
<host>
<task-uuid>
```

範例（真實檔案 `task-2026052618.mac.lock`）：
```
1716750000
mac
task-2026052618
```

存在意義：commit + push 成功 = 該 host 取得該 task 的 exclusive lock；其他 host pull 後看到此 lock 就跳過該 task。

### 7.3 Heartbeat schema（`.ai-shared/heartbeat/<host>-last.txt`）— v0.2 新增

純文字 1 行：
```
<unix-ts> <status> [task-uuid-if-busy]
```

範例：
```
1716748200 busy task-2026052701
1716750000 idle
1716752400 blocked task-2026052704 needs_operator
```

寫入時機：
- `/dev-start` step 1 開頭 → `<ts> busy starting`
- atomic claim 成功後 → `<ts> busy <task-uuid>`
- task 完成後 → `<ts> idle`
- task fail → `<ts> blocked <task-uuid> <reason-summary>`

讀取時機：
- 任何 claude session 在 Step 4 `read 4 queues` 時順便 `cat .ai-shared/heartbeat/*-last.txt`，得到其他 host 是否還活著的 snapshot
- v0.2 規則：`now - other_host_last_hb > 2h` → 視為 stale（過期），可考慮把該 host 的 platform-agnostic queue 拉一條到 SHARED 由其他人接（須 operator 確認）

### 7.4 `progress.md` schema（`.ai-shared/progress.md`）

5 section（順序固定）：
```markdown
## §1 ship gate matrix (per SPEC-60 — V1-V12)
- [x] V1 build all green ...
... (12 checkboxes total)

## §2 Wire module Stage 4 progress (per V0_7_0_DEFERRAL_INVENTORY.md)
- [x] 13/18 wires fully Stage 4 ...
... (4-5 checkboxes)

## §3 Phase H (frontend wire-up) Tier 0 ship-blocking surfaces
- [ ] Onboarding wired end-to-end ...
... (5 checkboxes)

## §4 Last-mile pre-release checks
- [ ] `grep -rE ...` = 0 hits
... (3-5 checkboxes)

## §5 Recent task completions (newest first, last 20)
- 2026-05-26 task-2026052610 mac H1.1 onboarding_wire Tauri commands
... (rolling list)
```

合併規則（多機同時 push 衝突時）：**union-of-checks** — 取 `[x]` 優先於 `[ ]`；§5 list 按 timestamp 排序 newest-first 後去重。

### 7.5 Done record schema（`.ai-shared/done/<task-uuid>.md`）

5 欄位 markdown：
```markdown
## <task-uuid>
- host: <host>
- branch: wip/<host>/<task-uuid>
- completed: <ISO-8601 timestamp>
- diff: <git diff main --shortstat>
- summary: <one-paragraph what was done>
```

存在意義：operator review wip branch 時的快速 context；歷史 audit trail。

### 7.6 Quota alerts schema（`.ai-shared/quota-alerts.md`）

Append-only markdown，每行：
```markdown
- `[<ISO-ts>]` **<EVENT-KIND>** host=`<host>` tool=`<tool>` — <detail>
```

`<EVENT-KIND>` 取值：`QUOTA-EXHAUSTED` / `TOOL-FAIL` / `ALL-EXHAUSTED`。

由 `dispatch-with-fallback.sh` 自動寫入（無需手動）；operator weekly reset（清檔 + commit `alerts: weekly reset`）。

---

## §8 Invariants

> 本節列舉本 spec 的**不變式**（invariant = 任何時刻都必須為真的命題）。違反 = bug。

### 8.1 Queue partitioning invariant（v2 rule 1+2 codify）

> **任何 task UUID 只能在 1 個 queue file 裡出現。**

形式化：對任意 task-uuid `T`，下式恆真：
```
| { F | F ∈ {mac, node-b, node-a, SHARED}.todo.md  ∧  T ∈ F.lines } | ≤ 1
```

附屬規則：
- Apple-bound task → **MUST** in `mac.todo.md`，不可 in `node-a.todo.md` 或 `SHARED.todo.md`
- Android-bound task → **MUST** in `node-b.todo.md`
- Linux-bound 或 Win-bound task → **MUST** in `node-a.todo.md`（或 `node-b.todo.md` 若有相關 toolchain）
- Agnostic task → **MAY** in 任一機 queue（指定 owner）或 `SHARED.todo.md`（atomic claim required）

**違反偵測**：CI lint script（v0.3 補）— `grep -h '^- \[' .ai-shared/queue/*.todo.md | awk '{print $3}' | sort | uniq -d` 應 always-empty。

### 8.2 File-area ownership invariant

> **§6.3 file-area ownership matrix 是硬約束（hard constraint）— 非 advisory。**

任何 commit touch 某檔案區（path prefix match）必須由該區的 owner host 產生（commit 時帶 `Host: <host>` trailer 或 commit message contain `(<host>):`）。

**違反偵測**：v0.3 加 pre-push hook 檢查；目前靠 operator review wip branch 時人工把關。

### 8.3 Claim atomicity invariant

> **任一 task 任一時刻最多被 1 個 host hold lock。**

實作：claim 動作 = (touch lockfile + edit queue → `[~]` + git commit + git push) 整包當成原子操作；git push 的 fast-forward 規則保證 race 只有一個贏家。Loser 必須 rollback（rm lockfile + 還原 queue mark）後重新挑 task。

**v2 強化**：claim 前必須 `git pull` 兩次 — 一次在 Step 4 讀 queue 之前，一次在 Step 6 push 之前（catch race window 內別人 push）。

### 8.4 Fetch-before-commit invariant（v2 rule 3 codify）

> **任何 claude session 在 commit 前必須 `git fetch origin && git merge-base --is-ancestor HEAD origin/main`（或等效檢查）通過。**

意義：保證 local main 跟 origin/main 同步或是 ahead，不是 behind 或 diverged。如果失敗 → `git pull --rebase --autostash` 後重試 commit。

由來：§2.2 node-a sync issue 真實事故。

### 8.5 Main branch immutability invariant

> **`origin/main` 只接受 operator 在 mac 上手動 `git merge` 的 commit；不允許任何 claude session 直接 push 到 main。**

例外：以下三類 file 允許 claude push 到 main（因 queue 操作本身需要 push）：
- `.ai-shared/queue/<host>.todo.md`（owner host edit）
- `.ai-shared/claimed/*.lock`（claim 動作的一部分）
- `.ai-shared/heartbeat/<host>-last.txt`（owner host edit）
- `.ai-shared/done/<task-uuid>.md`（owner host write）
- `.ai-shared/progress.md`（任一機 append-only edit）
- `.ai-shared/quota-alerts.md`（任一機 append-only）

實質工作（core/、app/、docs/、tests/、scripts/）的 commit 一律 push 到 `wip/<host>/<task-uuid>` branch；operator 在 mac 用 `gh pr merge --squash` 或本機 `git merge --no-ff` 進 main。

### 8.6 Subscription pool independence invariant

> **3 機 × 5 tool = 15 個 subscription pool 彼此獨立；同一 tool 同一 host 的 quota 只在該 host 該 tool 用完才算 exhausted；不可推論「mac codex 用完 → node-a codex 也用完」。**

意義：fallback chain 必須 per-host 獨立追蹤；`.ai-shared/quota-alerts.md` 的 alert 標 host，下次該 host 該 tool 才跳過。

### 8.7 Operator-attended invariant（Mode B 定義性質）

> **每個 task 的 claim 前 operator 必須在 console / chat 確認；不允許 claude 自己 batch-claim 多個 task 連續跑。**

例外：operator 主動說「跑下一個 5 個 task 不要問我」可暫時 batch（但仍每 task 完成寫 done.md）。

---

## §9 API surface（bash CLI interfaces）

> Process spec 的 API = bash script interfaces。所有以下檔案皆 grep 得到（§6.2 cite）。

### 9.1 `verify-tools.sh`

```
bash scripts/ai/verify-tools.sh [--json | --strict]
```

- `--json`：輸出 machine-readable JSON（`/dev-start` Step 1 可解析）
- `--strict`：任一 tool MISSING → exit 1
- No flag：human-readable table + known-issues warnings

Output schema（JSON mode）：
```json
{
  "host": "mac",
  "os": "Darwin",
  "checked_at": "2026-05-26T15:30:00+08:00",
  "tools": [
    {"name": "claude", "installed": true, "version": "1.x.y", "login_state": "OK"},
    ...
  ],
  "known_issues": ["agy headless mode unreliable on macOS ..."]
}
```

### 9.2 `dispatch-with-fallback.sh`

```
bash scripts/ai/dispatch-with-fallback.sh <prompt-file> [tool1 tool2 ... toolN]
```

無 tool 引數 → 預設 chain = `opencode codex agy`（claude subagent 留 claude 自己 spawn 不在 chain 內）。

行為：
1. 依序 try 每個 tool（`bash scripts/ai/dispatch.sh <tool> <prompt-file>`）
2. 成功（exit 0）→ stdout 印 result，exit 0
3. 失敗 → 看 stderr pattern match 是否 quota exhausted（per tool 不同正則 — 見 `dispatch-with-fallback.sh:45-64`）；log alert；跳下一個
4. 全部失敗 → log `ALL-EXHAUSTED`、Telegram notify（若 `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` env 或 `~/.spectyn-mesh/notify-config.json` 設定）、exit 1

### 9.3 `/dev-start` slash command

由 `.claude/commands/dev-start.md` 註冊。8-step ritual：

| Step | 動作 | invariant ref |
|---|---|---|
| 1 | verify-tools (OS-aware) | — |
| 2 | git pull --rebase --autostash | §8.4 |
| 3 | read CLAUDE.md / AGENTS.md / BIG-GOAL.md / MEMORY.md（內化） | — |
| 4 | read 4 queues + progress.md + blockers.md + quota-alerts.md | §8.1 |
| 5 | propose action（task + alts） | — |
| 6 | on operator confirm → atomic claim → checkout wip branch → dispatch | §8.3, §8.7 |
| 7 | on completion → mark queue done → write done.md → push wip | §8.5 |
| 8 | loop（operator confirms next task or stops） | §8.7 |

### 9.4 Helper scripts（既有 + v0.3 計畫）

| Script | 存在狀態 | 用途 |
|---|---|---|
| `scripts/ai/dispatch.sh` | shipped | 單 tool dispatch primitive |
| `scripts/ai/tool-select.sh` | shipped | task type → tool 偏好查詢 |
| `scripts/ai/sync-memory.sh` | shipped | claude memory → `.ai-shared/memory/` 同步 |
| `scripts/ai/queue-add.sh` | v0.3 計畫（per AUTO-DISPATCH-DESIGN §10） | operator 寫新 task 的 helper |
| `scripts/ai/queue-lint.sh` | v0.3 計畫（§8.1 違反偵測） | CI lint 確保 invariant |
| `scripts/ai/coordinator.sh` | v0.3 計畫 | watchdog + rebalance（Mode A 啟用後） |

---

## §10 End-to-end flow（從 idle 到 merge）

> 詳細 8-step 與 §9.3 對應；本節寫使用者視角的「一天典型路徑」。

### 10.1 Morning（mac，~10 min）

1. Operator 開 mac → 開 Claude Code 在 spectyn-mesh repo
2. 輸入 `/dev-start`
3. Claude 跑完 §9.3 step 1-5，顯示：
   ```
   Host: mac
   Tools available: claude (OK), codex (OK), opencode (OK), agy (UNKNOWN), gemini (MISSING)
   Queue: 3 pending in mac.todo.md, 2 in SHARED, 0 in node-a/node-b (其他機目前 offline 看不到 hb)
   Progress: 8/12 ship gates green; current focus = Phase H Tier 0
   Blockers: 0
   Proposed: task-2026052612 "Wave H3.3 iOS sim build + demo-30sec-life-hello smoke" (~30 min)
   Alternatives: task-2026052613 (macOS notarization), task-2026052640 (H2.1 React onboarding wiring — SHARED)
   Confirm? (y/n/pick #)
   ```
4. Operator 回 `y`
5. Claude 自動跑 step 6（atomic claim）→ checkout `wip/mac/task-2026052612` → dispatch

### 10.2 Throughout day（任一機，~5 min × N）

6. 同時 operator SSH 進 node-a（或 node-b 親自坐到），開 claude → `/dev-start` → 跑 ritual → claim 不同 task
7. 3 機平行跑各自 task；operator 用 mac 主要 review wip branches
8. 偶有 SHARED task → 兩機都看到 → 先 push 的贏（§8.3）
9. 完成的 task → push wip → operator 在 mac 上 `gh pr view <branch>` → `gh pr merge --squash`

### 10.3 Bedtime（mac，~10 min）

10. 跑 `git pull` 確認所有 wip branch + queue + progress 同步
11. 看 `.ai-shared/progress.md` 今天進度（V12 接近 GA 嗎？）
12. 看 `.ai-shared/quota-alerts.md` weekly reset 是否該做
13. 看每機 `.ai-shared/heartbeat/*-last.txt` 確認狀態（沒掛掉 / 沒卡 blocked）
14. （可選）operator 寫新 task 到對應 queue file（per §8.1 partitioning）
15. 收工

---

## §11 v2 lessons learned（2026-05-26 dogfood 後 codify）— **本檔靈魂章節**

> 這 5 條規則直接從今天踩到的 bug / inefficiency 提煉。每條附 (a) 真實事件、(b) 規則、(c) 強制機制（v0.3 計畫）。

### 11.1 Rule 1 — Queue partitioning 是硬規則不是 advisory

**事件**：H1.3 capture_focus_wire 在 `mac.todo.md` 列為 `task-2026052618 [apple, priority-high]`、同時在 `node-a.todo.md` 列為 `task-2026052632 [agnostic]`。兩機都看到自己的 queue 有 H1.3 → 都開始做 → 兩個 wip branch 都產生 capture_focus_wire 的 Tauri command 實作 → mac 先 merge 後 node-a 才知道 → 補救 commit `be9619d test(h1.3): cherry-pick 5 node-a unit tests` 把 node-a 寫的 unit test 搬到 mac 版本 → 30% wasted work（node-a 寫的 wire 主體被丟）。

**規則**：**任何 task 只能在 1 個 queue file 出現**（§8.1 invariant）。寫 task 時 operator（或未來自動工具）必須先 grep 4 queue 確認 UUID + topic 不重複；同 topic 不同 UUID 出現在多 queue = bug（即使 UUID 不同）。

**強制機制**（v0.3）：
- `scripts/ai/queue-lint.sh` — CI 跑 + pre-commit hook 跑；發現 dup → exit 1
- `scripts/ai/queue-add.sh` — operator 加 task 用此 helper，會自動 grep 4 queue 提示重複

### 11.2 Rule 2 — Platform-bound 強制歸屬

**事件**：H1.3 即使在 node-a queue 標 `agnostic`，本質上是 Tauri command（需要 Tauri config + mobile entry point），跟 Apple toolchain 高度耦合；node-a 寫的版本沒在 iOS sim 跑過驗證，等於沒過 H1 acceptance criteria。

**規則**（§8.1 附屬）：
- Apple-bound（凡 touch `core/src/ios/` / `app/src-tauri/ios/` / `scripts/package-ios.sh`） → **MUST** mac queue
- Android-bound（凡 touch `core/src/android/` 等） → **MUST** node-b queue
- Linux+Win-bound → **MUST** node-a queue
- Agnostic（pure Rust core / spec / docs / tests）→ **MAY** SHARED 或 owner queue
- **Tauri command 本質上 platform-related** — 不算 agnostic，要歸 mac（因 5-OS build 的 verify 主場在 mac）

**強制機制**：task brief 必填 `platform` 欄位；queue-lint.sh 檢查 platform vs queue file 一致性。

### 11.3 Rule 3 — Fetch-before-commit 必跑

**事件**：node-a 用 git bundle 從 mac 帶 commit catch-up 之後，local `main` ref 沒 fast-forward 到 origin/main（bundle 只 import object，不自動 update refs）；node-a 接著新 commit 是以 stale local main 為 parent → push 時 fast-forward 失敗 → claude 看到 reject 沒解釋清楚直接 retry / force → 短暫破壞 main 線性歷史；事後 git reset --hard 復原。

**規則**（§8.4 invariant）：
- 每次 commit 前跑 `git fetch origin && git merge-base --is-ancestor HEAD origin/main`
- 失敗 → `git pull --rebase --autostash` 後重試
- claude session 寫程式時觀察 commit / push 的 stderr，看到 `non-fast-forward` 必須停下來告訴 operator，**不可** silent retry / force push

**強制機制**：
- `.claude/commands/dev-start.md` Step 7 commit 段加 fetch-check
- `scripts/ai/check-sync.sh` 新 helper — 跑 ancestry check + 列差異 commit
- claude session prompt（在 ritual 內）明示：「push reject 後不可自動 force；要 surface to operator」

### 11.4 Rule 4 — Parallel dev OK only for platform-bound；agnostic = serial 或 SHARED-atomic

**事件**：v0.1 預設 parallel = 永遠好；實測 platform-bound 平行確實 3× 但 agnostic（如 H1.3）平行反而 race。

**規則**：
- Platform-bound task：mac / node-b / node-a 平行跑各自 OS-bound queue → safe（無重疊）
- Agnostic task：**只能在 1 個 owner queue 或 SHARED queue**（atomic claim 強制）；**不可同時在多 owner queue**
- 例外：純獨立 file（如 `core/tests/v6_perf_<area>.rs` 各機寫不同 area）可分散，但 task 仍要 1-queue-only

**強制機制**：queue-lint.sh 加 rule —「agnostic tag 不可 in 多 owner queue 且不在 SHARED」。

### 11.5 Rule 5 — Optional watchdog subagent every 5-10 min

**事件**：H1.3 race 是在 mac 先 push 後才被察覺；如果有個 watchdog 在 mac 跑 background 每 5 min `git pull + grep duplicate UUID` → 早 25 min 抓到，node-a 重複工作可砍掉一半。

**規則**（v0.3 optional）：
- mac 上跑一個 long-running claude subagent（background bash 或 cron-lite），每 5-10 min 跑：
  ```bash
  git fetch -q
  grep -h '^- \[' .ai-shared/queue/*.todo.md | awk '{print $3}' | sort | uniq -d > /tmp/queue-dup.txt
  [ -s /tmp/queue-dup.txt ] && notify_operator "duplicate queue entries: $(cat /tmp/queue-dup.txt)"
  ```
- 同 watchdog 順便檢查 heartbeat staleness（其他機 > 2h idle）+ wip branch 累積（> 5 unmerged = backlog warning）

**強制機制**：v0.3 寫 `scripts/ai/watchdog.sh`；operator 可選擇開或不開（不強制）。

### 11.6 Summary table

| # | Rule | Invariant ref | v0.3 enforcement |
|---|---|---|---|
| 1 | Queue partitioning 硬規則 | §8.1 | `queue-lint.sh` + `queue-add.sh` |
| 2 | Platform-bound 強制歸屬 | §8.1 附屬 | task brief `platform` field + lint |
| 3 | Fetch-before-commit 必跑 | §8.4 | ritual Step 7 + `check-sync.sh` |
| 4 | Parallel 只給 platform-bound | §8.1 + §8.3 | queue-lint.sh rule |
| 5 | Watchdog 5-10 min（optional） | — | `watchdog.sh` |

---

## §12 Performance & throughput budgets

> Process spec 的 perf budget = operator 體感 + throughput。

| Metric | Target (v0.2) | v0.1 actual | Measured by |
|---|---|---|---|
| Net throughput vs single-mac baseline | ≥ 2× | 1.4× | task count / day comparison |
| Operator daily intervention | ≤ 30 min | 45 min | self-log |
| Duplicate work rate | 0% | 30%（H1.3） | queue-lint.sh dup count / total task |
| `/dev-start` ritual wall-time（idle → propose） | ≤ 60 s | ~45 s | claude session wall clock |
| Atomic claim push race latency | ≤ 5 s | ~3 s | git push timing |
| Wip branch backlog before merge | ≤ 5 active | 6（peak） | `git branch -r | grep wip/ | wc -l` |
| Tool fallback time on quota exhaust | ≤ 10 s | — | `dispatch-with-fallback.sh` log |

---

## §13 Privacy / OSS-safety

### 13.1 Hostname / identity 隱蔽

- 本 spec 範例一律用 `mac` / `node-b` / `node-a` 三個 generic label，不出現實機 hostname（per memory `feedback_oss_safe_test_data`）
- queue file / done file / heartbeat file 不寫真實 IP / Tailscale name / 私人 email
- `.ai-shared/notify-config.json`（Telegram bot token）放 `~/.spectyn-mesh/` 不放 repo
- 任何 task brief 範例用 placeholder：`user42@example.com` / `127.0.0.1` / `~/.spectyn-mesh/<file>`

### 13.2 Subscription credential 隔離

- 各 tool 的 login state（`~/.claude/auth.json` / `~/.codex/auth.json` 等）**per-machine, never shared**；不放 repo、不放 `.ai-shared/`、不複製
- `verify-tools.sh` 只 report login boolean（exists / unknown），不讀 token 內容
- 換機 = 重新登入；不允許 transplant auth file

### 13.3 Wip branch 內容審查

- operator merge 前須 `gh pr diff <branch>` 看一眼，主要審：是否誤含 secret / 是否誤刪檔 / 是否 OSS-leak
- 任一 wip branch 含敏感資料 → `gh pr close --delete-branch` + 從 reflog 抹除 ref；如果已 push 到 origin → `git push origin --delete wip/<host>/<task-uuid>` + GitHub UI 確認

---

## §14 Migration

不適用（n/a）— 本 spec 是新流程，從 v0.5 single-machine baseline 直接切到 Mode B v0.1（2026-05-26 morning），再 in-place 升 v0.2（本 spec）。

未來 v0.3 → v1.0 → INFRA02（SPEC-26 cluster_dispatch eat-own-dogfood）的 migration plan 等 INFRA02 spec 撰寫時定義。

---

## §15 OoS / 暫不做（彙整）

- Mode A cron-unattended — Out of scope（留 v0.8.0 INFRA01 v0.3 或獨立 spec）
- spectyn 自家 cluster_dispatch eat-own-dogfood — OoS（留 INFRA02，基於 SPEC-26 Stage 4）
- Live collaborative editing — OoS（永久 Non-Goal）
- Per-machine cost dashboard — OoS（subscription model 不需要）
- iPad / 手機 operator interface — OoS（未來看需求）
- 跨機 debugger / live mirror — OoS（永久 Non-Goal）
- 跨機 GPU 分擔 — OoS（永久 Non-Goal）
- Auto-merge to main — OoS（NG6 永久 Non-Goal）
- Multi-operator（兩個人共用同 mesh）— OoS（留未來 contributor onboarding 時議）
- node-a 取代 mac 為 merge authority — OoS（v0.2 仍 mac authority）

---

## §16 Risks

| # | Risk | Likelihood | Impact | Mitigation | Owner |
|---|---|---|---|---|---|
| R1 | Queue partitioning 違反復發（H1.3-type race 再發生） | 中 | 中 | §11.1 + §11.2 規則 + v0.3 queue-lint.sh | Mark |
| R2 | git push race 兩機都贏（理論不可能但 GitHub 偶有 retry 異常） | 低 | 高 | atomic claim invariant §8.3 + git push 的 fast-forward 保證；萬一 → operator 手動清 claimed/ + 重排 | Mark |
| R3 | 訂閱配額 15 個 pool 同時用完 | 低 | 中 | fallback chain + Telegram alert（dispatch-with-fallback.sh）+ operator 等 5-7h reset | Mark |
| R4 | node-a always-on 機器掛掉（硬碟 / 電源） | 低 | 高 | mac + node-b 仍可獨立跑；待 node-a 修復；agnostic task 由 mac/node-b 暫接 | Mark |
| R5 | claude session 寫程式中途 hang（>30 min 無 progress） | 中 | 低 | operator 手動 ^C；無自動 watchdog（Mode A 才需） | Mark |
| R6 | 兩機同時改不同 wire 但都 touch `core/src/lib.rs` register block → merge conflict | 中 | 中 | §6.3 rotating per-spec assignment + task brief 指定 wire owner | Mark |
| R7 | OSS leak 從 wip branch 漏出 hostname / IP | 中 | 中（私密性） | §13.3 review + memory `feedback_oss_safe_test_data` 規則 | Mark |
| R8 | 本 spec v0.2 已 codify v1 lessons 但 v0.3 enforcement script 沒寫 → 規則靠紀律不靠工具 → 復發 | 中 | 中 | v0.3 sprint 在 Phase H 後立即排（task-2026052636 已在 node-a queue） | Mark |
| R9 | claude `Edit` 改錯 queue line（誤刪 task / 改錯 state） | 中 | 中 | git log + revert；queue-lint.sh 加 syntax check | Mark |
| R10 | mac 為 merge authority 但 mac 整週 offline（travel） | 中 | 中 | wip branch 累積在 origin 等回家；node-a + node-b 繼續累積 task；不阻塞 dev 只阻塞 merge | Mark |

---

## §17 Alternatives Considered + Abandoned Ideas

### 17.1 All-Mode-A（cron-unattended only，跳過 Mode B）

**方案**：直接做 cron 每 30 min 跑 claude headless（per AUTO-DISPATCH-DESIGN.md §8 template），operator 完全不介入，純靠 queue + heartbeat + watchdog 跑。

**為何沒選**：
1. Claude Code CLI 目前沒成熟的 `--auto` / `--yolo` flag for headless（AUTO-DISPATCH-DESIGN.md §14 open question）— 命令列 batch invocation 容易卡 mid-task user-input prompt
2. 沒有 operator-attended 階段就直接 unattended，**race condition + sync bug 全埋進夜裡看不到**；今天的 H1.3 race 就是在 operator 在場時抓到的，cron 模式可能默默產生 5 個版本的 wire 等天亮看
3. cron + claude API key 安全模型未定（per-machine 加密 key store 等問題）
4. operator 對「AI 一夜寫完 10 個 wire」的信任度尚未建立 — Mode B operator-attended 是建立信任的中間步驟

**什麼條件會回來**：v0.6.0 GA 後 + Mode B 跑滿 1 個月零事故 + claude CLI 出 stable headless flag → 再評估開 Mode A 替 overnight long-running task（如 cargo audit / V6 perf benchmark sweep）。

### 17.2 SPEC-26 cluster_dispatch eat-own-dogfood now（跳過 git substrate）

**方案**：直接讓 spectyn 自己的 `cluster_dispatch_wire` 跑 dev task 派工 — operator 寫 task 給 spectyn CLI，spectyn 透過 mDNS + Tailscale 路由到對應 capability host 的 spectyn serve，spectyn serve spawn claude 跑。完全 P2P，無 git substrate。

**為何沒選**：
1. **SPEC-26 cluster_dispatch_wire 還在 Stage 3 partial**（per memory `project_v060_stage_state`：8/18 wires 含 SPEC-26 deferred deps to v0.7.0+）— wire 自己都沒 ship-ready，先拿來跑 production dev workload = 自殺
2. **Eat-own-dogfood 的前提是被吃的功能可靠**；cluster_dispatch 沒被 1 個外部 production scenario 驗證過，直接拿來扛 GA-blocking dev workload 風險太大
3. git substrate 雖然 wire 級慢（每 commit 一次 round-trip），但**有 100% 可觀察性**（每個 claim / commit 都在 git log）— spectyn mesh 級則需要新的 observability stack（SPEC-07）才看得清楚
4. operator daily review 的 surface 還是 `gh pr` — 用 git substrate 直接接 gh，用 spectyn mesh 要額外做 dashboard

**什麼條件會回來**：SPEC-26 完整 Stage 4 + SPEC-07 observability 端對端整合 + 至少 3 個外部 production scenario 跑過 1 個月零事故 → 升 INFRA02。本 spec `Blocks` 欄位列了這個關係。

### 17.3 Single-machine no-Mode-B baseline（維持 v-0.5 序列開發）

**方案**：放棄 3 機協作；operator 只在 mac 一台機器一個 claude session 序列開發，node-a + node-b 純當 build verification farm（跑 cargo test for non-Mac OS）。

**為何沒選**：
1. v0.6.0 GA 在 6/15，從 5/26 算還剩 20 天；Phase H 剩餘 task 估算 50-80 工時，mac 一機序列跑 12 h/day × 20 day = 240 h budget 看似夠，但 operator 不可能 12 h/day 都坐 mac
2. **node-a always-on 浪費** — 那台機 24/7 開機只跑 build verify 太浪費
3. **node-b Android-bound 不可替代** — Android APK build / sign 只 node-b 能做；mac 一機就是缺料
4. **dogfood mesh philosophy 違背** — spectyn-mesh 的 BIG-GOAL.md §P1 「Cluster = first-class」，自己開發過程不用 cluster 那 spec 寫起來沒說服力

**什麼條件會回來**：3 機協作 race 失控（連續 2 週每天踩新 race）→ 暫退 single-machine 直到 INFRA02 ship-ready。

### 17.4 GitHub Actions self-hosted runners（替代 git substrate）

**方案**：3 台機器各跑 self-hosted GitHub Actions runner，operator 開 issue 寫 task，runner 拿 issue label 路由（mac label → mac runner），完成後自動 PR。

**為何沒選**：
1. GH Actions YAML 寫死的 step model 不適合 claude 動態決策（claude 中途決定 dispatch tool 順序、看 stderr 改策略 — YAML step 寫不出來）
2. self-hosted runner security 需要 GitHub App token + ephemeral runner，setup cost > Mode B
3. Runner 掛點時除錯流程比 git revert + 手動 push 複雜
4. GHA quota = pay-per-minute（self-hosted 免費但 cloud-hosted 不是）— 訂閱模型反而不適用

**什麼條件會回來**：未來 INFRA03（CI-as-cluster-peer）spec 寫時可能重新議。

---

## §SM Swarm-Migration 計畫（spectyn-coord → `/rpc/swarm`）

> task-2026052704。把 dev 派工從 **git push race（用 git 推送競賽搶任務）** 逐步搬到 spectyn-mesh 自己的 **`/rpc/swarm`（叢集扇出 RPC，cluster fan-out 遠端呼叫）**，讓 spectyn-mesh dogfood（自食其力、用自己跑自己）開發流程。

### SM.1 動機（Why）

現況：`scripts/ai/coord/leader.sh` + `follower.sh` 用「git push 誰先推誰得」搶 dev task；而 `core/src/swarm.rs` + `/rpc/swarm` 早就寫好，卻只服務 runtime（執行期）需求。**兩條路從未對話**。打通後：

- 派工統一一條路（git → swarm RPC）
- claim（認領任務）開銷從 ~30s（一次 git pull+commit+push round-trip）降到 <100ms（一次 HTTP POST）
- 對外故事：「spectyn 用自己的叢集跑自己的開發」

不一次切換的原因：git substrate（git 作為底層儲存）目前是唯一可審計、可 `git revert` 回滾的真相來源；swarm 還沒驗證能扛 dev 派工語意。故分 5 phase 漸進。

### SM.2 五階段表（每階段含 trigger / acceptance / rollback）

| Phase | 內容 | 目標時間 |
|---|---|---|
| Phase 0 | git race only（純 git 競賽，現況）| 2026-05 |
| Phase 1 | `swarm-bridge.sh` dry-run（只印 payload、不真派）| 2026-06 W1 |
| Phase 2 | Bridge mode（橋接模式）：leader 同時寫 git **又** POST swarm | 2026-06 W2-3 |
| Phase 3 | Cutover（切換）：swarm 為主、git 為 fallback（備援）| 2026-07 |
| Phase 4 | Pure swarm（純 swarm）：git race 退役 | 2026-08 |

**Phase 0 — git race only（基準線）**
- **Trigger（啟動條件）**：現況，無需動作。
- **Acceptance（驗收）**：leader.sh / follower.sh 正常派工（已是日常）。
- **Rollback（回滾）**：n/a（就是回滾目標）。

**Phase 1 — `swarm-bridge.sh` dry-run**
- **Trigger**：本 PoC（概念驗證）merge（即本 task）。
- **Acceptance**：`swarm-bridge.sh <brief> --dry-run` 對任一 brief 印出合法的 `SwarmRequest` JSON + 正確 `X-Cluster-Auth`（叢集驗證標頭）HMAC（雜湊訊息驗證碼）；不發任何真 POST。
- **Rollback**：刪 `swarm-bridge.sh`；零 runtime 影響（dry-run 不碰真叢集）。

**Phase 2 — Bridge mode（雙寫）**
- **Trigger**：`/rpc/swarm` 已能吃 coord brief 語意（Phase 2 另開 task 改 `SwarmRequest` 或加 `/rpc/coord/dispatch`）。
- **Acceptance**：leader 派一個 task 時，git queue **與** swarm job 兩邊都出現同一 task；兩邊狀態最終一致（eventual consistency，最終一致性）；git 仍是真相來源。
- **Rollback**：把 leader 的 swarm POST 那段註解掉 → 立刻退回 Phase 0/1（git 仍完整）。

**Phase 3 — Cutover（swarm 為主）**
- **Trigger**：Phase 2 雙寫連續 N 天（建議 ≥7 天）零不一致。
- **Acceptance**：新 task 經 swarm 派；git 只在 swarm POST 失敗時才寫（fallback）；follower 改 poll `/rpc/task/status`。
- **Rollback**：把「主/備」旗標反轉（swarm-primary → git-primary），不需改碼。

**Phase 4 — Pure swarm（git race 退役）**
- **Trigger**：Phase 3 連續 N 天零 fallback 觸發。
- **Acceptance**：leader.sh / follower.sh 的 git-race claim 邏輯刪除；heartbeat / done 紀錄仍可選擇性留在 git 供審計。
- **Rollback**：git tag `archive/pre-pure-swarm` 標在 Phase 3 末 commit，可 `git checkout` 還原 leader/follower 舊版。

### SM.3 範圍邊界

- **IN（本 task）**：本計畫表 + `swarm-bridge.sh` Phase 1 dry-run PoC + decision-log entry。
- **OUT（後續 phase）**：真的把 follower.sh 改用 swarm（Phase 2+）、改 `/rpc/swarm` 接受 brief 格式（Phase 2）、雙寫一致性測試（Phase 2）。

---

## §18 Open Questions & Decisions

| # | Question | Default assumption（v0.3 不另議決時採用） | When needed |
|---|---|---|---|
| Q1 | `queue-lint.sh` 該跑 pre-commit hook 還是 CI 還是兩者？ | **兩者**（pre-commit 早攔截 + CI 是 belt-and-suspenders） | v0.3 sprint |
| Q2 | watchdog（§11.5）跑哪台 — mac 還是 node-a？ | **mac**（operator 主要使用機；node-a always-on 反而沒人看 notification） | v0.3 sprint |
| Q3 | Operator 不在時，wip branch backlog 上限是多少觸發 pause？ | **5 unmerged wip = pause 新 task**（避免 review 滅頂） | v0.3 sprint |
| Q4 | Heartbeat staleness threshold（多久算離線）? | **2h idle = stale, 24h = offline alert** | v0.3 sprint（watchdog 啟用時） |
| Q5 | `.ai-shared/quota-alerts.md` weekly reset 是 automatic 還是 manual？ | **manual**（operator 一週看一次，順便檢視趨勢） | 目前 OK |
| Q6 | 新 contributor 加入 mesh（未來開源後）要新增 queue file 還是用 SHARED？ | **新增 queue file**（owner = contributor name 短碼），SHARED 仍存在 | future |
| Q7 | mac merge authority 是否有 backup（mac 整週 travel 時 node-a 暫接）？ | **不接**（wip branch 累積，operator 回家批次 merge） | future |
| Q8 | 是否該寫 `spectyn dev-status` CLI subcommand 暴露當前 mesh 狀態？（雞生蛋蛋生雞 — spectyn 自己用 spectyn）  | **v0.7.0 後再議**（先 ship GA） | v0.7.0+ |

---

## §19 Testing

> Process spec 的測試 = 流程驗證測試，不是單元測試。以下測項對應 §3.1 Goals。

### 19.1 測項清單

| Test ID | Goal | 驗證內容 | 方法 |
|---|---|---|---|
| `T-mode-b-throughput-measure` | G1 | 一週 task 完成數 ≥ 2× single-mac 一週 baseline | 對比 `.ai-shared/done/` 內 task 數（period 比 mac-only 週） |
| `T-mode-b-no-duplicate-claim` | G2 | queue file 內無重複 UUID | `grep -h '^- \[' .ai-shared/queue/*.todo.md | awk '{print $3}' | sort | uniq -d` 必空 |
| `T-mode-b-main-protected` | G3 | origin/main 所有 commit author = operator OR commit subject 含 `Merge wip/<host>/` | `git log origin/main --pretty=%s | grep -vE '^(Merge wip/|claim |queue\(|hb:|done\(|block\()'` 必只剩 operator commit |
| `T-mode-b-operator-time-log` | G4 | 一週 operator console 時間 ≤ 30 min/day average | self-log（手動 tally） |
| `T-mode-b-3-host-warm-up` | G5 | 任一機從 idle → 接首個 task ≤ 5 min | stopwatch on `/dev-start` |
| `T-mode-b-sync-invariant` | G6 | 沒有 `non-fast-forward` push 發生 | `.ai-shared/quota-alerts.md` + git reflog 無 force push 紀錄 |
| `T-mode-b-fallback-chain-works` | invariant §8.6 | opencode quota 模擬用完 → codex 自動接 | `bash dispatch-with-fallback.sh /tmp/dummy.txt` with fake quota error 環境 |

### 19.2 測項實作 owner

- T-mode-b-no-duplicate-claim：寫成 `scripts/ai/queue-lint.sh` → CI 跑（v0.3）
- T-mode-b-main-protected：寫成 `scripts/ai/check-main-purity.sh` → 每週 operator manual 跑
- 其餘：human-checked，不寫成自動測試（成本不划算）

### 19.3 Smoke 測 plan（v0.3 啟用前的 acceptance）

- Smoke 1：3 機 同時 `/dev-start` → 都成功 propose task 而非崩潰
- Smoke 2：故意把 H3.x 同 task UUID 放兩 queue → queue-lint.sh 抓到
- Smoke 3：故意斷網跑 `dispatch-with-fallback.sh` → 3 工具都 fail → Telegram 收到 alert
- Smoke 4：mac claude 跑 task A、同時 node-a claude 跑 task B（SHARED queue 的不同 task） → 兩個 wip branch 都產生 → operator 都能 merge

---

## §20 Appendices

### A. Sample queue + brief（OSS-safe placeholders）

`.ai-shared/queue/mac.todo.md` 範例片段：
```markdown
# Queue — mac (Apple-bound + heavy Rust + spec authority)

> **Owner**: only mac claude can claim. Others read but don't claim.
> **Schema**: `- [<state>] <task-uuid> [<tags>] <one-line brief>` (states: ` `=pending, `~`=in-progress, `x`=done, `!`=blocked)
> **Tags**: apple / agnostic / spec / ci / priority-high
> **Briefs**: `.ai-shared/queue/mac/<task-uuid>.brief.md`

## Active
- [ ] task-2026052701 [apple, priority-high] iOS sim build + 30s-hello smoke
- [~] task-2026052702 [spec] Write SPEC-81-INFRA-watchdog (v0.3 enforcement)
- [x] task-2026052703 [agnostic] Refactor coach_wire::aggregate to borrow events

## Backlog
- [ ] task-2026052704 [ci] Verify ship-gate-coverage.yml on first PR

## Done (audit trail — don't delete)
(empty)
```

`.ai-shared/queue/mac/task-2026052701.brief.md` 範例：
```markdown
# Task brief — task-2026052701

## Goal
iOS sim build + 30s-hello smoke pass.

## Context
- scripts/package-ios.sh --sim already exists
- 30s-hello demo at app/src/demo/Hello30Sec.tsx
- Acceptance per SPEC-28 §3 G2

## Acceptance criteria
- `bash scripts/package-ios.sh --sim` exit 0
- iOS sim shows hello screen within 30s
- screenshot at `/tmp/30s-hello-ios.png` saved

## Constraints
- No new Cargo deps
- OSS-safe placeholders only

## Branch
wip/mac/task-2026052701
```

### B. References

- [`scripts/ai/AUTO-DISPATCH-DESIGN.md`](../../../scripts/ai/AUTO-DISPATCH-DESIGN.md) — 原 design doc（Mode A + B 都在）
- [`.claude/commands/dev-start.md`](../../../.claude/commands/dev-start.md) — `/dev-start` ritual
- [`scripts/ai/verify-tools.sh`](../../../scripts/ai/verify-tools.sh) — Step 1 工具檢查
- [`scripts/ai/dispatch-with-fallback.sh`](../../../scripts/ai/dispatch-with-fallback.sh) — fallback chain
- [`scripts/ai/dispatch.sh`](../../../scripts/ai/dispatch.sh) — 單 tool dispatch primitive
- [`.ai-shared/tool-policy.md`](../../../.ai-shared/tool-policy.md) — per-task tool 偏好矩陣
- [`.ai-shared/queue/`](../../../.ai-shared/queue/) — 4 queue files
- [`.ai-shared/progress.md`](../../../.ai-shared/progress.md) — ship gate
- [`SPEC-26-SYSTEM-cluster-dispatch.md`](SPEC-26-SYSTEM-cluster-dispatch.md) — 未來取代 git substrate 的 spec
- [`BIG-GOAL.md`](../../BIG-GOAL.md) §Four pillars P1（line 25）— pillar grounding
- Memory `reference_3machine_subscription_gateway.md` — operator 跨 session 筆記

### C. Glossary（補完 §1.3 沒涵蓋的）

- **AVD（Android Virtual Device，Android 虛擬裝置）** — Android emulator 實例
- **NDK（Native Development Kit，原生開發套件）** — Android 編譯 Rust → ARM 二進位的工具鏈
- **MSI（Microsoft Installer，微軟安裝檔）** — Windows 安裝包格式
- **always-on（永開機）** — 24/7 不關機的機器；本 spec 指 node-a
- **pre-commit hook（提交前掛鉤）** — git 在執行 commit 前自動跑的 script，可拒絕 commit
- **fast-forward（快轉合併）** — git push / merge 的一種模式，要求 local ref 是 remote ref 的 ancestor 才允許
- **non-fast-forward（非快轉）** — push 失敗的常見原因，表示 local 跟 remote 已 diverge
- **reflog（refs 紀錄）** — git 內部記錄所有 ref 變動的 log，用於 recover 誤刪 / force push
- **squash merge（壓縮合併）** — 把 wip branch 的 N 個 commit 合成 1 個 commit merge 進 main
- **rebase autostash（變基自動暫存）** — `git pull --rebase --autostash` 自動把 dirty changes stash → rebase → 還原
- **lockfile（鎖檔）** — 表示資源被占用的 marker file，本 spec 指 `.ai-shared/claimed/*.lock`
- **substrate（底層基板）** — 上層系統依賴的底層；本 spec v1 substrate = git，v2 substrate 將為 spectyn mesh
- **eat-own-dogfood（吃自己狗食）** — 用自家產品開發自家產品；本 spec 是「半 dogfood」（用 mesh-philosophy 但 substrate 還是 git）

### D. Changelog

- **0.2.0 (2026-05-26 evening)** — codify 今天 dogfood 踩到的 5 條 lessons 到 §11；新增 §8.4 fetch-before-commit + §8.6 subscription pool independence invariants；補 §17.4 GitHub Actions abandoned alternative；補 §19 testing matrix（process-level）。
- **0.1.0 (2026-05-26 morning)** — initial Mode B shipped；6 個檔案 ship（verify-tools.sh / dispatch-with-fallback.sh / dev-start.md / 4 queue files / progress.md）；同日 evening 因 H1.3 race 觸發 v0.2 rewrite。

---

# INFRA spec 寫作硬規則（適用本檔）

1. 不引新 Cargo dep（process spec）— 確認 `Depends on: none`。
2. 不寫 user-facing copy（這是內部 infra，不是產品功能）。
3. OSS-safe — 範例一律 `mac/node-b/node-a` generic label，無真實 hostname / IP / email / Tailscale name。
4. CITE 既有檔（§6.2 + §20.B）— 不發明不存在的 script 名（v0.3 計畫的 script 明示「v0.3 計畫」）。
5. §11 是本檔靈魂 — 不可省（沒有它就退化回 design doc 不是 spec）。
6. §17 至少 3 個 abandoned（本檔 4 個）— 證明 design choice 有比較。
7. 文檔頂部明示「INFRA, dev-tooling, 不是 user-facing contract」— 避免被誤讀成 product spec。
