---
name: rebase-onto-main
version: 0.1.0
description: 帶領一個功能分支（feature branch）完成乾淨的 rebase（重定基底）到 main，以保守的預設值解決常見衝突。
triggers:
  - "rebase my branch"
  - "update branch from main"
  - "fix merge conflicts after pulling main"
tools:
  - shell
  - git_status
  - git_diff
  - git_log
inputs:
  branch: "要 rebase 的本機分支名稱。預設為目前的 HEAD。"
  base: "要 rebase 到的目標分支。預設為 'main'。"
outputs:
  - "rebase 完成、在 {base} 之上具有線性歷史（linear history）的分支"
  - "列出需要手動解決衝突之檔案的衝突報告（conflict report）"
tags:
  - git
  - housekeeping
created_at: "2026-05-15T00:00:00Z"
author: spectyn-mesh weekend push H2
---

# rebase-onto-main

使用非互動式（non-interactive）的 rebase，讓一個功能分支跟上 `main` 的最新狀態。本
skill（技能）採取保守做法：當某個衝突無法用已記載的啟發式規則（heuristics，經驗法則）
自動解決時，它會停下來並把發生衝突的路徑顯示出來，而不是用猜的。

## 步驟

1. `git fetch origin`
2. 確認工作樹（working tree）是乾淨的。若不是，則以自動產生的標籤進行 stash（暫存），
   並把該 stash 的參照（stash ref）記錄到衝突報告中。
3. `git rebase origin/{base}`（預設 base = `main`）。
4. 發生衝突時，依序嘗試下列啟發式規則：
   - lockfile（鎖定檔）變動（`Cargo.lock`、`package-lock.json`）：採用 `--theirs`，
     接著重新執行套件管理器（package manager）並將結果加入暫存區（stage）。
   - 在某個清單／陣列的兩側都是純新增（pure additions）：兩者都保留，並依字典序（lexical order）排列。
   - 其他任何情況：停止，回傳衝突報告。
5. 若步驟 2 有設定 stash ref，則執行 `git stash pop` 並重新加入暫存區。

## 失敗模式

- **進入時處於分離 HEAD（detached HEAD）狀態：** 中止，回傳錯誤。
- **rebase 進行中上游被改寫（upstream rewrites）：** 中止此次 rebase，回傳原始的 SHA。
- **工具缺失（沒有 `git`）：** 回傳清楚的「缺少相依套件（missing dependency）」錯誤。
