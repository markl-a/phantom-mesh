# CLAUDE.md — Claude Code 工作階段設定（非規劃文件）

> 跨工具規則位於 [`AGENTS.md`](AGENTS.md) — 請先閱讀；它是 source-of-truth 順序、
> repo 邊界、護欄（guardrail）與工作階段衛生（session hygiene）的 SSOT。本檔案僅
> 釘選 Claude Code 每次工作階段都必須在脈絡中具備的規則。

## 真實來源（Source of truth）
- 產品頂點（apex）：`docs/superpowers/BIG-GOAL.md`（FINAL 於 2026-06-11 重新鎖定）。衝突
  裁決順序：apex(BIG-GOAL) > OPERATING-STANDARD > SPEC-00-INDEX > SPEC leaf > epic > feature > PR；
  as-built（既成）程式碼勝過過時的 spec，之後再以 DRIFT 標記回填 spec
  （`docs/superpowers/DOCUMENTATION-CHARTER.md`）。
- 運行 SSOT（HOW／路線圖／治理／檔案標準摘要）：`docs/OPERATING-STANDARD.md`
  （已折入原 GOVERNANCE／FLEET-DEV／JOINT-DEV／ROADMAP-VISUAL 四份文件）。
- 完成執行 SSOT（這次「完成整個生態」的執行流程 — 開發→測試→部署→開源、五階段
  S→D→T→P→O、4–5 機多 AI 並行、§3.5 正確性方法論、§10 遵守機制）：
  `docs/ECOSYSTEM-MASTER-PLAN.md`（逐輪操作細節在其附屬手冊
  `docs/ECOSYSTEM-COMPLETION-PROCESS.md`）。執行任何「完成生態」相關工作前先載入它。

## 多 AI 委派（常設規則 — 操作者指令 2026-06-12）
實質性任務（設計、審查、大量編輯、研究、測試掃描）**必須扇出（fan out）給
本地 AI 三人組 — codex / opencode / agy — 而非單獨執行。** 完整規則請見 AGENTS.md §3.5。
運作機制：
- `.claude/skills/local-ai/ask.sh <codex|opencode|agy|auto|all> "<prompt>"` — 處理每個
  工具的怪癖（agy 需要其 ConPTY 路徑；在指派工作前，請先閱讀該 skill 的「Delegation lessons」）。
  codex 同時也註冊為一個 MCP 伺服器（`codex` 工具）。
- codex = 逐檔的機械式編輯／程式碼產生（一次呼叫一個檔案，提交前先 lint）；
  opencode = repo 檔案閱讀／綜整；agy = 問答／第二意見（純提問）；
  Claude = 編排（orchestration）＋對抗式驗證＋最終裁決。
- 非瑣碎的變更，唯有在 ≥2 個不同的 AI 表示 LGTM 後才可落地（double-gate）。
- 瑣碎的機械式操作不在此限。

## 驗證（Verification）
- 文件樹一致性 lint：`pwsh scripts/check-doc-tree.ps1`（必須維持 ALL GREEN 全綠）。
- 不要透過管線（pipe）遮蔽結束碼（exit code）；請在該命令本身上檢查 `$?`。
