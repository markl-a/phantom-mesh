# `tests-e2e/`

[English version](README.md)

phantom-mesh 的端對端（E2E）場景驗收。這些測試是半自動流程：
checklist 加上驗證命令。因為部分場景需要人類查看真實 UI、Telegram 或裝置，
所以不強制全部自動化。

> 測試堆疊位置：**L3 場景驗收**。

## 目錄結構

```text
tests-e2e/
├── README.md
├── README.zh-TW.md
├── run_tier1.sh
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
    └── <YYYY-Www>/
        └── T1.X-YYYY-MM-DD.md
```

## 執行方式

```bash
# 執行全部 Tier 1。每個人工步驟都會提示操作者。
./tests-e2e/run_tier1.sh

# 或手動執行單一場景
cat tests-e2e/scenarios/T1.8-tui-render-stress.md
# 依照步驟執行，並將結果存到 tests-e2e/results/<week>/T1.8-YYYY-MM-DD.md
```

## Tier 覆蓋範圍

| Tier | 場景數 | 狀態 |
|---|---:|---|
| Tier 1 | 8（T1.1-T1.8） | scaffold 已完成 |
| Tier 2 | 12（T2.1-T2.12） | 等 m1、acer、ayaneo onboarding 後加入 |
| Tier 3 | 18 | 延後到 v0.7.0+ |

每個場景的通過條件都寫在對應的 `.md` 中。

## Tracer 整合

當 `core/src/tracing/` 的 tracer 接進 runtime 後，每次場景執行也會產生：

```text
~/.phantom-mesh/traces/<task-id>.jsonl
```

結果文件應記錄 trace 路徑，方便後續 replay。

## PASS / FAIL 紀錄格式

```markdown
# T1.X - <name> - <YYYY-MM-DD>

Run by: <operator>
Machine: <z13/m1/acer/ayaneo>
Binary version: <output of `phantom --version`>

## Result
- [ ] PASS / [ ] PARTIAL / [ ] FAIL

## Notes
<observations, screenshots, trace path>

## V-matrix update
- doc 29 V<N>: <new status>
```

新增結果文件時，應在同一個 commit 更新 V-matrix 狀態。
