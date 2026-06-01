---
name: run-tests
version: 0.1.0
description: 執行完整的 cargo test workspace（工作區），擷取通過/失敗計數，並呈現第一個失敗的測試以便分流處理。
triggers:
  - "run the tests"
  - "cargo test"
  - "run cargo test workspace"
  - "are the tests passing"
  - "test the workspace"
tools:
  - shell
  - git_status
inputs:
  workspace_dir: "cargo workspace（工作區）根目錄的路徑。預設為儲存庫根目錄。"
  features: "選填，以逗號分隔的 feature flags（功能旗標），會傳遞給 cargo test。"
outputs:
  - "摘要行：K 個 crate 中共 N 個通過 / M 個失敗"
  - "第一個失敗的測試名稱與 stderr 摘錄（若有）"
tags:
  - testing
  - rust
  - ci
created_at: "2026-05-15T00:00:00Z"
author: phantom-mesh weekend push T32
---

# run-tests

跨整個 workspace（工作區）執行 `cargo test`，然後將輸出濃縮成一行摘要，外加
第一個失敗項目（若有）。這個 skill（技能）是保守的：它絕不會變動工作樹
（working tree），且只要有任何測試失敗便以非零碼結束，讓呼叫方可據此分支處理。

## 步驟

第一步是在我們花費數分鐘建置之前，先確保工作樹處於正常狀態。略過這項檢查曾讓
我們吃過虧 — 一個來自半套用 rebase 的過期 `Cargo.lock`，會以誤導性的
「package not found」訊息使測試失敗。

```bash
git status --short
```

接著，執行測試套件。我們使用 `--no-fail-fast`，這樣便能在單次執行中看到每一個
失敗，而不是在第一個失敗就停下；結尾的 LLM（大型語言模型）步驟會挑出資訊量最高
的那一個來呈現。

```bash
cargo test --workspace --no-fail-fast 2>&1 | tee /tmp/phantom-skill-run-tests.log
```

現在擷取摘要行。Cargo 會為每個 crate 印出 `test result: ok. N passed; M failed`，
所以我們用 grep 抓它。

```bash
grep -E "^test result:" /tmp/phantom-skill-run-tests.log | tail -20
```

一旦日誌寫到磁碟上，呼叫方就能決定要向 LLM（大型語言模型）索取一份人類可讀的
摘要，或是快速失敗結束。

## Prompt：摘要測試結果

根據擷取於 `/tmp/phantom-skill-run-tests.log` 的 cargo test 日誌，
產出一份 3 行報告：

1. 整個 workspace（工作區）所有 crate 的總通過/失敗計數。
2. 第一個失敗測試的名稱（若有），含其 module（模組）路徑。
3. 對失敗類別的一句猜測（編譯錯誤、斷言、
   panic（程式崩潰）、逾時、不穩定/網路問題）。

請勿包含原始的堆疊追蹤（stack trace）。每行控制在 100 個字元以內。

## 失敗模式

- **進入時工作區處於 dirty（有未提交變更）狀態：** 在摘要前面加一行警告，標明
  這些 dirty 的路徑，但仍繼續執行 — 本機編輯是作者的正常狀態。
- **`cargo` 不在 PATH 中：** 在 tee 任何內容到磁碟之前，以清楚的錯誤訊息中止。
- **日誌檔已存在於先前的執行：** 覆寫是刻意的；
  我們要的是全新的結果，而非附加在後。
