# 在 Windows 11 上安裝 phantom-mesh

已在 **Windows 11 23H2 + 24H2** 上原生測試（非 WSL）。建議使用 PowerShell 7
而非內建的 5.1，因為部分腳本會用到 `?.`
運算子（operator，運算子）以及結構化錯誤解析（structured-error parsing）。

若要用 WSL2：請改依[Linux 指南](INSTALL-LINUX.md)安裝 —— phantom
在 WSL 內可原生執行，你只會失去 `phantom service install` →
排程工作（Scheduled Task）這條路徑（WSL 無法存取 Windows 工作排程器）。

---

## 懶人包 —— 90 秒（原生 Windows）

```powershell
# In an elevated PowerShell (Run as Administrator) for the install step,
# then drop back to normal user shell for everyday use.

# 1. Install dependencies via winget
winget install --silent Git.Git Rustlang.Rustup tailscale.tailscale

# 2. Refresh PATH (or close + reopen PowerShell)
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Program Files\Git\cmd;$env:PATH"

# 3. Build phantom
cd $env:USERPROFILE\Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh\core
cargo install --path . --locked
phantom --version

# 4. First-time wizard
phantom onboarding

# 5. Start serve (in a separate PowerShell so it stays running)
Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden

# 6. Verify
phantom doctor
Start-Process "http://127.0.0.1:7878/projects"
```

---

## 捷徑：跨平台編譯的執行檔（cross-compiled binary，跳過 cargo build）

如果你有一台帶原始碼的 Mac/Linux 機器，可以在那台機器上
**跨平台編譯**（cross-compile）出 Windows 執行檔（比第一次在
Windows 上建置更快）：

在 Mac 上：
```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
cd ~/Documents/phantom-mesh/core
cargo build --release --target x86_64-pc-windows-gnu --bin phantom
# produces target/x86_64-pc-windows-gnu/release/phantom.exe (~33 MB)
```

接著把該 `.exe` 傳到 Windows（SCP / SMB / `python -m http.server`）
並放到 `$env:USERPROFILE\AppData\Local\Programs\phantom-mesh\phantom.exe`。

本倉庫裡的 `scripts/setup-node-a.ps1` 腳本會把整套流程自動化
—— 詳見檔案頂端的註解。

---

## 前置需求（Prereqs）

| 元件 | 用途 | 安裝方式 |
|---|---|---|
| Rust 工具鏈 ≥ 1.80 | 建置 phantom | `winget install Rustlang.Rustup` |
| `git`                 | 複製（clone）倉庫 | `winget install Git.Git` |
| Tailscale（可選）  | 跨機叢集（cluster）＋手機存取 | `winget install tailscale.tailscale` |
| PowerShell 7（建議） | 部分腳本使用較新語法 | `winget install Microsoft.PowerShell` |

---

## 詳細安裝步驟

### 1. 開啟 PowerShell（安裝時用系統管理員權限，日常使用用一般權限）

```powershell
# Start menu → search "PowerShell" → "Run as Administrator"
$PSVersionTable.PSVersion   # confirm 7.x
```

### 2. winget 安裝

```powershell
winget install --silent Git.Git
winget install --silent Rustlang.Rustup
winget install --silent tailscale.tailscale
winget install --silent --id Microsoft.PowerShell

# Close + reopen PowerShell so PATH is fresh
```

### 3. 建置 phantom

```powershell
cd $env:USERPROFILE\Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh\core
cargo install --path . --locked
phantom --version
```

`cargo install` 會把 `phantom.exe` 放在
`$env:USERPROFILE\.cargo\bin\phantom.exe`。Rustup 安裝程式會把
這個路徑加進 PATH；若沒有，請重新開啟 PowerShell。

### 4. 首次導引（onboarding）

```powershell
phantom onboarding
```

90 秒互動式精靈（wizard）。會寫出 `$env:USERPROFILE\.phantom-mesh\agents.toml`。

### 5. 啟動 phantom serve

phantom 目前還沒有原生的 Windows 服務安裝器（已規劃中）。
現階段有兩個選擇：

**選項 A：背景行程（重開機後需手動重啟）**
```powershell
Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden
```

**選項 B：登入時排程工作（Scheduled Task at logon，自動啟動）**
```powershell
$action  = New-ScheduledTaskAction -Execute "phantom" -Argument "serve"
$trigger = New-ScheduledTaskTrigger -AtLogon
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName "phantom-serve" -Action $action -Trigger $trigger -Settings $settings
Start-ScheduledTask -TaskName "phantom-serve"
```

要停止：
```powershell
Get-Process phantom | Stop-Process
Unregister-ScheduledTask -TaskName "phantom-serve" -Confirm:$false
```

### 6.（可選）每小時自動演化（autoevolve）

```powershell
phantom autoevolve schedule install --interval 3600
```

如果訊息顯示 "macOS + Windows only" 就表示設定成功；如果報錯，
就退而求其次手動建立排程工作：
```powershell
$action  = New-ScheduledTaskAction -Execute "phantom" -Argument "autoevolve --once"
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Hours 1)
Register-ScheduledTask -TaskName "phantom-autoevolve" -Action $action -Trigger $trigger
```

### 7.（可選）叢集（cluster）

編輯 `$env:USERPROFILE\.phantom-mesh\agents.toml`：

```toml
[cluster]
node_name      = "win-1"
cluster_secret = "<same secret across nodes>"
peers = [
  "http://<mac-tailscale-ip>:7878",      # mac (over Tailscale)
  "http://100.64.0.10:7878",    # other windows
]
```

接著執行 `tailscale up` 並驗證：
```powershell
Invoke-RestMethod http://<mac-tailscale-ip>:7878/healthz
```

---

## 驗證

### 1. 快速健康檢查

```powershell
phantom doctor
```

`phantom doctor` 在 Windows 上會跑 11 個彩色分區
（binary、config、permissions、provider keys、phantom serve、
Scheduled Task、network、autoevolve、identity、diagnostics、tools）。
每一行都應該是綠色的 `✓` 或黃色的 `⚠`。對於你尚未啟用的
功能，出現 `⚠` 是正常的（未使用的供應商金鑰、尚未執行的
autoevolve）。紅色的 `✗` 行才需要修正。

**健康的 Windows 安裝預期輸出：**

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, windows-x86_64, built 2026-05-11)
  ✓ path: C:\Users\you\.cargo\bin\phantom.exe

config
  ✓ agents.toml: C:\Users\you\.phantom-mesh\agents.toml
  ✓ ~/.phantom-mesh: exists

permissions
  ⚠ [permissions]: no rules → allow all (legacy default).
                    See docs/PERMISSIONS.md for the Tool(specifier) DSL.

provider keys
  ⚠ Anthropic: not in env or agents.toml
  ✓ Groq: env (gsk_L1…)
  ✓ Gemini: agents.toml
  ⚠ DeepSeek: not in env or agents.toml

phantom serve
  ✓ healthz: 200 OK on http://127.0.0.1:7878/healthz
  ✓ Scheduled Task: registered (last run ?)

network
  ⚠ Tailscale: not in PATH or not connected — `tailscale up`

autoevolve
  ⚠ history: no runs yet — `phantom autoevolve --once`
  ⚠ schedule: not scheduled — `phantom autoevolve schedule install`

identity
  ✓ identity: local-only (broker not deployed yet — login becomes available
              once phantommesh.io/healthz returns 200)

diagnostics
  ✓ crash logs: 0 (no panics recorded)
  ✓ events log: C:\Users\you\.phantom-mesh\events.jsonl (0 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

done.
```

**首次安裝時需留意的 ⚠ 行**（正常，不是錯誤）：
- `Anthropic: not in env` —— 你在導引時沒有選用它；
  如有需要，把 `ANTHROPIC_API_KEY` 加進環境變數或 `agents.toml`
- `Tailscale: not in PATH or not connected` —— 從具系統管理員權限的
  PowerShell 執行 `tailscale up`
- `autoevolve/history: no runs yet` —— 首次執行前屬正常；
  用 `phantom autoevolve --once` 修正
- `autoevolve/schedule: not scheduled` —— 若你略過該步驟則屬正常
- `identity: local-only (broker not deployed)` —— 屬正常；
  位於 phantommesh.io 的 broker（中介伺服器）尚未上線

**需要修正的紅色 ✗ 行：**
- `agents.toml: not found` → 執行 `phantom onboarding`
- `healthz: unreachable` → 啟動 phantom serve 或檢查 7878 連接埠
- `Scheduled Task: not installed` → 執行上面安裝章節中的
  排程工作（Scheduled Task）指令
- `Tailscale: not in PATH or not connected` → `tailscale up`

若要取得機器可讀的輸出：

```powershell
phantom doctor --json | ConvertFrom-Json | Select-Object status
phantom doctor --json | ConvertFrom-Json | Select-Object -ExpandProperty serve
phantom doctor --json | ConvertFrom-Json | Select-Object -ExpandProperty autoevolve
```

### 2. 開啟儀表板（dashboard）

```powershell
Start-Process "http://127.0.0.1:7878/projects"
```

應該會顯示 6 個專案磚（project tiles）＋叢集狀態＋最近活動。
每個 [Run Demo] 都會透過 SSE（Server-Sent Events，伺服器推送事件）即時串流輸出。

### 3. 功能掃描（feature sweep）

```powershell
phantom selftest                # 22+ feature checks
phantom selftest --p0-only      # critical checks only, ~4 s
```

---

## 與 Claude Code 的 MCP 整合

```powershell
claude mcp add phantom (Get-Command phantom).Source mcp
```

完成後，Claude Code 的工具面板（tool palette）會新增 `mcp__phantom__*` 工具。

---

## 更新

```powershell
cd $env:USERPROFILE\Documents\phantom-mesh
git pull
cd core
cargo install --path . --locked
Stop-ScheduledTask -TaskName "phantom-serve"
Start-ScheduledTask -TaskName "phantom-serve"
```

---

## 解除安裝

```powershell
Unregister-ScheduledTask -TaskName "phantom-autoevolve" -Confirm:$false -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "phantom-serve"      -Confirm:$false -ErrorAction SilentlyContinue
Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force
cargo uninstall phantom-mesh
Remove-Item -Recurse -Force $env:USERPROFILE\.phantom-mesh
Remove-Item -Recurse -Force $env:USERPROFILE\AppData\Local\Programs\phantom-mesh
```

---

## 疑難排解（Troubleshooting）

### `phantom doctor` 快速分流

執行 `phantom doctor`，並依以下順序找出失敗點：

| `phantom doctor` 行 | 原因 | 修正方式 |
|---|---|---|
| `✗ agents.toml: not found` | 尚未執行導引 | `phantom onboarding` |
| `✗ healthz: unreachable` | serve 沒有在執行 | `Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden` |
| `⚠ autoevolve/history: no runs yet` | 從未跑過第一次 | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | 尚未安裝排程 | `phantom autoevolve schedule install` |
| `⚠ Tailscale: not in PATH` | Tailscale 未安裝 | `winget install tailscale.tailscale` |
| `⚠ Tailscale: not connected` | 尚未登入 | 從具系統管理員權限的 PowerShell 執行 `tailscale up` |
| `⚠ crash logs: N recorded` | 最近有一次代理（agent）執行崩潰 | `phantom debug last` 讀取最新一筆 |
| `⚠ identity: local-only` | 屬正常（broker 尚未部署） | 無需修正 —— 這是正常狀態 |
| `⚠ events.jsonl: 0 bytes` | 首次執行屬正常 | 無需修正 —— 這是正常狀態 |
| `✗ [permissions]: parse error` | `agents.toml [permissions]` 區塊有語法錯誤 | 檢查 docs/PERMISSIONS.md 中的 DSL |

### 其他 PowerShell 層級的失敗

| 症狀 | 修正方式 |
|---|---|
| `cargo install` 因找不到 `link.exe` 而失敗 | 安裝 MSVC Build Tools：`winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| 安裝後出現 `phantom: command not found` | 重新開啟 PowerShell —— Rustup 要在重啟後才會加入 `~/.cargo/bin` |
| Defender 防火牆封鎖 7878 連接埠 | 新增輸入規則：`New-NetFirewallRule -DisplayName "phantom serve" -Direction Inbound -LocalPort 7878 -Protocol TCP -Action Allow` |
| `Scheduled Task` 無法啟動 | 工作排程器 GUI → 在工作上按右鍵 → 執行；檢查「上次執行結果」（Last Run Result） |
| `phantom autoevolve schedule install` 顯示 "macOS + Windows only" | 這正是你想看到的訊息 —— 用 `Get-ScheduledTask -TaskName "*phantom*"` 確認 |
| Tailscale GUI 顯示 "Logged out" | 開啟 Tailscale 系統匣（tray）圖示 → Log In；或從具系統管理員權限的 PowerShell 執行 `tailscale up` |
| `git clone` 失敗並出現 "Unable to find remote helper for 'https'" | `winget install Git.Git` —— 必須是官方的 Git for Windows，而非 WSL 版 |
| `phantom doctor` 輸出亂碼（ANSI 碼） | PowerShell 5.x 無法渲染 ANSI —— 升級到 PowerShell 7（`winget install Microsoft.PowerShell`） |

---

## 效能基準（參考機 node-a：i9-13900H，32 GB RAM，Win 11 24H2）

| 操作 | 時間 |
|---|---|
| 全新 `cargo install --path .`（首次建置） | ~3-4 分鐘 |
| 改動 1 個檔案後的增量重建 | 3-6 秒 |
| 下載跨平台編譯的 .exe（從 Mac 經 Tailscale 100 Mb/s） | 33 MB 約 ~3 秒 |
| `phantom doctor` 冷啟動 | ~1 秒 |
| `phantom selftest --p0-only` | ~4 秒 |
| HTTP `/api/projects` 冷啟動 | < 60 ms |

---

## 配套腳本：`scripts/setup-node-a.ps1`

若要將一台全新的 Win 11 機器免手動地引導（bootstrap）為叢集節點，
本倉庫附帶 `scripts/setup-node-a.ps1`。它會：

1. 驗證 Tailscale 已連線
2. 拉取 phantom.exe（從本機路徑或 HTTP URL）
3. 寫出一份合理的 `agents.toml`，並預先設定好 peers
4. 複製全部 6 個生態系倉庫
5.（可選）為資料分析（Data-Analysis）示範安裝 streamlit
6. 每小時排程 autoevolve
7. 在背景啟動 phantom serve

用法：
```powershell
cd phantom-mesh\scripts
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
.\setup-node-a.ps1 -PhantomBinarySource C:\path\to\phantom.exe -NodeName mywin
```

所有參數請見腳本的 docstring。
