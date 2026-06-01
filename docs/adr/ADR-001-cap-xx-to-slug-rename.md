---
id: ADR-001
date: 2026-05-23
status: accepted
title: CAP-XX numbering replaced by `<pillar>.<slug>`
context: |
  v0.3.5 之前 SPEC-01 自創 `CAP-P1-01` / `CAP-P3-04` 等編號系統，但 BIG-GOAL.md 只定義 4 pillar（P1~P4）+ ROADMAP 定義 7 epic（E001~E007）；CAP-XX 流水號是 spec 自己編的，缺乏對照原始 anchor 的途徑。隨時間 ~18 條 CAP 在不同 spec 引用時開始 drift：有用 `CAP-P1-01`、有用 bare `P1-01`、有 Mermaid graph 用 `P1_01`。新 contributor 與 AI sub-orchestrator 無法穩定 grep。
decision: |
  廢除自創 `CAP-XX` 流水號。改用 `<pillar>.<slug>` 命名（如 `P1.peer-wire`、`P3.mcp`、`X.coach`），其中 pillar (P1-P4) 來自 BIG-GOAL，slug 是描述性無編號。SPEC-01 §8 cheat sheet 重寫展示新 slug 對照 + 來源說明；舊 ID 在 §8 標題用 `(舊：P1.peer-wire)` 標出 1-2 個版本後移除。全文 ~380 處替換（CAP-P1-01 / bare P1-01 / mermaid P1_01 三種形式都換）。
consequences: |
  - 對齊 BIG-GOAL pillar — slug 直接 grep 到 BIG-GOAL.md pillar 段落，無中間翻譯層
  - 對齊 ROADMAP epic — slug 含描述性詞彙，與 commit `Serves: P3.mcp` 風格一致
  - 對齊 Mermaid syntax — slug 使用 `.` 分隔（如 `P1.peer-wire`），避開 Mermaid `_` 與 underscore-as-italic 衝突
  - 一次性成本：~380 處 ripgrep 替換、3 個 mermaid graph 重畫、5 個 cross-ref 表格重寫
  - 過渡期成本：舊 ID 別名保留 1-2 版本，spec 讀者短期看到 dual notation
alternatives_considered:
  - name: 保留 CAP-XX、加 cross-ref table 對照 pillar
    why_rejected: drift 已發生不是 future risk；增加對照表只是加層 indirection 不解 root cause
    when_to_revisit: null
  - name: 改用純數字 (CAP-001 ~ CAP-023) 不分 pillar
    why_rejected: 失去 pillar grouping 的可讀性；新增 capability 時編號不對齊原始 anchor
    when_to_revisit: null
supersedes: []
superseded_by: null
related_specs:
  - SPEC-01-FOUNDATION-bigGoal-mapping
  - SPEC-00-INDEX
---

## Long-form rationale

詳見 SPEC-01-FOUNDATION-bigGoal-mapping.md changelog v0.3.6 (2026-05-23) 與該檔 §8 開頭 cheat sheet。本 ADR 為該變更的正式 cross-spec 紀錄。
