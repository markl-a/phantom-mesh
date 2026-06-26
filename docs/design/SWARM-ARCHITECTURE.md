# phantom-mesh — 跨作業系統群集架構（Cross-OS Swarm Architecture）

> 一個在任何裝置（Linux / macOS / Windows / Android / iOS）上輸入的提示
> （prompt）如何被拆解、派發，並在異質機隊（heterogeneous fleet，組成不同的
> 機器群）上執行；以及每種作業系統（OS）作為代理人主機（agent host）究竟能被
> 推進到多遠。
>
> 範圍：本文是群集層（swarm layer，分散式協作層）的設計契約（design
> contract），位於既有單節點（single-node）`phantom serve` 常駐程式（daemon）
> 之上，後者已記載於 [ARCHITECTURE.md](./ARCHITECTURE.md)。
> 中樞輻射式（hub-and-spoke，一個中樞對多個分支）是 2026-05-20 上線時的
> **出貨（shipping）** 拓撲（topology，網路結構）；帶 CRDT 狀態的網格
> （mesh-with-CRDT-state）是路線圖（roadmap）項目，並非當前的設計目標。
>
> **非目標（Non-goal）**：假裝 iOS 與 Android 是對稱的。它們並不對稱。
> 本文有很大一部分存在的目的，就是要把這種不對稱講清楚，
> 並針對每種作業系統實際擅長的部分加以發揮。

---

## 1. 願景（Vision）

使用者拿著自己任一台裝置輸入一個提示，正確的工作就會在正確的機器上
被完成，而且過程是透明的（transparently，使用者無感）：

- 沙發上的 iPad → 規劃器（planner）挑選家中的 Mac 來做程式碼編輯，
  並挑選永遠開機的 node-a 來維持叢集心跳（cluster heartbeat）。
- 咖啡店裡的 Android → 規劃器在本機跑一個 3B（30 億參數）模型來回答
  「這個錯誤是什麼意思」，但把一個 200 個檔案的重構（refactor）透過
  Tailscale 丟回家中叢集處理。
- 桌前的 Mac → 協調者（coordinator）。負責派發；不一定親自執行。

心智模型（mental model）是 **一個邏輯代理人，多個實體執行者**
（one logical agent, many physical executors）。
使用者不必挑選哪台機器跑什麼；群集自己決定，同時保留明確的
覆寫（override）權限給使用者。

---

## 2. 節點分類（Node Taxonomy）

四種角色。單一裝置可以同時擁有多個角色。

| 角色 | 說明 | 典型主機 |
|------|-------------|---------------|
| **Coordinator（協調者）** | 擁有工作階段日誌（session log），執行規劃器，派發子任務。每個工作階段恰好一個（失敗時可重新選舉）。 | Mac、node-a、任何永遠開機的 Linux/Windows 機器 |
| **Full Agent（完整代理人）** | 可執行任意子任務：shell、git、瀏覽器、檔案編輯、本機大型語言模型（LLM）、雲端 LLM。 | Linux、macOS、Windows 桌機、Termux-on-Android |
| **Lite Agent（精簡代理人）** | 只能執行受限、預先宣告的子任務子集。不能執行 shell、不能任意執行子行程（subprocess）。 | iOS 原生 app、瀏覽器分頁 PWA、被鎖死的企業筆電 |
| **Thin Client（瘦客戶端）** | 僅作為使用者介面（UI）表層。送出提示、呈現串流輸出。不執行任何東西。 | iOS Safari / mobile.html、web `/projects`、tailnet 上任何未安裝 phantom 的裝置 |

目前 `core/src/mesh.rs` 中的 `PeerInfo` / `PeerStatus` 型別
已經帶有 `capabilities: Vec<String>` 與 `worker_caps: Vec<String>`。
群集層把允許哪些字串加以正式化（formalise），並用一個結構化的
**能力清單（Capability Manifest）**（§4）來擴充這個傳輸型別（wire type）。

一個裝置的角色是從它的清單（manifest）中**自然湧現（emergent）** 出來的，
而不是用自由格式的標籤宣告出來。一個沒有 `shell` 能力的節點
會自動成為 Lite Agent，不論它的設定檔對自己怎麼宣稱。

---

## 3. 傳輸、認證、探索（Transport, Auth, Discovery）

已經解決。在此重述，是為了讓群集設計不會重新發明它。

- **Transport（傳輸）** — HTTP/1.1 + JSON，跑在 Tailscale 的 WireGuard
  通道（tunnel）之上。協調者 IP 是那台永遠開機中樞的 tailnet 100.x 位址。
  不需要 NAT 穿透（NAT punching）、不需要 STUN、不需要自製的 P2P 堆疊。
- **Auth（認證）** — 每個 tailnet 共用一把 HMAC 密鑰（`PHANTOM_HMAC_KEY`）。
  每個 RPC 都帶 `X-Phantom-Sig: hmac-sha256(body)`。協調者在執行
  `phantom secret rotate` 時輪換密鑰；對等節點（peers）在下一次 ping 時重新載入。
- **Discovery（探索）** — MagicDNS（`mac.tailnet`、`rog.tailnet`）。對等節點
  啟動時在 `agents.toml` 中帶 `coordinator_url = "http://mac:7878"`，
  並在啟動時呼叫 `/rpc/join`。叢集範圍的對等清單會透過
  `/rpc/ping` 交換來傳播，所以新增一個節點只需要告訴它一個既有的對等節點即可。

未來的商用中介伺服器（broker，見
[COMMERCIAL-DESIGN.md](COMMERCIAL-DESIGN.md)）的存在，
是為了在**沒有**共用 tailnet 的情況下，在使用者之間中繼（relay）。
在單一 tailnet 上的開源（OSS）使用者並不需要它。

---

## 4. 能力清單（Capability Manifest，提議的擴充）

今天 `PeerStatus.capabilities` 是一個無結構的 `Vec<String>`。
群集規劃器需要結構。提議的傳輸型別，對 `PeerStatus` 採增量（additive）擴充：

```jsonc
{
  "node_id": "android-phone",
  "os": "android",                       // linux | macos | windows | android | ios
  "arch": "aarch64",
  "power": "battery",                    // battery | wall | always_on
  "role_hint": "full_agent",             // self-declared, planner verifies via tools

  "tools": {                             // declared tool capabilities
    "shell": true,
    "git": true,
    "browser": false,
    "subprocess": true,                  // can fork-exec arbitrary binaries
    "mobile_driver": "android",          // null | "android" | "ios"
    "gpu_inference": "vulkan",           // null | "metal" | "cuda" | "vulkan" | "ane"
    "file_edit_scope": "fs",             // "fs" = anywhere, "sandbox" = app-bundle only
    "long_running": true                 // can host >5min background tasks
  },

  "models": {
    "local": ["gemma-2b-q4_k", "phi3-mini-q4"],
    "remote_keys": ["anthropic", "openai", "groq"]
  },

  "limits": {
    "max_concurrent_tasks": 4,
    "max_task_seconds": 1800,
    "egress_bandwidth_kbps": 5000
  },

  "reachable_via": {
    "tailscale": "100.x.y.z:7878",
    "lan": "192.168.1.42:7878"
  }
}
```

規劃器透過**集合包含關係（set inclusion）** 做派發決策：一個子任務宣告
`required_caps = ["shell", "git"]`，排程器（scheduler）就挑選其 `tools`
為超集合（superset）能涵蓋這需求、且最便宜又健康的節點。這已經是既有
`worker_caps` 機制的形態——我們只是把它從一個扁平的能力清單擴充成
一個結構化的設定檔（profile）。

**實作（Implementation）**：對 `PeerStatus` 加上
`manifest: Option<NodeManifest>`。沒有這個欄位的舊對等節點仍可繼續運作——
規劃器退回到舊版（legacy）的 `capabilities` 字串，並把它們視為
回報作業系統上的 Full Agent。

---

## 5. 提示生命週期（Prompt Lifecycle，採 ReAct，而非大型 DAG）

規劃器**不**會在一開始就產出一個完整的有向無環圖（DAG，Directed Acyclic
Graph，任務依賴關係圖）。它產出下一個子任務、看結果、再決定下一個。
這是刻意採取 Claude-Code 風格的 ReAct（推理-行動交錯），而非
airflow 風格的 DAG。

```
┌─────────────┐   prompt    ┌─────────────────────────────────┐
│   Client    │────────────▶│       Coordinator                │
│ (iOS/web/…) │             │                                  │
└─────▲───────┘             │  ┌─────────────────────────────┐ │
      │                     │  │ Session Log (SQLite)         │ │
      │  SSE stream         │  │  • all turns + tool calls    │ │
      │  of progress        │  │  • truth source of state     │ │
      │                     │  └─────────────────────────────┘ │
      │                     │              ▲                   │
      │                     │              │                   │
      │                     │   ┌──────────┴──────────┐        │
      │                     │   │ Planner (LLM call)  │        │
      │                     │   │  next subtask only  │        │
      │                     │   └──────────┬──────────┘        │
      │                     │              │                   │
      │                     │   ┌──────────▼──────────┐        │
      │                     │   │ Scheduler            │        │
      │                     │   │ caps ⊇ required_caps │        │
      │                     │   └──────────┬──────────┘        │
      │                     │              │                   │
      │                     └──────────────┼───────────────────┘
      │                                    │
      │                          dispatch (HMAC-signed)
      │                                    │
      │                ┌───────────────────┼───────────────────┐
      │                ▼                   ▼                   ▼
      │           ┌─────────┐         ┌─────────┐         ┌─────────┐
      │           │  Mac    │         │  node-a    │         │  ROG    │
      │           │ shell+  │         │ always- │         │ android │
      │           │ git+API │         │ on hub  │         │ + mobile│
      │           └────┬────┘         └────┬────┘         └────┬────┘
      │                │ result            │ result            │ result
      │                └───────────────────┼───────────────────┘
      │                                    │
      └────────────────────────────────────┘  next planner turn
```

每一輪的預算（per-turn budget）：規劃器 LLM 呼叫（≤2 秒）、排程器決策
（<10 毫秒）、子任務執行（視情況而定）、結果回傳協調者（透過 Tailscale
<200 毫秒）、工作階段日誌附加寫入（<50 毫秒）。使用者看到的是來自執行節點
的串流 token（詞元），外加 SSE 通道上離散的 `step.start` /
`step.complete` 事件。

### 實例演練（Worked example）

iPhone 上的使用者輸入：
> 跑一輪 phantom-mobile demo，把 log 整成週報 PDF 上傳 Drive

規劃器一次只發出一個子任務：

| # | 子任務 | required_caps | 選定節點 | 原因 |
|---|---------|---------------|---------------|-----|
| 1 | `phantom-mobile make demo-mock` | `shell, mobile_driver=android` | android-phone | 唯一有該驅動的 Android 對等節點 |
| 2 | `collect logs/*.json`（在 #1 之後） | `shell, file_edit_scope=fs` | android-phone | 與 #1 同地共置，避免把原始 log 透過 Tailscale 串流傳輸 |
| 3 | 把 2,300 行的 log 摘要成 8 個重點 | `cloud_llm=anthropic` | mac | 有 API 金鑰；在這個長度下本機 3B 準確度不足 |
| 4 | 把 Typst 算繪（Render）成 PDF | `subprocess, has_typst` | mac | 只有 Mac 安裝了 Typst |
| 5 | 上傳到 Drive | `oauth_token=drive` | mac | OAuth 權杖綁定在 Mac 的鑰匙圈（keychain） |

iPhone 在 SSE 通道上看到串流進度。它從不執行任何東西。
整體延遲由步驟 1（實際的 demo 執行，約 40 秒）主導；
其餘一切都在 2 秒以內。

---

## 6. 狀態模型（State Model）

**集中式工作階段，無狀態工作節點（Centralised session, stateless
workers）。** 協調者的 SQLite 工作階段日誌是唯一真實來源（single source of
truth）。工作節點是純函式（pure functions）：
`(subtask_payload, env) → result_blob`。沒有任何工作節點在子任務之間
保存狀態；如果同一個工作節點在一個工作階段中拿到兩個子任務，
每一個子任務都自帶完整所需的上下文。

為什麼不用 CRDT 複製的狀態？因為對一個 1 協調者的部署來說，
它會讓我們付出 10 倍的實作複雜度，卻換來 0 倍的使用者好處。
協調者故障接手（failover，重新選舉一個對等節點當協調者）在路線圖上；
在那之前，協調者掛掉 = 工作階段暫停，重啟後從 SQLite 日誌恢復。可接受。

### 失敗語意（Failure semantics）

- **Heartbeat（心跳）** — 每個對等節點每 30 秒對協調者 ping 一次。
  連續三次未到 → 該對等節點被標記為不健康，正在執行中的子任務被重新派發。
- **Subtask timeout（子任務逾時）** — 從對等節點的 `limits.max_task_seconds`
  推導出來，並以規劃器每一步的預算為上限。逾時時協調者會（盡力）取消，
  並在另一個對等節點上重試。
- **Idempotency（冪等性）** — 每個子任務都帶一個 `task_id` UUID。對等節點
  必須以 `task_id` 去重（dedupe），這樣在網路不穩之後的重試
  才不會重複執行一個破壞性操作。這就是讓重新派發安全的契約。
- **Battery-host disappearance（電池主機消失）** — Android / iOS 主機
  經常離線（螢幕關閉、app 被暫停）。協調者不能假設它們會回來；
  派發給它們的子任務會在經過 `2 × typical_duration`（兩倍典型時長）後，
  積極地在接市電（wall-powered）的對等節點上重試。

---

## 7. 本機與雲端 LLM 路由（Local-vs-Cloud LLM Routing）

跑在協調者上的規劃器會逐步決定要呼叫本機模型還是遠端 API。
啟發法（Heuristics，經驗法則），依序為：

1. **子任務類別（Subtask category）** — 分類 / 抽取 / 短摘要 →
   預設用本機 3B。程式碼推理、多檔重構、「解釋這個堆疊追蹤
   （stack trace）」→ 雲端。
2. **輸入大小（Input size）** — 本機 3B 在大約 2 KB 輸入以內才有可用品質。
   超過就升級（escalate）。
3. **呼叫端在地性（Caller locality）** — 如果發起的客戶端是一台帶有可用
   本機模型的電池裝置，優先先在本機做路由決策（意圖分類 < 200 毫秒，
   省掉一次 Tailscale 來回）。
4. **成本上限（Cost ceiling）** — 協調者追蹤每個工作階段花掉的金額。
   超過一個可設定的上限（`session.cloud_budget_usd`），規劃器就降級為
   僅本機（local-only）並警告使用者。
5. **失敗備援（Failure fallback）** — 如果雲端 API 連續兩次回傳 429 / 5xx，
   規劃器切換到下一個可用的雲端供應商；如果全部失敗，就降級為本機，
   並掛上一個「降級品質（degraded quality）」橫幅。

從「子任務意圖」到「類別」的對應本身就是一次 LLM 呼叫（很便宜，
例如 Haiku 或本機 3B）。結果會以工作階段為單位快取（cache），
這樣同一種子任務重複出現時就不會付兩次路由成本。

---

## 8. 各作業系統設定檔（Per-OS Profiles）

### 8.1 Linux

完整代理人。Shell、git、瀏覽器（透過 playwright 的無頭 Chromium）、
GPU 推論（CUDA/Vulkan/ROCm）、任意子行程、長時間執行的 systemd 服務。
參考部署是 node-a 永遠開機中樞。

若 `power = wall | always_on` 則具備協調者能力。

### 8.2 macOS

完整代理人**並且**是參考協調者主機。額外加上：
- 在 Apple Silicon 上的 MLX 推論（`metal` GPU）
- 系統鑰匙圈（system keychain）中的 Drive / iCloud / Calendar OAuth 權杖
- 用於 autoevolve 每小時迴圈的 LaunchAgent

5/20 上線架構假設 Mac 是主要協調者；
node-a 在手動檢查清單（manual checklist）的第 #6 項之後成為協調者。

### 8.3 Windows

完整代理人。Windows 特有的怪異之處（quirks）：
- 透過 NSSM 或 `sc.exe create` 註冊服務
- WSL2 *不是* 同一個節點——它以一個獨立的 Linux 對等節點出現，
  有自己的清單。兩者可在同一台實體機器上並存。
- 某些設定下沒有原生 CUDA；規劃器從清單讀取 `gpu_inference`，
  不從作業系統推測。

### 8.4 Android（透過 Termux）

完整代理人，但有但書。`aarch64-linux-android` 的 phantom 二進位檔
已經能在 Termux 中建置並執行。可行的部分：

- 真正的 Linux 使用者空間（userland）：shell、git、curl、ssh、完整 POSIX
- 子行程 + fork-exec，在 Termux 內無沙箱限制
- 前景服務（Foreground service）+ 喚醒鎖（wake lock）→ 真正的長時間執行任務
- Vulkan 計算 → llama.cpp 可把運算卸載（offload）到 Adreno/Mali GPU
- MediaPipe LLM API → Gemma 2 B 跑在手機 NPU 上

不可行的部分：

- 任何在 Termux 家目錄之外的東西（Android 範圍化儲存，scoped storage）
- 加速度計 / 相機 / 原生感測器（與終端機等級的代理人無關）
- Android 12+ 上的 Termux 背景限制——必須宣告前景服務，
  並用 `Termux:Boot` 附加元件達成開機持續存活

一支旗艦級 Android（SD 8 Gen 3 等級）在 Termux 中跑 phantom，
是一個正當合格的叢集對等節點。使用者機隊中的 ROG 裝置就扮演這個角色。
RAM 小於 8 GB 的便宜手機應被歸類為 **Lite Agent**（其能力為
`local_models = []`）。

### 8.5 iOS — 見 §9

iOS 自成一節，因為它的約束與最佳化問題在本質上就不一樣。

---

## 9. iOS：我們能把它推到多接近 Android？（How Close to Android Can We Push?）

目前的 [../install/INSTALL-IOS.md](../install/INSTALL-IOS.md) 說「iOS 只是一個瘦客戶端」。
本節是把它推進到平台實際允許範圍的設計路徑。誠實估計的天花板大約是
**Android 能力的 60 %**，可透過三個分層的層級（tiers）達成。

### 9.1 硬性約束（不可協商）（The hard constraints, non-negotiable）

| 約束 | 來源 | 含意 |
|------------|--------|-------------|
| 不能 JIT（即時編譯，Just-In-Time compilation） | Apple `App Sandbox` 政策 | 排除任何需要動態程式碼產生（dynamic codegen）的 LLM 執行環境。排除快速 x86 模擬。 |
| 不能 fork-exec 非隨附（non-bundled）的二進位檔 | iOS 核心 + 沙箱 | `git`、`cargo`、`npm`、`python`——除非靜態連結進 app 套件（bundle）並簽章，否則全部禁止 |
| 不能無上限的背景執行 | iOS 任務聲明（task assertion）模型 | App 在切到背景後約 30 秒被暫停。`BGProcessingTask` 給的是最多幾分鐘的爆發式時段，沒有連續迴圈。 |
| 僅限 app 套件檔案系統 | 沙箱 | 無法編輯「使用者的 git repo」，因為根本沒有這種概念。該 repo 必須位於 app 的容器（container）內。 |
| App Store 審查 | 政策，如果透過商店散布 | 禁止「下載程式碼並執行」——被廣義解讀。側載（Sideload，TestFlight / AltStore / TrollStore）可規避此點。 |

### 9.2 iOS 部署的三個層級（Three tiers of iOS deployment）

挑選使用者願意接受的最高層級。

#### Tier 1 — 瘦客戶端（今天的狀態）（Thin client, today's state）

在 WKWebView 中算繪的 `mobile.html`，透過 Tailscale 用 SSE 與協調者對話。
零本機執行。零本機 LLM。可在任何未越獄（non-jailbroken）的 iPhone 上運作，
包括透過 App Store 散布。

它能做的：算繪輸出、接受輸入、推送/接收通知。在協調者可連線時
100 % 有用；離線時無用。

#### Tier 2 — Lite Agent（提議的升級）（the proposed upgrade）

一個原生的 SwiftUI app，透過 TestFlight（需付費的 Apple 開發者帳號）
或 AltStore/SideStore（免費，每 7 天重新簽章一次）側載。內嵌：

- **phantom core 作為靜態程式庫（static library）** — 把 Rust 代理人
  迴圈與 HTTP 客戶端編譯成 `staticlib`，並透過 `swift-bridge`
  或 `UniFFI` 連結。代理人迴圈在行程內（in-process）執行，無子行程。
- **Apple Foundation Models**（iOS 26+）— Apple 隨作業系統出貨的
  裝置端 3B 模型。零模型下載，透過 `FoundationModels` Swift 框架公開。
  用於意圖分類、短摘要、「使用者是在問問題還是下指令」的路由判斷。
- **MLX-Swift 備援** 用於 iOS 17 / 18，或當需要更強的本機模型時。
  在 app 套件內隨附一個 3 B Q4 權重（約 1.8 GB），透過 MLX 載入。
  iPhone 15 Pro+ 可達 15-25 tok/s（每秒詞元數）。
- **Tailscale 網路擴充功能（Network Extension）** — Apple 官方的
  Tailscale app 已經做到這件事。我們的 app 與 localhost 對話；
  Tailscale 永遠開啟的通道讓協調者位址可連線。
- **隨附工具（無子行程）（Bundled tools, no subprocess）** —
  - HTTP 抓取：原生 `URLSession`
  - JSON：原生 `Codable`
  - 唯讀 Git：`SwiftGit2`（libgit2 綁定到 Swift），可在 app 容器內的
    repo 上運作
  - 檔案編輯：僅限沙箱相對路徑
  - WebDAV / Drive / Dropbox 上傳：原生 HTTP，OAuth 透過
    `ASWebAuthenticationSession`

此清單中宣告的 `tools`：

```jsonc
{
  "shell": false,
  "git": "read_only",
  "browser": false,
  "subprocess": false,
  "mobile_driver": null,
  "gpu_inference": "ane",
  "file_edit_scope": "sandbox",
  "long_running": false
}
```

因此規劃器只在以下情況才把任務派發給 iOS Lite Agent：
- 意圖是對話 / 分類 / 摘要，**並且**
- 輸入能塞進本機模型的上下文，**並且**
- 不需要 shell 或外部檔案編輯。

對於其他一切，iOS 裝置就充當對家中協調者的瘦客戶端。
使用者兩種情況下看到的是同樣的 UI；替換（substitution）是透明的。

#### Tier 3 — 進階使用者 TrollStore / 越獄（Power-user TrollStore / jailbreak）

對於使用受支援 iOS 版本（14.0–16.6.1 與 17.0 透過 TrollStore；
任何 iOS 版本的越獄裝置）的使用者，我們可以出貨一個帶有適當權利
（entitlements）的建置，允許 `posix_spawn` 隨附的二進位檔。
那能解鎖約 **80 % 的 Android 能力**——真正的 `git`、真正的
`cargo`、真正包進 IPA 裡的直譯器（interpreters）。

這個層級不走 App Store 路徑。我們在 `docs/INSTALL-IOS-POWER.md`
（待撰寫）中記載它，到此為止。對於任何尚未熟悉 TrollStore 的使用者，
它都不是建議路徑。

### 9.3 彌補仍存在的缺口（Bridging the gaps that remain）

即使在 Tier 2，仍有一些 iOS 無法原生完成的工作。三個逃生口
（escape hatches）讓它們變得透明：

1. **靜默推送喚醒（Silent push wake-up）** — 當協調者有一個低延遲子任務
   要給 iOS lite agent 時（例如「分類這則傳入訊息」），它送出一個
   `content-available: 1` 的 APNs 推送。App 取得約 30 秒來執行並回報。
2. **x-callback-url 呼叫 a-Shell** — 當使用者明確想要一個 shell 指令時，
   我們透過 a-Shell 的 `x-callback-url` 機制開啟它，傳入指令，取回結果。
   這是 iOS 上真正的 shell 執行，只不過是沙箱化在 a-Shell 的容器內，
   而非我們的容器內。對已經安裝 a-Shell 的進階使用者有用。
3. **協調者委派（預設）（Coordinator delegation, the default）** —
   對於其他一切，iOS app 就是一個遙控器。這與 Tier 1 相同，
   只是由規劃器來做替換決策；不是使用者來做。

### 9.4 我們刻意不在 iOS 上做的事（What we deliberately don't do on iOS）

- **不用 iSH** — x86 使用者模式模擬確實很有趣，但對我們的代理人迴圈
  來說太慢。我們不會把它記載為受支援的路徑。
- **不做 Apple Watch 代理人** — RAM 不夠、沒有背景、除了一個「送出提示」
  的表層之外沒有實際使用情境，而那個 iPhone 已經涵蓋了。
- **不做「phantom 作為 Shortcut 動作」** — Shortcuts 整合層很淺
  （URL 機制進、結果出）。可愛，但同樣的捷徑透過 HTTPS 對協調者運作即可，
  根本不需要 iOS app。

---

## 10. 實作路線圖（Implementation Roadmap）

嚴格排序。每一步都是一個完整可出貨的改進。

| # | 步驟 | 工作量 | 前置條件 | 狀態 |
|---|------|--------|---------|--------|
| 1 | 用結構化的 `NodeManifest` 擴充 `PeerStatus` | 1 天 | — | 未開始 |
| 2 | 規劃器讀取清單以做子任務派發 | 2 天 | #1 | 未開始 |
| 3 | 規劃器中的本機與雲端 LLM 路由啟發法 | 1 天 | #2 | 未開始 |
| 4 | 每任務冪等鍵（`task_id` UUID）+ 去重 | 1 天 | — | 未開始 |
| 5 | 子任務逾時 + 心跳遺漏時重新派發 | 2 天 | #4 | 部分完成（心跳存在，重新派發尚無） |
| 6 | iOS Tier-2 SwiftUI app 骨架（尚無 LLM） | 3 天 | #1 | 未開始 |
| 7 | iOS Apple Foundation Models 整合 | 2 天 | #6、iOS 26 目標 | 未開始 |
| 8 | iOS MLX-Swift 備援模型套件 | 2 天 | #6 | 未開始 |
| 9 | Android Termux MediaPipe LLM 供應商 | 2 天 | — | 未開始 |
| 10 | 協調者故障接手（心跳遺漏時重新選舉） | 5 天 | #4、#5 | 未開始 |
| 11 | TrollStore IPA 建置目標 + INSTALL-IOS-POWER.md | 2 天 | #6 | 未開始 |

5/20 上線前的範圍：**以上皆無**。上線出貨的是既有的中樞輻射式架構、
既有的扁平能力清單，以及既有的瘦客戶端 iOS HTML。以上都是上線後的工作。

---

## 11. 非目標（Non-Goals）

- 帶 CRDT 複製狀態的完整網格（10 倍複雜度，在我們的規模下 0 % 使用者好處）
- 在沒有商用中介伺服器的情況下跨 tailnet 聯邦（federation）（用中介伺服器；那就是它存在的原因）
- 對稱的 iOS / Android 功能對等（受平台約束而不可能；我們不會假裝並非如此）
- 跨節點的即時協作編輯（不是群集的職責；那是 Y.js 或 Automerge 的領域）
- 低於 100 毫秒的跨節點任務派發（Tailscale + HMAC + JSON 解析已經要花約 50-150 毫秒；我們接受這個現實）

---

## 12. 開放問題（Open Questions）

1. 清單應該是自我回報（self-reported）還是協調者驗證（coordinator-verified）？自我回報比較簡單，但會讓惡意對等節點誇大宣稱（overclaim）。延後到 phantom-mesh 有多租戶（multi-tenant）部署時再說。
2. 當 iOS Lite Agent 做本機推論但結果是錯的，誰來付雲端重跑的費用？可能的政策：靜默重試一次，然後把成本攤到使用者面前。
3. 當家中協調者無法連線時，由 iOS 發起的工作階段中，規劃器本身要在哪裡執行？兩個選項：
   (a) iOS app 降級為僅本機並掛上橫幅；(b) app 內嵌一個能決定「把這個排入佇列，等協調者回來再說」的微型規劃器。為求簡單傾向選 (a)。

---

*最後更新：2026-05-13。作者：phantom-mesh 維護者。與 [ARCHITECTURE.md](./ARCHITECTURE.md)（單節點）以及 [../mesh/CLUSTER-SCALE.md](../mesh/CLUSTER-SCALE.md)（既有叢集運維）互為搭檔文件。*
