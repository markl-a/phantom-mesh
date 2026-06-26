---
name: audit-deps
version: 0.1.0
description: 在 workspace（工作區）上執行 cargo audit 與 cargo outdated，接著輸出一份 Markdown 報告，依 severity（嚴重度）分組安全通報，並依落後程度分組過期的 crate（套件）。
triggers:
  - "audit dependencies"
  - "check for vulnerable crates"
  - "are any dependencies out of date"
  - "cargo audit report"
  - "security scan rust deps"
tools:
  - shell
  - file_read
inputs:
  workspace_dir: "指向 cargo workspace 根目錄的路徑。預設為目前的工作目錄。"
  fail_on: "會導致本 skill（技能）以非零碼結束的嚴重度等級：'critical'、'high'、'any'。預設為 'high'。"
outputs:
  - "依嚴重度（critical、high、medium、low）分組的 Markdown 報告"
  - "過期 crate 的表格，含目前版本對照最新版本，以及落後的 major-bump（主版本跳升）數量"
  - "結束狀態：依 fail_on 門檻判定為乾淨則回傳 0，否則回傳 1"
tags:
  - security
  - dependencies
  - rust
  - maintenance
created_at: "2026-05-15T00:00:00Z"
author: phantom-mesh weekend push T32
---

# audit-deps

這是一個雙頭並進的相依性檢查：`cargo audit` 檢查已解析 lockfile（鎖定檔）中已知的 CVE（公開揭露的資安漏洞），而 `cargo outdated` 檢查我們尚未跟進的上游版本。輸出為單一份 Markdown 報告，方便人類直接貼進 PR（拉取請求）描述中。

## 步驟

在做任何其他事情之前，先確認這兩個工具都已安裝。兩者都不屬於預設的 cargo 工具鏈，而執行到一半才出現缺少二進位檔（missing-binary）的錯誤，會產生令人困惑的不完整報告。

```bash
cargo audit --version
cargo outdated --version
```

先執行安全稽核。`--json` 會提供可供我們過濾與分組的機器可讀輸出；人類可讀的格式較難可靠地分桶歸類。

```bash
cargo audit --json > /tmp/phantom-skill-audit.json 2>&1 || true
echo "exit: $?"
```

這裡我們刻意容許非零的結束碼——`cargo audit` 在發現安全通報時會回傳 1，但我們仍希望繼續執行並收集過期報告。最終的綜合結束碼會在最後一個步驟決定。

接著檢查上游版本的偏移（drift）。`--workspace --root-deps-only` 讓報告聚焦於我們直接相依的 crate；transitive（傳遞性）偏移無論如何都會透過 audit 間接被捕捉到。

```bash
cargo outdated --workspace --root-deps-only --format json \
  > /tmp/phantom-skill-outdated.json 2>&1 || true
```

## Prompt：格式化相依性報告

依據上面寫出的兩個 JSON 檔案，產生一份具有以下結構的 Markdown 報告：

```
# Dependency report — <ISO date>

## Security advisories
- Critical (N)
  - crate-name@version — CVE-YYYY-NNNN — one-line summary
- High (N)
- Medium (N)
- Low (N)

## Outdated dependencies
| Crate | Current | Latest | Major bumps behind |
|-------|---------|--------|--------------------|
| ...   | ...     | ...    | ...                |

## Recommendation
<2-3 sentences: which advisories MUST be fixed before release,
which outdated crates are safe to defer, and any upgrades that
risk breaking changes>
```

使用 `fail_on` 這個輸入來決定建議的語氣：

- `fail_on=critical`：只在 critical（嚴重）通報時封鎖。
- `fail_on=high`（預設）：在 critical 或 high（高）通報時封鎖。
- `fail_on=any`：只要有任何通報就封鎖。

不要捏造輸入 JSON 中不存在的 CVE 編號。

## 失敗模式

- **未安裝 `cargo audit` 或 `cargo outdated`：** 及早中止，並附上安裝指令（`cargo install cargo-audit cargo-outdated`），讓使用者可以修正後重新執行。
- **離線（無法抓取通報資料庫）：** audit 會發出警告，且可能產生過時的結果。把這個但書呈現在報告的最上方。
- **Workspace 沒有 `Cargo.lock`：** 中止——在沒有 lockfile 的純函式庫（library-only）工作區上，兩個工具都無法產生有用的輸出。
