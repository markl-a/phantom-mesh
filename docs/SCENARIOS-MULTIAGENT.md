# 場景 → phantom 多代理（multi-agent，多重 AI 代理）指令

> **目前在 phantom 0.4.0 上可運作的前 10 名**位於本檔案
> 最上方（下一節）。完整的 95 個場景腦力激盪緊接其後，作為一份
> 前瞻性藍圖（roadmap）——這些場景還需要額外的代理設定（agent config）、
> 事件鉤子（event hook）或尚未隨產品出貨的領域專屬工具。

---

## 🎯 前 10 名 — 已在 phantom 0.4.0 上驗證可運作（2026-05-11）

每個場景都是一條可直接貼上執行的指令。在全新安裝的環境上皆可乾淨地
結束、無需進一步設定（前提是你已執行過 `phantom onboarding`，
且環境變數中至少有一組供應商金鑰（provider key））。

### T1. 早會（morning standup）—— 看看一夜之間出了什麼貨
```bash
phantom autoevolve digest --since-hours 24
```
**模式（pattern）**：D（排程／事件驅動，由你閱讀結果）。
phantom autoevolve 透過 launchd 每小時執行一次；這條指令會讀取
過去 24 小時的提交（commit）＋排隊任務（queued task）＋失敗紀錄。
**實際輸出**：依狀態分類的計數（綠燈／已修復／失敗）、提交雜湊（commit sha）、
佇列深度（queue depth）。約 50 行。

### T2. 6 個專案的儀表板（dashboard），可從任何裝置存取
```bash
phantom serve &
open http://127.0.0.1:7878/projects
```
**模式**：A（單一節點主機；Tailscale 上每台裝置皆可連線）。
招募人員／手機／iPad 全都連到同一個 URL → 看到 6 個磚塊（tile）＋叢集（cluster）
狀態＋即時活動串流（live activity feed）。點按 [Run Demo] → 透過 SSE 串流輸出。

### T3. 跨整個程式碼庫（codebase）的唯讀（read-only）調查
```bash
phantom run --agent researcher "Find all callers of phantom_mesh::permission::Engine::evaluate. Cite files."
```
**模式**：A（單一唯讀子代理（sub-agent），無共享歷史）。
使用 content_search ＋ file_read 工具。回傳 markdown 條列。
牆鐘時間（wall-clock）約 30 秒。

### T4. 多步驟的編碼任務（重構／修 bug）
```bash
phantom run --agent coder "Add a unit test for permission::wildcard_match covering CJK whitespace edge case."
```
**模式**：A（搭配 file_edit ＋ cargo_check 工具的單一代理）。
寫出測試、執行 cargo、回報綠燈或失敗。首次執行（cargo 編譯）
牆鐘時間約 1-2 分鐘。

### T5. 對當前差異（diff）做程式碼審查（code review）
```bash
phantom run --agent reviewer "Review HEAD. Flag security issues, panic risks, dead code."
```
**模式**：A。審查者（reviewer）依設計即為唯讀——不會修改程式碼。
回傳一份帶有 line:column 參照的結構化 markdown 審查報告。

### T6. 平行研究扇出（fan-out，單機）
```bash
phantom run --agent master 'Run parallel_tasks for: [{agent:"researcher", prompt:"What does tokio CancellationToken actually do under the hood?"}, {agent:"coder", prompt:"Show a 10-line example of CancellationToken in select! with timeout"}]'
```
**模式**：B（透過 `parallel_tasks` 工具的單機扇出）。
兩個子代理並行執行；結果合併為單一回應。

### T7. 跨機派發（dispatch，Tailscale 叢集）
```bash
phantom run --node node-a --agent coder "Build the project here and report the binary size"
```
**模式**：C（透過 `subagent({node})` 的單一對等節點（peer）派發）。
從 `agents.toml` 的 `[cluster] peers` 挑選 `node-a`，透過 HMAC 驗證的
/rpc/message 將提示（prompt）送出，並回傳該對等節點的輸出，
帶有 `[subagent: coder@node-a · remote · 4.2s]` 標頭。

### T8. 權限政策（permission policy）強制執行
```bash
# Add to ~/.phantom-mesh/agents.toml:
#   [permissions]
#   deny  = ["Read(./.env)", "Bash(rm -rf *)"]
#   ask   = ["Bash"]
#   allow = ["Bash(git status)", "Bash(cargo check)"]
phantom doctor              # verify rules parsed
phantom run --agent coder "What's in ./.env?"     # → denied
```
**模式**：A，前方加上權限引擎（permission engine）。
招募人員可即時看到拒絕鏈（deny chain）被觸發。`phantom doctor`
會顯示「4 rules parsed (2 deny, 1 ask, 1 allow); statically denied:
web_fetch (will be hidden from LLM tool list)」。

### T9. 背景自我改善迴圈（self-improvement loop）
```bash
echo "Add a docstring to permission::wildcard_match explaining the glob semantics" \
  >> ~/.phantom-mesh/autoevolve.queue.txt
phantom autoevolve schedule status     # confirm hourly cadence
# (then walk away — checks back in via T1 tomorrow)
```
**模式**：D。當 cargo 為綠燈時，Autoevolve 會撿起排隊任務，
派發一個 evolve 代理去執行，成功後提交（commit）＋推送（push）。

### T10. 透過 MCP 橋接（MCP-bridged）從 Claude Code 使用
```bash
claude mcp add phantom $(which phantom) mcp
# Then in a Claude Code session in any repo:
#   "Use mcp__phantom__subagent to dispatch this PR review to the
#    `reviewer` agent."
```
**模式**：從 Claude Code 的角度看是 A；phantom 的 50+ 個工具
會以 `mcp__phantom__*` 的形式，與 Claude Code 的內建工具並列出現。

---

## 📌 驗證方式

```bash
phantom selftest --feature mcp           # T10 path
phantom selftest --feature projects-dashboard  # T2 path
phantom selftest --feature cluster-rpc   # T7 path
phantom selftest --feature permission-dsl  # T8 path
phantom selftest --feature autoevolve-queue  # T9 path
phantom selftest --feature digest        # T1 path
./scripts/test-mcp-tools.sh              # T3-T6 paths
```

全部 18 個 selftest 功能 ＋ 38 個 shell 測試檢查，透過 CI 把關每一次推送
（`.github/workflows/ci-shell-tests.yml`）。

---

## 🔭 前瞻性的 95 個場景腦力激盪

本線以下的一切皆為**未來工作＋設計空間（design space）**——
上方 10 個場景所代表的種子案例（seed-case）背後更廣大的願景。它們皆未
被 phantom 0.4.0 的基礎能力所阻擋；它們需要：
- 8 個額外的代理角色（fetcher、synthesizer、standup、triage、
  digestor、reporter、coach、local）——直接了當的設定
- 用於「當 X 發生時觸發」的事件鉤子（下方的 [TODO: events]
  標記）——phantom-mesh 藍圖項目

請把這份文件當作藍圖來讀，而非功能清單。

---

> 與 `_planning-audit/archived/misc-strategy/USE-SCENARIOS.md` 為姊妹篇。
> 該腦力激盪中的每個場景都會得到一條確定性（deterministic）的 phantom
> 命令列與一個多代理拓樸（topology）。請把本檔案當作各場景的
> 實作規格（implementation spec）。

下方引用的四種執行模式：

| 模式 | 何時使用 | phantom 指令形態 |
|---|---|---|
| **A** 單一代理 | 問答／摘要，無需平行處理 | `mcp__phantom__subagent({agent: "X", prompt: "..."})` |
| **B** 單機平行 | 同一節點上多個獨立的子任務 | `mcp__phantom__parallel_tasks([{agent, prompt}, ...])` |
| **C** 跨網格（cross-mesh）分散式 | 不同硬體／隱私區（privacy zone）／地點 | `mcp__phantom__subagent({node: "host:port", agent, prompt})`，扇出時再加 `parallel_tasks` |
| **D** 排程／事件驅動 | 週期性或由外部事件觸發 | `phantom autoevolve schedule install --interval N --agent X`（單發週期性）；標記 `[TODO: events]` 的事件鉤子尚未出貨 |

一個場景可使用多種模式——例如 S3（夜間重構）即為
**D + C**：午夜排程，扇出至各 GPU 機器。

下方引用的代理角色名稱：

```
master       general-purpose, default
coder        focused tool-using engineer
reviewer     read-only diff/PR analyst
researcher   web/paper/RSS scanner
fetcher      web fetch + scrape (mobile-IP-aware on Android)
synthesizer  collates outputs from N parallel agents
standup      git-log / activity → markdown summary
triage       inbox / alert / event categorizer
digestor     long → short summarizer (RSS, podcasts, papers)
reporter     periodic report builder (weekly / monthly)
coach        goal-aware push-style trainer (fitness / learning)
local        on-device MLX agent (privacy-locked)
```

上述 12 個角色中，有 4 個（`master/coder/reviewer/researcher`）
今日已隨 `configs/agents.*.toml` 出貨；其餘 8 個需作為
第二天（Day 2）實作的一部分，加入 `agents.toml`。

---

## A. 軟體工程師（Software Engineer）—— 工作（S1–S15）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| S1 | 當前 repo 小 bug fix | A | `subagent({agent:"coder", prompt:"<bug>"})` | —（Claude Code 已涵蓋；phantom 不應重複） |
| **S2** | 跨 3–5 repo 改動 | **C** | `parallel_tasks([{node:"laptop", agent:"coder", prompt:"repo A: …"}, {node:"node-a", agent:"coder", prompt:"repo B: …"}, …])` | repo 探索 ＋ 每節點 git_clone 啟動引導（bootstrap） |
| **S3** | Overnight 重構（500+ 檔） | **D + C** | `autoevolve schedule install --interval 3600 --target test --agent coder --distributed` | 已出貨 |
| S4 | PR review 1000+ 行 | **B** | `parallel_tasks([{agent:"reviewer", prompt:"chunk 1: lines 1-200"}, …, {agent:"synthesizer", prompt:"merge findings"}])` | gh PR 擷取工具（或沿用既有的 `git_diff`） |
| S5 | 凌晨 3 點從手機做 on-call 除錯 | C + D | 行動 UI → `subagent({node:"home-daemon", agent:"master", prompt:"<incident>"})` | telegram_listen ＋ telegram_send 工具 |
| S6 | CI 失敗 → 代理分析 | **D** | `[event: github_workflow_run] → subagent({agent:"reviewer", prompt:"analyze failed log"})` | 事件鉤子框架 |
| S7 | 探索陌生 codebase | A + memory | `subagent({agent:"master", prompt:"explore"})` + `memory_store("codebase:<repo>:<area>", findings)` | — |
| S8 | DB／框架遷移（migration） | D + 核可閘門（approval gate） | `autoevolve --watch --no-commit --target test` ＋ 手動 `/perm` 閘門 | 手動核可 UI 閘門（已部分完成：`/perm diff`） |
| **S9** | 每日 standup 整理 | **D** | `autoevolve schedule install --interval 86400` 執行 `subagent({agent:"standup", prompt:"git log --since=yesterday"})` | `standup` 代理角色 ＋ 用於遞送的 telegram_send |
| S10 | 工作交接 | A + 記憶匯出（memory export） | `subagent({agent:"master", prompt:"summarize project state"})` + `memory_list \| jq export` | memory_export 至 markdown |
| S11 | 依賴升版／CVE | **D + B** | `autoevolve schedule install --interval 86400` 執行 `parallel_tasks([{agent:"researcher", prompt:"check <pkg>"}, …])` | npm/cargo audit 工具封裝 |
| S12 | 壓力測試／效能分析 | C | `subagent({node:"perf-box", agent:"coder", prompt:"run k6 / criterion / iperf"})` | bench harness 腳本 |
| S13 | 寫 RFC／設計文件 | A + memory | `subagent({agent:"master", ctx: memory_recall + git_diff})` | — |
| S14 | Regex／shell 單行指令 | A | `subagent({agent:"master"})` | —（不與 ChatGPT 做區隔） |
| **S15** | 多分支並行實驗 | **B + worktree** | `parallel_tasks([{agent:"coder", prompt:"impl approach A in worktree-A"}, {agent:"coder", prompt:"impl approach B in worktree-B"}, {agent:"reviewer", prompt:"compare both"}])` | worktree 生成輔助（`phantom worktree create`）；已透過 Claude Code 代理隔離部分完成 |

---

## B. 資料科學家（Data Scientist）—— 工作（D1–D15）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| D1 | Jupyter EDA | A | （Copilot 已涵蓋） | — |
| D2 | 資料清理 pipeline | C | `subagent({node:"data-box", agent:"coder", prompt:"clean pipeline"})` | 資料就在 worker 附近——node 標籤 |
| **D3** | 長時訓練（GPU box）| **C + D** | `subagent({node:"gpu-box", agent:"coder", prompt:"train"})`，接著 `autoevolve schedule install --interval 60 --target test --agent reporter` 監看 nvidia-smi | gpu_status 工具 ＋ telegram_send |
| **D4** | 超參數掃描（Hyperparam sweep） | **B + C** | `parallel_tasks([{node:"gpu1", agent:"coder", prompt:"lr=1e-3"}, {node:"gpu2", agent:"coder", prompt:"lr=3e-4"}, …])` | 共享實驗儲存（mlflow / wandb / fs） |
| D5 | Paper 復現 | C + sandbox | `subagent({node:"sandbox", agent:"coder", prompt:"reproduce <paper>"})` | 環境隔離工具（docker/conda） |
| D6 | 模型結果變差 debug | A + memory | `subagent({agent:"researcher", prompt:"compare runs"})` + `memory_recall("experiments")` | mlflow/wandb 擷取工具 |
| D7 | 百 GB 資料 local-only | C | `subagent({node:"data-box", agent:"local"})`（provider=mlx-local；絕不離開本機） | —（已涵蓋：MLX 供應商） |
| D8 | 利害關係人（stakeholder）報告 | **B** | `parallel_tasks([{agent:"researcher", prompt:"data section"}, {agent:"researcher", prompt:"figures"}, {agent:"synthesizer", prompt:"merge"}])` | plot_render 工具（matplotlib/altair） |
| D9 | SQL 大查詢（30 分鐘以上） | **B + D** | `bash_run_background({command:"psql ..."})` + `autoevolve schedule install --interval 60` 輪詢結果 | sql_run 工具（具型別的結果） |
| D10 | 論文閱讀 | **D + B** | `autoevolve schedule install --interval 86400` 執行 `parallel_tasks([{agent:"researcher", prompt:"<arxiv id>"}, ...])` + `memory_store` | rss/arxiv 擷取 ＋ pdf_read 工具 |
| D11 | Slack 問指標 | C | （行動 UI）→ `subagent({node:"data-box", agent:"coder", prompt:"<sql>"})` | sql_run 工具 ＋ slack 監聽 |
| D12 | 標註 pipeline | **D** | `[event: new_data] → parallel_tasks([{agent:"local", prompt:"label batch 1"}, …])` | 事件鉤子 ＋ 裝置端標註分類器 |
| D13 | Feature store 維護 | **D + B** | `autoevolve schedule install --interval 86400` 執行回填（backfill）的 parallel_tasks | feature_backfill 工具 |
| **D14** | 漂移（drift）／schema 警示 | **D** | `[event: drift_alarm] → subagent({agent:"triage", prompt:"explain drift; severity?"})` | 漂移偵測器 ＋ slack 通知 |
| D15 | 可重現性（reproducibility）檢查 | A + memory | `subagent({agent:"coder", prompt:"reproduce <run-id>"})` + `memory_recall("env-fingerprint")` | 環境指紋（env fingerprint）工具 |

---

## C. 韌體工程師（Firmware Engineer）—— 工作（F1–F15）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| F1 | Datasheet → 暫存器（register）程式碼 | **B** | `parallel_tasks([{agent:"researcher", prompt:"read pdf <chip-X>"}, {agent:"coder", prompt:"emit register init"}])` + `memory_store("vendor:<chip>", traps)` | pdf_read ＋ table_extract 工具 |
| **F2** | build → flash → UART 迴圈 | **D** | `[event: file_changed *.c] → parallel_tasks([{agent:"coder", prompt:"build"}, {agent:"coder", prompt:"flash"}, {agent:"researcher", prompt:"watch UART for boot string"}])` | fswatch 工具 ＋ serial_read 工具 |
| **F3** | 板子 bring-up | **C** | 行動 UI → `subagent({node:"bench-pc", agent:"coder", prompt:"<step>"})` | bench-pc 作為 node 標籤 |
| F4 | 邏輯分析儀（logic analyzer）大檔 | **C** | `subagent({node:"bench-pc", agent:"local", prompt:"parse capture"})` | sigrok / saleae 匯出工具 |
| F5 | Bootloader／RTOS debug | **C** | `subagent({node:"bench-pc", agent:"coder", prompt:"…"})` | gdb_run / openocd 工具 |
| **F6** | 跨編譯多目標（multi target） | **B + C** | `parallel_tasks([{node:"linux1", agent:"coder", prompt:"target arm"}, {node:"linux2", agent:"coder", prompt:"target riscv"}, {node:"win1", agent:"coder", prompt:"target win"}])` | 每個目標在各節點上的工具鏈（toolchain） |
| F7 | OTA 上百台 device | **C** | `parallel_tasks([{node:f"dev-{i}", agent:"coder", prompt:"flash + verify"} for i in range(100)])` | 裝置探索 ＋ 批量 flash |
| **F8** | Power 分析 | **C + D** | `subagent({node:"bench-pc", agent:"coder", prompt:"sample power"})` ＋ 行動端遠端監看 | scope_read 工具 |
| F9 | NDA 程式碼 | C local-only | `subagent({node:"local", agent:"local", prompt:"…"})` provider=mlx-local | —（MLX 供應商已實現此項） |
| F10 | 出差現場 debug | C | 行動 UI → `subagent({node:"home-mesh", agent:"master"})` | 行動 UI 已出貨 |
| F11 | HW pytest | **C** | `subagent({node:"hw-rig", agent:"coder", prompt:"pytest -k boot"})` | hw rig 作為帶標籤的節點 |
| **F12** | Soak test 72hr | **D** | `bash_run_background` + `autoevolve schedule --interval 600` 輪詢異常 ＋ telegram 警示 | telegram_send ＋ 異常閾值判定 |
| F13 | FW + HW log 關聯 | **B** | `parallel_tasks([{agent:"researcher", prompt:"FW log"}, {agent:"researcher", prompt:"HW log"}, {agent:"synthesizer", prompt:"timeline"}])` | log 時間戳正規化器 |
| F14 | FW 回歸二分搜尋（regression bisect） | **D + B** | 每個 bisect 步驟執行 `autoevolve --max-rounds 20 --target test`（＋ 步驟之間 flash） | git bisect 調度（orchestration） |
| F15 | Vendor SDK 記憶 | A + memory | 將 `memory_recall("vendor:<sdk>:traps")` 注入代理 ctx | 依供應商範圍化（scoped）的記憶 |

---

## D. 工作 —— 跨人設共通（X1–X10）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| X1 | 長跑斷線續跑 | D | 透過 `bash_run_background` 建立檢查點（checkpoint）＋ 透過 job_id 輪詢恢復 | 已出貨 |
| X2 | 手機監看／下指令 | C | 行動 UI → `subagent({node, ...})` | 已出貨 |
| X3 | 多機協作 | C | `parallel_tasks`，每個任務帶 `node:` | 已出貨 |
| X4 | 會話跨裝置 | A + 持久化（persistence） | `[session.*]` ＋ `~/.phantom-mesh/conversations/` 已透過 iCloud／叢集共享 | iCloud 同步串接（規劃中） |
| X5 | 每日／每週排程 | D | `autoevolve schedule install --interval N` | 已出貨 |
| X6 | 敏感資料 local-only | C | 路由至 `agent.local`（mlx-local 供應商） | 已出貨 |
| X7 | 多 agent 並行實驗 | B | `parallel_tasks` + worktree | worktree 輔助（與 S15 相同缺口） |
| X8 | 跨 session 持久記憶 | A + memory | `memory_*` 工具 | 依情境範圍化（per-repo、per-goal） |
| **X9** | 事件觸發（git/ci/slack/email/webhook） | **D** | `[TODO: event bus]` | 事件匯流排（event bus）＋ 每觸發器註冊 |
| X10 | 成本路由 | A | agents.toml 中已有供應商輪替（provider rotation） | 已出貨 |

---

## E. SWE —— 生活（sP1–sP12）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| **sP1** | self-host 半夜炸 | **D** | `[event: heartbeat_fail] → subagent({agent:"triage"}) → telegram_send` | 事件匯流排 ＋ telegram_send |
| sP2 | Home Assistant 自動化 | A | `subagent({agent:"coder", prompt:"YAML for: ..."})` | HA API 工具 |
| sP3 | 家人技術支援 | C | 行動 UI → `subagent({node:"family-pc", agent:"master"})` | 給非 tailnet 家用 pc 的反向隧道（reverse-tunnel） |
| sP4 | OSS 副業專案 | **D + C** | `autoevolve schedule install --interval 21600 --agent coder --distributed` | 已出貨 |
| **sP5** | 私帳本／投資追蹤 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"fetch bank A export"}, …])` | 銀行 csv 解析器；絕不離開本機 |
| **sP6** | RSS／HN digest | **D + B** | `autoevolve schedule install --interval 86400` 執行 `parallel_tasks([{agent:"digestor", prompt:"feed url"}, ...])` + `telegram_send` | rss_fetch ＋ telegram_send ＋ `digestor` 角色 |
| sP7 | 小孩 STEM 陪學 | A + safety | `subagent({agent:"coach"})` ＋ 內容過濾器（content filter） | 內容過濾器；`coach` 角色 |
| **sP8** | 旅行規劃 | **B** | `parallel_tasks([{agent:"researcher", prompt:"flights"}, {agent:"researcher", prompt:"hotels"}, {agent:"researcher", prompt:"things to do"}, {agent:"synthesizer", prompt:"itinerary"}])` | 用於訂房網站的 browser_agent |
| sP9 | 部落格／文寫作 | A + memory | `subagent({agent:"master", prompt:"draft from notes"})` + `memory_recall` | — |
| sP10 | 學新框架 | A + memory | `subagent({agent:"coach", prompt:"next chapter; my level=..."})` | 每主題進度追蹤器 |
| sP11 | 照片歸檔 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"caption batch i"} for i in range(N)])` | image_caption 工具（Vision／多模態） |
| sP12 | 備份演練 | D | `autoevolve schedule install --interval 604800` 執行 `subagent({agent:"coder", prompt:"verify backups"})` | restic / borg 工具封裝 |

---

## F. DS —— 生活（dP1–dP12）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| **dP1** | Apple Health/Strava 分析 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"export+parse"}, {agent:"local", prompt:"plot trends"}])` | health_export 工具（HK XML、Strava OAuth） |
| dP2 | 個人理財回測（backtest） | A local-only | `subagent({agent:"local", prompt:"backtest"})` | csv 載入器 |
| dP3 | 睡眠／健身週報 | D | `autoevolve schedule --interval 604800 --agent reporter` | reporter 角色 ＋ 繪圖器 |
| dP4 | Kaggle 家用 GPU | C | `subagent({node:"home-gpu", agent:"coder", prompt:"submission iter"})` | 已出貨 |
| dP5 | arxiv 追新 paper | **D + B** | `autoevolve schedule --interval 86400` 依分類執行 parallel_tasks ＋ memory | rss/arxiv 擷取器 ＋ pdf_read |
| dP6 | 部落格／翻譯論文 | **B** | `parallel_tasks([{agent:"researcher"}, {agent:"synthesizer"}])` | pdf_read |
| dP7 | 家人健康 | C local-only | `subagent({node:"local", agent:"local"})` | 隱私分區（privacy partition）（已隱含存在） |
| dP8 | 育兒 data | A local-only | `subagent({agent:"local"})` | app 匯出工具 |
| dP9 | 房／車市場研究 | **B + browser** | `parallel_tasks([{agent:"fetcher"} per listing])` + `synthesizer` | browser_agent |
| dP10 | 小孩作業 | A + memory | `subagent({agent:"coach"})` ＋ 每孩記憶 | 兒童檔案記憶範圍 |
| dP11 | 社群經營 stats | **D** | `autoevolve schedule --interval 86400 --agent reporter` | 平台擷取器 |
| dP12 | 個人筆記 RAG | A | `memory_search` 已出貨，掛接至代理 ctx | 依來源範圍化（notion / obsidian） |

---

## G. FW —— 生活（fP1–fP12）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| fP1 | 3D 印表機／切片器（slicer） | A + memory | `subagent({agent:"researcher"})` + `memory_recall("vendor:<printer>")` | — |
| fP2 | Home lab 設定 | A | `subagent({agent:"coder", prompt:"audit configs"})` | — |
| fP3 | ESP32 玩具 | A + memory | `subagent({agent:"coder"})` ＋ memory | — |
| fP4 | 復古運算（Retro computing） | A | `subagent({agent:"researcher", prompt:"forum scrape"})` | browser_agent |
| fP5 | OBD／ECU | A local-only | `subagent({node:"car-pi", agent:"local"})` | obd-ii 工具 |
| fP6 | Ham／SDR | A | `subagent({agent:"coach"})` | — |
| fP7 | 智慧家電 firmware | **D** | `[event: file_changed] → build+flash`（＝ 在家版的 F2 模式） | 與 F2 相同的缺口 |
| fP8 | 電子排障 | **C** | `subagent({node:"bench-pc"})` | scope_read |
| fP9 | 孩子電子實驗 | A | `subagent({agent:"coach"})` | — |
| fP10 | 家中裝置 firmware 追蹤 | **D** | `autoevolve schedule --interval 86400 --agent researcher` | 裝置 firmware 版本檢查器 |
| fP11 | 二手零件比價 | **B** | `parallel_tasks([{agent:"fetcher", prompt:"site X"}, ...])` | browser_agent |
| fP12 | 修壞掉的 3C | A | `subagent({agent:"researcher"})` ＋ memory ＋ pdf_read | pdf_read |

---

## H. 純生活共通（XP1–XP12）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| **XP1** | Email 分流（triage） | **D** | `[event: imap_new] → subagent({agent:"triage", prompt:"categorize + draft reply"})` | imap 擷取 ＋ 草稿撰寫 |
| XP2 | 家庭行事曆協調 | A | `subagent({agent:"researcher", prompt:"merge calendars"})` | 行事曆（calendar）工具 |
| XP3 | 食譜／購物清單 | A | `subagent({agent:"researcher"})` | — |
| XP4 | 健身／飲食日記 | D | `autoevolve schedule --interval 86400 --agent reporter` | health_export |
| XP5 | 閱讀／podcast 摘要 | **B + D** | `autoevolve schedule + parallel_tasks([{agent:"digestor"}])` | podcast_transcribe ＋ pdf_read |
| XP6 | 語言學習 | A | `subagent({agent:"coach"})` | — |
| XP7 | 寫日記 | A local-only | `subagent({agent:"local"})` | 日記儲存；絕對隱私 |
| XP8 | 紀念日提醒 | D | `autoevolve schedule --interval 86400 --agent triage` | 行事曆工具 |
| XP9 | 旅行戶外 | **B** | 同 sP8 | browser_agent |
| XP10 | 報稅／政府公文 | A + pdf | `subagent({agent:"master"})` ＋ pdf_read | pdf_read |
| XP11 | 看醫生前後 | A local-only | `subagent({agent:"local"})` | 日記儲存 |
| XP12 | 智慧家電語音 | A | `subagent({agent:"master"})` ＋ HA bridge | HA 工具 ＋ 語音輸入 |

---

## I. 目標驅動的場景 —— 轉職（J1–J8）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| J1 | 履歷依 JD 客製 | **B** | `parallel_tasks([{agent:"researcher", prompt:"JD"}, {agent:"reviewer", prompt:"current resume"}, {agent:"synthesizer", prompt:"diff"}])` | 履歷解析器 |
| J2 | 投遞追蹤 | A + memory | 帶有狀態列舉（status enum）的 `memory_*` schema | 每份申請的記憶範圍 |
| J3 | 自動掃職缺 | **D + B** | `autoevolve schedule --interval 86400` + `parallel_tasks([{agent:"fetcher"} per board])` | linkedin/104/cake 的 browser_agent |
| J4 | Cover letter ＋ 研究 | **B** | `parallel_tasks([{agent:"researcher"}, {agent:"reviewer", prompt:"draft"}])` | — |
| J5 | 推薦人協調 | A | `subagent({agent:"master"})` | 行事曆工具 |
| J6 | 薪資協商 | A + memory | `subagent({agent:"researcher", prompt:"market data"})` ＋ memory | levels.fyi / glassdoor 擷取 |
| J7 | Offer 比較矩陣 | A | `subagent({agent:"reviewer"})` | — |
| J8 | 面試行程 | D | `autoevolve schedule --interval 3600 --agent triage`（行事曆） | 行事曆工具 |

## I.b — 面試準備（I1–I7）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| I1 | 題庫排程 | D | `autoevolve schedule --interval 86400 --agent coach`（間隔重複，spaced repetition） | sr-card 儲存 |
| I2 | System design | A | `subagent({agent:"coach"})` | — |
| I3 | Behavioral STAR | A + memory | `subagent({agent:"coach"})` ＋ 每個故事的記憶 | — |
| I4 | 模擬面試 | A | `subagent({agent:"master", prompt:"act as interviewer"})` | 串流語音輸入／輸出 |
| I5 | 錄影語音分析 | C | `subagent({node:"local", agent:"local"})` | whisper 本機工具 |
| I6 | 弱項診斷 | A + memory | `memory_recall("interview:misses")` | — |
| I7 | 公司題型研究 | **B** | 對 leetcode 標籤 ＋ glassdoor 執行 `parallel_tasks` | glassdoor 擷取 |

## I.c — 證照（C1–C5）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| C1 | 讀書計畫 | A + memory | `subagent({agent:"coach"})` | 行事曆 |
| C2 | 題庫 SR | D | `autoevolve schedule --interval 86400 --agent coach` | sr-card 儲存 |
| C3 | 錯題 cluster | A | `subagent({agent:"reviewer"})` | — |
| C4 | 模擬考 | A | `subagent({agent:"coach"})` | — |
| C5 | 報名提醒 | D | `autoevolve schedule + calendar` | 行事曆 |

---

## J. 持久性目標 —— 健康（H1–H7）

| ID | 場景 | 模式 | 指令 | 缺少項目 |
|---|---|---|---|---|
| H1 | 飲食拍照 → 熱量 | A + 多模態本機 | `subagent({agent:"local", input:"@image"})` | image_caption ＋ 營養資料庫 |
| H2 | 運動週達成率 | **D** | `autoevolve schedule --interval 604800 --agent reporter` | health_export |
| H3 | 體重趨勢 ＋ 停滯期 | D + memory | `autoevolve schedule + memory_recall` | health_export ＋ 繪圖器 |
| H4 | 睡眠／步數／心率 相關性 | **B** | `parallel_tasks([{agent:"researcher", prompt:"hr"}, {agent:"researcher", prompt:"sleep"}, {agent:"synthesizer"}])` | health_export ＋ 相關性（correlation）工具 |
| H5 | 餐食計畫 | A | `subagent({agent:"researcher"})` | — |
| H6 | 卡關建議 | A + memory | `subagent({agent:"coach"})` | — |
| H7 | 教練 push | D + telegram | `autoevolve schedule --interval 86400 --agent coach` + `telegram_send` | telegram_send |

## J.b — 金錢（M1–M6）

| ID | 模式 | 指令 | 缺少項目 |
|---|---|---|---|
| M1 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"bank A"}, …])` | 銀行匯出工具 |
| M2 | D | `autoevolve schedule --interval 86400 --agent reporter` | 預算規則 |
| M3 | A | `subagent({agent:"reviewer", prompt:"audit subscriptions"})` | 銀行解析器 |
| M4 | D | `autoevolve schedule + memory` | — |
| M5 | A + memory | `subagent({agent:"coach"})` + memory_recall | — |
| M6 | A | `subagent({agent:"researcher"})` | — |

## J.c — 投資（V1–V6）

| ID | 模式 | 指令 | 缺少項目 |
|---|---|---|---|
| V1 | A | `subagent({agent:"reporter"})` | 券商 API |
| V2 | D | `autoevolve schedule --interval 86400` | — |
| V3 | D | `autoevolve schedule --interval 86400` | — |
| V4 | **B** | `parallel_tasks([{agent:"researcher"} per ticker])` | rss/news 擷取 |
| V5 | A + memory | `memory_*` | — |
| V6 | D | `autoevolve schedule --interval 604800 --agent reporter` | — |

## J.d — 副業（B1–B7）

| ID | 模式 | 指令 | 缺少項目 |
|---|---|---|---|
| B1 | A | `subagent({agent:"researcher"})` | — |
| B2 | （沿用工作中的 S/D/F 模式） | — | — |
| B3 | D | `autoevolve schedule + scheduler tool` | social-post 工具 |
| B4 | A + memory | `subagent({agent:"triage"})` | imap |
| B5 | D | `autoevolve schedule --interval 604800 --agent reporter` | — |
| B6 | **D + B** | `autoevolve schedule + parallel_tasks` | browser_agent |
| B7 | D | `autoevolve schedule --interval 604800 --agent reviewer` | — |

---

## K. 長期視野目標（LS / LD / LF / LX，22 條）

這些全都是「代理在數月／數年間維護一個目標」——實作形態相同，
僅在領域上有所不同：

```
phantom autoevolve schedule install --interval 2592000 \
    --agent reporter \
    --target test  # or a custom 'review-goal' target
```

並在 `~/.phantom-mesh/goals.toml` 中搭配一個 `[goal.<name>]` 區塊：

```toml
[goal.fire]
horizon = "10y"
metrics = ["net_worth", "savings_rate"]
check_in = "monthly"
agent   = "coach"
memory_scope = "money/fire"

[goal.staff_promotion]
horizon = "2y"
metrics = ["scope", "impact_doc_count"]
check_in = "quarterly"
agent   = "coach"
memory_scope = "career"
```

那個目標資料模型（goal data model）是落地 L4/L5 視野的**最大缺片**。
95 個場景中約有 30 個倚賴它。

---

## 涵蓋率摘要

| 模式 | 它服務的場景數 | 佔 95 的百分比 |
|---|---|---|
| **A** 單一代理（聊天） | 18 | 19% |
| **B** 單機平行 | 28 | 29% |
| **C** 跨網格分散式 | 17 | 18% |
| **D** 排程／事件驅動 | 32 | 34% |

（許多場景使用 2 種模式——總數加總會超過 100%。）

## 前 12 名缺片（依場景影響力排序）

| # | 缺少項目 | 服務的場景 | 工作量 |
|---|---|---|---|
| 1 | **Telegram 發送工具** | sP1, sP6, S5, F12, D14, H7, M2, V3, …（約 25） | S |
| 2 | **事件匯流排**（git/ci/slack/imap/webhook 鉤子） | S6, X9, sP1, F2/fP7, D12, D14, XP1, XP8, …（約 20） | L |
| 3 | **目標資料模型** ＋ `[goal.*]` toml ＋ check-in 節奏 | 全部 L4/L5（約 30） | L |
| 4 | **PDF 閱讀器／表格擷取** | F1, fP12, dP5, dP6, XP10, …（約 10） | M |
| 5 | **瀏覽器代理（Browser agent）**（playwright） | sP8, dP9, J3, fP4, fP11, B6, …（約 10） | L |
| 6 | **行事曆工具** | XP2, XP8, J5, J8, I8, C5, M2（約 7） | M |
| 7 | **圖片說明／OCR（多模態）** | sP11, H1, F4, fP12（約 6） | L |
| 8 | **SQL 執行工具** | D9, D11, dP2, F11（約 5） | M |
| 9 | **RSS／arxiv 擷取** | sP6, dP5, V4（約 5） | S |
| 10 | **健康匯出工具**（HK XML, Strava） | dP1, dP3, H1–H4（約 6） | M |
| 11 | **銀行 CSV 解析器**（隱私鎖定） | sP5, dP2, M1, M3（約 4） | M |
| 12 | **語音輸入／輸出**（whisper 本機 ＋ tts） | I4, I5, XP6, XP12, sP3（約 5） | L |

## 前 8 名缺少的代理角色

`agents.toml` 今日已有 master / coder / reviewer / researcher。我們還需要：

```
fetcher       browser_agent + http_get + memory_store
synthesizer   read multiple agent outputs → single summary
standup       git tools + memory; outputs markdown bullet list
triage        classifies + drafts; reads imap/slack/event bus
digestor      input → 3-line summary + key links
reporter      template-driven periodic report (daily/weekly/monthly)
coach         goal-aware, push-style; reads [goal.*] + history
local         provider=mlx-local; tools restricted to no-network
```

將這些作為第二天（Day 2）工作加入 `configs/agents.coordinator.toml`。

---

## 實作藍圖（roadmap）

**第 1 天 —— 本檔案**（現在）。這份對映即是規格。

**第 2 天（約 6 小時）** —— _Telegram ＋ 8 個代理角色 ＋ cron 式排程器_。
落地約 25 個 D 模式場景（通知與每日任務）。

**第 3 天（約 6 小時）** —— _SQL 執行 ＋ RSS ＋ 行事曆 ＋ 銀行 CSV（local-only）_。
從 D / B / J 再落地約 15 個。

**第 4 天（約 6 小時）** —— _PDF ＋ 多模態 image_caption ＋ health_export_。
落地 F 韌體研究場景 ＋ H1–H4 ＋ dP1。

**第 5 天（約 8 小時）** —— _事件匯流排_（git/ci/slack/imap webhook 接收器）。
解鎖 S6, X9, F2/fP7, sP1, XP1——所有「自動反應（auto-react）」叢集。

**第 6 天（約 6 小時）** —— _瀏覽器代理_（playwright）＋ glassdoor / linkedin
擷取器。落地 sP8, dP9, J3, B6, fP4/fP11。

**第 7 天（約 8 小時）** —— _目標資料模型_ ＋ `[goal.<name>]` toml ＋ check-in
LaunchAgent（每個目標一個）。落地全部 L4/L5（約 30 個場景）。

**第 8 天以後** —— 語音輸入／輸出 ＋ 各平台打磨 ＋ 端對端實戰演練（dogfood）前 5 個
真實場景。

= 約 46 工時 / 7 個日曆天全職 = 完整 95 個場景的涵蓋，
同時具備單機與跨網格的多代理路徑。

---

## 如何使用本檔案

1. 依 ID（S2, dP1, …）**挑一個場景**
2. 閱讀其**模式**欄——那就是多代理拓樸
3. 執行／客製其**指令**欄——那就是 phantom 的單行指令
4. 若**缺少項目**欄有寫東西，那就是在該工具／角色加入前的硬阻擋；
   上方藍圖列出了每個缺口何時補上。
5. 執行後，將結果附加到 `~/.phantom-mesh/scenarios.log`，
   讓未來的你（與未來的 phantom autoevolve）能從中學習。

本文件是 phantom 的 CLI 介面與 95 個場景腦力激盪之間的契約。
隨兩者演進，請保持兩邊同步。
