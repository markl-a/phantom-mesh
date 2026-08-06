# `docs/tdd/`

Test-Driven Development（TDD，測試驅動開發）的工作文件。這些文件不綁定
特定工具，Claude、Gemini、Codex、Antigravity 或人類都可以閱讀。

> **狀態更新（2026-06-11）**：原本作為即時 P0 SSOT 的 `docs/tdd/INDEX.md`
> 已於 commit `9914cc6b` 移除，`results.log` / `notes.md` 也不再維護。
> 目前的 TDD 入口是 [`AGENTS.md` §8「TDD & cross-tool dev loop」](../../AGENTS.md)
> 加上仍存在的 driver scripts `scripts/tdd/`（含 `tdd-status.sh`）。
> 若需要歷史 P0 清單，可從 `9914cc6b^` 取回 `INDEX.md`。

| 檔案 | 用途 |
|---|---|
| [`workflow.zh-TW.md`](workflow.zh-TW.md) | TDD 循環、規則與允許偏離流程的情況 |

## Driver scripts

請參閱 [`../../scripts/tdd/README.zh-TW.md`](../../scripts/tdd/README.zh-TW.md)，
session 開始時跑 `./scripts/tdd/tdd-status.sh`。

## 規劃來源

完整 TDD 規劃位於
[`../planning/sprint-2026-05-18/31-spectyn-mesh-tdd-comprehensive-plan-2026-05-18.md`](../planning/sprint-2026-05-18/31-spectyn-mesh-tdd-comprehensive-plan-2026-05-18.md)。
內容包含 P0/P1/P2 分層、各平台分配、四週執行表與跨工具 framework 設計。

## 同步方向

```text
完整 TDD 規劃的 P0 清單
        |
        | 手動同步（INDEX.md 移除後此鏈結已斷）
        v
scripts/tdd/*.sh（tdd-status.sh / tdd-next.sh / tdd-run.sh / tdd-mark-done.sh）
```

> 註：上述流程原本以 `docs/tdd/INDEX.md` 為樞紐；該檔案移除後，driver
> scripts 仍可獨立執行測試，但已沒有 repo 內的即時 P0 checklist SSOT。
> 後續若要恢復清單式追蹤，請參考 AGENTS.md §8 與 `scripts/tdd/`。
