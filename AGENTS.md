# Phantom Mesh — Agent 指南

> **本檔是跨工具 SSOT（single source of truth，唯一真實來源）**，供任何在本 repo 工作的 AI 編碼工具
> （Claude Code、Codex、Antigravity、Gemini CLI……）使用。本檔即以繁體中文撰寫，為正本。

## 1. Source of Truth（真實來源）

當你接手本 repo 時，依下列順序閱讀：

1. [`docs/superpowers/BIG-GOAL.md`](docs/superpowers/BIG-GOAL.md) — **唯一的產品錨點 /
   apex（FINAL re-lock 2026-06-11）**。它取代 `docs/_archive/NORTH-STAR.md`（現已加上
   SUPERSEDED 橫幅標記）以及所有更舊的框架定位。衝突裁決順序定義於
   [`docs/OPERATING-STANDARD.md`](docs/OPERATING-STANDARD.md) §4 與
   [`docs/superpowers/DOCUMENTATION-CHARTER.md`](docs/superpowers/DOCUMENTATION-CHARTER.md)
   （apex(BIG-GOAL) > OPERATING-STANDARD > SPEC-00-INDEX > SPEC leaf > epic > feature > PR；as-built 程式碼
   勝過過時的 spec，之後再以 DRIFT 標記回填該 spec）。
2. [`SESSION_RESUME.md`](SESSION_RESUME.md) — 戰術狀態：哪些工作進行中、哪些被卡住，以及
   下一個具體步驟。
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — 高層架構（pivot 前；當它與 BIG-GOAL 及
   深層 spec 樹分歧時，以後者為準）。
4. [`PHANTOM.md`](PHANTOM.md) — 快速架構草圖。
5. [`docs/_archive/MASTER-SPEC.md`](docs/_archive/MASTER-SPEC.md) — 綜整後的快照（從屬於 BIG-GOAL；
   帶有已知的過時計數——見其橫幅）。
6. `AGENTS.md`（本檔）— 跨工具規則。

> ⚠️ 在 2026-06-12 之前，本節曾把 NORTH-STAR 指為錨點並聲稱它取代了
> BIG-GOAL——那是**搞反了**（INV-1 違規），並悄悄地把每一次 AI session 都引導到
> 錯誤的 apex。若還有任何其他文件聲稱 NORTH-STAR 是現行版本，那是錯的；請回報。

## 2. Repo 邊界

| Path | 職責 |
|---|---|
| `core/` | Rust runtime：providers、tools、MCP、mesh、serve、agent loop、REPL |
| `crates/pm-types/` | 在 `core/` 與 Tauri 之間共享的型別 |
| `app/src-tauri/` | 桌面 + 行動端 Tauri 殼層、OS 整合、sidecar |
| `app/src/` | TypeScript + React + Tailwind web 前端 |
| `configs/` | 各裝置的 agent 設定範本 |
| `scripts/` | 公開工具 |

**不要**在 legacy 或 vendored 的鷹架下新增產品功能（例如 `app/src/pages/legacy/`、
`apple-oauth-relay/`、`src/clawtex/` — 已封存；在乾淨的樹中並不存在）。行動端目標只使用
`app/src-tauri/`。

## 3. 架構原則

1. **Contract first（契約優先）** — 在撰寫實作前，先於 `crates/pm-types/` 定義型別。
2. **Swappable capabilities（可替換能力）** — providers、tools、channels 全部都可透過 registry 替換。
3. **Thin runtime spine（精簡的 runtime 主幹）** — `core/` 掌管編排；CLI、web 與 Tauri 共用契約。
4. **Surface-neutral behavior（表層中立的行為）** — 任何東西都不得只在 browser fallback 模式下才能運作。
5. **Subagent-first UX** — Claude Code 與 Codex 是一等公民消費者。

## 3.5 多 AI 委派（standing rule — operator directive 2026-06-12）

**實質性的工作必須 fan out 到本機的 AI 三人組（codex / opencode / agy），而不是在正在閱讀本檔
的那個 assistant 裡單獨執行。** 每一個都有獨立的配額池；單獨執行會浪費整個 fleet 並燒掉
共享的配額池。

- 透過 `.claude/skills/local-ai/ask.sh <tool> "<prompt>"` 呼叫——它是每個工具各種 quirks 的
  SSOT（codex 的 bypass flags、opencode 的 timeout-kill、針對 upstream #76 的 agy ConPTY 變通
  與 auth-race 診斷）。請先閱讀其 `SKILL.md` 的 "Delegation lessons"。
- 分工（在本機已驗證 2026-06-12）：**codex** = 限定範圍的 per-file
  機械式編輯 / codegen（每次呼叫 ONE file；commit 前先 `git diff` + repo lint）；
  **opencode** = repo 檔案閱讀 + 綜整（輸入必須位於 repo 內）；
  **agy** = Q&A / 第二意見（純問題——它可以執行 tools；用 `--sandbox` 來拒絕）；
  **claude** = 編排、對抗式驗證、最終判斷。
- **落地前要 double-gate** 非 trivial 的變更：≥2 個*不同*的 AI 審查並 LGTM
  （DEV-QUALITY-LOOP）。撰寫者 AI 絕不單獨審查自己的 diff。
- 豁免：trivial 的機械式操作（一行修正、改名、狀態檢查、緊急修正）。

## 4. 結束 session 前

1. 更新 `SESSION_RESUME.md`：你做了什麼、什麼被卡住、下一個具體步驟。
2. 只有在策略確實改變時才更新規劃文件。
3. Rust 變更後，在 `core/` 執行 `cargo check`。
4. **除非使用者明確要求，否則不要 commit。**（對於本 owner，推送到非 `main` 分支是
   預先授權的；`main` 仍需明確要求——見 §5。）

## 5. Guardrails（護欄）

- 不要在 repo 根目錄新增頂層的 `*.md` 規劃文件。
- 不要重新引入已封存的 FREEZE / SLICE / SPRINT / TODO 文件。
- 絕不 commit secrets。`agents.toml` 與 `.env*` 已被 gitignore。
- **只用分支。在沒有使用者明確要求下，絕不動 `main`** ——不 commit、不 push、
  不 `reset`、不 merge。
- 在開放任何公開內容之前，絕不繞過 `git filter-repo` 的 secret-cleaning 計畫。
- **不要遮蔽 exit codes。** 絕不透過 pipe 讀取 test/gate 結果（`cmd | grep` 擷取的是
  `grep` 的 exit，而非 `cmd` 的）。改為導向到檔案，並直接檢查 `$?`。

## 6. 並行工作與 worktrees

**絕不在同一個工作目錄中執行兩個 assistant session** ——它們共用 `.git/index`、
`target/`、`node_modules/` 與 lockfiles，會悄悄地互相覆蓋。請使用 worktree：

```bash
git worktree add .worktrees/<topic> -b feat/<topic> <base-branch>
```

命名：worktree 為 `.worktrees/<topic>`，分支為 `feat/<topic>`。在 Windows 上，Defender 可能在
Cargo 期間鎖定檔案——設定 `$env:CARGO_TARGET_DIR='D:/tmp/phantom-windows-target'`。

**Hot files**（先完成 + merge 一個 session，第二個才能碰這些）：`core/src/bin/phantom.rs`、
`core/src/platform/mod.rs`、`core/Cargo.toml`、`app/src-tauri/Cargo.toml`、
`app/src-tauri/tauri.conf.json`、`app/src-tauri/capabilities/*.json`、`Cargo.lock`、
`app/package.json`、`.github/workflows/*.yml`。

**破壞性操作需要明確要求**：`git push --force[-with-lease]`、刪除/改寫已推送的
tag、對已推送的 commits 執行 `git reset --hard`、`git stash drop`/`clear`、對 dirty 的
worktree 執行 `git worktree remove --force`。

## 7. AI 工具派工

用 [`.claude/skills/local-ai/ask.sh`](.claude/skills/local-ai/ask.sh) 把一塊工作派給本機 AI 工具
（單一工具，擷取輸出），並用 **≥2-不同-AI consensus gate**
[`scripts/local-ai/review.sh`](scripts/local-ai/review.sh) `<git-range>` 審查它。若要在另一台
機器上透過 tailnet 執行工作，使用 [`scripts/dev-cluster/run-task.sh`](scripts/dev-cluster/run-task.sh)
（SSH-fire-one-shot；hosts 定義於 [`scripts/dev-cluster/hosts.sh`](scripts/dev-cluster/hosts.sh)）。
工具的 fallback 排序定義在那些 scripts 內——讓每一個 slice 保持在約 200 行輸出與數
分鐘的 wall time 之內。**`agy` 是 session-bound 的**（在 SSH network-logon 下會回傳空白）；請在
本機呼叫它，並關閉 stdin（`</dev/null`）以免 `-p`/`exec` 卡住。

## 8. TDD 與跨工具 dev loop

目前存在的各工具入口點：Claude Code 讀取 `.claude/skills/`；legacy 的 TDD 輔助工具
位於 `scripts/tdd/` 之下（`./scripts/tdd/tdd-status.sh` → `tdd-next.sh` → 寫測試、看到它
變紅 → 最小實作 → 變綠 → `tdd-mark-done.sh`）。這套 TDD 針對 v0.6.0 的 delta，而非 100%
覆蓋率。較重的 **acceleration dev-loop**（spec-gated、governed）是 §10。

## 9. AI 工具自動接線（打開資料夾 → tools 即已連接）

打開此資料夾會讓 AI 工具取得 phantom 工具帶，**除了一次性的 trust 提示外無需任何手動設定**：

- **Claude Code** — [`.mcp.json`](.mcp.json)（已 commit，`command: phantom` 從 `PATH` 解析）
  在你核可一次 workspace trust 後即自動載入 phantom MCP（50+ tools）；project skills 來自
  [`.claude/skills/`](.claude/skills/)；[`.claude/settings.json`](.claude/settings.json)
  註冊一個**唯讀**的 `SessionStart` hook，用於*印出* dev-node / review-gate 提示——它
  從不自動執行任何東西。
- **Codex** — 每台機器註冊同一個 MCP 一次：
  `codex mcp remove phantom 2>/dev/null; codex mcp add phantom -- phantom mcp`
  （`PATH` 上的純 `phantom`，與 `.mcp.json` 相符）。用 `codex mcp get phantom` 驗證——`command`
  必須是 `phantom`，**而非**指向某個 build dir 的絕對路徑（過時的絕對路徑會在 binary 移動時
  悄悄破壞 server）。
- 需要 `phantom` 在 `PATH` 上（`~/.local/bin/phantom`）。這些檔案**接線 MCP/skills**；本
  文件承載的是*規則*。兩者都不會自動加入任何 dev channel——加入 loop 是一個手動的動作。

## 10. Acceleration dev-loop 框架與 governance

跨機器的 dev accelerator（把產品蓋得更快的**手段**——絕非產品本身；大型的部分受 §0.1 把關，
見 [`docs/_archive/EXECUTION-PLAN.md`](docs/_archive/EXECUTION-PLAN.md)）。整條鏈，全部位於
`scripts/` 之下：

- **spec-gate** [`scripts/dev-loop/spec-gate.sh`](scripts/dev-loop/spec-gate.sh) — 一個 task 只能
  從完整的 `[spec]` envelope 執行（capability ∈ sense|learn|nudge|dispatch + component +
  acceptance + 非空的 scope）。
- **review** [`scripts/local-ai/review.sh`](scripts/local-ai/review.sh) — ≥2-不同-AI consensus。
- **deviation-handler** [`scripts/dev-loop/deviation-handler.sh`](scripts/dev-loop/deviation-handler.sh)
  — 強制執行 governance R1–R5（見 [`docs/dev/AUTONOMY-GOVERNANCE.md`](docs/dev/AUTONOMY-GOVERNANCE.md)）：
  out-of-scope / forbidden-zone / not-reviewed → 正規化 ≤2 輪，否則 escalate 為 needs-human；
  branches-only；只寫 dev-loop ledger。
- **commute-loop** [`scripts/dev-loop/commute-loop.sh`](scripts/dev-loop/commute-loop.sh) — 一個
  SUPERVISED、有界的、無人值守的 pass，掃過一個 spec backlog（你在離開前授權一次 run）。

**arm 任何 loop 的前置條件：** moat-pollution wall 必須通過——
[`scripts/dev-loop/pollution-wall-check.sh`](scripts/dev-loop/pollution-wall-check.sh) 證明
dev-loop 鏈絕不寫入產品的 `partner-signals` moat ledger（機器流量不得污染
「我是否真的有在用這個？」）。多機器的*並行*聯合開發（一個持久的 tailnet
channel、跨機器的 atomic lease）受 **§0.1 把關**——不要預先建造它。

**owner 在哪裡掌舵**（完整契約：[`docs/dev/AUTONOMY-GOVERNANCE.md`](docs/dev/AUTONOMY-GOVERNANCE.md)
§0）。無人值守 ≠ 無定義——方向是 owner 的，結果是 owner 的，落在三個點上：
(1) **before** — owner 撰寫/編輯 `[spec]` envelope（改 spec = 改方向）；
`spec-gate` 會擋下任何沒有 spec 的 task。(2) **during** — drift 會被*浮現*為 needs-human
提案（`~/.phantom-mesh/deviation-proposals.jsonl`、`status.sh`），絕不臆測。(3) **after** —
沒有任何東西會抵達 `main`；工作以 **branches-only** 落地供 owner 審查（`commute-report-*.md`、
那個 integration 分支）並 merge。§0.1 gate 是從「branches-only 由你審查」到
「取得信任後 auto-merge 進 core」的節流閥。

## Recent context

<!-- phantom and `/share AGENTS.md --append` write rolling status here; keep entries short + dated. -->
- 2026-06-09 — Root `AGENTS.md` created (was referenced by ~15 files but missing). Codex phantom MCP
  re-pointed at bare `phantom`. Added `scripts/dev-loop/pollution-wall-check.sh` (the moat-pollution
  precondition). See `SESSION_RESUME.md` for the live next step.
