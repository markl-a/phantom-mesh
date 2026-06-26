---
name: generate-changelog
version: 0.1.0
description: 走訪兩個 ref（git 參考點，可為 tag、branch 或 SHA）之間的 git log，依 conventional-commit（慣例式提交）型別將 commit（提交）分組，並輸出一份可直接用於發行說明檔的 Markdown 變更紀錄。
triggers:
  - "generate a changelog"
  - "build release notes"
  - "what changed since the last release"
  - "draft changelog between tags"
  - "summarize recent commits"
tools:
  - shell
  - git_log
inputs:
  from_ref: "較舊的 ref（tag、branch 或 SHA）——變更紀錄不含此 commit。"
  to_ref: "較新的 ref。預設為 HEAD。"
  output_path: "Markdown 檔的寫入位置。預設為 CHANGELOG.draft.md。"
outputs:
  - "依 feat / fix / docs / chore / other 分組的 Markdown 變更紀錄"
  - "歸入每個段落的 commit 數量"
tags:
  - release
  - docs
  - git
created_at: "2026-05-15T00:00:00Z"
author: phantom-mesh weekend push T32
---

# generate-changelog

產生一份 Conventional-Commits（慣例式提交）風格的變更紀錄，涵蓋兩個 ref 之間落地的所有內容。此 skill（技能）為唯讀：它不會打 tag、不會 push（推送），也不會修改指定輸出路徑以外的任何檔案。

## Steps

在動手做任何事之前，先確認兩個 ref 都存在——若 ref 未知，`git rev-parse --verify`
會以非零碼結束，讓我們能乾淨地提早失敗。

```bash
git rev-parse --verify "${FROM_REF:-HEAD~50}^{commit}"
git rev-parse --verify "${TO_REF:-HEAD}^{commit}"
```

接著收集原始的 commit 清單。我們使用 `%h %s`（短 SHA + 主旨），讓
輸出每個 commit 一行、方便稍後分類。`--no-merges` 會把
merge（合併）泡泡排除在變更紀錄之外——它們幾乎不帶有用的訊息。

```bash
git log --no-merges --pretty=format:'%h %s' "${FROM_REF:-HEAD~50}..${TO_REF:-HEAD}" \
  > /tmp/phantom-skill-changelog-raw.txt
wc -l /tmp/phantom-skill-changelog-raw.txt
```

原始清單現在已存在磁碟上。後續每個步驟都能讀取它，而不必
重跑 `git log`——在很深的歷史上，重跑雖然便宜但並非免費。

依 Conventional Commit 型別將 commit 分類。我們用 grep 比對
我們關心的前綴 token（標記）；其餘一律歸入 `chore/other`。

```bash
{
  echo "## Features"
  grep -E "^\S+ feat(\(|:)" /tmp/phantom-skill-changelog-raw.txt || echo "_(none)_"
  echo
  echo "## Fixes"
  grep -E "^\S+ fix(\(|:)" /tmp/phantom-skill-changelog-raw.txt || echo "_(none)_"
  echo
  echo "## Docs"
  grep -E "^\S+ docs(\(|:)" /tmp/phantom-skill-changelog-raw.txt || echo "_(none)_"
  echo
  echo "## Other"
  grep -Ev "^\S+ (feat|fix|docs)(\(|:)" /tmp/phantom-skill-changelog-raw.txt || echo "_(none)_"
} > "${OUTPUT_PATH:-CHANGELOG.draft.md}"
```

## Prompt: Polish the changelog

讀取位於 `${OUTPUT_PATH:-CHANGELOG.draft.md}` 的檔案，並將每一行 commit
改寫成一句面向使用者（而非貢獻者）的話：

- 去掉慣例式提交前綴（`feat:`、`fix(scope):` 等）。
- 在可能的情況下，把內部用語（變數名、檔案路徑）替換成使用者看得到的
  功能名稱。
- 若某個 commit 主旨太簡短而無法轉述，就原封不動保留，並加上
  `TODO:` 標記，讓真人之後再過一遍。

保留段落標題與每行開頭的短 SHA。
不要捏造 commit 主旨中沒有的功能。

## Failure modes

- **任一 ref 無法解析：** 在寫入輸出檔之前中止——絕不
  輸出半空的變更紀錄。
- **範圍內沒有 commit：** 仍然寫出檔案，並以 `_(none)_` 佔位符填入，
  讓呼叫端能區分「執行成功、沒有可回報的內容」與
  「skill 當掉」。
- **輸出路徑是目錄或唯讀：** 原封不動地把作業系統錯誤呈現出來。
