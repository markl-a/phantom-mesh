# 安全政策（Security Policy）

## 支援的版本（Supported Versions）

| 版本    | 是否支援           |
|---------|--------------------|
| 0.6.x   | :white_check_mark: |
| < 0.6   | :x:                |

安全修補（security fix）僅回溯移植（backport）到最新的 `0.6.x` 發行線。較舊的
版本應升級到目前的發行版。

## 回報漏洞（Reporting Vulnerabilities）

**請勿為安全漏洞開啟公開的 issue（問題單）。**

優先管道 —— **GitHub 私密安全公告（private security advisory）**：

1. 前往儲存庫（repository）的 **Security**（安全）分頁。
2. 點選 **Report a vulnerability**（回報漏洞），這會開啟一則只有
   維護者（maintainer）看得到的私密公告。
3. 填寫細節（見下方檢查清單）。

若你無法使用 GitHub 安全公告，可改為寄信給維護者的
安全別名信箱（security alias）：`security@spectyn-mesh.dev`（若專案另有
公布的聯絡方式，請以該方式為準）。

請包含以下內容：
- 漏洞描述
- 重現步驟
- 受影響的版本
- 你的聯絡資訊以利後續追蹤

我們力求在 **72 小時**內確認收到回報，並在 **7 天**內修補重大問題。
請在公開揭露前給我們合理的揭露緩衝期（disclosure window）。

## 目前已實作的安全功能（Currently Implemented Security Features）

以下功能已在 v0.6.x 發行線中實作：

- Shell 命令白名單（allowlist，只有事先核准的命令才能執行）
- 工作區沙箱（workspace sandbox，檔案 I/O 限制在指定的工作區目錄內）
- 完整確認模式（full confirmation mode，所有 AI 操作都需要使用者確認）
- 執行前對白名單進行路徑正規化（path canonicalization）
- L1 正規表示式（regex）防護欄（8 種以上的模式，阻擋提示注入（prompt injection）與越獄（jailbreak）嘗試）
- L2 LLM-as-Judge（以大型語言模型作為裁判，評估工具輸出的安全性）
- 注入防護（injection guard，偵測多語言覆寫、ChatML、base64、混淆手法）
- 叢集（cluster）通訊採用共享密鑰（shared-secret）Bearer token 驗證
- 無遙測（telemetry）—— Spectyn Mesh 不會回傳資料（phone home）

## 安全模型（Security Model）

Spectyn Mesh 採用**本地優先（local-first）**架構，搭配縱深防禦（defense-in-depth）。

### 節點身分與驗證（Node Identity & Authentication）

- 叢集驗證使用**共享密鑰（shared secret）**（透過 HTTP 的 Bearer token）
- 每個節點的 Ed25519 金鑰對（keypair）身分，以及 ChaCha20-Poly1305 憑證加密（credential encryption），列在 v0.6 之後的路線圖（roadmap）上

### 注入防禦（Injection Defense）

- **L1 防護欄（Guardrail）**：8 種以上的正規表示式模式，阻擋提示注入、越獄嘗試與危險指令
- **L2 LLM-as-Judge**：由獨立的大型語言模型評估工具輸出的安全性
- **注入防護（Injection Guard）**：偵測多語言覆寫、ChatML 注入、base64 酬載（payload）、混淆手法

### 工具執行安全（Tool Execution Safety）

- **Shell 命令白名單**：只有事先核准的命令才能執行
- **工作區沙箱**：檔案 I/O 限制在指定的工作區目錄內
- **完整確認模式**：所有 AI 操作在執行前都需要使用者確認
- **路徑正規化**：所有檔案路徑在執行前都會被展開並對照白名單檢查

### 網路安全（Network Security）

- **無遙測**：Spectyn Mesh 不會回傳資料或蒐集使用情形
- **叢集通訊**：節點之間採用具 Bearer token 驗證的 HTTP
- **WireGuard/Tailscale**：建議用於跨網路的叢集節點

## 資料處理（Data Handling）

所有資料皆儲存在本地的 `~/.spectyn-mesh/`：

| 儲存區 | 檔案 | 內容 |
|-------|------|------|
| 核心資料庫（Core DB） | `core.db` | 任務、對話、cron 排程工作 |
| 對話（Conversations） | `conversations.db` | 聊天工作階段（session）歷史 |
| 叢集（Cluster） | `cluster.db` | 對等節點（peer）登錄、節點狀態 |
| 事件（Events） | `data_event_index` 資料表 | 帶有每節點定序（per-node sequencing）的領域事件 |

SQLite 資料庫使用 **WAL 模式（WAL mode，預寫式記錄）**以確保當機安全（crash safety）。結構描述遷移（schema migration）在套用變更前會自動建立備份。

## 貢獻者準則：避免密鑰外洩（Contributor Guidelines: Avoiding Secret Leaks）

- **切勿提交（commit）`agents.toml`** —— 它內含實際的 API 金鑰（live API key）。它已列在 `.gitignore` 中。
- 使用 `agents.toml.example` 作為範本，其中只放佔位（placeholder）值。
- 在提交前，執行 `git diff --staged` 並檢查是否有任何形似金鑰的字串。
- 若你不慎提交了密鑰：立即撤銷（revoke）該金鑰，接著在推送（push）前使用 `scripts/clean-history.sh` 清除歷史紀錄。
- CI 流水線（CI pipeline，`security.yml`）會在每次推送與 PR（pull request，拉取請求）時執行 [gitleaks](https://github.com/gitleaks/gitleaks)，自動攔截外洩。

## 威脅模型（Threat Model）

Spectyn Mesh 信任本地環境。它**不**防範以下情況：
- 已遭入侵的作業系統
- 具有本地檔案系統存取權的惡意使用者
- 已攻破你 VPN 的網路攻擊者

請保護好你的機器與 VPN 憑證以保障你的資料安全。

---

*最後更新：2026-05-29*
