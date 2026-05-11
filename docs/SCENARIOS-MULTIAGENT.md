# Scenarios → phantom multi-agent commands

> **Top 10 that work TODAY on phantom 0.4.0** are at the top of this
> file (next section). The full 95-scenario brainstorm follows them as
> a forward-looking roadmap — those need additional agent configs,
> event hooks, or domain-specific tools that aren't shipped yet.

---

## 🎯 Top 10 — verified working on phantom 0.4.0 (2026-05-11)

Each scenario is one paste-able command. All exit cleanly on a fresh
install with no further setup (assuming you've run `phantom onboarding`
and have at least one provider key in env).

### T1. Morning standup — what shipped overnight
```bash
phantom autoevolve digest --since-hours 24
```
**Pattern**: D (scheduled / event-driven, you read the result).
phantom autoevolve runs hourly via launchd; this reads the
last 24 h of commits + queued tasks + failures.
**Real output**: counts by status (green/fixed/failed), commit shas,
queue depth. ~50 lines.

### T2. 6-project dashboard, accessible from any device
```bash
phantom serve &
open http://127.0.0.1:7878/projects
```
**Pattern**: A (single node hosts; every device on Tailscale connects).
Recruiter / phone / iPad all hit one URL → see 6 tiles + cluster
status + live activity feed. Tap [Run Demo] → SSE-streamed output.

### T3. Read-only investigation across the codebase
```bash
phantom run --agent researcher "Find all callers of phantom_mesh::permission::Engine::evaluate. Cite files."
```
**Pattern**: A (one read-only sub-agent, no shared history).
Uses content_search + file_read tools. Returns markdown bullets.
~30 s wall-clock.

### T4. Multi-step coding task (refactor / fix bug)
```bash
phantom run --agent coder "Add a unit test for permission::wildcard_match covering CJK whitespace edge case."
```
**Pattern**: A (single agent with file_edit + cargo_check tools).
Writes the test, runs cargo, reports green or failures. ~1-2 min
wall-clock on first run (cargo compile).

### T5. Code review on the current diff
```bash
phantom run --agent reviewer "Review HEAD. Flag security issues, panic risks, dead code."
```
**Pattern**: A. Reviewer is read-only by design — won't modify code.
Returns a structured markdown review with line:column references.

### T6. Parallel research fan-out (single machine)
```bash
phantom run --agent master 'Run parallel_tasks for: [{agent:"researcher", prompt:"What does tokio CancellationToken actually do under the hood?"}, {agent:"coder", prompt:"Show a 10-line example of CancellationToken in select! with timeout"}]'
```
**Pattern**: B (single-machine fan-out via `parallel_tasks` tool).
Both subagents run concurrently; results joined into one response.

### T7. Cross-machine dispatch (Tailscale cluster)
```bash
phantom run --node yoyogood --agent coder "Build the project here and report the binary size"
```
**Pattern**: C (single peer dispatch via `subagent({node})`).
Picks `yoyogood` from `agents.toml` `[cluster] peers`, ships the
prompt via HMAC-auth'd /rpc/message, returns the peer's output
with `[subagent: coder@yoyogood · remote · 4.2s]` header.

### T8. Permission policy enforcement
```bash
# Add to ~/.phantom-mesh/agents.toml:
#   [permissions]
#   deny  = ["Read(./.env)", "Bash(rm -rf *)"]
#   ask   = ["Bash"]
#   allow = ["Bash(git status)", "Bash(cargo check)"]
phantom doctor              # verify rules parsed
phantom run --agent coder "What's in ./.env?"     # → denied
```
**Pattern**: A with permission engine in front.
Recruiter sees the deny chain trip in real time. `phantom doctor`
shows "4 rules parsed (2 deny, 1 ask, 1 allow); statically denied:
web_fetch (will be hidden from LLM tool list)".

### T9. Background self-improvement loop
```bash
echo "Add a docstring to permission::wildcard_match explaining the glob semantics" \
  >> ~/.phantom-mesh/autoevolve.queue.txt
phantom autoevolve schedule status     # confirm hourly cadence
# (then walk away — checks back in via T1 tomorrow)
```
**Pattern**: D. Autoevolve picks up queued tasks when cargo is green,
dispatches an evolve agent to do them, commits + pushes on success.

### T10. MCP-bridged use from Claude Code
```bash
claude mcp add phantom $(which phantom) mcp
# Then in a Claude Code session in any repo:
#   "Use mcp__phantom__subagent to dispatch this PR review to the
#    `reviewer` agent."
```
**Pattern**: A from Claude Code's perspective; phantom's 50+ tools
appear as `mcp__phantom__*` alongside Claude Code's built-ins.

---

## 📌 Verified by

```bash
phantom selftest --feature mcp           # T10 path
phantom selftest --feature projects-dashboard  # T2 path
phantom selftest --feature cluster-rpc   # T7 path
phantom selftest --feature permission-dsl  # T8 path
phantom selftest --feature autoevolve-queue  # T9 path
phantom selftest --feature digest        # T1 path
./scripts/test-mcp-tools.sh              # T3-T6 paths
```

All 18 selftest features + 38 shell-test checks gate every push via CI
(`.github/workflows/ci-shell-tests.yml`).

---

## 🔭 Forward-looking 95-scenario brainstorm

Everything below this line is **future work + design space** — the
broader vision the 10 above are seed-cases of. None are blocked by
phantom 0.4.0 fundamentals; they need:
- 8 additional agent roles (fetcher, synthesizer, standup, triage,
  digestor, reporter, coach, local) — straightforward configs
- Event hooks for "fires when X happens" (the [TODO: events]
  markers below) — phantom-mesh roadmap item

Read this as a roadmap, not a feature list.

---

> Companion to `_planning-audit/archived/misc-strategy/USE-SCENARIOS.md`.
> Every scenario in that brainstorm gets a deterministic phantom
> command-line and a multi-agent topology. Treat this file as the
> per-scenario implementation spec.

The four execution patterns referenced below:

| Pattern | When | phantom command shape |
|---|---|---|
| **A** Single agent | Q&A / summarization, no parallelism needed | `mcp__phantom__subagent({agent: "X", prompt: "..."})` |
| **B** Single-machine parallel | Multiple independent subtasks on the same node | `mcp__phantom__parallel_tasks([{agent, prompt}, ...])` |
| **C** Cross-mesh distributed | Different hardware / privacy zones / locations | `mcp__phantom__subagent({node: "host:port", agent, prompt})` plus `parallel_tasks` for fan-out |
| **D** Scheduled / event-driven | Recurring or triggered by external event | `phantom autoevolve schedule install --interval N --agent X` (one-shot recurring); event hooks marked `[TODO: events]` are not yet shipped |

A scenario can use multiple patterns — e.g. S3 (overnight refactor) is
**D + C**: scheduled at midnight, fans out to GPU boxes.

Agent role names referenced below:

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

Of the 12 roles above, 4 (`master/coder/reviewer/researcher`) ship
in `configs/agents.*.toml` today; the other 8 need to be added to
`agents.toml` as part of Day 2 implementation.

---

## A. Software Engineer — Work (S1–S15)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| S1 | 當前 repo 小 bug fix | A | `subagent({agent:"coder", prompt:"<bug>"})` | — (Claude Code already covers; phantom shouldn't) |
| **S2** | 跨 3–5 repo 改動 | **C** | `parallel_tasks([{node:"laptop", agent:"coder", prompt:"repo A: …"}, {node:"yoyogood", agent:"coder", prompt:"repo B: …"}, …])` | repo-discovery + per-node git_clone bootstrap |
| **S3** | Overnight 重構（500+ 檔） | **D + C** | `autoevolve schedule install --interval 3600 --target test --agent coder --distributed` | already shipped |
| S4 | PR review 1000+ 行 | **B** | `parallel_tasks([{agent:"reviewer", prompt:"chunk 1: lines 1-200"}, …, {agent:"synthesizer", prompt:"merge findings"}])` | gh PR fetch tool (or use existing `git_diff`) |
| S5 | 3am on-call debug from phone | C + D | mobile UI → `subagent({node:"home-daemon", agent:"master", prompt:"<incident>"})` | telegram_listen + telegram_send tools |
| S6 | CI 失敗 → agent 分析 | **D** | `[event: github_workflow_run] → subagent({agent:"reviewer", prompt:"analyze failed log"})` | event hook framework |
| S7 | 探索陌生 codebase | A + memory | `subagent({agent:"master", prompt:"explore"})` + `memory_store("codebase:<repo>:<area>", findings)` | — |
| S8 | DB / framework migration | D + approval gate | `autoevolve --watch --no-commit --target test` + manual `/perm` gate | manual approval UI gate (already partial: `/perm diff`) |
| **S9** | 每日 standup 整理 | **D** | `autoevolve schedule install --interval 86400` running `subagent({agent:"standup", prompt:"git log --since=yesterday"})` | `standup` agent role + telegram_send for delivery |
| S10 | 工作交接 | A + memory export | `subagent({agent:"master", prompt:"summarize project state"})` + `memory_list \| jq export` | memory_export to markdown |
| S11 | 依賴升版 / CVE | **D + B** | `autoevolve schedule install --interval 86400` running `parallel_tasks([{agent:"researcher", prompt:"check <pkg>"}, …])` | npm/cargo audit tool wrapper |
| S12 | 壓力測試 / 效能分析 | C | `subagent({node:"perf-box", agent:"coder", prompt:"run k6 / criterion / iperf"})` | bench harness scripts |
| S13 | 寫 RFC / 設計文件 | A + memory | `subagent({agent:"master", ctx: memory_recall + git_diff})` | — |
| S14 | Regex / shell one-liner | A | `subagent({agent:"master"})` | — (don't differentiate from ChatGPT) |
| **S15** | 多分支並行實驗 | **B + worktree** | `parallel_tasks([{agent:"coder", prompt:"impl approach A in worktree-A"}, {agent:"coder", prompt:"impl approach B in worktree-B"}, {agent:"reviewer", prompt:"compare both"}])` | worktree spawn helper (`phantom worktree create`); already partial via Claude Code Agent isolation |

---

## B. Data Scientist — Work (D1–D15)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| D1 | Jupyter EDA | A | (Copilot covers) | — |
| D2 | 資料清理 pipeline | C | `subagent({node:"data-box", agent:"coder", prompt:"clean pipeline"})` | data lives near the worker — node label |
| **D3** | 長時訓練（GPU box）| **C + D** | `subagent({node:"gpu-box", agent:"coder", prompt:"train"})` then `autoevolve schedule install --interval 60 --target test --agent reporter` watching nvidia-smi | gpu_status tool + telegram_send |
| **D4** | Hyperparam sweep | **B + C** | `parallel_tasks([{node:"gpu1", agent:"coder", prompt:"lr=1e-3"}, {node:"gpu2", agent:"coder", prompt:"lr=3e-4"}, …])` | shared experiment store (mlflow / wandb / fs) |
| D5 | Paper 復現 | C + sandbox | `subagent({node:"sandbox", agent:"coder", prompt:"reproduce <paper>"})` | env-isolation tool (docker/conda) |
| D6 | 模型結果變差 debug | A + memory | `subagent({agent:"researcher", prompt:"compare runs"})` + `memory_recall("experiments")` | mlflow/wandb fetch tool |
| D7 | 百 GB 資料 local-only | C | `subagent({node:"data-box", agent:"local"})` (provider=mlx-local; never leaves machine) | — (already covered: MLX provider) |
| D8 | Stakeholder report | **B** | `parallel_tasks([{agent:"researcher", prompt:"data section"}, {agent:"researcher", prompt:"figures"}, {agent:"synthesizer", prompt:"merge"}])` | plot_render tool (matplotlib/altair) |
| D9 | SQL 大查詢 (30 min+) | **B + D** | `bash_run_background({command:"psql ..."})` + `autoevolve schedule install --interval 60` polling result | sql_run tool (typed result) |
| D10 | 論文閱讀 | **D + B** | `autoevolve schedule install --interval 86400` running `parallel_tasks([{agent:"researcher", prompt:"<arxiv id>"}, ...])` + `memory_store` | rss/arxiv fetch + pdf_read tools |
| D11 | Slack 問指標 | C | (mobile UI) → `subagent({node:"data-box", agent:"coder", prompt:"<sql>"})` | sql_run tool + slack listen |
| D12 | 標註 pipeline | **D** | `[event: new_data] → parallel_tasks([{agent:"local", prompt:"label batch 1"}, …])` | event hook + on-device label classifier |
| D13 | Feature store 維護 | **D + B** | `autoevolve schedule install --interval 86400` running parallel_tasks of backfills | feature_backfill tool |
| **D14** | Drift / schema alert | **D** | `[event: drift_alarm] → subagent({agent:"triage", prompt:"explain drift; severity?"})` | drift detector + slack notif |
| D15 | Reproducibility check | A + memory | `subagent({agent:"coder", prompt:"reproduce <run-id>"})` + `memory_recall("env-fingerprint")` | env fingerprint tool |

---

## C. Firmware Engineer — Work (F1–F15)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| F1 | Datasheet → register code | **B** | `parallel_tasks([{agent:"researcher", prompt:"read pdf <chip-X>"}, {agent:"coder", prompt:"emit register init"}])` + `memory_store("vendor:<chip>", traps)` | pdf_read + table_extract tools |
| **F2** | build → flash → UART loop | **D** | `[event: file_changed *.c] → parallel_tasks([{agent:"coder", prompt:"build"}, {agent:"coder", prompt:"flash"}, {agent:"researcher", prompt:"watch UART for boot string"}])` | fswatch tool + serial_read tool |
| **F3** | 板子 bring-up | **C** | mobile UI → `subagent({node:"bench-pc", agent:"coder", prompt:"<step>"})` | bench-pc as a node label |
| F4 | Logic analyzer 大檔 | **C** | `subagent({node:"bench-pc", agent:"local", prompt:"parse capture"})` | sigrok / saleae export tool |
| F5 | Bootloader / RTOS debug | **C** | `subagent({node:"bench-pc", agent:"coder", prompt:"…"})` | gdb_run / openocd tool |
| **F6** | Cross-compile multi target | **B + C** | `parallel_tasks([{node:"linux1", agent:"coder", prompt:"target arm"}, {node:"linux2", agent:"coder", prompt:"target riscv"}, {node:"win1", agent:"coder", prompt:"target win"}])` | per-target toolchain on each node |
| F7 | OTA 上百台 device | **C** | `parallel_tasks([{node:f"dev-{i}", agent:"coder", prompt:"flash + verify"} for i in range(100)])` | device discovery + bulk flash |
| **F8** | Power 分析 | **C + D** | `subagent({node:"bench-pc", agent:"coder", prompt:"sample power"})` + mobile remote watch | scope_read tool |
| F9 | NDA code | C local-only | `subagent({node:"local", agent:"local", prompt:"…"})` provider=mlx-local | — (MLX provider already lands this) |
| F10 | 出差現場 debug | C | mobile UI → `subagent({node:"home-mesh", agent:"master"})` | mobile UI is shipped |
| F11 | HW pytest | **C** | `subagent({node:"hw-rig", agent:"coder", prompt:"pytest -k boot"})` | hw rig as a labeled node |
| **F12** | Soak test 72hr | **D** | `bash_run_background` + `autoevolve schedule --interval 600` polling for anomaly + telegram alert | telegram_send + anomaly thresholding |
| F13 | FW + HW log correlate | **B** | `parallel_tasks([{agent:"researcher", prompt:"FW log"}, {agent:"researcher", prompt:"HW log"}, {agent:"synthesizer", prompt:"timeline"}])` | log timestamp normalizer |
| F14 | FW regression bisect | **D + B** | `autoevolve --max-rounds 20 --target test` per bisect step (+ flash between) | git bisect orchestration |
| F15 | Vendor SDK memory | A + memory | `memory_recall("vendor:<sdk>:traps")` injected into agent ctx | scoped memory by vendor |

---

## D. Work — cross-persona common (X1–X10)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| X1 | 長跑斷線續跑 | D | checkpoint via `bash_run_background` + resume via job_id polling | already shipped |
| X2 | 手機監看 / 下指令 | C | mobile UI → `subagent({node, ...})` | shipped |
| X3 | 多機協作 | C | `parallel_tasks` with `node:` per task | shipped |
| X4 | 會話跨裝置 | A + persistence | `[session.*]` + `~/.phantom-mesh/conversations/` is already shared via iCloud / cluster | iCloud sync wire-up (planned) |
| X5 | 每日 / 每週排程 | D | `autoevolve schedule install --interval N` | shipped |
| X6 | 敏感資料 local-only | C | route to `agent.local` (mlx-local provider) | shipped |
| X7 | 多 agent 並行實驗 | B | `parallel_tasks` + worktree | worktree helper (S15 same gap) |
| X8 | 跨 session 持久記憶 | A + memory | `memory_*` tools | scope-by-context (per-repo, per-goal) |
| **X9** | 事件觸發（git/ci/slack/email/webhook） | **D** | `[TODO: event bus]` | event bus + per-trigger registration |
| X10 | 成本路由 | A | provider rotation already in agents.toml | shipped |

---

## E. SWE — Life (sP1–sP12)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| **sP1** | self-host 半夜炸 | **D** | `[event: heartbeat_fail] → subagent({agent:"triage"}) → telegram_send` | event bus + telegram_send |
| sP2 | Home Assistant 自動化 | A | `subagent({agent:"coder", prompt:"YAML for: ..."})` | HA API tool |
| sP3 | 家人技術支援 | C | mobile UI → `subagent({node:"family-pc", agent:"master"})` | reverse-tunnel for non-tailnet family pc |
| sP4 | OSS side project | **D + C** | `autoevolve schedule install --interval 21600 --agent coder --distributed` | shipped |
| **sP5** | 私帳本 / 投資追蹤 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"fetch bank A export"}, …])` | bank csv parser; never leaves local |
| **sP6** | RSS / HN digest | **D + B** | `autoevolve schedule install --interval 86400` running `parallel_tasks([{agent:"digestor", prompt:"feed url"}, ...])` + `telegram_send` | rss_fetch + telegram_send + `digestor` role |
| sP7 | 小孩 STEM 陪學 | A + safety | `subagent({agent:"coach"})` + content filter | content filter; `coach` role |
| **sP8** | 旅行規劃 | **B** | `parallel_tasks([{agent:"researcher", prompt:"flights"}, {agent:"researcher", prompt:"hotels"}, {agent:"researcher", prompt:"things to do"}, {agent:"synthesizer", prompt:"itinerary"}])` | browser_agent for booking sites |
| sP9 | 部落格 / 文寫作 | A + memory | `subagent({agent:"master", prompt:"draft from notes"})` + `memory_recall` | — |
| sP10 | 學新框架 | A + memory | `subagent({agent:"coach", prompt:"next chapter; my level=..."})` | progress tracker per-topic |
| sP11 | 照片歸檔 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"caption batch i"} for i in range(N)])` | image_caption tool (Vision/Multimodal) |
| sP12 | 備份演練 | D | `autoevolve schedule install --interval 604800` running `subagent({agent:"coder", prompt:"verify backups"})` | restic / borg tool wrapper |

---

## F. DS — Life (dP1–dP12)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| **dP1** | Apple Health/Strava 分析 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"export+parse"}, {agent:"local", prompt:"plot trends"}])` | health_export tool (HK XML, Strava OAuth) |
| dP2 | 個人理財回測 | A local-only | `subagent({agent:"local", prompt:"backtest"})` | csv loaders |
| dP3 | 睡眠 / 健身週報 | D | `autoevolve schedule --interval 604800 --agent reporter` | reporter role + plotter |
| dP4 | Kaggle 家用 GPU | C | `subagent({node:"home-gpu", agent:"coder", prompt:"submission iter"})` | shipped |
| dP5 | arxiv 追新 paper | **D + B** | `autoevolve schedule --interval 86400` running parallel_tasks per category + memory | rss/arxiv fetcher + pdf_read |
| dP6 | 部落格 / 翻譯論文 | **B** | `parallel_tasks([{agent:"researcher"}, {agent:"synthesizer"}])` | pdf_read |
| dP7 | 家人健康 | C local-only | `subagent({node:"local", agent:"local"})` | privacy partition (already implicit) |
| dP8 | 育兒 data | A local-only | `subagent({agent:"local"})` | app-export tools |
| dP9 | 房 / 車市場研究 | **B + browser** | `parallel_tasks([{agent:"fetcher"} per listing])` + `synthesizer` | browser_agent |
| dP10 | 小孩作業 | A + memory | `subagent({agent:"coach"})` + per-child memory | child profile memory scope |
| dP11 | 社群經營 stats | **D** | `autoevolve schedule --interval 86400 --agent reporter` | platform fetchers |
| dP12 | 個人筆記 RAG | A | `memory_search` already shipped, hooked into agent ctx | scope by source (notion / obsidian) |

---

## G. FW — Life (fP1–fP12)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| fP1 | 3D printer / slicer | A + memory | `subagent({agent:"researcher"})` + `memory_recall("vendor:<printer>")` | — |
| fP2 | Home lab config | A | `subagent({agent:"coder", prompt:"audit configs"})` | — |
| fP3 | ESP32 玩具 | A + memory | `subagent({agent:"coder"})` + memory | — |
| fP4 | Retro computing | A | `subagent({agent:"researcher", prompt:"forum scrape"})` | browser_agent |
| fP5 | OBD / ECU | A local-only | `subagent({node:"car-pi", agent:"local"})` | obd-ii tool |
| fP6 | Ham / SDR | A | `subagent({agent:"coach"})` | — |
| fP7 | 智慧家電 firmware | **D** | `[event: file_changed] → build+flash` (= F2 pattern at home) | same gaps as F2 |
| fP8 | 電子排障 | **C** | `subagent({node:"bench-pc"})` | scope_read |
| fP9 | 孩子電子實驗 | A | `subagent({agent:"coach"})` | — |
| fP10 | 家中裝置 firmware 追 | **D** | `autoevolve schedule --interval 86400 --agent researcher` | device firmware version checker |
| fP11 | 二手零件 比價 | **B** | `parallel_tasks([{agent:"fetcher", prompt:"site X"}, ...])` | browser_agent |
| fP12 | 修壞掉的 3C | A | `subagent({agent:"researcher"})` + memory + pdf_read | pdf_read |

---

## H. Pure life common (XP1–XP12)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| **XP1** | Email triage | **D** | `[event: imap_new] → subagent({agent:"triage", prompt:"categorize + draft reply"})` | imap fetch + draft compose |
| XP2 | 家庭行事曆協調 | A | `subagent({agent:"researcher", prompt:"merge calendars"})` | calendar tool |
| XP3 | 食譜 / 購物清單 | A | `subagent({agent:"researcher"})` | — |
| XP4 | 健身 / 飲食日記 | D | `autoevolve schedule --interval 86400 --agent reporter` | health_export |
| XP5 | 閱讀 / podcast 摘要 | **B + D** | `autoevolve schedule + parallel_tasks([{agent:"digestor"}])` | podcast_transcribe + pdf_read |
| XP6 | 語言學習 | A | `subagent({agent:"coach"})` | — |
| XP7 | 寫日記 | A local-only | `subagent({agent:"local"})` | journal store; absolute privacy |
| XP8 | 紀念日提醒 | D | `autoevolve schedule --interval 86400 --agent triage` | calendar tool |
| XP9 | 旅行戶外 | **B** | same as sP8 | browser_agent |
| XP10 | 報稅 / 政府公文 | A + pdf | `subagent({agent:"master"})` + pdf_read | pdf_read |
| XP11 | 看醫生 前後 | A local-only | `subagent({agent:"local"})` | journal store |
| XP12 | 智慧家電語音 | A | `subagent({agent:"master"})` + HA bridge | HA tool + voice input |

---

## I. Goal-driven scenarios — Career change (J1–J8)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| J1 | 履歷依 JD 客製 | **B** | `parallel_tasks([{agent:"researcher", prompt:"JD"}, {agent:"reviewer", prompt:"current resume"}, {agent:"synthesizer", prompt:"diff"}])` | resume parser |
| J2 | 投遞追蹤 | A + memory | `memory_*` schema with status enum | per-application memory scope |
| J3 | 自動掃職缺 | **D + B** | `autoevolve schedule --interval 86400` + `parallel_tasks([{agent:"fetcher"} per board])` | linkedin/104/cake browser_agents |
| J4 | Cover letter + research | **B** | `parallel_tasks([{agent:"researcher"}, {agent:"reviewer", prompt:"draft"}])` | — |
| J5 | 推薦人協調 | A | `subagent({agent:"master"})` | calendar tool |
| J6 | 薪資協商 | A + memory | `subagent({agent:"researcher", prompt:"market data"})` + memory | levels.fyi / glassdoor scrape |
| J7 | Offer 比較矩陣 | A | `subagent({agent:"reviewer"})` | — |
| J8 | 面試行程 | D | `autoevolve schedule --interval 3600 --agent triage` (calendar) | calendar tool |

## I.b — Interview prep (I1–I7)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| I1 | 題庫排程 | D | `autoevolve schedule --interval 86400 --agent coach` (spaced repetition) | sr-card store |
| I2 | System design | A | `subagent({agent:"coach"})` | — |
| I3 | Behavioral STAR | A + memory | `subagent({agent:"coach"})` + per-story memory | — |
| I4 | Mock interview | A | `subagent({agent:"master", prompt:"act as interviewer"})` | streaming voice in/out |
| I5 | 錄影語音分析 | C | `subagent({node:"local", agent:"local"})` | whisper local tool |
| I6 | 弱項診斷 | A + memory | `memory_recall("interview:misses")` | — |
| I7 | 公司題型研究 | **B** | `parallel_tasks` over leetcode tags + glassdoor | glassdoor scrape |

## I.c — Certification (C1–C5)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| C1 | 讀書計畫 | A + memory | `subagent({agent:"coach"})` | calendar |
| C2 | 題庫 SR | D | `autoevolve schedule --interval 86400 --agent coach` | sr-card store |
| C3 | 錯題 cluster | A | `subagent({agent:"reviewer"})` | — |
| C4 | 模擬考 | A | `subagent({agent:"coach"})` | — |
| C5 | 報名提醒 | D | `autoevolve schedule + calendar` | calendar |

---

## J. Persistent goals — Health (H1–H7)

| ID | Scenario | Pattern | Command | Missing |
|---|---|---|---|---|
| H1 | 飲食拍照 → 熱量 | A + multimodal local | `subagent({agent:"local", input:"@image"})` | image_caption + nutrition db |
| H2 | 運動週達成率 | **D** | `autoevolve schedule --interval 604800 --agent reporter` | health_export |
| H3 | 體重趨勢 + 停滯期 | D + memory | `autoevolve schedule + memory_recall` | health_export + plotter |
| H4 | 睡眠 / 步數 / 心率 corr. | **B** | `parallel_tasks([{agent:"researcher", prompt:"hr"}, {agent:"researcher", prompt:"sleep"}, {agent:"synthesizer"}])` | health_export + correlation tool |
| H5 | 餐食計畫 | A | `subagent({agent:"researcher"})` | — |
| H6 | 卡關建議 | A + memory | `subagent({agent:"coach"})` | — |
| H7 | 教練 push | D + telegram | `autoevolve schedule --interval 86400 --agent coach` + `telegram_send` | telegram_send |

## J.b — Money (M1–M6)

| ID | Pattern | Command | Missing |
|---|---|---|---|
| M1 | **B local-only** | `parallel_tasks([{agent:"local", prompt:"bank A"}, …])` | bank export tools |
| M2 | D | `autoevolve schedule --interval 86400 --agent reporter` | budget rules |
| M3 | A | `subagent({agent:"reviewer", prompt:"audit subscriptions"})` | bank parser |
| M4 | D | `autoevolve schedule + memory` | — |
| M5 | A + memory | `subagent({agent:"coach"})` + memory_recall | — |
| M6 | A | `subagent({agent:"researcher"})` | — |

## J.c — Investments (V1–V6)

| ID | Pattern | Command | Missing |
|---|---|---|---|
| V1 | A | `subagent({agent:"reporter"})` | broker API |
| V2 | D | `autoevolve schedule --interval 86400` | — |
| V3 | D | `autoevolve schedule --interval 86400` | — |
| V4 | **B** | `parallel_tasks([{agent:"researcher"} per ticker])` | rss/news fetch |
| V5 | A + memory | `memory_*` | — |
| V6 | D | `autoevolve schedule --interval 604800 --agent reporter` | — |

## J.d — Side hustle (B1–B7)

| ID | Pattern | Command | Missing |
|---|---|---|---|
| B1 | A | `subagent({agent:"researcher"})` | — |
| B2 | (uses S/D/F patterns from work) | — | — |
| B3 | D | `autoevolve schedule + scheduler tool` | social-post tool |
| B4 | A + memory | `subagent({agent:"triage"})` | imap |
| B5 | D | `autoevolve schedule --interval 604800 --agent reporter` | — |
| B6 | **D + B** | `autoevolve schedule + parallel_tasks` | browser_agent |
| B7 | D | `autoevolve schedule --interval 604800 --agent reviewer` | — |

---

## K. Long-horizon goals (LS / LD / LF / LX, 22 entries)

These are all "agent maintains a goal over months/years" — same
implementation shape, differ only in domain:

```
phantom autoevolve schedule install --interval 2592000 \
    --agent reporter \
    --target test  # or a custom 'review-goal' target
```

with a `[goal.<name>]` block in `~/.phantom-mesh/goals.toml`:

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

That goal data model is the **biggest missing piece** to land the L4/L5
horizons. ~30 of the 95 scenarios depend on it.

---

## Coverage summary

| Pattern | Scenarios it serves | % of 95 |
|---|---|---|
| **A** Single-agent (chat) | 18 | 19% |
| **B** Single-machine parallel | 28 | 29% |
| **C** Cross-mesh distributed | 17 | 18% |
| **D** Scheduled / event-driven | 32 | 34% |

(Many scenarios use 2 patterns — totals add over 100%.)

## Top 12 missing pieces (by scenario impact)

| # | Missing | Scenarios served | Effort |
|---|---|---|---|
| 1 | **Telegram send tool** | sP1, sP6, S5, F12, D14, H7, M2, V3, … (~25) | S |
| 2 | **Event bus** (git/ci/slack/imap/webhook hooks) | S6, X9, sP1, F2/fP7, D12, D14, XP1, XP8, … (~20) | L |
| 3 | **Goal data model** + `[goal.*]` toml + check-in cadence | all L4/L5 (~30) | L |
| 4 | **PDF reader / table extract** | F1, fP12, dP5, dP6, XP10, … (~10) | M |
| 5 | **Browser agent** (playwright) | sP8, dP9, J3, fP4, fP11, B6, … (~10) | L |
| 6 | **Calendar tool** | XP2, XP8, J5, J8, I8, C5, M2 (~7) | M |
| 7 | **Image caption / OCR (multimodal)** | sP11, H1, F4, fP12 (~6) | L |
| 8 | **SQL run tool** | D9, D11, dP2, F11 (~5) | M |
| 9 | **RSS / arxiv fetch** | sP6, dP5, V4 (~5) | S |
| 10 | **Health export tool** (HK XML, Strava) | dP1, dP3, H1–H4 (~6) | M |
| 11 | **Bank CSV parser** (privacy-locked) | sP5, dP2, M1, M3 (~4) | M |
| 12 | **Voice input/output** (whisper local + tts) | I4, I5, XP6, XP12, sP3 (~5) | L |

## Top 8 missing agent roles

`agents.toml` today has master / coder / reviewer / researcher. We need:

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

Add these to `configs/agents.coordinator.toml` as Day-2 work.

---

## Implementation roadmap

**Day 1 — this file** (now). Mapping is the spec.

**Day 2 (~6 hr)** — _Telegram + 8 agent roles + cron-style scheduler_.
Lands ~25 D-pattern scenarios (notifications and dailies).

**Day 3 (~6 hr)** — _SQL run + RSS + Calendar + bank-CSV (local-only)_.
Lands ~15 more from D / B / J.

**Day 4 (~6 hr)** — _PDF + multimodal image_caption + health_export_.
Lands the F-firmware research scenarios + H1–H4 + dP1.

**Day 5 (~8 hr)** — _Event bus_ (git/ci/slack/imap webhook receivers).
Unlocks S6, X9, F2/fP7, sP1, XP1 — all the "auto-react" cluster.

**Day 6 (~6 hr)** — _Browser agent_ (playwright) + glassdoor / linkedin
fetchers. Lands sP8, dP9, J3, B6, fP4/fP11.

**Day 7 (~8 hr)** — _Goal data model_ + `[goal.<name>]` toml + check-in
LaunchAgent (one per goal). Lands all L4/L5 (~30 scenarios).

**Day 8+** — Voice in/out + per-platform polish + dogfood the top 5
real scenarios end-to-end.

= ~46 working hours / 7 calendar days full-time = full 95-scenario
coverage with both single-machine and cross-mesh multi-agent paths.

---

## How to use this file

1. **Pick a scenario** by ID (S2, dP1, …)
2. Read its **Pattern** column — that's the multi-agent topology
3. Run / customize its **Command** column — that's the phantom one-liner
4. If **Missing** column says something, that's a hard block until the
   tool/role is added; the roadmap above lists when each gap closes.
5. After running, append the outcome to `~/.phantom-mesh/scenarios.log`
   so future-you (and future-phantom autoevolve) can learn from it.

This document is the contract between phantom's CLI surface and the
95-scenario brainstorm. Keep them in sync as both evolve.
