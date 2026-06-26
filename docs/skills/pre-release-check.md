---
name: pre-release-check
version: 0.1.0
description: 複合式發布前關卡（gate）—— 在授權發布前，驗證工作樹（working tree）乾淨、測試通過、Cargo.toml 內版本號已遞增（bumped），以及對應的 git 標籤（tag）已存在。
triggers:
  - "pre-release check"
  - "is this ready to release"
  - "release readiness"
  - "can we cut a release"
  - "verify release gate"
tools:
  - shell
  - git_status
  - git_log
  - file_read
inputs:
  expected_version: "此次發布所採用的 Semver（語意化版本）字串（例如 '0.6.0'）。必填。"
  tag_prefix: "標籤時加在版本號前方的前綴（prefix）。預設為 'v'（即 v0.6.0）。"
  workspace_crate: "要檢查版本的 Cargo.toml 路徑。預設為 core/Cargo.toml。"
outputs:
  - "四個關卡各自的通過／失敗狀態"
  - "整體是否可發布的布林值（boolean）"
  - "第一個失敗的關卡，附帶修補提示（remediation hint）"
tags:
  - release
  - ci
  - composite
  - gating
created_at: "2026-05-16T00:00:00Z"
author: phantom-mesh weekend push T32
---

# pre-release-check

一個複合式關卡（gate），串接四個成本較低的檢查。任何一項失敗都會讓最終建議
短路（short-circuit），但每項檢查仍會全部執行，讓呼叫者看到完整全貌，而不是
修好一個問題後在重跑時才發現下一個。

此技能（skill）是唯讀（read-only）的：它絕不會自行打標籤、推送（push）或遞增版本號。
那些都是明確的人為決策，存放在獨立的 `cut-release` 技能中。

## Gate 1: clean working tree（工作樹乾淨）

從髒的工作樹（dirty tree）建置出的發布版本是無法重現（unreproducible）的，我們拒絕授權這種發布。
只有當未追蹤檔案（untracked files）符合標準忽略樣式（build artifacts、IDE 設定）時才會被容忍——
在 `.gitignore` 設定正確的情況下，`git status --porcelain` 已經會過濾掉它們。

```bash
git status --porcelain
test -z "$(git status --porcelain)" && echo "GATE1_CLEAN" || echo "GATE1_DIRTY"
```

## Gate 2: tests pass（測試通過）

我們執行整個工作區（workspace）的測試套件。這個複合關卡刻意「不」委派給
`run-tests` 技能——我們希望此關卡自我完備（self-contained），讓呼叫者能夠
精確稽核（audit）究竟是什麼條件把關了這次發布。

```bash
cargo test --workspace --no-fail-fast --quiet \
  && echo "GATE2_PASS" \
  || echo "GATE2_FAIL"
```

## Gate 3: version is bumped（版本號已遞增）

從選定的工作區 crate 的 Cargo.toml 讀取版本號，並與 `expected_version` 比對。
為了相容各種格式化工具（formatters），我們接受 `version = "x.y.z"` 或
`version="x.y.z"` 任一形式。

```bash
CRATE_PATH="${WORKSPACE_CRATE:-core/Cargo.toml}"
ACTUAL=$(grep -E '^version\s*=' "$CRATE_PATH" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
echo "actual=$ACTUAL expected=${EXPECTED_VERSION}"
test "$ACTUAL" = "${EXPECTED_VERSION}" && echo "GATE3_MATCH" || echo "GATE3_MISMATCH"
```

## Gate 4: matching tag exists（對應標籤已存在）

發布標籤必須已存在於本機。推送標籤是另一個獨立的人為步驟；我們只驗證該產出物
（artifact）已就位，這樣就絕不會授權一個還得臨時憑空捏造標籤的發布。

```bash
TAG="${TAG_PREFIX:-v}${EXPECTED_VERSION}"
git rev-parse --verify "refs/tags/${TAG}" >/dev/null 2>&1 \
  && echo "GATE4_TAG_PRESENT (${TAG})" \
  || echo "GATE4_TAG_MISSING (${TAG})"
```

## Prompt: Render the release-readiness verdict（產出發布就緒裁決）

根據上方輸出的四行 GATEn_* 哨兵標記（sentinel）行，請完全依照以下範本產出最終報告：

```
# Pre-release check — <EXPECTED_VERSION>

| Gate | Status |
|------|--------|
| 1. Clean working tree | <PASS|FAIL> |
| 2. Tests pass         | <PASS|FAIL> |
| 3. Version bumped     | <PASS|FAIL> |
| 4. Tag exists         | <PASS|FAIL> |

Verdict: <READY TO RELEASE | NOT READY>

Next step: <one sentence — the first failing gate's remediation,
            or "run cut-release" if all gates pass>
```

缺少哨兵標記（sentinel）即視為該關卡 FAIL。若每個關卡皆為 PASS，裁決即為
READY TO RELEASE，下一步為 `run cut-release`。否則，下一步的句子只能引用「第一個」
失敗的關卡——使用者應先修好那一個、重跑，然後再往下處理。

## Failure modes（失敗情境）

- **PATH 上沒有 `cargo`：** Gate 2 失敗，並給出指向 rustup 的修補提示；
  關卡 1、3、4 仍會回報，讓使用者取得完整全貌。
- **未提供 `expected_version`：** 在執行任何關卡前即中止——沒有目標版本的話，
  輸出將毫無意義。
- **標籤存在但「不」指向 HEAD：** 該關卡仍會通過（因為標籤確實存在），
  但提示應在「Next step」行中提及此 SHA 不符（mismatch），好讓人類注意到。
