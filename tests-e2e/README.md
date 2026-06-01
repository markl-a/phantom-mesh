# `tests-e2e/`

phantom-mesh 的端對端（end-to-end，端點到端點）情境驗收。半自動化
（檢查清單 + 驗證命令）—— 這些情境需要人眼實際盯著 UI（使用者介面）／
Telegram／裝置，所以我們不嘗試完全自動化它們。

> 測試堆疊中的層級（layer）：**L3 場景驗收**
> 背景脈絡見 `goal_plan/docs/29 §3`，V1-V11 矩陣（matrix）見 `§4`，
> 完整 38 情境分類法（taxonomy）見 `goal_plan/docs/28 §2`。

## 目錄結構

```text
tests-e2e/
├── README.md                            (this file)
├── run_tier1.sh                         half-auto runner for 8 Tier 1 scenarios
├── scenarios/
│   ├── T1.1-telegram-one-line.md
│   ├── T1.2-web-search.md
│   ├── T1.3-night-shift.md
│   ├── T1.4-skill-auto-extract.md
│   ├── T1.5-provider-failover.md
│   ├── T1.6-webview2-install.md
│   ├── T1.7-service-reboot-acl.md
│   └── T1.8-tui-render-stress.md
└── results/
    └── <YYYY-Www>/                      one folder per ISO week
        └── T1.X-YYYY-MM-DD.md           filled-in result per scenario per run
```

## 執行方式

```bash
# Run all Tier 1 (prompts you for each one's manual step)
./tests-e2e/run_tier1.sh

# Or pick a specific scenario manually
cat tests-e2e/scenarios/T1.8-tui-render-stress.md
# (follow the steps, save result to tests-e2e/results/<week>/T1.8-YYYY-MM-DD.md)
```

## Tier（層級）涵蓋範圍

| Tier | 情境 | 狀態 |
|---|---|---|
| Tier 1 | 8 (T1.1-T1.8) | 🟢 此處 scaffold（鷹架雛型）已完成 |
| Tier 2 | 12 (T2.1-T2.12) | 🟡 於 mac/node-b/node-a 上線後加入 |
| Tier 3 | 18 (deferred，延後) | ⚪ v0.7.0+ |

每個情境的通過條件（pass criteria）編寫在各自的情境 `.md` 中。

## Tracer（追蹤器）整合

當 tracer（追蹤器，見 `core/src/tracing/`）接入執行階段（runtime）後，每次
情境執行也會在
`~/.phantom-mesh/traces/<task-id>.jsonl` 落下一份 JSONL 追蹤紀錄。結果檔
應引用該追蹤路徑，方便你日後重播（replay）。

## 通過／失敗紀錄

結果檔使用以下格式：

```markdown
# T1.X — <name> — <YYYY-MM-DD>

Run by: <operator>
Machine: <node-a/mac/node-b>
Binary version: <output of `phantom --version`>

## Result
- [ ] PASS / [ ] PARTIAL / [ ] FAIL

## Notes
<observations, screenshots, trace path>

## V-matrix update
- doc 29 §4 V<N>: <new status>
```

請在與結果檔相同的 commit（提交）中一併更新 doc 29 §4 V<N> 的狀態。
