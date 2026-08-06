# `scripts/tdd/`

測試驅動開發（Test-Driven Development，TDD）的 dev-loop（開發循環）腳本。**Tool-neutral（工具中立）** — 可從
Claude Code / Gemini CLI / Codex CLI / Antigravity（任何具備 bash 工具的 agent（代理））呼叫。

> 測試堆疊中的層級：**L0 TDD driver（驅動器）**（位於 L1 單元測試之前）
> 完整計畫請見 `goal_plan/docs/31`，
> 即時 P0 測試清單請見 `docs/tdd/INDEX.md`，
> 跨工具進入點請見 `AGENTS.md §10`。

## 腳本

| 腳本 | 用途 | 回傳值 |
|---|---|---|
| `tdd-next.sh` | 從 `docs/tdd/INDEX.md` 印出下一個紅燈（red）的 P0 測試 | exit 0 + 該行 / 若全部綠燈則 exit 1 |
| `tdd-run.sh <name>` | 執行 `cargo test --lib <name> -- --test-threads=1` | exit 0 綠燈（green）/ 非零值表示紅燈（red） |
| `tdd-status.sh` | 印出 P0 進度（依平台分類細項 + 接下來 5 個紅燈） | exit 0 |
| `tdd-mark-done.sh <name>` | 在 INDEX.md 中把 `- [ ]` 改為 `- [x]` + 將時間戳記附加到 results.log | exit 0 完成 / exit 1 找不到 |
| `tdd-loop.sh` | 互動式循環：next → wait → run → mark → next | exit 0 |

## 用法（任何工具）

```bash
# Where am I?
./scripts/tdd/tdd-status.sh

# What's next?
./scripts/tdd/tdd-next.sh

# Run a specific test
./scripts/tdd/tdd-run.sh tracing::tests::write_three_events_in_order

# Mark done (after green)
./scripts/tdd/tdd-mark-done.sh tracing::tests::write_three_events_in_order

# Interactive loop(recommended for solo session)
./scripts/tdd/tdd-loop.sh
```

## 各工具的進入點

每個 agent 工具都有自己的 slash command（斜線命令）目錄。它們全都呼叫
這同一批 bash 腳本：

- `.claude/commands/tdd-*.md` — Claude Code 斜線命令
- `.gemini/commands/tdd-*.toml` — Gemini CLI 斜線命令
- `.codex/AGENTS.md` — Codex 讀取 AGENTS.md；本目錄補充 Codex 專屬說明
- `.antigravity/AGENTS.md` — Antigravity 進入點

## 慣例：checkbox（核取方塊）放在哪裡

`docs/tdd/INDEX.md` 是**唯一可信來源（source of truth）**。格式：

```markdown
- [ ] WIN | tui::tests::clear_widget_called_on_resize | V11 | 2h
- [x] LIN | (done examples) | V5 | 1h
```

`tdd-mark-done.sh` 只會翻轉**第一個**符合的 `- [ ]` 行 —
若同一測試名稱出現多次（不同平台），請明確替它們加上標籤
（例如 `tui_tests_clear_widget_called_on_resize_win`）。

## 結果記錄檔（Results log）

`docs/tdd/results.log` — 每完成一個測試就附加一行（append-only，僅可附加）：
```
2026-05-19T03:42:11Z | dev | tui::tests::clear_widget_called_on_resize | green
```

適用於：每週檢視、GA（正式上市）就緒度評估、commit message（提交訊息）產生。

## 限制

- **唯一可信來源式的鷹架（scaffolding）** — INDEX.md 目前是由
  `goal_plan/docs/31 §6` 手動產生。自動同步是 v0.7.0 的功能。
- **無 flaky（不穩定）測試重試機制** — 若某測試不穩定（例如 `SPECTYN_AUTO_APPROVE`
  環境變數競態），請使用 `--test-threads=1`（`tdd-run.sh` 中已內建）。
- **僅支援 Bash** — Windows 使用者需要 Git Bash 或 WSL。PowerShell 移植版
  是後續工作（優先度低，因為多數 agent 本來就是跑 bash）。
