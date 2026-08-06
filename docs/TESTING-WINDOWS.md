# Windows 二進位檔的端對端測試

如何驗證全新建置的 `spectyn.exe` 沒有讓 2026-05-01 node-a 強化整修（hardening sweep）所修的 18 個修正出現任何回歸（regression，舊功能再度故障）。完整測試套件位於
`scripts/test-windows.ps1` — 本文件說明每個階段（phase）涵蓋什麼、
通過時應該長什麼樣，以及如何解讀失敗訊息。

## TL;DR（重點摘要）

```powershell
$env:OPENROUTER_API_KEY = 'sk-or-v1-...'
.\scripts\test-windows.ps1
```

輸出結尾會是下列其中之一：

```
ALL CLEAR    -> exit 0, ship it
FAILED       -> exit code = number of failed phases, investigate
```

每個階段：`[PASS]`（綠色）、`[SKIP]`（黃色 — 環境限制
或加了 `-Skip*` 旗標）、`[FAIL]`（紅色 — 真正的回歸）。

### 已知的執行器（runner）注意事項（Windows PowerShell 5.1）

這個測試執行器在 PS 5.1 上只能做到盡力而為（best-effort），因為有兩個眾所周知、
語言本身無法讓我們乾淨地避開的怪異行為（quirk）：

1. **OpenRouter 免費方案（free-tier）的速率限制。** 階段 3 會連續發出兩次
   真實的 LLM（大型語言模型）呼叫。如果你一直用同一把金鑰（key）從另一個
   shell 猛打，第二次呼叫可能會回 429，斷言（assertion）就會抓不到
   `$0.0000`。等 60 秒後用 `-Phase 3` 重跑。
2. **`cmd /c` 引號巢狀（quote nesting）。** 任何在提示詞（prompt）裡內嵌雙引號的內容
   都必須以單一個經過 shell 跳脫（shell-escaped）的字串傳入。
   執行器因此使用單字提示詞（`hi`、`hello`）。
   不要把它們改成多字片語。
3. **`evolve` 的工作目錄（cwd）相依性。** 階段 6 會先 cd 進 `core/` 再
   呼叫 `spectyn evolve`，因為代理人（agent）是透過 shell 工具以相對路徑
   執行 `cargo test`，而它需要上一層目錄裡的 `Cargo.toml`。
   少了這步，evolve 會以 `stopped after N
   rounds` 結束，而不是 `EVOLVE_DONE`。

如果某個階段在你已手動驗證過的乾淨二進位檔上仍間歇性失敗，
請把執行器輸出當成煙霧訊號（smoke signal，初步警示）而非最終判決 —
本文件其餘部分的手動流程才是權威的真相來源。

## 先決條件（Pre-requisites）

| 項目 | 如何檢查 |
|---|---|
| `spectyn` 在 PATH 上 | `Get-Command spectyn` 解析到 `~\.local\bin\spectyn.exe` |
| 已部署最新二進位檔 | `spectyn --version` 與 `git log -1 --format='%h'` 相符（若有未提交變更則帶 `+`） |
| 已設定 OpenRouter 環境變數 | `$env:OPENROUTER_API_KEY` 非空（階段 3、6 需要） |
| 沒有 `spectyn serve` 已在執行 | 執行器會自行用自己的連接埠 kill 並重啟 |
| 已移除管理員遺留的 `SpectynServe` 工作 | 見[下方 §4.1](#41-管理員遺留工作會擋住階段-4) — 僅影響階段 4 |

## 八個階段

### 階段 1 — 起飛前準備（Pre-flight setup）

停掉任何殘留的 `spectyn.exe`、確認 PATH、檢查 `OPENROUTER_API_KEY`。
很快。如果這步失敗，後面全部都跑不動。

### 階段 2 — 唯讀煙霧測試（Read-only smoke）

碰觸每一個不會變更狀態的頂層介面：

- `spectyn --version` → 預期 `spectyn 0.4.0 (<sha>+, windows-x86_64, ...)`
- `spectyn doctor` → 預期出現 `configured port` 那一行，且 `provider keys` 中列出 `OpenRouter`
- `spectyn mcp` initialize + tools/list → 預期 ≥40 個工具（目前 49 個）
- `spectyn self-update --dry-run` → 預期目標為 `spectyn-x86_64-pc-windows.exe`
- `spectyn mlx status` / `spectyn snapshot apply` → 預期出現優雅的「僅限 Apple」拒絕訊息

如果 `doctor` 顯示 `OpenRouter: not in env or agents.toml`，表示你的 shell 沒有
看到環境變數 — 請重啟 PowerShell 或設定 User 範圍（User-scope）的登錄檔（registry）值。

### 階段 3 — LLM 往返（master + coder 經由 OpenRouter）

對 `openrouter.ai` 的真實網路呼叫。使用免費 Llama 方案，所以成本是 `$0.0000`。

- `spectyn -c "..."` master 代理人 → 預期輸出橫幅（banner）中出現 `$0.0000`。
- `spectyn -c --agent coder "..."` coder 代理人 → 同上。
- MCP `tools/call shell { cwd: "~" }` 回歸測試 → 預期 `home_test\r\n[exit code: 0]`，且**不**出現 `cwd '~' does not exist` 錯誤。這是 commit `7752330` 修掉的波浪號展開（tilde-expansion）錯誤。

如果 LLM 呼叫以 `404 model … not found` 失敗，表示你的 `agents.toml` 供應商（provider）路由錯誤（模型名稱屬於另一個供應商）。階段 2 的 `doctor` `provider keys` 區段應該已經抓到這個問題。

### 階段 4 — `spectyn service install/status/uninstall`

對 **SpectynServe** 排程工作（Scheduled Task）做往返測試：

- 透過 `Register-ScheduledTask -AtLogOn -User $env:USERNAME` 安裝（用 PowerShell，而非 `schtasks /SC ONLOGON` — 見 commit `8259330`）。
- 為設定的連接埠加上 Defender 防火牆規則（盡力而為 — 需要管理員權限）。
- 一旦 spectyn serve 啟動，status 會回報 `registered: yes`、`last run: <timestamp>`、`last state: running`。
- uninstall 會移除工作 + 殺掉 spectyn.exe + 刪掉防火牆規則。

#### 4.1 管理員遺留工作會擋住階段 4

如果先前某次**提權（elevated）**安裝留下了 `SpectynServe` 工作，你的
使用者層級（user-level）`spectyn service install` 會回傳：

```
PermissionDenied (HRESULT 0x80070005): Register-ScheduledTask
```

階段 4 會偵測到這個狀況並**跳過（skip）**，附上清楚的下一步。要清除：

```powershell
# from an elevated (admin) PowerShell
Unregister-ScheduledTask -TaskName 'SpectynServe' -Confirm:$false
```

如果連這樣都回傳 Access Denied：

```powershell
schtasks /Delete /TN "SpectynServe" /F          # admin cmd / PowerShell
```

或開啟 `taskschd.msc` → Task Scheduler Library → 在 `SpectynServe` 上按右鍵 → Delete。

再從你正常的（非管理員）shell 重跑階段 4：

```powershell
.\scripts\test-windows.ps1 -Phase 4
```

### 階段 5 — `spectyn autoevolve schedule install/status/uninstall`

對 **SpectynAutoevolve** 工作做往返測試。名稱與 SpectynServe 不同，所以不必擔心管理員遺留問題。確認：

- 間隔（interval）從 XML 算出後呈現為 ISO 8601（`PT1H`），而非在地化（localized）字串。
- `last state: never run` 正確顯示（Windows pre-2000 佔位符已在伺服器端過濾掉 — commit `8259330`，之後再精修）。

### 階段 6 — `autoevolve --once` + `evolve --max-rounds 1`

真實的 cargo 流程（pipeline）+ 真實的 LLM：

- `autoevolve --once --target check` 應印出 `cargo check green — nothing to evolve.`。這證明 commit `4d3f78d` 的 `CARGO_TARGET_DIR=~/.spectyn-mesh/autoevolve-target` 隔離 + 防毒鎖定（AV-lock）偵測有運作 — 少了這個，被 Defender 鎖住的 `build-script-build.exe` 會錯誤地觸發 LLM，我們就在 2026-05-01 12:38–12:57 遇到 `main.rs` 從 1330 位元組被覆寫成 8 位元組的事故。
- `evolve --max-rounds 1 --target check` 應走完一個回合（round），並在 `$0.0000` 下印出 `EVOLVE_DONE: all tests pass`。

### 階段 7 — `spectyn serve` 並行負載

在背景作業（background job）中啟動 `spectyn serve --port <ServePort>`，然後：

- 16 路並行（16-way parallel）的 `/healthz` 探測 → 16 個全部都必須回傳 `200`。
- `/api/version` → 預期出現 `version`、`commit`、`target=windows`、`wire_version`。
- `/rpc/ping` → 預期出現 `wire_version`、`core_sha`、`spectyn_version`（macOS 合併後的契約，來自 commit `2680ff6`）。
- Stop-Process 乾淨關閉。

已知：30 個以上的混合並行（`/healthz` + `/api/version` + `/api/status`）會很慢，因為 `/api/status` 有一個全域鎖（global lock）— 見[§7.1 已知效能問題](#71-已知效能問題)。

### 階段 8 — 波浪號邊界案例（edge cases）+ broken-pipe（管線中斷）panic 抑制

最後的健全性檢查（sanity check）：

- `cwd: "~/"`（帶尾端斜線的波浪號）→ 展開為 `$HOME`。
- 管線關閉（Pipe-close）：`spectyn doctor | Select-Object -First 1` 必須以 0 結束，且 `~/.spectyn-mesh/crashes/` 中沒有新增條目。修正前，每次經過管線的呼叫都會洩漏一筆當機記錄（crash log）；commit `f0ec83b` 安裝了 panic-hook 過濾器。

## 階段對應 → commits

| 階段 | 驗證的 commit |
|---|---|
| 2 doctor `OpenRouter` | `68d02d3` |
| 2 doctor `configured port` | `b50621a` |
| 3 cwd:'~' 回歸 | `7752330` |
| 4 service install（SpectynServe + PowerShell） | `8259330`、`6f9344f` |
| 4 防火牆規則自動安裝 | `b50621a` |
| 4 status `last state` 解碼 | `eec7a12` |
| 5 schedule install/status/uninstall | `3efbed2` |
| 5 schedule status XML 間隔 | `8259330` |
| 6 autoevolve 防毒鎖定偵測 | `4d3f78d` |
| 7 /rpc/ping 上的 wire_version | 從 macOS 合併 `2680ff6` |
| 7 `--port` 旗標被遵守 | `eec7a12` |
| 8 broken-pipe panic 抑制 | `f0ec83b` |
| 8 cwd:'~/foo' 展開 | `7752330` |

## §7.1 已知效能問題

`/api/status` 有一個會把請求序列化（serialise）的全域互斥鎖（mutex）。30 個並行
混合命中（hit）約需 ~25 秒。單一請求與 4 並行則沒問題。已歸檔為
非阻塞（non-blocking）— 修法是把讀取側的叢集快照（cluster snapshot）縮小到
更細粒度的鎖（finer-grained lock）。

## 重新部署 + 重新測試迴圈

對 `core/` 做任何程式碼變更後：

```powershell
.\scripts\build-windows.ps1 -Deploy        # rebuild + cp to ~/.spectyn-mesh/bin + ~/.local/bin
.\scripts\test-windows.ps1                 # full E2E sweep
```

如果只改了文件 / 設定，可以略過重新建置 — 已部署的二進位檔
沒有變動。
