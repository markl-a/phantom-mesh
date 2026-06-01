# phantom-mesh 定位（2026-05-15）

本文件是 phantom-mesh 對外自我描述的權威來源（canonical source）——
電梯簡報（elevator pitch）、我們是什麼、我們不是什麼、我們在生態系中與誰相鄰，
以及哪些口號（slogan）已停用。

本文件取代 `README.md`、`docs/MULTI-AGENT-DEEP-ANALYSIS.md`、簡報投影片
或演講摘要中任何較舊的定位文案。

## 1. 核心簡報（The pitch）

> **phantom-mesh 是一套個人、多裝置的 AI 代理人執行環境（agent runtime）——
> 讓你的代理人橫跨 Mac / Windows / Linux / iOS / Android，
> 運行於本地優先（local-first）的 P2P mesh（點對點網狀網路）之上，
> 具備閉環自我改進（closed-loop self-improvement）與跨 11+ 供應商的
> BYO-LLM（自帶大型語言模型）。**

三個承載重量的關鍵詞，依重要性排序：

- **Personal（個人）** — 預設單一使用者、單一 mesh；不需要中央租戶
  broker（中介伺服器）。OSS（開源）二進位檔在沒有任何雲端帳號的情況下也能完整運作
  （依 `README.md` §Contributing 的約束規則）。
- **Multi-device（多裝置）** — 同一個代理人身分、記憶與工具目錄，
  跟著使用者橫跨他們擁有的每一台機器（今日已上線 5 個平台）。
  此 mesh 是本地優先的；雲端存在（cloud presence）是選擇性加入（opt-in）。
- **Closed-loop self-improvement（閉環自我改進）** — `phantom autoevolve` 監看
  倉庫（repo），在測試轉紅（失敗）時生成 `phantom evolve`，轉綠（通過）時自動提交。
  每次迭代都會附加寫入 `~/.phantom-mesh/autoevolve.log`；下一輪
  會把近期的修正讀回 LLM 提示（prompt）中作為記憶。

## 2. 已停用的口號（請勿使用這些）

| 口號 | 停用原因 | 替代說法 |
|---|---|---|
| "Tailscale for AI agents" | Tailscale Aperture（2026 年 2 月公開測試版）現已成為擁有該說法的官方商業產品。繼續使用它，輕則造成混淆，重則引發商標摩擦。 | "Personal, multi-device AI agent runtime over a local-first P2P mesh." |
| "No project simultaneously has Rust + P2P + agent runtime" | OpenFang（github.com/RightNow-AI/openfang，MIT，約 16.8K stars，Rust + 自有 P2P + MCP + A2A）已於 2026 年推出此組合。該主張在事實上是錯誤的。 | "Personal multi-device focus + closed-loop self-improvement + 5-platform mobile story" —— phantom-mesh 現今真正的差異化。 |
| "Syncthing for AI agents" | 尚未在真實場景中驗證；可留作備案。不要拿它當主打。 | 無 —— 除非使用者自然而然地採用它，否則捨棄。 |

## 3. phantom-mesh 在 2026 生態系中的位置

以下是最常被與我們混淆的專案，以及誠實的差異（delta）。

### 3.1 OpenFang —— 最接近的直接競爭者

- Repo（倉庫）：`github.com/RightNow-AI/openfang`（MIT，Rust，137K LOC（程式碼行數），約 16.8K stars）
- 他們提供的：代理人作業系統（agent OS）+ 自有 P2P 協定（HMAC-SHA256 雙向認證）+
  MCP + A2A + 排程代理人（scheduled agents）。
- **他們今日勝過 phantom-mesh 之處：** A2A 協定支援、更大的
  社群、更成熟的排程代理人語意。
- **phantom-mesh 仍具差異化之處：** 行動原生（mobile-native）的故事（已簽章的
  Tauri APK + 已簽章的 IPA + Android 上的 Termux 真實 worker（工作節點））、`phantom evolve`
  閉環自我修復、Apple Silicon 上的 MLX 本地 LLM、在每個支援的作業系統上
  單一二進位檔（single-binary）安裝。
- 誠實的框架：OpenFang 是偏向桌面／伺服器的 Rust 代理人 mesh，
  具備更豐富的協定面；phantom-mesh 則是行動 + 自我改進
  + 單一二進位檔處處可用的變體。

### 3.2 OpenCode (sst/opencode)

- Repo（倉庫）：`github.com/sst/opencode`（MIT，TypeScript，約 160K stars）
- 一個 Anthropic-Claude-Code 風格的終端機編碼代理人，具備一流的
  TS 外掛 API，位於 `.opencode/plugins/*.ts`。
- **與 phantom-mesh 的關係：** 互補，而非競爭。我們可以
  成為 OpenCode 使用者的 phantom-mesh-as-provider 外掛（約 200 LOC），而
  OpenCode 也可以被無頭（headlessly）呼叫，作為 phantom-mesh 的子代理人（subagent）。
- **重要的名稱消歧：** "OpenCode"（sst，編碼代理人）不是
  "OpenClaw"（peterst，WhatsApp/Telegram/Slack 個人助理）。我們
  與兩者都整合，分屬不同軌道（track）。

### 3.3 OpenCode Zen 閘道（gateway）

- LLM 閘道，位於 `https://opencode.ai/zen/v1`，相容 OpenAI。
- 已於 2026-05-15 驗證為線上 —— 確認 7 個免費級別（free-tier）模型（見下方 §6）。
- phantom-mesh 提供一個指向 `/zen/v1` 的 `opencode` 供應商型別；
  見 `agents.toml.example`。

### 3.4 Hermes Agent (NousResearch)

- Repo（倉庫）：`github.com/NousResearch/hermes-agent`（MIT，Python）
- 是我們 Curator（策展者）+ Skill Document（技能文件）+ FTS5 記憶
  模式的架構參考（見週末衝刺軌道 H1–H5）。
- **授權警告：** 姊妹倉庫 `hermes-agent-self-evolution`
  沒有 LICENSE 檔案 → 保留一切權利（all rights reserved）→ 架構模式可以參考，
  逐字（verbatim）程式碼則不可使用。

### 3.5 Microsoft Agent Governance Toolkit（詞彙對齊）

- `opensource.microsoft.com/blog/2026/04/02/...` —— MIT，提供一個 Rust SDK，
  引入去中心化代理人識別碼（DID，decentralized agent IDs）與 IATP（inter-agent trust
  protocol，代理人間信任協定）作為標準詞彙。
- 我們在後續文件中採用此詞彙：
  - 在受眾具治理（governance）意識時，以 "Agent Mesh" 取代 "agent cluster"。
  - 以 "Agent DID" 作為每節點身分（per-node identity）的權威用語（我們今日對映到
    既有的 `node_name` + Tailscale 穩定 IP；完整的 DID 對齊
    仍在路線圖（roadmap）上，尚未推出）。
  - 討論代理人間認證時引用 "IATP" —— 我們透過 Tailscale 的 HMAC-SHA256
    即為等效層。
- 目標：別再重新發明命名。當業界已有標準用語時，就用
  它。

### 3.6 Tailscale Aperture（口號吞噬者）

- `tailscale.com/blog/aperture-public-beta`（2026 年 2 月）。
- 與身分連結的 AI 閘道（identity-linked AI gateway），稽核 MCP + LLM 工作階段（session）。
- 迫使我們永久放棄 "Tailscale for AI agents" 這個口號。
- 我們仍持續推薦 Tailscale 作為
  phantom-mesh 叢集（cluster）的建議覆蓋層（overlay）（見 `README.md` §Cluster）—— Aperture 與 phantom-
  mesh 並不衝突，只是位於不同的層。

### 3.7 Apple Foundation Models

- 隨 iOS/iPadOS/macOS 26 推出（2025 年 9 月）。裝置端約 3B 的 LLM、Swift API、
  工具呼叫（tool-calling）正式上線（GA，一般可用）。
- **對定位的影響：** 「在 iOS 上自帶（BYO）本地 LLM」的訴求對 Apple
  原生使用者而言不再具差異化。iOS 定位轉向：
  跨裝置同步、非 Apple 供應商、大於 3B 等級的模型、
  跨 app 暫停（app suspensions）的代理人迴圈（agent-loop）持續性。

### 3.8 MCP + A2A 協定堆疊

- MCP（`modelcontextprotocol.io`）—— 代理人 ↔ 工具，截至 2026-05-15
  規格仍為 `2025-11-25`。
- A2A（`a2a-protocol.org`）—— 代理人 ↔ 代理人，Linux Foundation，150+ 組織，
  約 22K stars，v1.2。
- phantom-mesh 今日已支援 MCP（`phantom mcp` 將 50 個工具公開為
  `mcp__phantom__*`）。A2A 仍在路線圖上；尚未推出。

## 4. 反向訴求（我們不是什麼）

- 我們不是企業級代理人控制平面（control plane）。Cloud／Enterprise
  版本的規劃是為了 OSS 核心的永續性；它永遠不是
  必經之路。
- 我們不是代理人閒談／角色扮演框架
  （AutoGPT／BabyAGI／經典 Swarm）。單一代理人 + 乾淨的工具集在
  生產環境中勝出；我們遵循這項共識。
- 我們不是無程式碼（no-code）視覺化建構工具（Langflow／Flowise／Dify）。
  那些是用於原型開發；phantom-mesh 是執行環境基礎設施（runtime infra）。
- 我們不追逐基準測試（benchmark）排行榜。Berkeley 已證明（2026 年 4 月）
  全部 8 個主要代理人基準測試都被獎勵駭客（reward-hack）刷到約 100%；我們改以
  使用者回報的真實任務吞吐量（throughput）來衡量。

## 5. 標準一句話文案（已核可用於投影片／演講／主視覺文案）

- "Personal, multi-device AI agent runtime — your agent everywhere you
  work, over a local-first P2P mesh."
- "Five-platform single binary — Mac, Linux, Windows, Android, iOS — that
  improves itself while you sleep."
- "BYO-LLM across 11+ providers (Anthropic, OpenAI, Gemini, Groq,
  Cerebras, NVIDIA NIM, OpenCode Zen, OpenRouter, Mistral, Ollama, MLX
  local) with automatic failover."
- "MCP-server today, A2A on the roadmap, governance vocabulary aligned
  with Microsoft's Agent DID / IATP."

## 6. OpenCode Zen 免費模型 —— 已於 2026-05-15 驗證為線上

事實來源（source of truth）：`curl https://opencode.ai/zen/v1/models | jq`。
在 2026-05-15 確認存活的七個免費級別
（與一個隱身（stealth））模型：

| 模型 id | 級別 |
|---|---|
| `big-pickle` | 隱身（免費，無 `-free` 後綴） |
| `deepseek-v4-flash-free` | 免費 |
| `minimax-m2.5-free` | 免費 |
| `nemotron-3-super-free` | 免費 |
| `qwen3.6-plus-free` | 免費 |
| `ring-2.6-1t-free` | 免費（原為 `ling-2.6-flash-free`，已更名） |
| `trinity-large-preview-free` | 免費 |

注意：`hy3-preview-free`（列於較早的文件中）已**消失**，並由
`trinity-large-preview-free` 與 `big-pickle` 取代。列出之前務必透過
線上的 `/v1/models` 端點（endpoint）重新驗證。

`agents.toml.example` 以註解形式在
`[providers.opencode]` 區塊下攜帶此清單，讓使用者不必
追著本文件跑，就能取得當前的免費級別名稱。

## 7. 更新節奏

- 每當釋出說明（release notes）刷新時，就重新驗證 OpenCode Zen 模型清單
  （當前節奏：每次次要版本（minor release））。
- 每季重新掃描競爭者清單（§3），或當有使用者回報聲稱
  出現新的直接競爭者時 —— 以先發生者為準。
- 停用／提升口號（§2）僅在作者 + 審閱者簽核後進行，以
  避免 README 與本文件之間產生漂移（drift）。
