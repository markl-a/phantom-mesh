# `scripts/tdd/`

[English version](README.md)

Test-Driven Development（TDD，測試驅動開發）的 dev-loop scripts。這些腳本
不綁定特定工具，Claude Code、Gemini CLI、Codex CLI、Antigravity 或任何
有 bash 的 agent 都可以呼叫。

> 測試堆疊位置：**L0 TDD driver**，在 L1 unit tests 之前。
> 完整計畫請看 `docs/planning/sprint-2026-05-18/31-...md`，
> 即時 P0 清單請看 `docs/tdd/INDEX.md`。

## Scripts

| Script | 用途 | 回傳值 |
|---|---|---|
| `tdd-next.sh` | 顯示 `docs/tdd/INDEX.md` 中下一個 red P0 測試 | 找到測試時 exit 0；全部 green 時 exit 1 |
| `tdd-run.sh <name>` | 執行 `cargo test --lib <name> -- --test-threads=1` | green 時 exit 0；red 時非 0 |
| `tdd-status.sh` | 顯示 P0 進度、各平台拆分與接下來 5 個 red 測試 | exit 0 |
| `tdd-mark-done.sh <name>` | 將 `- [ ]` 改成 `- [x]`，並 append 到 results log | 成功時 exit 0；找不到時 exit 1 |
| `tdd-loop.sh` | 互動式循環：next → 等待 → run → mark → next | exit 0 |

## 使用方式

```bash
# 查看目前進度
./scripts/tdd/tdd-status.sh

# 查看下一個測試
./scripts/tdd/tdd-next.sh

# 執行指定測試
./scripts/tdd/tdd-run.sh tracing::tests::write_three_events_in_order

# Green 後標記完成
./scripts/tdd/tdd-mark-done.sh tracing::tests::write_three_events_in_order

# 互動式循環，單人 session 建議使用
./scripts/tdd/tdd-loop.sh
```

## 各工具入口

所有工具最後都會呼叫同一組 bash scripts：

- `.claude/commands/tdd-*.md`：Claude Code slash commands
- `.gemini/commands/tdd-*.toml`：Gemini CLI slash commands
- `.codex/AGENTS.md`：Codex 額外說明
- `.antigravity/AGENTS.md`：Antigravity 額外說明

## Checkbox 慣例

`docs/tdd/INDEX.md` 是 SSOT，格式如下：

```markdown
- [ ] WIN | tui::tests::clear_widget_called_on_resize | V11 | 2h
- [x] LIN | (done examples) | V5 | 1h
```

`tdd-mark-done.sh` 只會修改第一個符合的 `- [ ]`。如果測試名稱在不同平台
重複出現，請加入平台後綴，例如 `tui_tests_clear_widget_called_on_resize_win`。

## Results log

`docs/tdd/results.log` 每完成一個測試就 append 一行：

```text
2026-05-19T03:42:11Z | m4932 | tui::tests::clear_widget_called_on_resize | green
```

可用於每週 review、GA readiness 與產生 commit message。

## 限制

- `INDEX.md` 目前仍是從完整規劃手動同步；自動同步延後到 v0.7.0。
- 不會自動 retry flaky test。環境變數 race 等問題使用
  `--test-threads=1`，`tdd-run.sh` 已內建。
- 目前只有 bash 版本。Windows 使用者需要 Git Bash 或 WSL。
