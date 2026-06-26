# 安裝時的二進位檔驗證

每個 `phantom` / `phantom-mesh` 安裝腳本都會在**標記為可執行或移入 PATH（執行路徑）之前**，
拿下載到的二進位檔（binary）與已發布的 SHA256 sidecar（隨附校驗檔）比對。
本文件說明：

1. 此驗證所緩解的威脅模型（threat model，攻擊情境模型）。
2. 各個發布通道（distribution channel）的正規（canonical）校驗值存放位置。
3. fail-closed（失敗即拒絕）行為，以及兩個逃生閥（escape-hatch，例外旁路）環境變數
   （僅在少數有充分理由的情況下使用）。
4. 驗證失敗時該怎麼做。

## 1. 威脅模型

此驗證可防範三種具體的攻擊者：

| 攻擊者 | 未驗證 | 有驗證 |
|---|---|---|
| **線路上的 MITM（中間人攻擊）** — 任何能劫持 `http://coord/dist/phantom-…` 請求的一方 | 替換掉二進位檔；使用者會以自身身分執行它（並透過 `phantom service install`，在每次登入時執行）。 | 透過 HTTPS 取得的 sidecar 不會與被掉包的二進位檔吻合 → 安裝中止；二進位檔被刪除。 |
| **被攻陷的 tailnet 節點** — 攻擊者取得任一對等節點（peer，例如遺失的筆電）的 WireGuard 私鑰並加入 tailnet | 可向每台重新執行安裝程式的機器供應替換過的二進位檔。 | 同上 — sidecar 位於官方 R2 / GitHub Releases 的 URL，而非協調器（coordinator，協調節點）上。單靠攻陷 tailnet 並無法產生一個與惡意二進位檔吻合的 sidecar。 |
| **惡意的 GitHub PAT（個人存取權杖）** — 攻擊者把偽造的資產（asset）上傳到某個 release（發行版） | 新 release 的 sidecar 也可能一併被偽造，但發出 sidecar 的 CI（持續整合）工作流程是在程式碼倉庫內執行、並可在 `.github/workflows/release-daemon.yml` 中稽核。任何繞過 CI 的帶外（out-of-band）資產上傳都可被偵測（沒有 sidecar＝安裝程式拒絕）。 | 不吻合／缺少 sidecar → 安裝程式中止。 |

此驗證**並非**用來取代程式碼簽章金鑰的信任根（trust root）
（minisign / sigstore）。在 SHA256 這層之上加入 minisign 已列入
路線圖（roadmap，audit finding #1 recommendation 1）。SHA256 這層是
最低限度的縱深防禦（defence-in-depth）：它把門檻從「MITM 任一節點」提高到
「同時對協調器與 github.com 在 HTTPS 上發動 MITM」。

## 2. 正規 SHA256 的存放位置

慣例是 **`<binary-url>.sha256`** — 與二進位檔相同的路徑，
後面接上 `.sha256`。

### GitHub Releases（`release-daemon.yml`）

每個 release 都會發布每個資產各自的 `.sha256` sidecar，**並且**附帶一份合併的
`SHA256SUMS` 清單（manifest）：

```
https://github.com/markl-a/phantom-mesh/releases/download/v0.1.0/phantom-mesh-x86_64-linux
https://github.com/markl-a/phantom-mesh/releases/download/v0.1.0/phantom-mesh-x86_64-linux.sha256   ← 由安裝程式取用
https://github.com/markl-a/phantom-mesh/releases/download/v0.1.0/SHA256SUMS                         ← 參考清單
```

每個 sidecar 的格式正是 `sha256sum <name>` 的輸出：

```
26d39c…b71e  phantom-mesh-x86_64-linux
```

取用者：`setup-cloud-linux.sh`、`dev/deploy-gcp.sh`、`update-daemon.sh`。

### Cloudflare R2（phantommesh.io，透過 `publish-phantom-binary.yml`）

每次 R2 上傳也會在相同的 key 後綴推送一個 `.sha256` 伴隨物件（companion object）：

```
https://phantommesh.io/dist/phantom-linux-x86_64
https://phantommesh.io/dist/phantom-linux-x86_64.sha256   ← 由安裝程式取用
```

取用者：`install-mac.sh`、`install-phantom-windows.ps1`、
`termux-setup.sh`（後者是在 `PHANTOM_URL` 指向 phantommesh.io
而非協調器時）。

### 相對於協調器的 `<COORD>/dist/`

當操作者針對自己的 tailnet 協調器執行 `install-mac.sh` 時，
協調器應同時鏡像（mirror）二進位檔**與**
`.sha256` sidecar（例如在協調器端執行 `sha256sum dist/* > dist/SHA256SUMS`，
並把每個檔案各自的 sidecar 以
`dist/<name>.sha256` 暴露出來）。若你的協調器尚未設定為
發出 sidecar，請參閱下方的逃生閥 — 但要清楚其中的取捨。

## 3. fail-closed 行為 + 逃生閥

預設行為是 **fail-closed（失敗即拒絕）**：

- 純 `http://` 下載 URL 會被拒絕。
- 缺少或格式錯誤的 `.sha256` sidecar → 安裝程式刪除二進位檔
  並以 exit 1 結束。
- SHA256 不吻合 → 安裝程式刪除二進位檔並以 exit 1 結束。

存在兩個逃生閥，皆需透過環境變數明確啟用（opt-in）。兩者都會在
stderr（標準錯誤輸出）上發出醒目的警告；在下列少數情況之外都不建議使用。

### `PHANTOM_ALLOW_INSECURE=1`

允許二進位檔或 sidecar 使用純 `http://` URL。僅在以下情況有正當理由：
- 完全 air-gapped（實體隔離斷網）、未接任何外部網路的 tailnet
- 協調器跑在 localhost（本機）上的開發迴圈

在正式（production）部署中設定此項是已知的不良做法，
且每次安裝都會出現在稽核日誌（audit log）中。

### `PHANTOM_SKIP_VERIFY=1`

完全跳過 SHA256 檢查。僅在以下情況有正當理由：
- 全新的協調器已上傳二進位檔，但對應的
  `.sha256` sidecar 尚未產生（首次起步引導，first-cut bootstrap）
- 真正 air-gapped 的安裝，二進位檔以帶外方式透過
  USB 隨身碟交付，且操作者已自行雜湊（hash）過該檔案

當沒有已發布的 sidecar 存在時，這是**唯一**可行的安裝方式。

## 4. 驗證失敗時該怎麼做

安裝程式會印出類似以下的內容：

```
✗ SHA256 mismatch for /tmp/phantom.XXXXXX
  expected: 26d39c…b71e
  actual:   a14b32…0099
  Source:   https://phantommesh.io/dist/phantom-linux-x86_64.sha256
  The downloaded binary has been deleted.
```

處理檢查清單：

1. **不要為了「讓它跑起來」而以 `PHANTOM_SKIP_VERIFY=1` 重新執行。**
   檢查失敗的整個重點，就在於有東西出錯了。
2. 直接重新抓取該 sidecar 並檢視它：
   ```
   curl -sSL https://phantommesh.io/dist/phantom-linux-x86_64.sha256
   ```
   若該檔案是空的／404／HTML — 表示正規鏡像站掛了，或
   資產名稱有誤。請提交一個 issue（問題回報）。
3. 把 `expected`（預期）那一行與 GitHub 上最新的 release notes（發行說明）
   （`https://github.com/markl-a/phantom-mesh/releases`）比對。release 頁面
   內嵌了 SHA256SUMS 的內容。
4. 若已發布的 sidecar 與你的安裝程式所宣稱的預期值吻合，
   **但**你的本機雜湊值不同，那你就有以下其中一種情況的證據：
   - 下載過程中磁碟損毀（罕見；重試一次）
   - 進行中的 MITM（在咖啡店 wifi 上較可能發生）
   - 被攻陷的鏡像站

   在第 2 與第 3 種情況下，不要再從該網路重試安裝。
   切換到已知良好的網路再重新執行；若不吻合持續存在，
   請提交一份安全性回報（`SECURITY.md`）。
5. 被刪除的二進位檔已經消失 — 除了一般的安裝位置外，
   磁碟上沒有需要清理的東西。

## 實作參考

- `scripts/_verify-download.sh` — 由每個 POSIX 安裝腳本 source（引用）的
  共用 bash 輔助腳本
- `scripts/_verify-download.ps1` — 由每個 Windows 安裝腳本 dot-source（點引用）的
  共用 PowerShell 輔助腳本
- `.github/workflows/release-daemon.yml` — 為每個 GitHub release 發出每個資產的
  `.sha256` sidecar + 合併的 `SHA256SUMS`
- `.github/workflows/publish-phantom-binary.yml` — 在每次 R2 上傳時發出每個資產的
  `.sha256` sidecar
- `docs/superpowers/audits/2026-05-16-scripts-audit.md` §1、§2 —
  本次修補所處理的 CRIT-1 + CRIT-2 發現
