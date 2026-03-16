# Teloxide — Telegram Bot API Rust 函式庫架構文檔

## 1. 專案概覽

**Teloxide** 是一個非同步 Rust Telegram Bot API 用戶端函式庫，建基於 Tokio 執行時。該項目提供完整的 TBA (Telegram Bot API) 9.1 版本支援，並採用現代化非同步設計模式。

### 核心特性
- 完全非同步（基於 Tokio）
- 零不安全程式碼（forbid unsafe_code）
- 適配器模式（Adaptors）支援可組合功能
- 多種傳輸協議支援（原生 TLS、Rustls）
- 自動流量控制與快取機制
- 請求多部分編碼支援

### 版本資訊
- **當前版本**：0.13.0
- **編譯器最低版本**：rustc 1.82+
- **API 版本支援**：Telegram Bot API 9.1

---

## 2. 目錄結構

```
teloxide-core/
├── src/
│   ├── lib.rs                          # 主入口與模組聲明
│   ├── bot.rs                          # Bot 結構與構造方法
│   ├── bot/
│   │   ├── api.rs                      # Requester trait impl
│   │   └── download.rs                 # 檔案下載邏輯
│   ├── adaptors/
│   │   ├── mod.rs                      # 適配器模組管理
│   │   ├── cache_me.rs                 # GetMe 快取適配器
│   │   ├── trace.rs                    # 請求追蹤適配器
│   │   ├── erased.rs                   # 類型擦除適配器
│   │   ├── throttle/
│   │   │   ├── mod.rs                  # 流量控制核心
│   │   │   ├── request.rs              # 單一請求限制
│   │   │   ├── worker.rs               # 背景工作進程
│   │   │   ├── settings.rs             # 限制配置
│   │   │   └── request_lock.rs         # 鎖定機制
│   │   └── parse_mode.rs               # 預設格式模式
│   ├── net/
│   │   ├── mod.rs                      # HTTP 用戶端設定
│   │   ├── download.rs                 # 檔案下載實作
│   │   ├── request.rs                  # HTTP 請求包裝
│   │   └── telegram_response.rs        # Telegram 回應解析
│   ├── payloads/
│   │   ├── mod.rs                      # 負載結構索引
│   │   ├── codegen.rs                  # 程式碼生成（測試用）
│   │   └── [100+ payload files]        # 各式 API 呼叫負載
│   ├── requests/
│   │   ├── mod.rs                      # Requester trait 定義
│   │   ├── payload.rs                  # Payload trait
│   │   └── response.rs                 # 回應結果類型
│   ├── types/
│   │   ├── mod.rs                      # Telegram 類型定義
│   │   ├── message.rs                  # 訊息類型
│   │   ├── chat.rs                     # 聊天類型
│   │   └── [多個類型檔案]              # 使用者、更新等
│   ├── errors.rs                       # 錯誤類型定義
│   ├── prelude.rs                      # 常見類型重新匯出
│   ├── serde_multipart.rs              # 序列化 multipart
│   ├── util.rs                         # 實用工具函式
│   └── local_macros.rs                 # 內部輔助巨集
├── examples/
│   ├── erased.rs                       # 類型擦除示例
│   └── self_info.rs                    # GetMe 示例
├── Cargo.toml                          # 依賴管理與特性旗標
└── tests/                              # 單元測試
```

---

## 3. 核心 Trait 與結構

### 3.1 核心結構

```rust
// bot.rs - 主要請求發送者
pub struct Bot {
    token: Arc<str>,                    // Bot 令牌
    api_url: Arc<reqwest::Url>,        // API 端點 URL
    client: Client,                     // HTTP 用戶端
}
```

**構造方法**：
- `Bot::new(token)` - 使用預設 HTTP 用戶端
- `Bot::with_client(token, client)` - 使用自訂用戶端
- `Bot::from_env()` - 從環境變數讀取（TELOXIDE_TOKEN）
- `Bot::from_env_with_client(client)` - 環境變數 + 自訂用戶端

### 3.2 核心 Trait

```rust
// requests/mod.rs - 請求者 trait
pub trait Requester {
    // 100+ TBA 方法實作位置
    async fn get_me(&self) -> ResponseResult<User>;
    async fn send_message(
        &self,
        chat_id: ChatId,
        text: impl Into<String>,
    ) -> ResponseResult<Message>;
    // ... 其他 API 方法
}

// net/download.rs - 檔案下載 trait
pub trait Download<'a> {
    async fn download_file(
        &self,
        file_path: &str,
    ) -> Result<Vec<u8>, DownloadError>;
}

// 適配器核心
pub trait Adaptor<R: Requester> {
    fn apply(&self, bot: R) -> Self;
}
```

### 3.3 適配器結構

```rust
// adaptors/cache_me.rs - GetMe 快取
pub struct CacheMe<R> {
    inner: R,                          // 底層 Requester
    me: Arc<Mutex<Option<User>>>,      // 快取的使用者資料
}

// adaptors/throttle.rs - 流量控制
pub struct Throttle<R> {
    inner: R,
    settings: Arc<ThrottleSettings>,   // 限制設定
    state: Arc<ThrottleState>,         // 運行時狀態
}

// adaptors/trace.rs - 請求追蹤
pub struct Trace<R> {
    inner: R,
    settings: Arc<TraceSettings>,
}

// adaptors/erased.rs - 類型擦除
pub struct ErasedRequester {
    inner: Arc<dyn ErasedRequesterInterface>,
}
```

### 3.4 負載與回應

```rust
// requests/payload.rs - 負載 trait
pub trait Payload: Serialize + Clone + Debug {
    type Output: DeserializeOwned;
}

// requests/response.rs - 回應類型
pub type ResponseResult<T> = Result<T, RequestError>;

pub enum RequestError {
    Api(ApiError),                     // Telegram API 錯誤
    Network(reqwest::Error),           // 網路錯誤
    Serialize(serde_json::Error),     // 序列化錯誤
}
```

---

## 4. 啟動流程

### 4.1 初始化序列

```
┌─────────────────────────────────────────────────────────┐
│ 應用程式啟動                                              │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 1. 環境設定 / Bot::from_env()                            │
│    - 讀取 TELOXIDE_TOKEN 環境變數                        │
│    - 讀取 TELOXIDE_API_URL（可選）                       │
│    - 讀取 TELOXIDE_PROXY（可選）                         │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 2. HTTP 用戶端初始化 (net::client_from_env)              │
│    - 建立 reqwest::Client                               │
│    - 應用代理設定（若存在）                              │
│    - 設定 TLS（native-tls 或 rustls）                   │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 3. Bot 結構建立 (Bot::with_client)                       │
│    - 儲存令牌（Arc<str>）                                │
│    - 設定 API URL（預設或自訂）                          │
│    - 綁定 HTTP 用戶端                                    │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 4. 適配器應用（選擇性）                                  │
│    bot.throttle(...)      # 流量控制                     │
│    .cache_me()            # 快取 GetMe                   │
│    .trace(...)            # 追蹤請求                     │
│    .erased()              # 類型擦除                     │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 5. 準備就緒 - 可開始發送請求                             │
│    bot.get_me().await                                   │
│    bot.send_message(chat_id, "Hello").await             │
└─────────────────────────────────────────────────────────┘
```

### 4.2 請求處理流程

```
┌─────────────────────────────────────────────────────────┐
│ API 呼叫 (e.g., bot.send_message(...))                  │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 適配器層處理（若已應用）                                │
│ - Throttle: 檢查速率限制                                 │
│ - Trace: 記錄開始                                        │
│ - CacheMe: 檢查快取                                      │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ Payload 建構與序列化                                     │
│ - 組建 SendMessage 負載結構                              │
│ - 序列化為 JSON 或 multipart/form-data                   │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ HTTP 請求發送 (net::request)                            │
│ POST /api/botXXX/sendMessage                            │
│ - 使用 reqwest::Client                                  │
│ - 應用代理、TLS 設定                                     │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ Telegram API 回應                                        │
│ - 解析 JSON 回應                                         │
│ - 驗證 ok 欄位                                           │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 結果處理                                                 │
│ - 成功: 回傳 Message 物件                                │
│ - 失敗: 返回 RequestError                                │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│ 適配器後處理                                             │
│ - Trace: 記錄結束                                        │
│ - CacheMe: 更新快取                                      │
└─────────────────────────────────────────────────────────┘
```

---

## 5. 資料流 ASCII 圖

### 5.1 完整請求週期

```
       User Code
           │
           ▼
    ┌─────────────┐
    │ Bot Instance│
    │  (Client)   │
    └──────┬──────┘
           │
    ┌──────▼──────────────┐
    │  Adaptor Layer      │  ◄─── Throttle, Cache, Trace, Erase
    │  (Optional)         │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────────────┐
    │ Requester Trait Impl        │
    │ (API Methods: send_message, │
    │  get_me, etc.)              │
    └──────┬──────────────────────┘
           │
    ┌──────▼──────────────┐
    │ Payload Generation  │
    │ & Serialization     │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────┐
    │ HTTP Layer          │
    │ (reqwest::Client)   │
    ├─────────────────────┤
    │ - TLS Setup         │
    │ - Proxy Support     │
    │ - Multipart Encode  │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────┐
    │ Network             │
    │ Telegram API        │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────┐
    │ Response Parsing    │
    │ (telegram_response) │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────┐
    │ Deserialization     │
    │ Error Handling      │
    └──────┬──────────────┘
           │
    ┌──────▼──────────────┐
    │ Result<T> Return    │
    │ ResponseResult<T>   │
    └─────────────────────┘
```

### 5.2 適配器堆疊

```
    Application
         │
         ▼
    ┌────────────────────┐
    │  Erased Adaptor    │  ◄─── 類型擦除（可選）
    └─────────┬──────────┘
              │
    ┌─────────▼──────────┐
    │  Trace Adaptor     │  ◄─── 請求追蹤（可選）
    └─────────┬──────────┘
              │
    ┌─────────▼──────────┐
    │ Throttle Adaptor   │  ◄─── 速率限制（可選）
    │ (RateLimiter)      │
    └─────────┬──────────┘
              │
    ┌─────────▼──────────┐
    │ CacheMe Adaptor    │  ◄─── GetMe 快取（可選）
    └─────────┬──────────┘
              │
    ┌─────────▼──────────┐
    │  Core Bot          │  ◄─── 裸露 Bot
    │  (Requester)       │
    └────────────────────┘
```

---

## 6. 子系統清單

### P0 優先級（核心必需）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **Bot 核心** | `bot.rs` | 主要 HTTP 用戶端、令牌管理、API URL 處理 | 穩定 |
| **Requester Trait** | `requests/mod.rs` | 100+ TBA 方法定義與實作 | 穩定 |
| **Net 層** | `net/mod.rs` | HTTP 用戶端設定、TLS/代理支援 | 穩定 |
| **Payload 系統** | `payloads/mod.rs` | API 呼叫負載結構與序列化 | 穩定 |
| **類型系統** | `types/mod.rs` | Telegram 資料結構映射 | 穩定 |
| **錯誤處理** | `errors.rs` | ApiError、RequestError、DownloadError | 穩定 |

### P1 優先級（高級功能）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **Throttle 適配器** | `adaptors/throttle/` | 自動速率限制、背景工作進程 | 穩定 |
| **Cache 適配器** | `adaptors/cache_me.rs` | GetMe 結果快取 | 穩定 |
| **Trace 適配器** | `adaptors/trace.rs` | 請求/回應追蹤與日誌 | 穩定 |
| **Erase 適配器** | `adaptors/erased.rs` | 類型擦除（Box dyn） | 穩定 |
| **檔案下載** | `net/download.rs` | 檔案下載與快取 | 穩定 |

### P2 優先級（支援/測試）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **Multipart 編碼** | `serde_multipart.rs` | multipart/form-data 序列化 | 穩定 |
| **程式碼生成** | `codegen.rs` | TBA schema 代碼生成 | 測試用 |
| **實用工具** | `util.rs` | 輔助函式、巨集 | 穩定 |
| **預設格式** | `adaptors/parse_mode.rs` | 預設 ParseMode 適配 | 穩定 |

---

## 7. 關鍵設計模式

### 7.1 適配器模式（Chain of Responsibility）

```rust
let bot = Bot::from_env()
    .throttle(/* 設定 */)      // 添加限制
    .cache_me()                 // 添加快取
    .trace(/* 設定 */)          // 添加追蹤
    .erased();                  // 類型擦除
```

每層適配器可組合應用，形成請求處理管道。

### 7.2 非同步流程

所有 I/O 操作使用 async/await，基於 Tokio：

```rust
let me = bot.get_me().await?;  // 完全非同步
```

### 7.3 零複製設計

使用 `Arc<str>` 和 `Arc<Url>` 減少記憶體複製：

```rust
pub struct Bot {
    token: Arc<str>,          // 多執行緒安全、低成本複製
    api_url: Arc<reqwest::Url>,
    client: Client,           // 內部共享
}
```

---

## 8. 特性旗標（Features）

| 旗標 | 預設值 | 說明 |
|------|--------|------|
| `native-tls` | ✓ | 使用 native-tls（平台原生 TLS） |
| `rustls` | ✗ | 使用 rustls（Rust 實作） |
| `trace_adaptor` | ✗ | 啟用 Trace 適配器 |
| `erased` | ✗ | 啟用類型擦除適配器 |
| `throttle` | ✗ | 啟用 Throttle 適配器 |
| `cache_me` | ✗ | 啟用 CacheMe 適配器 |
| `full` | ✗ | 啟用所有功能（除 nightly、TLS） |
| `nightly` | ✗ | 啟用 nightly-only 最佳化 |

---

## 9. 常見使用模式

### 9.1 基本訊息發送

```rust
use teloxide_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::from_env();

    let me = bot.get_me().await?;
    println!("Bot: {}", me.user.first_name);

    bot.send_message(ChatId(123), "Hello!").await?;
    Ok(())
}
```

### 9.2 使用適配器

```rust
let bot = Bot::from_env()
    .throttle(ThrottleSettings::default())
    .cache_me()
    .trace(TraceSettings::default());

// 自動流量控制、快取 GetMe、追蹤所有請求
let me = bot.get_me().await?;  // 第二次調用將使用快取
```

### 9.3 自訂 HTTP 用戶端

```rust
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(30))
    .build()?;

let bot = Bot::with_client("TOKEN", client);
```

---

## 10. 執行時考量

### 10.1 成本分析

- **Token 複製成本**：低（Arc）
- **Bot 複製成本**：低（Client 內部共享）
- **請求序列化成本**：中（JSON/multipart）
- **網路 I/O 成本**：高（主要瓶頸）

### 10.2 效能優化

1. **適配器選擇**：根據需要啟用
2. **GetMe 快取**：使用 CacheMe 避免重複調用
3. **速率控制**：使用 Throttle 預防 429 錯誤
4. **連線復用**：reqwest::Client 自動實現

---

## 11. 集成點（與 clawtex-core 相關）

### 11.1 Telegram 訊息輸入

teloxide 可作為 clawtex-core 的 Telegram 後端：

```
Telegram User
    │
    ▼
Telegram Bot API
    │
    ▼
teloxide-core Bot
    │
    ▼
clawtex-core Daemon (port 7878)
    │
    ▼
LLM Model (Ollama/Anthropic/etc)
```

### 11.2 適配器與 clawtex 工具的關係

- **Throttle**：防止 API 限制，類似 clawtex 的 `rate_limit` 配置
- **Cache**：類似 clawtex 的訊息快取
- **Trace**：類似 clawtex 的日誌記錄

---

## 12. 依賴關係圖

```
teloxide-core
├── tokio              # 非同步執行時
├── reqwest            # HTTP 用戶端
├── serde              # 序列化框架
├── serde_json         # JSON 編碼
├── native-tls/rustls  # TLS 實作
└── [dev deps]
    ├── tokio-test
    ├── serde_test
    └── ...
```

---

## 13. 測試策略

### 13.1 單元測試

- 位置：各模組內 `#[cfg(test)]`
- 範圍：負載序列化、類型轉換、錯誤處理

### 13.2 集成測試

- 位置：`tests/` 目錄
- 範圍：完整 API 呼叫（需要有效的 Bot 令牌）

### 13.3 程式碼生成測試

- `codegen.rs`：驗證 TBA schema 和生成的程式碼

---

## 14. 版本歷史與向後相容性

- **0.13.0**：當前版本（支援 TBA 9.1）
- **向後相容性**：遵循語義版本控制
- **特性穩定性**：核心 API 高度穩定，適配器為選擇性

---

## 15. 安全考量

### 15.1 安全策略

```rust
#![forbid(unsafe_code)]  // 零 unsafe 程式碼
```

- 完全依賴 Rust 的型別系統
- reqwest 提供 TLS 安全
- 無記憶體安全漏洞

### 15.2 秘密管理

- Bot 令牌應儲存在環境變數：`TELOXIDE_TOKEN`
- 不在日誌中列印完整令牌
- Trace 適配器可過濾敏感資訊

---

## 16. 故障排除與常見問題

### 16.1 常見錯誤

| 錯誤 | 原因 | 解決方案 |
|-----|------|---------|
| `RequestError::Api(ApiError)` | Telegram 拒絕請求 | 檢查參數、權限、速率限制 |
| `DownloadError::Network` | 網路故障 | 重試、檢查代理設定 |
| 429 Too Many Requests | 超過速率限制 | 使用 Throttle 適配器 |

### 16.2 調試技巧

```rust
let bot = Bot::from_env().trace(Default::default());
// 啟用詳細日誌
```

設定環境變數：

```bash
export RUST_LOG=teloxide=debug
```

---

## 17. 文檔參考

- **官方文檔**：https://docs.rs/teloxide-core
- **GitHub**：https://github.com/teloxide/teloxide
- **Telegram Bot API**：https://core.telegram.org/bots/api

---

## 18. 小結

Teloxide 是高質量 Rust Telegram Bot API 客戶端，設計上強調：

1. **非同步優先**：完全基於 Tokio
2. **可組合性**：適配器模式支援功能疊加
3. **安全性**：零 unsafe 程式碼
4. **易用性**：簡潔的 API 設計
5. **靈活性**：支援自訂 HTTP 用戶端、代理、TLS

與 clawtex-core 集成時，可作為高性能的 Telegram 訊息來源，並通過適配器層實現流量控制、快取等高級功能。

