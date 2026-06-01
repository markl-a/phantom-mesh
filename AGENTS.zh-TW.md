# Phantom Mesh - Agent Guide（繁體中文版）

[English version](AGENTS.md)

> 本文件是 `AGENTS.md` 的繁體中文閱讀版。實際規則仍以 `AGENTS.md` 為 SSOT。

## 1. Source of Truth

接手 repo 時依序閱讀：

1. `_planning-audit/MASTER-PLAN.md`：策略歷史與 master plan。
2. `SESSION_RESUME.md`：戰術狀態、目前進度、下一步。
3. `docs/ARCHITECTURE.md`：高階架構。
4. `PHANTOM.md`：快速架構草圖。
5. `AGENTS.md`：跨工具規則。

2026-05-19 Life Node pivot 後，產品方向以：

- `docs/superpowers/BIG-GOAL.md`
- `docs/superpowers/specs/2026-05-19-life-node-pivot.md`

為準。不要讀 `_planning-audit/archived/`，除非正在做歷史研究。

## 2. Repo 邊界

| 路徑 | 責任 |
|---|---|
| `core/` | Rust runtime、providers、tools、MCP、mesh、serve、agent loop、REPL |
| `crates/pm-types/` | `core/` 與 Tauri 共用型別 |
| `app/src-tauri/` | Desktop + mobile Tauri shell、OS integration、sidecar |
| `app/src/` | TypeScript + React + Tailwind web frontend |
| `configs/` | 各裝置 agent config templates |
| `scripts/` | 公開 tooling |

不要在以下路徑新增產品功能：

- `apple-oauth-relay/`
- `app/src/pages/legacy/`
- `src/clawtex/`

Mobile target 只使用 `app/src-tauri/`。

## 3. 架構原則

1. **Contract first：**先在 `crates/pm-types/` 定義型別，再寫實作。
2. **可替換 capability：**providers、tools、channels 都要能透過 registry 替換。
3. **Thin runtime spine：**`core/` 負責 orchestration；CLI、web、Tauri 共用 contracts。
4. **Surface-neutral behavior：**不能只在 browser fallback mode 可用。
5. **Subagent-first UX：**Claude Code 與 Codex 是第一級 consumer。

## 4. Session 結束前

1. 更新 `SESSION_RESUME.md`：做了什麼、blocked 項目、下一個具體步驟。
2. 策略改變時才更新 `_planning-audit/MASTER-PLAN.md`。
3. Rust code 改動後在 `core/` 執行 `cargo check`。
4. 未經使用者明確要求，不要 commit。

## 5. Guardrails

- 不要在 repo root 新增 top-level `*.md` 規劃文件。
- 不要重新引入 archived FREEZE、SLICE、SPRINT、TODO 文件。
- 不要 commit secrets。`agents.toml` 與 `.env*` 已 gitignore。
- 未經明確要求，不要 push 到 `main`。
- 公開 repo 前不能跳過 `git filter-repo` secret-cleaning 計畫。

## 6. 平行工作與 Worktree

### 硬性規則

**絕對不要讓兩個 assistant sessions 在同一個 working directory 工作。**
它們會共用 `.git/index`、`target/`、`node_modules/` 與 lockfiles，容易靜默覆寫。

平行工作必須使用 `git worktree`：

```bash
git worktree add .worktrees/<topic> -b feat/<topic> phase1-r1-foundations
```

命名慣例：

- Worktree：`.worktrees/<topic>`
- Branch：`feat/<topic>`

Windows worktree 內執行 Cargo 時，Defender 可能鎖定檔案。可使用：

```powershell
$env:CARGO_TARGET_DIR='D:/tmp/phantom-windows-target'
cargo check
```

### 容易衝突的 hot files

- `core/src/bin/phantom.rs`
- `core/src/platform/mod.rs`
- `core/Cargo.toml`
- `app/src-tauri/Cargo.toml`
- `app/src-tauri/tauri.conf.json`
- `app/src-tauri/capabilities/*.json`
- `Cargo.lock`
- `app/package.json`
- `.github/workflows/*.yml`

如果兩個 sessions 都需要修改 hot file，先完成並 merge 第一個，再開始第二個。

### 破壞性操作需要明確確認

未經使用者明確要求，不得執行：

- `git push --force` 或 `--force-with-lease`
- 刪除並重寫公開 tag
- 對已 push commit 執行 `git reset --hard`
- `git stash drop` 或 `git stash clear`
- 對有未 commit 變更的 worktree 執行 `git worktree remove --force`

## 7. AI Tool Dispatch Policy

預設外部工具優先：

```text
opencode (free) -> codex -> agy -> claude-subagent
```

預設流程：

1. Orchestrator 將工作切成每段不超過 200 行輸出、5 分鐘 wall time。
2. 使用 `bash scripts/ai/dispatch.sh <tool> <prompt-file>` 派工。
3. 驗證 output 格式、OSS-safe 與 anchor alignment。
4. Mermaid、git、架構決策與跨 stage stitch 由 orchestrator 處理。
5. 完成 stitch、4-track audit 與 commit。

跨工具共享記憶放在：

- `.ai-shared/memory/`
- `.ai-shared/skills/`
- `.ai-shared/prompts/`

## 8. TDD 與跨工具 Dev Loop

Canonical TDD plan：

```text
docs/planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md
```

即時 P0 checklist：

```text
docs/tdd/INDEX.md
```

工作流程：

1. `./scripts/tdd/tdd-status.sh`
2. `./scripts/tdd/tdd-next.sh`
3. 先寫測試。
4. `./scripts/tdd/tdd-run.sh <name>`，確認 red。
5. 寫最小實作。
6. 再次執行測試，確認 green。
7. `./scripts/tdd/tdd-mark-done.sh <name>`
8. 回到步驟 1。

各工具入口：

| 工具 | 入口 |
|---|---|
| Claude Code | `.claude/commands/tdd-*.md` |
| Gemini CLI | `.gemini/commands/tdd-*.toml` |
| Codex CLI | `.codex/AGENTS.md` |
| Antigravity | `.antigravity/AGENTS.md` |

這套 TDD 不追求 100% coverage，也不是 greenfield 流程；它針對 v0.6.0 delta。

