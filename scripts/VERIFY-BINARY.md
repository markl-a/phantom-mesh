# `verify-binary.{sh,ps1}` — spectyn binary 健康檢查

Health-check a `spectyn` binary to confirm it works. Two parity implementations:
- `verify-binary.sh` — bash / zsh(Linux, macOS, WSL, Git Bash on Windows)
- `verify-binary.ps1` — PowerShell 5.1+(Windows)

兩者跑同一組 check + 接同一組參數。

## 為什麼

當你 build 出一個 target 的 binary(Win / Linux / Mac / Android),希望**一個指令就能回答「這顆 binary 還活著嗎」**,不用每個 OS 記 5 個不同的驗證命令。

這是 `goal_plan/docs/29 §4 V1-V11 驗收矩陣`的**執行端** — 矩陣判定「哪些 track 必須綠 = 可 ship」,本 script 判定「這顆 binary 還活著沒」。

## Check 清單

| # | Check | `--quick` 跳? | 需 LLM key? |
|---|---|---|---|
| 1 | binary 檔存在 + 可執行 | — | no |
| 2 | `spectyn --version` exit 0 | — | no |
| 3 | `spectyn --version --short` 為 SemVer | — | no |
| 4 | 若給 `--expect-version`,要相符 | — | no |
| 5 | `spectyn doctor` exit 0 | ✓ | no |
| 6 | `spectyn doctor --json` 可解析為 JSON | ✓ | no |
| 7 | `spectyn selftest --p0-only` exit 0 | ✓ | **yes**(僅 `--full`)|

## 3 個模式

| 模式 | Checks | 用途 |
|---|---|---|
| `--quick` | 1-4 | CI smoke;不需設定 env |
| (預設) | 1-6 | 本機 dev 驗證 |
| `--full` | 1-7 | release candidate;需 provider key |

## 用法

### Bash / Git Bash / WSL

```bash
# 基本
./scripts/verify-binary.sh /usr/local/bin/spectyn

# CI 模式(沒 doctor / 沒 LLM)
./scripts/verify-binary.sh ./target/release/spectyn --quick

# 嚴格版本比對
./scripts/verify-binary.sh ./target/release/spectyn --expect-version 0.6.0

# 含 selftest 的完整檢查
./scripts/verify-binary.sh /usr/local/bin/spectyn --full

# 機讀格式
./scripts/verify-binary.sh /usr/local/bin/spectyn --json
```

### PowerShell

```powershell
# 基本
.\scripts\verify-binary.ps1 -BinaryPath C:\Users\me\.cargo\bin\spectyn.exe

# CI 模式
.\scripts\verify-binary.ps1 -BinaryPath .\target\release\spectyn.exe -Quick

# 嚴格版本比對
.\scripts\verify-binary.ps1 -BinaryPath .\target\release\spectyn.exe -ExpectVersion 0.6.0

# 含 selftest 的完整檢查
.\scripts\verify-binary.ps1 -BinaryPath C:\Users\me\.cargo\bin\spectyn.exe -Full

# 機讀格式
.\scripts\verify-binary.ps1 -BinaryPath C:\Users\me\.cargo\bin\spectyn.exe -Json
```

## Exit codes

| Exit | 意義 |
|---|---|
| 0 | 所有 non-skipped check 通過 |
| 1 | 一個以上 check 失敗 |
| 2 | 參數錯誤(如沒給 binary path) |

## 範例輸出(人類版)

```text
spectyn verify-binary 0.1.0
  binary:   /usr/local/bin/spectyn
  duration: 3s

  ✓ binary_exists_executable    /usr/local/bin/spectyn
  ✓ version_runs                spectyn 0.6.0 (a1b2c3d, x86_64-unknown-linux-gnu, built 2026-05-18)
  ✓ version_short_semver        0.6.0
  ∘ version_match_expected      no --expect-version given
  ✓ doctor_runs                 exit 0; 42 lines
  ✓ doctor_json_parseable       valid JSON
  ∘ selftest_p0                 --full not given

PASS: 5/5 checks passed (2 skipped)
```

## 範例 JSON 輸出

```json
{
  "script": "verify-binary.sh",
  "script_version": "0.1.0",
  "binary": "/usr/local/bin/spectyn",
  "duration_seconds": 3,
  "summary": { "pass": 5, "fail": 0, "skip": 2 },
  "exit_code": 0,
  "checks": [
    { "name": "binary_exists_executable", "status": "pass", "detail": "/usr/local/bin/spectyn" },
    { "name": "version_runs",             "status": "pass", "detail": "spectyn 0.6.0 ..." },
    { "name": "version_short_semver",     "status": "pass", "detail": "0.6.0" },
    { "name": "version_match_expected",   "status": "skip", "detail": "no --expect-version given" },
    { "name": "doctor_runs",              "status": "pass", "detail": "exit 0; 42 lines" },
    { "name": "doctor_json_parseable",    "status": "pass", "detail": "valid JSON" },
    { "name": "selftest_p0",              "status": "skip", "detail": "--full not given" }
  ]
}
```

## 整合到 CI

`pr-build-matrix.yml`(等 GH Actions billing 修復後再 enable `pull_request:` trigger)每 target 加最後一步:

```yaml
- name: Verify binary
  run: bash scripts/verify-binary.sh ./target/${{ matrix.target }}/release/spectyn --quick --json
  shell: bash

# Windows runner:
- name: Verify binary (Windows)
  run: pwsh scripts\verify-binary.ps1 -BinaryPath .\target\${{ matrix.target }}\release\spectyn.exe -Quick -Json
  shell: pwsh
```

## 已知限制

- **`spectyn doctor --json` 在 0.4.0 是 stub**(印彩色文字,忽略 `--json` flag)。Script 寬鬆處理:這項只顯示 `skip` 不算 `fail`。從 0.5.0 起應該真的吐 JSON。
- **`spectyn selftest --p0-only` 走 agent → 需要 `~/.spectyn-mesh/env` 或 `agents.toml` 設好 LLM provider key**。除非你把 secret 接到 CI 否則別在 CI 用 `--full`。
- **Windows 用 bash 版**:能用(透過 Git Bash),但 path 參數要用 forward slash 或 escaped backslash。
- **沒測網路存取**:`spectyn serve` 真的能起 + healthz 回 200 OK 沒在這檢查範圍;這是 §29 §4 V-track 矩陣的事。

## 相關文件

- `goal_plan/docs/29-spectyn-mesh-testing-framework-2026-05-18.md` §4 — V1-V11 GA 驗收矩陣(本 script 是執行端)
- `goal_plan/docs/30-spectyn-mesh-user-flows-per-platform-2026-05-18.md` §1.4 — 完整 CLI 命令詞典含 `spectyn doctor`, `spectyn selftest`, `spectyn --version`
- `scripts/_verify-download.sh` — 不同 scope:那邊驗 SHA256 download integrity,這邊驗 binary functional health
