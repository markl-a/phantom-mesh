# `docs/tdd/`

[English version](README.md)

Test-Driven Development（TDD，測試驅動開發）的工作文件。這些文件不綁定
特定工具，Claude、Gemini、Codex、Antigravity 或人類都可以閱讀。

| 檔案 | 用途 |
|---|---|
| `INDEX.md` | 即時 P0 測試清單，也是目前 TDD scripts 使用的 SSOT |
| `workflow.md` | TDD 循環、規則與允許偏離流程的情況 |
| `results.log` | 已完成測試的 append-only 時間戳記錄 |
| `notes.md` | 選用：處理 P0 時發現但不在範圍內的事項 |

## Driver scripts

請參閱 [`../../scripts/tdd/README.zh-TW.md`](../../scripts/tdd/README.zh-TW.md)。

## 規劃來源

完整 TDD 規劃位於
[`../planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md`](../planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md)。
內容包含 P0/P1/P2 分層、各平台分配、四週執行表與跨工具 framework 設計。

## 同步方向

```text
完整 TDD 規劃的 P0 清單
        |
        | 手動同步
        v
docs/tdd/INDEX.md
        |
        +--> scripts/tdd/*.sh
        |
        +--> results.log
```

當完整規劃中的 P0 清單變更時：

1. 從規劃文件重新產生 `INDEX.md`，保留既有 `[x]` 標記。
2. Commit 並 push。
3. 其他工具會在下次 session 讀到新的清單。
