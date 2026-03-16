# ZeroClaw 架構概覽

## 專案定位

**ZeroClaw** 是一個高效能的 Rust 原生自主代理運行時，針對最小化開銷、最大化可靠性和可擴展性優化。採用零假設設計：不依賴 Python、不依賴容器，200KB 二進制、毫秒級啟動、支援從樹莓派到伺服器的全平台執行。核心特點是特徵驅動（trait-driven）模組化架構，易於擴展提供者、通道、工具、記憶體後端和硬體週邊。

---

## 目錄結構

### 核心編排

| 模組 | 用途 |
|------|------|
| `agent/` | Agent 物件、構建器、LLM 迴圈、記憶體載入器、工具分派、提示構建 |
| `agent/agent.rs` | Agent 核心類別，管理提供者、工具、記憶體、觀察者 |
| `agent/loop_.rs` | 主編排迴圈 — 使用者訊息→記憶體載入→LLM 呼叫→工具執行→回應 |
| `agent/dispatcher.rs` | 工具分派器 — 解析 LLM 函數呼叫、執行、彙聚結果 |
| `agent/prompt.rs` | 系統提示構建 — 注入工具規格、記憶體、身份、技能 |
| `agent/classifier.rs` | 查詢分類器 — 路由訊息至最佳模型/提供者 |

### 配置與啟動

| 模組 | 用途 |
|------|------|
| `config/` | TOML 配置載入、驗證、結構描述匯出 |
| `config/schema.rs` | 配置資料結構（Agent、通道、提供者、硬體、隧道） |
| `config/traits.rs` | 設定特徵（擴展點） |
| `runtime/` | 執行環境適配器（原生、Docker、邊緣） — 宣告能力、適配行為 |

### 通訊通道

| 模組 | 用途 |
|------|------|
| `channels/` | Telegram、Discord、Slack、Matrix、Lark、MQTT、Nostr、WhatsApp、郵件等 |
| `channels/traits.rs` | Channel 特徵 — 接收訊息、傳送回應、訂閱/取消訂閱 |
| `channels/telegram.rs` | Telegram 輪詢/webhook 整合 |
| `channels/discord.rs` | Discord WebSocket 客戶端 |
| `channels/email_channel.rs` | IMAP/SMTP 郵件 |

### 提供者

| 模組 | 用途 |
|------|------|
| `providers/` | Anthropic、OpenAI、Ollama、Gemini、Groq、GLM、旋轉、分類器、金鑰池 |
| `providers/traits.rs` | Provider 特徵 — 聊天完成、流、能力宣告 |
| `providers/anthropic.rs` | Anthropic Claude API（含擴展思考） |
| `providers/openai.rs` | OpenAI GPT 模型 |
| `providers/ollama.rs` | 本地 Ollama 推論 |
| `providers/router.rs` | 動態路由多個提供者 |
| `providers/fallback.rs` | 故障轉移策略 |

### 記憶體與向量搜尋

| 模組 | 用途 |
|------|------|
| `memory/` | Markdown、SQLite、Qdrant 向量、嵌入、快取、快照 |
| `memory/traits.rs` | Memory 特徵 — 儲存、回憶、忘記、搜尋 |
| `memory/markdown.rs` | 本地 Markdown 檔案持久化 |
| `memory/vector.rs` | Qdrant 向量資料庫整合 |
| `memory/embeddings.rs` | 嵌入模型（Nomic、OpenAI）及批次編碼 |
| `memory/snapshot.rs` | 時間點記憶體快照 |

### 工具與技能

| 模組 | 用途 |
|------|------|
| `tools/` | 24+ 工具：shell、檔案讀寫編輯、web 搜尋、HTTP、glob、記憶體、瀏覽器、email、PDF 匯出等 |
| `tools/traits.rs` | Tool 特徵 — 執行、參數架構、規格 |
| `tools/shell.rs` | 殼層執行（含沙箱隔離） |
| `tools/file_read.rs`、`file_write.rs`、`file_edit.rs` | 檔案操作 |
| `tools/glob_search.rs`、`content_search.rs` | 檔案系統探索 |
| `tools/browser.rs` | Headless 瀏覽器自動化 |
| `skills/` | 技能 — 高階能力組成（寫程式、部署、內容生成等） |

### 安全與隔離

| 模組 | 用途 |
|------|------|
| `security/` | Estop、沙箱（Landlock、Bubblewrap、Firejail）、配對、祕密加密儲存 |
| `security/traits.rs` | Sandbox 特徵 — 包裹命令、平台可用性檢查 |
| `security/estop.rs` | 緊急停止開關 — 所有迴圈內可檢查中止訊號 |
| `security/pairing.rs` | 設備配對驗證（Telegram 一次性代碼） |
| `security/secret_store.rs` | ChaCha20-Poly1305 加密憑證 |

### 觀測與成本追蹤

| 模組 | 用途 |
|------|------|
| `observability/` | 事件、日誌、Prometheus 指標、OpenTelemetry OTLP 匯出 |
| `observability/traits.rs` | Observer 特徵 — 發出事件（代理開始/結束、LLM 要求、工具呼叫） |
| `cost/` | 成本追蹤 — 令牌計算、估計 USD 成本、SQLite 記錄 |
| `cost/tracker.rs` | 提供者成本查詢及彙總 |
| `health/` | 健康檢查 — 元件狀態旗標 |

### 閘門與工作流

| 模組 | 用途 |
|-----|------|
| `approval/` | 人類迴圈 Telegram 核准要求 |
| `cron/` | 排程工作 — cron 表達式、儲存、執行 |
| `sop/` | SOP（標準作業程序）多階段工作流、條件閘門、派遣、審計 |

### 硬體與週邊

| 模組 | 用途 |
|------|------|
| `peripherals/` | STM32、樹莓派 GPIO、Arduino、週邊工廠 |
| `peripherals/traits.rs` | Peripheral 特徵 — 連接、斷開、工具公開 |
| `hardware/` | 硬體內省 — USB 列舉、序列埠發現、記憶體讀取 |
| `hardware/introspect.rs` | 主機板識別（nusb、probe-rs） |

### 通道與隧道

| 模組 | 用途 |
|------|------|
| `gateway/` | Axum HTTP 伺服器 — webhook、WebSocket、SSE、REST API |
| `gateway/api.rs` | REST 端點 — 訊息、記憶體、cron、SOP 管理 |
| `gateway/ws.rs` | WebSocket — 即時聊天、流式回應 |
| `gateway/sse.rs` | 伺服器傳送事件 — 進度流 |
| `tunnel/` | Cloudflare、ngrok、Tailscale、自訂隧道 — 公開內部網關 |

### 啟動與後臺程序

| 模組 | 用途 |
|------|------|
| `daemon.rs` | 後臺程序迴圈 — 啟動通道、閘門、心跳、狀態寫入器 |
| `heartbeat/` | 心跳引擎 — 定期健康檢查檔案、故障恢復 |
| `service.rs` | systemd 單元生成 |

### RAG 與多模態

| 模組 | 用途 |
|------|------|
| `rag/` | 文件提取、PDF 解析、分塊、向量化 |
| `multimodal/` | 視覺理解 — 螢幕截圖、圖像分析、OCR |

### 身份與認證

| 模組 | 用途 |
|------|------|
| `identity/` | 身份設定文件（名稱、背景、行為風格） |
| `auth/` | Anthropic token、OAuth（Gemini、OpenAI）、基本認證 |

---

## 核心特徵與 Struct

### 編排迴圈

| 名稱 | 檔案 | 用途 |
|------|------|------|
| `Agent` | `src/agent/agent.rs` | 主代理類別 — 管理工具、記憶體、提供者、觀察者、歷史紀錄 |
| `AgentBuilder` | `src/agent/agent.rs` | 構建器模式 — 組合代理依賴 |
| `ToolDispatcher` | `src/agent/dispatcher.rs` | 特徵 — 解析 XML/JSON 工具呼叫並執行 |
| `NativeToolDispatcher` | `src/agent/dispatcher.rs` | 實作 — Anthropic native_tools 格式 |
| `MemoryLoader` | `src/agent/memory_loader.rs` | 特徵 — 載入相關記憶體到提示 |

### 核心特徵（擴展點）

| 特徵名稱 | 檔案 | 說明 |
|---------|------|------|
| `Provider` | `src/providers/traits.rs` | 6 個方法 — 聊天完成、流、能力檢查、成本、標記化 |
| `Tool` | `src/tools/traits.rs` | 4 個方法 — 名稱、描述、參數架構、執行 |
| `Memory` | `src/memory/traits.rs` | 8 個方法 — 儲存、回憶、忘記、搜尋、匯出、清理 |
| `Channel` | `src/channels/traits.rs` | 4 個方法 — 啟動、停止、傳送、訂閱接收器 |
| `Observer` | `src/observability/traits.rs` | 1 個方法 — 發出事件 |
| `RuntimeAdapter` | `src/runtime/traits.rs` | 6 個方法 — 名稱、能力宣告、儲存路徑、記憶體預算 |
| `Sandbox` | `src/security/traits.rs` | 3 個方法 — 包裹命令、可用性檢查、名稱 |
| `Peripheral` | `src/peripherals/traits.rs` | 5 個方法 — 連接、斷開、工具公開、狀態查詢 |

### 配置

| 結構 | 檔案 | 用途 |
|------|------|------|
| `Config` | `src/config/schema.rs` | 頂層配置 — 代理、提供者、通道、記憶體、安全等 |
| `AgentConfig` | `src/config/schema.rs` | 代理參數 — 模型、溫度、工具迭代限制 |
| `ChannelsConfig` | `src/config/schema.rs` | 所有通道的配置集合 |
| `SecurityConfig` | `src/config/schema.rs` | 安全策略、沙箱選擇、速率限制 |

---

## 啟動流程

### main.rs → 初始化順序

1. **CLI 解析** (`main.rs`)
   - Clap 指令解析 (daemon, chat, config, doctor, onboard, gateway)

2. **配置載入** (`main.rs` → `config/mod.rs`)
   - 讀取 `~/.config/zeroclaw.toml` (或 Windows XDG)
   - 環境變數覆蓋
   - TOML 驗證及結構序列化

3. **日誌初始化** (`main.rs`)
   - tracing-subscriber 設定 (formatter、環境篩選)
   - OpenTelemetry OTLP 匯出 (可選)

4. **安全初始化** (`security/mod.rs`)
   - 沙箱後端選擇 (Landlock > Bubblewrap > Firejail > 無)
   - Estop 訊號監聽 (SIGTERM/SIGINT)
   - 祕密儲存解密

5. **記憶體後端啟動** (`memory/mod.rs`)
   - Markdown 或 SQLite 初始化
   - Qdrant 向量 DB 連接 (如果啟用)
   - 嵌入模型載入

6. **提供者建立** (`providers/mod.rs`)
   - 選定的主提供者 (Anthropic/OpenAI/Ollama/等)
   - 旋轉、故障轉移、分類器包裝
   - 祕密鑰匙注入

7. **工具工廠** (`tools/mod.rs`)
   - 根據配置啟用工具 (shell、檔案、web、瀏覽器、email 等)
   - 沙箱包裝

8. **通道啟動** (`channels/mod.rs` → `daemon.rs`)
   - 各已配置通道的非同步工作 (Telegram 輪詢、Discord WebSocket)
   - 訂閱使用者訊息流

9. **Agent 構建** (`agent/agent.rs`)
   - 組合所有依賴 (工具、記憶體、提供者、觀察者)
   - 系統提示建立

10. **Gateway 啟動** (`gateway/mod.rs`)
    - Axum 路由註冊 (REST、WebSocket、SSE)
    - 監聽埠 (預設 8080)

11. **後臺程序迴圈** (`daemon.rs`)
    - 啟動通道監督者、閘門引擎、心跳迴圈
    - 等候 Ctrl+C / SIGTERM

---

## 資料流圖

```
┌─────────────────────────────────────────────────────────────────┐
│ 入口點                                                           │
├─────────────────────────────────────────────────────────────────┤
│  Telegram        Discord        IMAP/SMTP      Webhook   CLI    │
│     │               │               │             │        │     │
└──────┬───────────────┬───────────────┬─────────────┬────────┘    │
       │               │               │             │             │
       └───────────────┼───────────────┼─────────────┘             │
                       ▼               ▼                            │
             ┌─────────────────────────────┐                       │
             │   Channel Inbox Queue       │                       │
             │  (Arc<Mutex<VecDeque>>)     │                       │
             └────────────┬────────────────┘                       │
                          │                                        │
                          ▼                                        │
        ┌─────────────────────────────────┐                       │
        │  agent/loop_.rs::process_msg    │                       │
        │  (主編排迴圈)                     │                       │
        └────────┬────────────────────────┘                       │
                 │                                                 │
        ┌────────▼─────────────────────┐                          │
        │  1. 記憶體載入器              │  (DefaultMemoryLoader)  │
        │     related entries → prompt  │  搜尋相關記憶體         │
        └────────┬─────────────────────┘                          │
                 │                                                 │
        ┌────────▼──────────────────────────┐                     │
        │  2. 系統提示構建                  │                     │
        │     + 工具規格 JSON                │  (SystemPromptBuilder)
        │     + 記憶體上下文                │                     │
        │     + 身份設定                    │                     │
        │     + 技能 (可選)                 │                     │
        └────────┬──────────────────────────┘                     │
                 │                                                 │
        ┌────────▼──────────────────────────┐                     │
        │  3. LLM 呼叫                      │                     │
        │     Provider::chat_completion()   │                     │
        │     （anthropic, openai, etc）    │                     │
        └────────┬──────────────────────────┘                     │
                 │                                                 │
        ┌────────▼────────────────────────────┐                   │
        │  4. 工具分派 (if tool calls)        │                   │
        │     - 解析工具呼叫 (Native/XML)     │                   │
        │     - 取得工具規格                  │                   │
        │     - 驗證參數                      │                   │
        │     - 執行工具 (沙箱隔離)           │                   │
        │     - 彙聚結果                      │                   │
        └────────┬────────────────────────────┘                   │
                 │                                                 │
        ┌────────▼────────────────────────────────┐               │
        │  5. 迴圈控制                           │               │
        │     - max_tool_iterations 檢查         │               │
        │     - 繼續 (回步驟 3) 或終止迴圈       │               │
        └────────┬────────────────────────────────┘               │
                 │                                                 │
        ┌────────▼──────────────┐                                  │
        │  6. 回應準備          │                                  │
        │     - 憑證刮擦         │  (scrub_credentials)            │
        │     - 流式寫入         │  (若配置流式傳輸)               │
        │     - 記憶體自動儲存   │  (若啟用)                      │
        └────────┬──────────────┘                                  │
                 │                                                 │
        ┌────────▼──────────────────────┐                          │
        │  7. Channel 傳送               │                          │
        │     channel.send_message()     │                          │
        │     (Telegram、Discord 等)     │                          │
        └────────┬──────────────────────┘                          │
                 │                                                 │
        ┌────────▼──────────────┐                                  │
        │  觀察者發出事件       │                                  │
        │  (成本、效能指標)    │                                  │
        └───────────────────────┘                                  │
```

### 詳細流程（tool 迴圈）

```
[LLM 回應: tool_calls=[...]]
          │
          ▼
┌──────────────────────────┐
│ Tool Dispatcher          │
│  .parse_tool_calls()     │  (原生或 XML)
└──────┬───────────────────┘
       │
       ├─→ for each call:
       │
       ├────┬─────────────────────────────────┐
       │    │ Tool Registry Lookup             │
       │    │ tool_name → Box<dyn Tool>        │
       └────┼─────────────────────────────────┘
            │
            ▼
       ┌─────────────────────────┐
       │ Tool::execute()         │
       │ (非同步)                │
       │                         │
       │ [Sandbox 隔離]          │ (if shell tool)
       │ [憑證刮擦]              │ (after execution)
       │                         │
       │ ToolResult {            │
       │   success: bool,        │
       │   output: String,       │
       │   error: Option<String> │
       │ }                       │
       └────┬────────────────────┘
            │
            ├─ next iteration (步驟 3)
            │  或
            ├─ 終止 (max iterations 達到)
```

---

## 子系統清單

### P0 — 至關重要

| 子系統 | 簡述 | 負責檔案 |
|--------|------|---------|
| **代理迴圈** | 使用者訊息→記憶體→提示→LLM→工具分派→回應 | `agent/{loop_,agent,dispatcher}.rs` |
| **提供者抽象** | 統一的 LLM API — 支援多個後端故障轉移 | `providers/traits.rs` + `{anthropic,openai,ollama}.rs` |
| **工具執行** | 沙箱隔離、安全性檢查、結果彙聚 | `tools/{traits,shell,dispatcher}.rs` + `security/` |
| **配置管理** | TOML 載入、驗證、執行時覆蓋 | `config/{mod,schema}.rs` |
| **記憶體持久化** | 長期事實、會話日誌的儲存與回憶 | `memory/{traits,markdown,vector}.rs` |

### P1 — 重要功能

| 子系統 | 簡述 | 負責檔案 |
|--------|------|---------|
| **通道多工** | 同時監聽多個輸入源（Telegram、Discord、WebSocket） | `channels/{mod,traits,telegram,discord}.rs` |
| **Gateway 伺服器** | HTTP/REST/WebSocket API、webhook 入口 | `gateway/{mod,api,ws,sse}.rs` |
| **人類核准迴圈** | 敏感操作的 Telegram 確認 | `approval/mod.rs` |
| **成本追蹤** | 令牌計算、USD 估算、SQLite 日誌 | `cost/{mod,tracker}.rs` |
| **安全隔離** | 沙箱（Landlock/Bubblewrap）、Estop、祕密加密 | `security/{sandbox,estop,secret_store}.rs` |

### P2 — 可選擴展

| 子系統 | 簡述 | 負責檔案 |
|--------|------|---------|
| **工作流/SOP** | 多階段工作流、條件閘門、派遣 | `sop/{mod,gates,dispatch}.rs` |
| **排程工作** | Cron 表達式、定期執行 | `cron/{schedule,store}.rs` |
| **硬體週邊** | STM32/RPi GPIO 連接與工具公開 | `peripherals/{traits,mod}.rs` + `hardware/` |
| **RAG 文件** | PDF 解析、分塊、向量化 | `rag/mod.rs` + `memory/vector.rs` |
| **多模態視覺** | 螢幕截圖、圖像分析、OCR | `multimodal/mod.rs` |
| **隧道與公開** | Cloudflare/ngrok/Tailscale 公開內部網關 | `tunnel/{cloudflare,ngrok,tailscale}.rs` |

### P3 — 社群/實驗

| 子系統 | 簡述 | 負責檔案 |
|--------|------|---------|
| **技能鍛造** | AI 自動化發現、評估、集成新工具 | `skillforge/{scout,evaluate,integrate}.rs` |
| **觀測可觀測性** | OpenTelemetry OTLP、Prometheus 指標 | `observability/{multi,runtime_trace}.rs` |
| **身份與角色扮演** | 自訂身份檔案、行為風格注入 | `identity/mod.rs` |

---

## 擴展點速查

### 新增提供者

實作 `src/providers/traits.rs::Provider` 特徵：
- `name()`, `model()` — 識別
- `chat_completion()`, `stream()` — LLM 呼叫
- `get_cost()`, `validate_request()` — 成本與驗證
- 註冊於 `providers/mod.rs::factory()`

**範例**: Ollama、Groq、GLM、Bedrock

### 新增通道

實作 `src/channels/traits.rs::Channel` 特徵：
- `start()` — 啟動偵聽迴圈
- `send_message()` — 傳送回應
- `name()` — 識別
- 註冊於 `channels/mod.rs::factory()`

**範例**: WhatsApp、Slack、iMessage、Matrix

### 新增工具

實作 `src/tools/traits.rs::Tool` 特徵：
- `name()`, `description()` — 元資料
- `parameters_schema()` — JSON 架構
- `execute()` — 非同步執行
- 註冊於 `tools/mod.rs::builtin_tools()`

**範例**: shell、檔案、browser、email、API 呼叫

### 新增記憶體後端

實作 `src/memory/traits.rs::Memory` 特徵：
- `store()`, `recall()`, `forget()` — 基本操作
- `search()`, `export()` — 查詢
- 註冊於 `memory/mod.rs::factory()`

**範例**: SQLite、Qdrant 向量、Postgres

### 新增硬體週邊

實作 `src/peripherals/traits.rs::Peripheral` 特徵：
- `connect()`, `disconnect()` — 生命週期
- `tools()` — 暴露的工具
- 註冊於 `peripherals/mod.rs::factory()`

**範例**: STM32 序列埠、RPi GPIO、Arduino

---

## 執行流程摘要

```
zeroclaw daemon
  ├─ Config 載入 (TOML)
  ├─ 日誌 + 安全初始化
  ├─ 記憶體後端啟動 (SQLite/Markdown + Qdrant)
  ├─ 提供者構建 (Anthropic + 旋轉/故障轉移)
  ├─ 工具工廠 (shell/file/browser/email + 沙箱)
  ├─ 通道啟動非同步迴圈
  │  ├─ Telegram 輪詢 / Webhook
  │  ├─ Discord WebSocket
  │  ├─ IMAP 監聽
  │  └─ ...
  ├─ Agent 構建 (工具 + 記憶體 + 提供者 + 觀察者)
  ├─ Gateway 啟動 (Axum, REST/WS)
  └─ 後臺迴圈
     ├─ 通道監督者 (故障恢復)
     ├─ 心跳引擎 (健康檢查)
     └─ 等候 SIGTERM / Ctrl+C

  使用者訊息進來 (任何通道)
    → Agent::process_message()
      → 記憶體載入
      → 系統提示構建
      → Provider::chat_completion()
      → (tool calls?) → 工具分派與執行
      → (循環?) → 下一輪 LLM 呼叫
      → 回應傳送 (channel.send_message)
      → 觀察者事件發出 (成本、指標)
```

---

## 參考

- **Cargo.toml**: 編譯設定、特徵旗標（hardware、channel-*、observability-otel、sandbox-*）
- **CLAUDE.md**: PR 準則、風險分級、反模式
- **docs/**: 設定指南、API 文件、硬體周邊、安全政策
- **test/**: 元件、整合、系統、實況測試
