---
id: ADR-002
date: 2026-05-24
status: accepted
title: agy CLI excluded from remote (Windows+SSH) dispatch; codex primary on remote
context: |
  在 Mac 端 agy (Antigravity / Gemini) 是 dispatch.sh 首選工具（Google free quota + 1M context + plan-mode 強）。但在遠端 Windows 機器經 SSH 跑 agy 時，碰到 antigravity-cli upstream open bug #76 + #102 — agy 在 Windows+SSH 下會 hang 20+ 分鐘無回應，無法 fail-fast 也無法手動中斷。重試 3 次同樣結果。詳見 `appendix/research-2026-05-24-agy-windows-ssh.md`。
decision: |
  在 `.ai-shared/tool-policy.md` §1 分 Mac 端與 Remote 端 dispatch order：
  - Mac 端: `agy → claude-subagent → codex → opencode`
  - Remote 端: `codex (--skip-git-repo-check) → opencode (free model) → claude (--print --allowedTools Bash)`，agy 顯式標 ❌
  `scripts/ai/tool-select.sh` 不偵測平台（操作者負責用對 wrapper script）；遠端 wrapper 自行覆寫 `AI_TOOL_OVERRIDE` 跳過 agy。
consequences: |
  - 遠端 dispatch 不再 hang 20+ 分鐘，恢復 wall-clock 預期
  - 但遠端失去 Gemini 1M context 與 plan-mode 優勢，wire-format/structural task 改靠 codex（已驗 100% hit rate，可接受）
  - 需維護兩份 dispatch 偏好（Mac vs Remote）— `.ai-shared/tool-policy.md` 已分節
  - 若 upstream antigravity-cli 修了 #76 + #102，本 ADR 可被 superseded
alternatives_considered:
  - name: 在所有平台繼續用 agy 並設 timeout 5min 後自動 fallback
    why_rejected: 仍浪費 5min wall clock；agy hang 時 child process 不清乾淨，留 zombie
    when_to_revisit: upstream 修 issue #76 + #102 後
  - name: 在遠端跑 agy 但用 Linux WSL 而非 native Windows
    why_rejected: 增加環境複雜度；WSL 與 native 之間 PATH/cred 同步麻煩；upstream 也標 WSL 仍可能踩坑
    when_to_revisit: null
  - name: 自製 agy wrapper 加 plan-mode bypass
    why_rejected: 維護自製 fork 成本高；plan-mode 是 upstream 的 design choice 不適合 patch
    when_to_revisit: null
supersedes: []
superseded_by: null
related_specs:
  - SPEC-09-FOUNDATION-decision-log
---

## Long-form rationale

詳見 `.ai-shared/tool-policy.md` §1 Remote 端順序表 + appendix `research-2026-05-24-agy-windows-ssh.md`。
