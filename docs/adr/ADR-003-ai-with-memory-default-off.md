---
id: ADR-003
date: 2026-05-24
status: accepted
title: "`AI_WITH_MEMORY` default OFF in `scripts/ai/dispatch.sh`"
context: |
  最初設計 dispatch.sh 預設把 `.ai-shared/memory/index.md` auto-prepend 到每個 CLI prompt，目的是讓所有外部工具（agy/codex/opencode）共享跨工具記憶。實測 2026-05-24 大 prompt（> 30KB）配合 auto-prepend 在 agy 與 opencode 上 stuck 20+ 分鐘，CLI 既不 fail 也不返回；codex 較不受影響但仍偶發 stall。Root cause 推測為 CLI argv 處理大 string 的 OS-level pipe buffering 與工具自己的 streaming 設計衝突。
decision: |
  在 `scripts/ai/dispatch.sh` (line 41-46 註解處) 把 auto-prepend 改為 opt-in：必須顯式 `AI_WITH_MEMORY=1 bash dispatch.sh ...` 才會 prepend memory block。預設關閉。Stage prompts 改為自帶必要 context（inline），多數 staged prompt 已這樣寫，影響面有限。
consequences: |
  - dispatch wall-clock 從 20+ min 降回正常 (~30s-5min)
  - 跨工具共享 memory 改為「stage prompts 自帶 context」+「operator 明確需要時才 set env var」兩條 path
  - 缺點：忘 set env var 時，CLI 拿不到 `.ai-shared/memory/index.md` 的最新筆記；mitigation: stage prompt template 內列出 prompt 應該帶哪些 anchor
  - 對 `scripts/ai/sync-memory.sh` 影響：仍維護 `.ai-shared/memory/`，但 propagation 不再自動
alternatives_considered:
  - name: 自動偵測 prompt size，小於 N KB 才 auto-prepend
    why_rejected: 增加複雜度；N 的 threshold 各工具不同；遇到 edge case 時 debugging 痛
    when_to_revisit: 若 staged prompt context 缺漏成為實際痛點
  - name: 在 dispatch.sh 中把 memory 寫到 temp 檔、用 file path 傳給 CLI
    why_rejected: 各 CLI argv 處理 file vs stdin vs inline 不一致；agy `-p` flag 只吃 inline；opencode `run` 也類似
    when_to_revisit: null
  - name: 維持 auto-prepend 但加 5min CLI timeout
    why_rejected: 仍浪費 5min；不能修 root cause；symptom 治療不可持續
    when_to_revisit: upstream CLI 改善 argv handling 後
supersedes: []
superseded_by: null
related_specs:
  - SPEC-09-FOUNDATION-decision-log
---

## Long-form rationale

詳見 `scripts/ai/dispatch.sh` line 41-46 inline comment。本 ADR 把該 inline 註解結構化、加 alternatives + status，成為 cross-spec 可引用的決策。
