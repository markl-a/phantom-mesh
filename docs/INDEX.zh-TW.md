# Phantom Mesh 文件索引

[English version](INDEX.md)

這份文件是產品規格與測試文件的快速入口。目標是區分「目前有效的
source of truth（SSOT，單一事實來源）」與歷史文件，避免誤用已被取代的規劃。

## 先看這裡

要開始開發功能，建議依序閱讀：

1. [`../AGENTS.zh-TW.md`](../AGENTS.zh-TW.md) - repo 規則、目錄邊界與 TDD 工作流程。
2. [`superpowers/BIG-GOAL.zh-TW.md`](superpowers/BIG-GOAL.zh-TW.md) - 目前產品方向：
   4 個支柱、2 條產品線、3 個執行原則。已於 2026-05-19 重新鎖定。
3. [`superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md`](superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md)
   - v0.6.0 Life Node pivot、目前 epics、功能範圍與 roadmap。
4. [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md)
   - 實作時使用的 deep spec（深度規格）總目錄。
5. [`../SESSION_RESUME.md`](../SESSION_RESUME.md) - 最新交接狀態與下一個具體工作。

需要了解系統架構時，再讀 [`ARCHITECTURE.zh-TW.md`](ARCHITECTURE.zh-TW.md)。原始架構文件早於
Life Node pivot，因此可以用來理解架構，但不是目前產品範圍的最終依據。

## 目前有效的規格

### 產品方向

| 文件 | 用途 |
|---|---|
| [`superpowers/BIG-GOAL.zh-TW.md`](superpowers/BIG-GOAL.zh-TW.md) | v0.6.0 週期不可任意修改的產品方向 |
| [`superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md`](superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md) | 已核准的 pivot 規格與 epic 重整 |
| [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) | deep spec 總目錄與閱讀順序 |

### 目前使用中的 Epics

v0.6.0 的 epic 規格集中在
[`superpowers/specs/_current/`](superpowers/specs/_current/)：

| Epic | 規格 | 規格內記錄的狀態 |
|---|---|---|
| E001 | [`跨主機 cluster smoke`](superpowers/specs/_current/E001-cross-host-cluster-smoke.zh-TW.md) | 維護中 |
| E002 | [`多模態 capture pipeline`](superpowers/specs/_current/E002-multimodal-capture-pipeline.zh-TW.md) | 已完成 |
| E003 | [`Coach node 與 daily review`](superpowers/specs/_current/E003-coach-node-daily-review.zh-TW.md) | 尚未開始 |
| E004 | [`加密儲存層`](superpowers/specs/_current/E004-encrypted-storage-layer.zh-TW.md) | 已完成 |
| E005 | [`Hermes 技能擷取`](superpowers/specs/_current/E005-hermes-skill-extraction.zh-TW.md) | 尚未開始 |
| E006 | [`30 秒 Life hello`](superpowers/specs/_current/E006-30-second-hello-world.zh-TW.md) | 尚未開始 |
| E007 | [`v0.6.0 發布準備`](superpowers/specs/_current/E007-v060-release-prep.zh-TW.md) | 已接受 |

### 從規格到程式碼

| 文件 | 用途 |
|---|---|
| [`superpowers/CONTRIBUTING-spec-to-product.md`](superpowers/CONTRIBUTING-spec-to-product.md) | 開發者導覽：spec → types → implementation → product |
| [`superpowers/SPEC-TO-CODE-PLAYBOOK.md`](superpowers/SPEC-TO-CODE-PLAYBOOK.md) | 分階段實作的詳細 playbook |

## 測試文件

### GA Gate：優先看這裡

| 文件 | 用途 |
|---|---|
| [`tdd/INDEX.md`](tdd/INDEX.md) | 即時 P0 清單，也是 TDD scripts 的 SSOT |
| [`tdd/workflow.zh-TW.md`](tdd/workflow.zh-TW.md) | red → green → mark done 流程與例外規則 |
| [`tdd/README.zh-TW.md`](tdd/README.zh-TW.md) | `docs/tdd/` 目錄說明 |
| [`../scripts/tdd/README.zh-TW.md`](../scripts/tdd/README.zh-TW.md) | TDD scripts 使用方式 |

建立本索引時，live checklist 共有 `168` 個 P0 項目：`80` 個已完成，
`88` 個待處理。請以 [`tdd/INDEX.md`](tdd/INDEX.md) 為準，不要採用舊規劃文件
中的數量。

常用指令：

```bash
./scripts/tdd/tdd-status.sh
./scripts/tdd/tdd-next.sh
./scripts/tdd/tdd-run.sh <test-name>
./scripts/tdd/tdd-mark-done.sh <test-name>
```

### 測試規劃與覆蓋範圍

| 文件 | 用途 |
|---|---|
| [`planning/lifecycle-tests/FIVE-PLATFORM-EXECUTION-PLAN.zh-TW.md`](planning/lifecycle-tests/FIVE-PLATFORM-EXECUTION-PLAN.zh-TW.md) | 五平台從登入到完整使用場景的執行計畫 |
| [`planning/lifecycle-tests/L0-GOLDEN-PATH.zh-TW.md`](planning/lifecycle-tests/L0-GOLDEN-PATH.zh-TW.md) | 五平台逐項執行與記錄 checklist |
| [`planning/lifecycle-tests/L0-GOLDEN-PATH-RUNBOOK.zh-TW.md`](planning/lifecycle-tests/L0-GOLDEN-PATH-RUNBOOK.zh-TW.md) | 五平台從安裝、登入到完整使用情境的逐步驗證手冊 |
| [`planning/lifecycle-tests/MASTER-END-TO-END-TEST-DEVELOPMENT-FLOW.zh-TW.md`](planning/lifecycle-tests/MASTER-END-TO-END-TEST-DEVELOPMENT-FLOW.zh-TW.md) | 從 Big Goal、spec、案例、除錯、五平台驗證到 release gate 的測試開發主流程 |
| [`planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md`](planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md) | P0/P1/P2 TDD 計畫與平台分配 |
| [`planning/lifecycle-tests/README.md`](planning/lifecycle-tests/README.md) | 五平台生命週期測試百科 |
| [`superpowers/specs/v060-deep-spec/SPEC-60-TESTING-strategy.md`](superpowers/specs/v060-deep-spec/SPEC-60-TESTING-strategy.md) | V-track 測試策略 |
| [`superpowers/specs/v060-deep-spec/SPEC-61-TESTING-scenarios.md`](superpowers/specs/v060-deep-spec/SPEC-61-TESTING-scenarios.md) | 場景測試總表 |

[`planning/lifecycle-tests/`](planning/lifecycle-tests/) 下的文件是 wide-net
生命週期測試百科，不等於 GA checklist。

### 可執行與人工測試場景

| 文件 | 用途 |
|---|---|
| [`../scripts/phantom-test/README.zh-TW.md`](../scripts/phantom-test/README.zh-TW.md) | 黑箱測試：CLI、HTTP/RPC、磁碟狀態與真實 round-trip |
| [`../tests-e2e/README.zh-TW.md`](../tests-e2e/README.zh-TW.md) | 需要人工協助的 Tier-1 E2E 場景 |
| [`architecture/selftest-harness.zh-TW.md`](architecture/selftest-harness.zh-TW.md) | Self-test harness 架構 |

建立本索引時，`scripts/phantom-test/scenarios/` 內有 `36` 個測試腳本，
`tests-e2e/scenarios/` 內有 `8` 份 Tier-1 場景文件。

## 歷史文件

以下路徑只適合追溯歷史，不應作為目前實作依據：

| 路徑 | 用途 |
|---|---|
| [`../_planning-audit/MASTER-PLAN.md`](../_planning-audit/MASTER-PLAN.md) | 策略歷史與 audit trail |
| [`../_planning-audit/archived/`](../_planning-audit/archived/) | 已被取代的規劃；只有歷史研究才需要讀 |
| [`superpowers/specs/_archived/`](superpowers/specs/_archived/) | 為了保留追蹤紀錄而封存的舊功能規格 |

## 已知文件漂移

建立本索引時，已知有以下不一致：

- 舊 TDD 文件寫的是 `150` 或 `152` 個 P0 項目，但目前 live
  [`tdd/INDEX.md`](tdd/INDEX.md) 實際有 `168` 個。
- [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) 仍列出較舊的
  `16` 個場景快照，但 scenario 目錄目前實際有 `36` 個腳本。
- deep spec 總目錄與檔案系統數量尚未完全同步。新增或重新編號 spec 前，
  先查閱 [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md)，
  再檢查實際目錄。

## 快速判斷表

| 問題 | 請閱讀 |
|---|---|
| 現在要做的產品是什麼？ | [`superpowers/BIG-GOAL.zh-TW.md`](superpowers/BIG-GOAL.zh-TW.md) |
| Life Node pivot 改了什麼？ | [`superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md`](superpowers/specs/2026-05-19-life-node-pivot.zh-TW.md) |
| 我的實作應遵循哪份規格？ | [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) |
| 下一個 P0 測試是什麼？ | [`tdd/INDEX.md`](tdd/INDEX.md) 或 `./scripts/tdd/tdd-next.sh` |
| 如何執行黑箱驗證？ | [`../scripts/phantom-test/README.zh-TW.md`](../scripts/phantom-test/README.zh-TW.md) |
| 最新交接狀態在哪裡？ | [`../SESSION_RESUME.md`](../SESSION_RESUME.md) |

## 翻譯策略

- 英文原文保留不動，中文副本使用 `.zh-TW.md` 後綴。
- 已經以中文或中英混合撰寫的 deep spec、TDD 長篇規劃與 lifecycle 文件，
  中文索引直接連到原檔，避免維護兩份相同內容。
- [`tdd/INDEX.md`](tdd/INDEX.md) 是會持續更新 checkbox 的 live SSOT，因此不建立
  可獨立修改的翻譯副本。測試名稱、平台代碼與 V-track 保持原樣，避免 scripts
  與文件狀態分離。
