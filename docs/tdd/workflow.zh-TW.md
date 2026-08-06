# TDD 工作流程（跨工具、ratchet-style）

[English version](workflow.md)

適用對象：任何接手本 repo 的 agent（Claude Code、Gemini CLI、Codex CLI、
Antigravity）或人類操作者。流程不綁定特定工具。

## 循環

```text
tdd-next
   |
   v
先寫測試 --> tdd-run 確認 RED
                     |
                     v
               寫最小實作
                     |
                     v
               tdd-run 確認 GREEN
                     |
                     v
               tdd-mark-done
                     |
                     +--> 下一個測試
```

## 規則

1. **測試優先。** 一定要先確認 red，再寫實作。如果還沒改程式碼就已經
   green，代表測試寫錯了。
2. **只寫讓測試變 green 的最小程式碼。** 不要在同一步驟混入 refactor。
   Green 之後才能在測試保護下整理程式碼。
3. **一次只處理一個測試。** 目前測試 green 且已在 `INDEX.md` 標記完成後，
   才能選下一個。
4. **不要超出範圍。** P0 測試只處理 v0.6.0 GA。如果發現 P1/P2 問題，
   記錄到 `docs/tdd/notes.md`，然後繼續目前 P0。
5. **跨平台測試要維持 cfg branch 對等。** 如果新增
   `#[cfg(target_os = "windows")]` 測試，也要補 Linux、Mac、Android
   版本，除非功能本來就是平台專用，例如 Mac-only 的 `spectyn snapshot`。

## 常用 scripts

| 時機 | Script | 用途 |
|---|---|---|
| Session 開始 | `./scripts/tdd/tdd-status.sh` | 顯示進度與接下來 5 個 red 測試 |
| 選測試 | `./scripts/tdd/tdd-next.sh` | 顯示下一個 red 測試 |
| 驗證 red | `./scripts/tdd/tdd-run.sh <name>` | 執行測試並回傳 exit code |
| 驗證 green | `./scripts/tdd/tdd-run.sh <name>` | 再次執行測試並回傳 exit code |
| 標記完成 | `./scripts/tdd/tdd-mark-done.sh <name>` | 勾選 checkbox 並寫入 log |
| 自動循環 | `./scripts/tdd/tdd-loop.sh` | 互動式執行完整循環 |

## Artifact 位置

| Artifact | 路徑 | 修改者 |
|---|---|---|
| 測試清單 | `docs/tdd/INDEX.md` | `tdd-mark-done.sh` 自動修改；初次同步由人處理 |
| 結果 log | `docs/tdd/results.log` | `tdd-mark-done.sh` 以 append-only 方式寫入 |
| 規劃來源 | `docs/planning/sprint-2026-05-18/31-spectyn-mesh-tdd-comprehensive-plan-2026-05-18.md` | 人類 |
| 範圍外備註 | `docs/tdd/notes.md` | 人類 |
| 執行產物 | `target-tdd/`（gitignored） | Cargo |

## 允許偏離流程的情況

- **既有 flaky test：**寫 characterization test 固定目前行為，加入
  `// FLAKY: see docs/tdd/notes.md#flaky-<name>` 註解，並在完成紀錄中註明。
- **需要但目前沒有實體硬體：**在 `tdd-loop.sh` 中按 `s` 跳過，不要標記
  完成。測試保留為 `- [ ]`，交給有設備的人處理。
- **發現更深層架構問題：**記錄到 `docs/tdd/notes.md`，另外開 issue。
  如果原始 assertion 仍成立，可以標記目前 P0 完成。

## V-track 與測試名稱的關係

`INDEX.md` 中每個測試名稱都有 V-track tag，例如 `V1`、`V11`、`PF-2b`。
測試轉為 green 後：

1. `tdd-mark-done.sh` 勾選 checkbox。
2. 人類確認該 tag 的所有依賴測試都 green 後，手動更新 V-matrix。
3. V-matrix row 完成後，可再從 daily progress tracker 連回這裡。

## 這套流程不是什麼

- **不是 100% coverage。**
- **不是技能庫 upstream tools 的測試框架。**它們使用 upstream CI。
- **不是 greenfield 流程。**目前 codebase 已有約 11K LOC 與 633 個測試；
  這套流程針對 v0.6.0 delta，而不是從零重建。
