# OpenCrust 深度技術分析（加深版）

> **專案**: opencrust-org/opencrust
> **版本**: 0.1.21 (Rust 2024 edition, rust-version 1.85)
> **授權**: MIT
> **分析日期**: 2026-03-13
> **分析目的**: 深度解剖 OpenCrust 架構設計，重點分析 Config 熱重載、AES-256-GCM 金鑰保險庫、Channel Lifecycle/Sender 分離、Arc-safe 共用狀態等核心模式，為 clawtex-core 提供具體可移植的 Rust 實作參考

---

## 目錄

1. [專案結構與 Workspace 架構](#1-專案結構與-workspace-架構)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [核心 Trait 設計](#3-核心-trait-設計)
4. [Config 熱重載系統（深度分析）](#4-config-熱重載系統深度分析)
5. [AES-256-GCM 金鑰保險庫 + OS Keychain（深度分析）](#5-aes-256-gcm-金鑰保險庫--os-keychain深度分析)
6. [Channel Lifecycle / Sender 分離（深度分析）](#6-channel-lifecycle--sender-分離深度分析)
7. [Arc-safe 共用狀態 AppState（深度分析）](#7-arc-safe-共用狀態-appstate深度分析)
8. [AgentRuntime 與工具循環](#8-agentruntime-與工具循環)
9. [WebSocket 閘道安全設計](#9-websocket-閘道安全設計)
10. [輸入驗證與 Prompt Injection 偵測](#10-輸入驗證與-prompt-injection-偵測)
11. [日誌 Redaction 機制](#11-日誌-redaction-機制)
12. [向量搜尋與記憶體系統](#12-向量搜尋與記憶體系統)
13. [Context Window 管理與 Rolling Summarization](#13-context-window-管理與-rolling-summarization)
14. [DNA 個性化系統](#14-dna-個性化系統)
15. [Provider 系統](#15-provider-系統)
16. [MCP 工具橋接](#16-mcp-工具橋接)
17. [排程系統（Heartbeat）](#17-排程系統heartbeat)
18. [測試架構](#18-測試架構)
19. [與 clawtex-core 的全面對比](#19-與-clawtex-core-的全面對比)

---

## 1. 專案結構與 Workspace 架構

OpenCrust 採用 11-crate Cargo workspace，實現嚴格的關注點分離：

```
opencrust/
  Cargo.toml                     # workspace 根設定
  crates/
    opencrust-cli/               # CLI 二進位入口點、daemon 管理、init wizard
    opencrust-gateway/           # axum WebSocket 閘道、HTTP API、session 管理
    opencrust-config/            # YAML/TOML 載入、熱重載（notify crate）
    opencrust-channels/          # Telegram、Discord、Slack、WhatsApp、iMessage 頻道
    opencrust-agents/            # LLM provider trait、工具系統、MCP 客戶端、Agent Runtime
    opencrust-db/                # SQLite 記憶體、向量搜尋（sqlite-vec）、session store
    opencrust-plugins/           # WASM 外掛沙箱（wasmtime, feature-gated）
    opencrust-media/             # 多媒體處理（scaffolded, 未完成）
    opencrust-security/          # 加密金鑰保險庫、allowlist、pairing code、input validation
    opencrust-skills/            # SKILL.md 解析器、掃描器、安裝器
    opencrust-common/            # 共用型別、Error enum、Message struct
  docs/                          # mdBook 文件
  assets/                        # logo、webchat 前端 JS/CSS
```

### 關鍵 Workspace 依賴

| 類別 | Crate | 用途 |
|------|-------|------|
| 非同步執行 | `tokio` (full) | 核心 async runtime |
| HTTP/WS | `axum` 0.8 + ws、`reqwest`、`tower-http` | 閘道伺服器 + HTTP 客戶端 |
| 序列化 | `serde`、`serde_json`、`serde_yaml`、`toml` | 多格式設定 |
| 資料庫 | `rusqlite` (bundled)、`sqlite-vec` | SQLite + 向量搜尋 |
| 加密 | `ring` (AES-256-GCM, PBKDF2)、`keyring` | 金鑰保險庫 + OS keychain |
| 頻道 | `teloxide`、`serenity`/`poise`、`tokio-tungstenite` | Telegram、Discord、Slack |
| 檔案監聽 | `notify` | 設定熱重載 |
| MCP | `rmcp` 1.1 | Model Context Protocol |
| 外掛 | `wasmtime` 41 | WASM 沙箱（feature-gated） |
| 並行 Map | `dashmap` | 無鎖並發 HashMap |

### Release Profile

```toml
[profile.release]
lto = true          # 完整連結時間最佳化
codegen-units = 1   # 最大最佳化
strip = true        # 移除 debug symbols
panic = "abort"     # 不展開 stack
```

結果：16 MB 單一二進位檔、13 MB 閒置記憶體、3 ms 冷啟動。

### Workspace 架構的優勢

1. **獨立編譯快取** — 修改 `opencrust-security` 不重新編譯 `opencrust-channels`
2. **Feature-gated 選擇性編譯** — 不需 Discord 的用戶不需編譯 serenity
3. **清晰的依賴邊界** — crate 之間的依賴關係明確可追蹤
4. **並行測試** — 各 crate 的 `#[cfg(test)]` 獨立執行

---

## 2. 進入點與啟動流程

**檔案**: `crates/opencrust-cli/src/main.rs`

### CLI 架構

```rust
#[derive(Parser)]
#[command(name = "opencrust", version, about = "OpenCrust - Personal AI Assistant")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(long, default_value = "info", global = true)]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    Start { host, port, daemon },
    Stop,
    Restart { host, port, daemon },
    Status,
    Init,
    Channel { action },
    Plugin { action },   // #[cfg(feature = "plugins")]
    Skill { action },
    Mcp { action },
    Migrate { action },
    Update { yes },
    Rollback,
}
```

### GatewayServer::run() 完整啟動序列

**檔案**: `crates/opencrust-gateway/src/server.rs` (行 35-290)

```
GatewayServer::run()
  |
  1.  build_agent_runtime(&config)           # 建立 AgentRuntime + 註冊 providers
  2.  build_mcp_tools(&config)               # 連接 MCP 伺服器、橋接工具
  3.  build_channels(&config)                # 建立 ChannelRegistry
  4.  AppState::new(config, agents, channels) # 組裝共用狀態
  5.  SessionStore::open(sessions.db)        # SQLite session 持久化
  6.  註冊 Schedule/Cancel/List Heartbeat 工具
  7.  ConfigWatcher::start(config.yml)       # 設定熱重載 (notify crate)
  8.  Box::leak(watcher)                     # 延長 watcher 生命週期至程序結束
  9.  state.set_config_watcher(rx)           # 注入 watch::Receiver
  10. Arc::new(state)                        # 所有欄位凍結為 Arc
  11. spawn_session_cleanup()                # 定時清理過期 session
  12. spawn_config_applier()                 # 監聽設定變更
  13. spawn_dna_watcher()                    # 監聽 dna.md 熱重載
  14. MCP health monitor                     # 自動重連 MCP 伺服器
  15. 依序啟動 Discord、Scheduler、Telegram、Slack、iMessage、WhatsApp 頻道
  16. build_router() -> axum::serve()        # 綁定 TCP，啟動 HTTP/WS 閘道
```

啟動順序的關鍵設計：

```rust
// crates/opencrust-gateway/src/server.rs:122
let state = Arc::new(state);

// 頻道啟動模式：create_sender() 先抽出輕量 handle
// 然後 channel 本身 move into spawn
let telegram_channels = build_telegram_channels(&state.config, &state);
for mut channel in telegram_channels {
    let sender: Arc<dyn ChannelSender> = Arc::from(channel.create_sender());
    state.channel_senders.insert(sender.channel_type().to_string(), sender);
    tokio::spawn(async move {
        if let Err(e) = channel.connect().await {
            warn!("telegram channel failed to connect: {e}");
            return;
        }
        shutdown_signal().await;
        channel.disconnect().await.ok();
    });
}
```

**重要設計決策**：

1. `Box::leak(watcher)` — ConfigWatcher 被故意 leak，確保 notify 的 watcher 在整個程序生命週期內活著。這是一個正確的權衡：程序結束時 OS 回收記憶體，避免了用 Arc 跨越多個 spawn 的複雜性。

2. **daemon fork 在 tokio runtime 之前** — 避免 fork 後 epoll/kqueue FD 失效。

3. **graceful shutdown** — `tokio::select!` 同時監聽 Ctrl+C 和 SIGTERM：

```rust
// crates/opencrust-gateway/src/server.rs:293-315
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => info!("received Ctrl+C, shutting down"),
        () = terminate => info!("received SIGTERM, shutting down"),
    }
}
```

---

## 3. 核心 Trait 設計

### LlmProvider Trait

**檔案**: `crates/opencrust-agents/src/providers.rs`

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse>;
    async fn stream_complete(&self, _request: &LlmRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;
    fn configured_model(&self) -> Option<&str>;
    async fn available_models(&self) -> Result<Vec<String>>;
    async fn health_check(&self) -> Result<bool>;
}
```

特色：`stream_complete()` 有預設實作回傳不支援錯誤，允許 provider 選擇性實作串流。

### Tool Trait

**檔案**: `crates/opencrust-agents/src/tools/mod.rs`

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn execute(&self, context: &ToolContext, input: serde_json::Value) -> Result<ToolOutput>;
}

pub struct ToolContext {
    pub session_id: String,
    pub user_id: Option<String>,
    pub heartbeat_depth: u8,  // 0=正常, 1+=排程執行, 最多到 3 層
}
```

### Error Enum

**檔案**: `crates/opencrust-common/src/error.rs`

```rust
#[derive(Error, Debug)]
pub enum Error {
    Config(String),
    Channel(String),
    Agent(String),
    Database(String),
    Plugin(String),
    Security(String),
    Media(String),
    Gateway(String),
    Skill(String),
    Mcp(String),
    Io(#[from] std::io::Error),
    Serialization(#[from] serde_json::Error),
    NotFound(String),
    Unauthorized(String),
    Other(String),
}
```

---

## 4. Config 熱重載系統（深度分析）

**檔案**: `crates/opencrust-config/src/watcher.rs` (96 行)

### 4.1 完整原始碼解析

```rust
// crates/opencrust-config/src/watcher.rs:1-96
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;

const DEBOUNCE_MS: u64 = 500;

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,  // 持有 watcher 防止 drop
}
```

ConfigWatcher 的核心設計是一個 **三層管道**：

```
[notify crate filesystem events]
         |
    event filter (EventKind::Create | Modify + filename match)
         |
    mpsc channel (buffer=8)
         |
    debounce (500ms sleep + drain)
         |
    reload_config() → parse YAML/TOML
         |
    watch::channel → broadcast to all receivers
```

### 4.2 檔案系統事件處理

```rust
// crates/opencrust-config/src/watcher.rs:22-53
pub fn start(
    config_path: PathBuf,
    initial_config: AppConfig,
) -> Result<(Self, watch::Receiver<AppConfig>), notify::Error> {
    let (tx, rx) = watch::channel(initial_config);

    // 關鍵設計：監聽父目錄而非檔案本身
    let watch_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let target_filename = config_path.file_name().unwrap_or_default().to_os_string();

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(8);

    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                let dominated =
                    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));
                if dominated {
                    let touches_config = event.paths.iter()
                        .any(|p| p.file_name().map(|f| f == target_filename).unwrap_or(false));
                    if touches_config {
                        let _ = notify_tx.try_send(());
                    }
                }
            }
        })?;

    watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;
```

**為什麼監聽父目錄而非檔案本身？**

這是一個常被忽略的重要設計。大多數文字編輯器（vim、VS Code、nano）的保存機制是：

1. 寫入臨時檔案（如 `config.yml.tmp`）
2. 原子 rename 為目標檔名

如果直接監聽 `config.yml`，在步驟 1 時原檔案可能被刪除/取代，notify watcher 會失去對該 inode 的追蹤。監聽父目錄則能捕捉到 rename（Create）事件。

### 4.3 Debounce 機制

```rust
// crates/opencrust-config/src/watcher.rs:55-82
tokio::spawn(async move {
    loop {
        // 等待任何 filesystem event
        if notify_rx.recv().await.is_none() {
            break; // channel closed
        }
        // Debounce: 等待 500ms，然後 drain 所有累積的事件
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
        while notify_rx.try_recv().is_ok() {}

        // 重新讀取設定
        match reload_config(&cfg_path) {
            Ok(new_config) => {
                info!("config reloaded from {}", cfg_path.display());
                let _ = tx.send(new_config);
            }
            Err(e) => {
                warn!("config reload failed (keeping previous config): {e}");
            }
        }
    }
});
```

Debounce 的資料流時間線：

```
t=0ms     t=5ms     t=10ms              t=500ms         t=505ms
[event1]  [event2]  [event3]  ...       [drain all]     [reload_config()]
  |          |         |                      |                |
  +--recv()--+         +---try_recv()---drain-+                |
  |                                                            |
  +----------- sleep(500ms) ---------------------------------→|
                                                   tx.send(new_config)
```

這個 debounce 模式確保：
- 編輯器的多步保存操作只觸發一次 reload
- `try_recv().is_ok()` 循環 drain 保證不會累積 pending 事件
- 500ms 足以覆蓋大多數編輯器的 atomic write 流程

### 4.4 設定解析（多格式支援）

```rust
// crates/opencrust-config/src/watcher.rs:85-96
fn reload_config(path: &Path) -> Result<AppConfig, String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "yml" | "yaml" => {
            serde_yaml::from_str(&contents).map_err(|e| format!("YAML parse error: {e}"))
        }
        "toml" => toml::from_str(&contents).map_err(|e| format!("TOML parse error: {e}")),
        other => Err(format!("unsupported config extension: {other}")),
    }
}
```

### 4.5 watch::channel 語義

`tokio::sync::watch` 是一個 **single-producer, multi-consumer** 廣播頻道，只保留最新值：

- **Producer**（ConfigWatcher spawn）：`tx.send(new_config)` 覆蓋之前的值
- **Consumer**（AppState.config_rx）：`rx.changed().await` 等待新值，`rx.borrow()` 讀取最新值
- **語義**：如果 consumer 讀取慢，中間的值會被跳過（只看最新）

這完美符合 config 熱重載的場景：你只需要最新的 config，不需要每個中間版本。

### 4.6 Config Applier（消費端）

**檔案**: `crates/opencrust-gateway/src/state.rs` (行 448-473)

```rust
pub fn spawn_config_applier(self: &Arc<Self>) {
    let Some(mut rx) = self.config_rx.clone() else {
        return;  // 沒有 watcher 就直接返回
    };
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let new_config = rx.borrow().clone();
            if let Some(prompt) = &new_config.agent.system_prompt {
                info!("config reloaded: system_prompt updated (len={})", prompt.len());
            }
            if let Some(max_tokens) = new_config.agent.max_tokens {
                info!("config reloaded: max_tokens={max_tokens}");
            }
            if let Some(level) = &new_config.log_level {
                info!("config reloaded: log_level={level}");
            }
        }
        warn!("config watcher channel closed");
    });
}
```

目前的 config applier 只是記錄日誌。實際的 config 讀取通過 `current_config()` 方法：

```rust
// crates/opencrust-gateway/src/state.rs:100-106
pub fn current_config(&self) -> AppConfig {
    if let Some(rx) = &self.config_rx {
        rx.borrow().clone()  // 從 watch channel 讀取最新 config
    } else {
        self.config.clone()   // fallback 到初始 config
    }
}
```

### 4.7 DNA 熱重載（同模式的第二實例）

**檔案**: `crates/opencrust-gateway/src/server.rs` (行 319-387)

```rust
fn spawn_dna_watcher(state: Arc<AppState>, config_dir: PathBuf) {
    let dna_filename = std::ffi::OsStr::new("dna.md");
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(8);

    let watcher_result = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            // 比 ConfigWatcher 多監聽 Remove 事件
            let dominated = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            );
            if dominated {
                let touches_dna = event.paths.iter()
                    .any(|p| p.file_name().map(|f| f == dna_filename).unwrap_or(false));
                if touches_dna {
                    let _ = notify_tx.try_send(());
                }
            }
        }
    });

    // ... watcher setup ...

    tokio::spawn(async move {
        let _watcher = watcher;  // 移入 spawn 防止 drop
        loop {
            if notify_rx.recv().await.is_none() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            while notify_rx.try_recv().is_ok() {}

            match std::fs::read_to_string(&dna_path) {
                Ok(content) if !content.trim().is_empty() => {
                    state.agents.set_dna_content(Some(content));
                    info!("dna.md reloaded");
                }
                Ok(_) => {
                    state.agents.set_dna_content(None);
                    info!("dna.md is empty, cleared DNA content");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    state.agents.set_dna_content(None);
                    info!("dna.md removed, cleared DNA content");
                }
                Err(e) => warn!("failed to read dna.md: {e}"),
            }
        }
    });
}
```

DNA watcher 與 ConfigWatcher 的差異：
- 多監聽 `EventKind::Remove(_)` — DNA 刪除時要清空內容
- 直接操作 `state.agents.set_dna_content()` 而非 watch channel
- watcher 移入 spawn 而非 Box::leak（更乾淨但有 spawn 生命週期限制）

### 4.8 錯誤處理策略

```
config reload 失敗
    → warn! 記錄錯誤
    → 保持先前有效的 config
    → 不中斷服務

DNA 讀取失敗
    → 檔案不存在 → 清空 DNA（正常流程）
    → 讀取錯誤 → warn! 記錄，保持先前 DNA
    → 內容為空 → 清空 DNA

notify watcher 建立失敗
    → warn! 記錄
    → 直接 return，不啟動監聽
    → 系統繼續運作（只是沒有熱重載）
```

### 4.9 效能分析

| 操作 | 成本 |
|------|------|
| notify 事件監聽 | 零 CPU（OS kernel callback） |
| Debounce sleep | 500ms 延遲（可調整） |
| Config 解析 | ~1ms（小型 YAML/TOML） |
| watch::send() | O(1)（覆蓋內部值） |
| watch::borrow() | O(1)（RwLock read，通常無競爭） |
| AppConfig::clone() | O(n)（n = config 大小，含 HashMap clone） |

**瓶頸分析**：每次 `current_config()` 呼叫都會 `.clone()` 整個 AppConfig。對於高頻呼叫場景，可以用 `Arc<AppConfig>` 替代，讓 clone 變成 Arc::clone（O(1)）。

### 4.10 Clawtex 實作建議

clawtex-core 目前修改 `agents.toml` 後需要重啟 daemon。以下是移植 OpenCrust 熱重載模式的具體 Rust 實作：

```rust
// src/config_watcher.rs（建議新增）
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEBOUNCE_MS: u64 = 500;

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    pub fn start(
        config_path: PathBuf,
        initial_config: crate::config::AgentsConfig,
    ) -> anyhow::Result<(Self, watch::Receiver<crate::config::AgentsConfig>)> {
        let (tx, rx) = watch::channel(initial_config);
        let watch_dir = config_path.parent()
            .unwrap_or(Path::new(".")).to_path_buf();
        let target_filename = config_path.file_name()
            .unwrap_or_default().to_os_string();

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(8);

        let mut watcher = notify::recommended_watcher(
            move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                        let touches = event.paths.iter()
                            .any(|p| p.file_name().map(|f| f == target_filename).unwrap_or(false));
                        if touches {
                            let _ = notify_tx.try_send(());
                        }
                    }
                }
            }
        )?;

        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

        tokio::spawn(async move {
            loop {
                if notify_rx.recv().await.is_none() { break; }
                tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
                while notify_rx.try_recv().is_ok() {}

                match std::fs::read_to_string(&config_path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| toml::from_str(&s).map_err(|e| e.to_string()))
                {
                    Ok(new_config) => {
                        tracing::info!("agents.toml reloaded");
                        let _ = tx.send(new_config);
                    }
                    Err(e) => {
                        tracing::warn!("config reload failed: {e}");
                    }
                }
            }
        });

        Ok((Self { _watcher: watcher }, rx))
    }
}
```

適用場景：
- `agents.toml` 的 system_prompt、routing、rate_limit 等可熱更新
- Provider API key rotation 可即時生效
- Hand 設定（hands/*.toml）也可以套用相同模式

---

## 5. AES-256-GCM 金鑰保險庫 + OS Keychain（深度分析）

**檔案**: `crates/opencrust-security/src/credentials.rs` (597 行)

### 5.1 架構總覽

```
                  ┌─────────────────────┐
                  │   CredentialVault    │
                  │                     │
                  │ path: PathBuf       │
                  │ derived_key: Vec<u8>│ ← PBKDF2(passphrase, salt, 600K)
                  │ salt: Vec<u8>       │
                  │ entries: HashMap    │ ← 解密後的 key-value store
                  └─────────┬───────────┘
                            │
                   save() / open()
                            │
                  ┌─────────▼───────────┐
                  │     VaultFile       │
                  │ (on-disk JSON)      │
                  │                     │
                  │ salt: base64        │
                  │ nonce: base64       │ ← 每次 save() 重新生成
                  │ ciphertext: base64  │ ← AES-256-GCM(entries)
                  └─────────────────────┘
```

### 5.2 密碼學細節

**金鑰衍生（PBKDF2）**：

```rust
// crates/opencrust-security/src/credentials.rs:485-496
const PBKDF2_ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 32;
const KEY_LEN: usize = 32;   // 256 bits

fn derive_key(passphrase: &str, salt: &[u8]) -> Vec<u8> {
    let iterations = NonZeroU32::new(PBKDF2_ITERATIONS).expect("iterations > 0");
    let mut key = vec![0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        passphrase.as_bytes(),
        &mut key,
    );
    key
}
```

600,000 次 PBKDF2 迭代是 OWASP 2024 建議值的上界。在現代硬體上（AMD Ryzen AI MAX+ 395），這需要 ~300ms，足以抵抗離線暴力破解但不會造成使用者感知的延遲。

**加密（AES-256-GCM）**：

```rust
// crates/opencrust-security/src/credentials.rs:158-201
pub fn save(&self) -> Result<(), CredentialError> {
    let plaintext = serde_json::to_vec(&self.entries)?;

    let rng = SystemRandom::new();
    let mut nonce_bytes = vec![0u8; NONCE_LEN];  // 12 bytes
    rng.fill(&mut nonce_bytes)?;

    let key = make_aead_key(&self.derived_key)?;
    let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)?;

    let mut in_out = plaintext;
    // seal_in_place_append_tag: 加密 + 追加 16 byte GCM tag
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)?;

    let vault_file = VaultFile {
        salt: BASE64.encode(&self.salt),
        nonce: BASE64.encode(&nonce_bytes),
        ciphertext: BASE64.encode(&in_out),
    };

    // 建立父目錄
    if let Some(parent) = self.path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(&vault_file)?;
    std::fs::write(&self.path, json)?;
    Ok(())
}
```

**關鍵安全特性**：
- 每次 `save()` 都生成新的 12-byte nonce → 即使內容不變，密文也不同
- GCM 模式提供 **認證加密** — 任何篡改都會被偵測
- `Aad::empty()` — 沒有額外驗證數據（可以改進，見建議）
- `ring::LessSafeKey` — ring 的命名暗示你需要自行管理 nonce 唯一性

**解密流程**：

```rust
// crates/opencrust-security/src/credentials.rs:92-136
pub fn open(path: &Path, passphrase: &str) -> Result<Self, CredentialError> {
    let contents = std::fs::read_to_string(path)?;
    let vault_file: VaultFile = serde_json::from_str(&contents)?;

    let salt = BASE64.decode(&vault_file.salt)?;
    let nonce_bytes = BASE64.decode(&vault_file.nonce)?;
    let mut ciphertext = BASE64.decode(&vault_file.ciphertext)?;

    let derived_key = derive_key(passphrase, &salt);
    let key = make_aead_key(&derived_key)?;
    let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)?;

    // 就地解密（修改 ciphertext buffer）
    let plaintext = key.open_in_place(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| CredentialError::WrongPassphrase)?;

    let entries: HashMap<String, String> = serde_json::from_slice(plaintext)?;
    Ok(Self { path: path.to_path_buf(), derived_key, salt, entries })
}
```

### 5.3 三級 Passphrase 解析

```rust
// crates/opencrust-security/src/credentials.rs:308-338
fn resolve_vault_passphrase(vault_path: &Path, allow_create: bool) -> Option<String> {
    // 既有 vault（allow_create = false）：
    if !allow_create {
        // 1. OS Keychain 優先（已驗證過的 passphrase）
        if let Some(keyring_passphrase) = read_keyring_passphrase(vault_path) {
            return Some(keyring_passphrase);
        }
        // 2. 環境變數
        return vault_env_passphrase();
    }

    // 新建 vault（allow_create = true）：
    // 1. 環境變數（用戶明確指定的）
    if let Some(env_passphrase) = vault_env_passphrase() {
        // 同時鏡像到 OS keychain
        if !write_keyring_passphrase(vault_path, &env_passphrase) {
            warn!("could not mirror vault passphrase to OS keychain; continuing with env var");
        }
        return Some(env_passphrase);
    }

    // 2. OS Keychain（可能已有）
    if let Some(keyring_passphrase) = read_keyring_passphrase(vault_path) {
        return Some(keyring_passphrase);
    }

    // 3. 自動生成 32 byte random passphrase → 存入 OS keychain
    let generated = generate_passphrase()?;
    if write_keyring_passphrase(vault_path, &generated) {
        info!("generated vault passphrase and stored it in OS keychain");
        Some(generated)
    } else {
        warn!("failed to store generated vault passphrase in OS keychain");
        None
    }
}
```

Passphrase 解析的資料流：

```
既有 vault 打開：
    OS Keychain → 環境變數 → 失敗（返回 None）

新 vault 建立：
    環境變數 → (鏡像到 Keychain) → OS Keychain → 自動生成 → (存入 Keychain)
```

### 5.4 OS Keychain 整合

```rust
// crates/opencrust-security/src/credentials.rs:357-404
const KEYRING_SERVICE: &str = "opencrust";
const KEYRING_ACCOUNT_PREFIX: &str = "vault-passphrase";

fn keyring_account_for_path(vault_path: &Path) -> String {
    // 用 SHA-256 hash 路徑作為 account name
    let path_bytes = vault_path.to_string_lossy();
    let hash = digest(&SHA256, path_bytes.as_bytes());
    let mut hex = String::with_capacity(hash.as_ref().len() * 2);
    for byte in hash.as_ref() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("{KEYRING_ACCOUNT_PREFIX}:{hex}")
}

fn read_keyring_passphrase(vault_path: &Path) -> Option<String> {
    let entry = keyring_entry_for_path(vault_path)?;
    match entry.get_password() {
        Ok(value) if value.trim().is_empty() => None,
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            warn!("could not read vault passphrase from OS keychain: {err}");
            None
        }
    }
}
```

`keyring` crate 的跨平台映射：
- **Windows**: Credential Manager（`wincred`）
- **macOS**: Keychain（`security`）
- **Linux**: Secret Service D-Bus API（`libsecret`）

### 5.5 讀取快取（VaultGetCache）

```rust
// crates/opencrust-security/src/credentials.rs:22-39
#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultFileFingerprint {
    len: u64,
    modified_unix_nanos: u128,
}

#[derive(Debug, Clone)]
struct VaultGetCacheEntry {
    fingerprint: VaultFileFingerprint,
    passphrase_hash: [u8; 32],
    entries: HashMap<String, String>,
}

static VAULT_GET_CACHE: OnceLock<RwLock<HashMap<PathBuf, VaultGetCacheEntry>>> = OnceLock::new();
```

快取邏輯：

```rust
// crates/opencrust-security/src/credentials.rs:206-239
pub fn try_vault_get(vault_path: &Path, key: &str) -> Option<String> {
    if !CredentialVault::exists(vault_path) { return None; }

    let passphrase = resolve_vault_passphrase(vault_path, false)?;
    let passphrase_hash = hash_passphrase(&passphrase);
    let fingerprint_before_open = vault_file_fingerprint(vault_path);

    // 快取命中檢查：fingerprint + passphrase hash 都必須匹配
    if let Some(fingerprint) = fingerprint_before_open.as_ref()
        && let Some(value) = cached_vault_value(vault_path, key, fingerprint, &passphrase_hash)
    {
        return value;  // 從快取返回（Option<String>）
    }

    // 快取未命中：完整解密
    match CredentialVault::open(vault_path, &passphrase) {
        Ok(vault) => {
            let value = vault.get(key).map(|s| s.to_string());
            // 更新快取
            if let Some(fingerprint) = vault_file_fingerprint(vault_path)
                .or(fingerprint_before_open) {
                cache_vault_entries(vault_path, fingerprint, passphrase_hash, vault.entries.clone());
            }
            value
        }
        Err(e) => { warn!("could not open credential vault: {e}"); None }
    }
}
```

快取失效策略：

```
fingerprint = (file_len, modified_unix_nanos)

快取命中條件：
  1. vault_path 存在於快取中
  2. 檔案 fingerprint 匹配（長度 + 修改時間）
  3. passphrase SHA-256 hash 匹配

任何條件不滿足 → 重新解密整個 vault → 更新快取
```

**巧妙之處**：使用 `OnceLock<RwLock<HashMap>>` 而非 `Mutex`，允許多個讀取者同時存取快取，只在寫入時獨佔。且 `OnceLock` 確保 HashMap 只初始化一次。

### 5.6 Vault Set 操作

```rust
// crates/opencrust-security/src/credentials.rs:247-297
pub fn try_vault_set(vault_path: &Path, key: &str, value: &str) -> bool {
    let vault_exists = CredentialVault::exists(vault_path);
    let passphrase = match resolve_vault_passphrase(vault_path, !vault_exists) {
        Some(p) => p,
        None => { warn!("try_vault_set: no vault passphrase available"); return false; }
    };

    let mut vault = if vault_exists {
        CredentialVault::open(vault_path, &passphrase)?
    } else {
        CredentialVault::create(vault_path, &passphrase)?
    };

    vault.set(key, value);
    match vault.save() {
        Ok(()) => {
            // 更新快取（或在失敗時 invalidate）
            if let Some(fingerprint) = vault_file_fingerprint(vault_path) {
                cache_vault_entries(vault_path, fingerprint, passphrase_hash, vault.entries.clone());
            } else {
                invalidate_vault_cache(vault_path);
            }
            true
        }
        Err(e) => { warn!("try_vault_set: failed to save vault: {e}"); false }
    }
}
```

### 5.7 錯誤類型設計

```rust
// crates/opencrust-security/src/credentials.rs:504-516
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("vault already exists: {0}")]
    AlreadyExists(String),
    #[error("wrong passphrase or corrupted vault")]
    WrongPassphrase,                    // GCM 解密失敗時
    #[error("cryptographic error: {0}")]
    Crypto(String),
    #[error("vault format error: {0}")]
    Format(String),                     // JSON 解析失敗
    #[error("I/O error: {0}")]
    Io(String),
}
```

注意 `WrongPassphrase` 不洩漏具體錯誤細節（不區分「密碼錯」和「數據損壞」），這是安全最佳實踐。

### 5.8 RwLock Poison 恢復

```rust
// crates/opencrust-security/src/credentials.rs:434-446
fn cached_vault_value(...) -> Option<Option<String>> {
    let cache = vault_get_cache();
    let guard = match cache.read() {
        Ok(g) => g,
        Err(poisoned) => {
            warn!("vault read cache lock poisoned; recovering");
            poisoned.into_inner()  // 恢復 poisoned lock
        }
    };
    // ...
}
```

所有 RwLock 存取都處理 poison 情況，這在多執行緒 panic 場景下很重要。

### 5.9 與 clawtex-core 加密系統的比較

| 特性 | OpenCrust | clawtex-core |
|------|-----------|--------------|
| 加密演算法 | AES-256-GCM (ring) | ChaCha20-Poly1305 |
| 金鑰衍生 | PBKDF2 600K iterations | 直接使用 key |
| Passphrase 來源 | OS Keychain > 環境變數 > 自動生成 | 環境變數 / 設定檔 |
| 讀取快取 | OnceLock<RwLock<HashMap>> + fingerprint | 無 |
| 密文格式 | JSON (salt+nonce+ciphertext base64) | `enc2:` prefix + binary |
| 外部修改偵測 | 檔案 fingerprint (len + modified_nanos) | 無 |

### 5.10 Clawtex 實作建議

clawtex-core 的 ChaCha20-Poly1305 加密本身很好，但缺少 OS Keychain 整合和讀取快取。建議：

```rust
// src/security/keychain.rs（建議新增）
use keyring::Entry;

const KEYRING_SERVICE: &str = "clawtex";

pub fn get_secret_key() -> Option<String> {
    // 1. 環境變數
    if let Ok(key) = std::env::var("CLAWTEX_SECRET_KEY") {
        return Some(key);
    }
    // 2. OS Keychain
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, "master-key") {
        if let Ok(password) = entry.get_password() {
            return Some(password);
        }
    }
    // 3. 自動生成
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new().fill(&mut bytes).ok()?;
    let key = base64::encode(bytes);
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, "master-key") {
        entry.set_password(&key).ok();
    }
    Some(key)
}
```

也建議加入讀取快取，避免每次存取 API key 都要解密：

```rust
use std::sync::OnceLock;
use std::sync::RwLock;
use std::collections::HashMap;

static SECRET_CACHE: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

pub fn cached_decrypt(key: &str) -> Option<String> {
    let cache = SECRET_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(guard) = cache.read() {
        if let Some(value) = guard.get(key) {
            return Some(value.clone());
        }
    }
    // 快取未命中，解密並存入快取
    let decrypted = decrypt_secret(key)?;
    if let Ok(mut guard) = cache.write() {
        guard.insert(key.to_string(), decrypted.clone());
    }
    Some(decrypted)
}
```

---

## 6. Channel Lifecycle / Sender 分離（深度分析）

**檔案**: `crates/opencrust-channels/src/traits.rs` (60 行)

### 6.1 完整 Trait 定義

```rust
// crates/opencrust-channels/src/traits.rs:1-60

/// 生命週期管理（connect, disconnect, status）
#[async_trait]
pub trait ChannelLifecycle: Send {
    /// 人類可讀的顯示名稱
    fn display_name(&self) -> &str;

    /// 連接到外部服務
    async fn connect(&mut self) -> Result<()>;

    /// 優雅斷開
    async fn disconnect(&mut self) -> Result<()>;

    /// 當前連接狀態
    fn status(&self) -> ChannelStatus;

    /// 建立輕量的 send-only handle
    /// 回傳的 sender 獨立於生命週期，可以用 Arc 包裝後跨 task 共用
    fn create_sender(&self) -> Box<dyn ChannelSender>;
}

/// Send-only 介面，用於通過頻道發送訊息
/// 設計為可以用 Arc 包裝後跨 task 共用（例如排程器）
#[async_trait]
pub trait ChannelSender: Send + Sync {
    /// 頻道類型唯一識別符
    fn channel_type(&self) -> &str;

    /// 通過此頻道發送訊息
    async fn send_message(&self, message: &Message) -> Result<()>;
}

/// 便利 trait：同時具備 lifecycle + sender
pub trait Channel: ChannelLifecycle + ChannelSender {}
impl<T: ChannelLifecycle + ChannelSender> Channel for T {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum ChannelEvent {
    MessageReceived(Message),
    StatusChanged(ChannelStatus),
    Error(String),
}
```

### 6.2 設計動機與所有權分析

為什麼需要分離？看看沒有分離時的問題：

```rust
// 假設沒有分離的設計
pub trait Channel: Send + Sync {
    async fn connect(&mut self) -> Result<()>;     // 需要 &mut self
    async fn send_message(&self, msg: &Message) -> Result<()>;  // 需要 &self
}

// 問題：connect() 需要 &mut self，send_message() 需要 &self
// 如果 channel 被 Arc 包裝，就無法呼叫 connect()
// 如果 channel 不被 Arc 包裝，就無法在多個 task 中 send_message()
```

OpenCrust 的解決方案：

```
                  ┌──────────────────────────┐
                  │    TelegramChannel       │
                  │                          │
                  │ + connect(&mut self)     │ ← 獨佔所有權
                  │ + disconnect(&mut self)  │
                  │ + create_sender(&self)   │ ← 製造輕量 handle
                  └──────────┬───────────────┘
                             │
                   create_sender()
                             │
                  ┌──────────▼───────────────┐
                  │    TelegramSender        │
                  │                          │
                  │ bot_token: String        │ ← 只有發送需要的最小狀態
                  │ + send_message(&self)    │ ← 可以 Arc<dyn ChannelSender>
                  └──────────────────────────┘
```

### 6.3 Server 端的使用模式

**檔案**: `crates/opencrust-gateway/src/server.rs` (行 139-152)

```rust
// 每個頻道的啟動模式完全一致
let telegram_channels = build_telegram_channels(&state.config, &state);
for mut channel in telegram_channels {
    // Step 1: 在 move 之前抽出 sender
    let sender: Arc<dyn ChannelSender> = Arc::from(channel.create_sender());

    // Step 2: 將 sender 存入共用狀態（Arc<DashMap>）
    state.channel_senders.insert(sender.channel_type().to_string(), sender);

    // Step 3: channel 本身 move into spawn（獨佔所有權）
    tokio::spawn(async move {
        if let Err(e) = channel.connect().await {
            warn!("telegram channel failed to connect: {e}");
            return;
        }
        shutdown_signal().await;
        channel.disconnect().await.ok();
    });
}
```

這個模式的所有權分析：

```
channel (owned by spawn task)
    │
    ├── connect(&mut self)     ← spawn task 獨佔
    ├── disconnect(&mut self)  ← spawn task 獨佔
    │
    └── sender (Arc<dyn ChannelSender>, shared)
        │
        ├── state.channel_senders  ← 全局可存取
        ├── scheduler task         ← 排程發送
        ├── WebSocket handler      ← 即時回覆
        └── A2A handler            ← 跨 agent 通訊
```

### 6.4 Sender 在排程系統中的使用

**檔案**: `crates/opencrust-gateway/src/server.rs` (行 526-535)

```rust
// execute_scheduled_task() 中
// 4. Best-effort delivery to channel adapter via sender handle.
if let Some(sender) = state.channel_senders.get(delivery_channel) {
    if let Err(e) = sender.send_message(&response_msg).await {
        tracing::error!("Failed to send scheduled response: {e}");
    }
} else {
    tracing::warn!(
        "Scheduled response persisted but no channel sender registered for: {}",
        delivery_channel
    );
}
```

注意 `state.channel_senders` 是 `DashMap<String, Arc<dyn ChannelSender>>`，`get()` 回傳 `Ref<'_, String, Arc<dyn ChannelSender>>`，整個操作都不需要任何 mutex。

### 6.5 ChannelStatus 狀態機

```
    Disconnected ──connect()──→ Connecting ──success──→ Connected
         ↑                                                 │
         │                                          error/timeout
         │                                                 │
    disconnect()                                  Reconnecting
         ↑                                                 │
         │                                           retry success
         └─────── Error(reason) ←── max retries ──── │
                                                      │
                                                  retry success ──→ Connected
```

### 6.6 Slack Channel 具體實作

**檔案**: `crates/opencrust-channels/src/slack/mod.rs`

```rust
pub struct SlackChannel {
    bot_token: String,
    app_token: String,
    on_message: SlackOnMessageFn,
    group_filter: SlackGroupFilter,
    bot_user_id: Option<String>,
    shutdown_tx: Option<watch::Sender<bool>>,  // 優雅關閉
}

// Sender 只需要 bot_token
pub struct SlackSender {
    bot_token: String,
}

impl ChannelLifecycle for SlackChannel {
    fn create_sender(&self) -> Box<dyn ChannelSender> {
        Box::new(SlackSender {
            bot_token: self.bot_token.clone(),
        })
    }
}

impl ChannelSender for SlackSender {
    fn channel_type(&self) -> &str { "slack" }
    async fn send_message(&self, message: &Message) -> Result<()> {
        // 使用 bot_token 呼叫 Slack API
        api::post_message(&self.bot_token, ...).await
    }
}
```

### 6.7 與 clawtex-core 的差距

clawtex-core 目前只有 Telegram 一個頻道，且 Telegram bot 是直接在 `src/telegram.rs` 中以 teloxide 的 `Bot` 物件操作。主要問題：

1. **沒有 trait 抽象** — Telegram 邏輯直接硬編碼，未來加 Discord/Slack 需要大改
2. **沒有 Sender 分離** — Hands engine 和 cron scheduler 發送訊息需要存取完整的 Bot 物件
3. **沒有 ChannelStatus** — 無法查詢頻道連線狀態

### 6.8 Clawtex 實作建議

```rust
// src/channels/mod.rs（建議新增）
use async_trait::async_trait;
use crate::message::OutboundMessage;

#[async_trait]
pub trait ChannelLifecycle: Send {
    fn display_name(&self) -> &str;
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
    fn status(&self) -> ChannelStatus;
    fn create_sender(&self) -> Box<dyn ChannelSender>;
}

#[async_trait]
pub trait ChannelSender: Send + Sync {
    fn channel_type(&self) -> &str;
    async fn send_text(&self, chat_id: &str, text: &str) -> anyhow::Result<()>;
    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub enum ChannelStatus {
    Disconnected,
    Connected,
    Error(String),
}

// src/channels/telegram.rs
pub struct TelegramChannel {
    bot: teloxide::Bot,
    // ... 完整的 Telegram 設定
}

pub struct TelegramSender {
    bot: teloxide::Bot,  // Bot 本身是 Clone + Send + Sync
}

impl ChannelLifecycle for TelegramChannel {
    fn create_sender(&self) -> Box<dyn ChannelSender> {
        Box::new(TelegramSender { bot: self.bot.clone() })
    }
}

impl ChannelSender for TelegramSender {
    fn channel_type(&self) -> &str { "telegram" }
    async fn send_text(&self, chat_id: &str, text: &str) -> anyhow::Result<()> {
        let chat = teloxide::types::ChatId(chat_id.parse()?);
        self.bot.send_message(chat, text).await?;
        Ok(())
    }
}
```

這個改造讓 Hands engine 和 cron scheduler 可以持有 `Arc<dyn ChannelSender>` 發送訊息，而不需要存取完整的 Bot 物件。

---

## 7. Arc-safe 共用狀態 AppState（深度分析）

**檔案**: `crates/opencrust-gateway/src/state.rs` (583 行)

### 7.1 完整結構定義

```rust
// crates/opencrust-gateway/src/state.rs:20-46
const SESSION_TTL: Duration = Duration::from_secs(3600);  // 1 小時
const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);  // 5 分鐘

pub struct AppState {
    // 不可變欄位（初始化後不變）
    pub config: AppConfig,
    pub channels: ChannelRegistry,
    pub agents: AgentRuntime,

    // 並發安全的可變欄位
    pub sessions: DashMap<String, SessionState>,
    pub channel_senders: DashMap<String, Arc<dyn ChannelSender>>,
    pub a2a_tasks: DashMap<String, A2ATask>,
    pub mcp_manager_arc: Option<Arc<McpManager>>,
    pub session_store: Option<Arc<Mutex<SessionStore>>>,
    session_summaries: DashMap<String, String>,

    // 原子操作欄位
    google_workspace_integration_connected: AtomicBool,
    google_workspace_email: RwLock<Option<String>>,
    google_oauth_states: DashMap<String, Instant>,
    google_oauth_runtime_config: RwLock<Option<GoogleOAuthRuntimeConfig>>,

    // Config 熱重載接收器
    config_rx: Option<watch::Receiver<AppConfig>>,
}

pub type SharedState = Arc<AppState>;
```

### 7.2 並發安全策略分析

AppState 採用了 **多策略並發安全** 設計，每個欄位根據存取模式選擇最合適的同步原語：

```
┌────────────────────────┬──────────────────────┬──────────────────────────────┐
│ 欄位                    │ 同步原語              │ 選擇理由                      │
├────────────────────────┼──────────────────────┼──────────────────────────────┤
│ config                 │ 不可變（初始化後）      │ 只讀，不需要同步              │
│ channels               │ 不可變                │ 只讀                         │
│ agents                 │ 內部 RwLock           │ providers 可動態添加         │
│ sessions               │ DashMap              │ 高頻讀寫，分片鎖              │
│ channel_senders        │ DashMap              │ 啟動時寫入，運行時高頻讀取     │
│ session_summaries      │ DashMap              │ per-session 讀寫              │
│ session_store          │ Arc<Mutex>           │ SQLite 本身是單寫多讀         │
│ google_*_connected     │ AtomicBool           │ 單一 boolean，最低成本        │
│ google_*_email         │ RwLock               │ 偶爾寫入，讀多寫少            │
│ google_oauth_states    │ DashMap              │ 短暫 token，需要快速 insert   │
│ config_rx              │ watch::Receiver      │ 廣播模式，只保留最新值         │
└────────────────────────┴──────────────────────┴──────────────────────────────┘
```

### 7.3 DashMap vs RwLock<HashMap> 的選擇

DashMap 內部將 HashMap 分為 N 個分片（shard），每個分片有獨立的 RwLock。這意味著：

- **不同 session 的操作完全不互鎖** — session A 和 session B 可能在不同分片
- **讀操作幾乎零成本** — 只需要 acquire 該分片的 read lock
- **迭代需要依序鎖定所有分片** — `retain()` 等操作有短暫的 per-shard lock

```rust
// DashMap 使用示例：session 管理
// crates/opencrust-gateway/src/state.rs:410-431
pub fn cleanup_expired_sessions(&self) -> usize {
    let now = Instant::now();
    let mut removed = 0;

    // retain() 逐分片鎖定，刪除過期 session
    self.sessions.retain(|_id, session| {
        if !session.connected && now.duration_since(session.last_active) > SESSION_TTL {
            removed += 1;
            false
        } else {
            true
        }
    });

    // 同步清理 summaries
    self.session_summaries
        .retain(|session_id, _| self.sessions.contains_key(session_id));

    if removed > 0 {
        info!("cleaned up {removed} expired sessions");
    }
    removed
}
```

### 7.4 Session 生命週期

```rust
// crates/opencrust-gateway/src/state.rs:55-67
pub struct SessionState {
    pub id: String,
    pub user_id: Option<String>,
    pub channel_id: Option<String>,
    pub history: Vec<ChatMessage>,
    pub connected: bool,
    pub created_at: Instant,
    pub last_active: Instant,
}
```

生命週期狀態機：

```
create_session()
    │
    ▼
Connected (connected=true, last_active=now)
    │
    ├── text message → update last_active
    ├── pong → update last_active
    │
    ▼
disconnect_session()
    │
    ▼
Disconnected (connected=false, last_active=now)
    │
    ├── resume_session() → Connected
    │
    ▼ (after SESSION_TTL = 1 hour)
cleanup_expired_sessions()
    │
    ▼
Removed (sessions.retain())
```

### 7.5 Session 水合（Hydrate）

**檔案**: `crates/opencrust-gateway/src/state.rs` (行 232-308)

```rust
pub async fn hydrate_session_history(
    &self,
    session_id: &str,
    channel_id: Option<&str>,
    user_id: Option<&str>,
) {
    // 1. 確保 session 存在
    if !self.sessions.contains_key(session_id) {
        self.create_session_with_id(session_id.to_string());
    }

    // 2. 更新 session metadata
    if let Some(mut session) = self.sessions.get_mut(session_id) {
        if let Some(channel) = channel_id { session.channel_id = Some(channel.to_string()); }
        if let Some(user) = user_id { session.user_id = Some(user.to_string()); }
        session.connected = true;
        session.last_active = Instant::now();
    }

    // 3. 從 SQLite 載入歷史（如果記憶體中沒有）
    let should_load = self.sessions.get(session_id)
        .map(|s| s.history.is_empty()).unwrap_or(false);

    if should_load {
        let guard = store.lock().await;
        match guard.load_recent_messages(session_id, 100) {
            Ok(messages) => {
                // 轉換為 ChatMessage
                loaded_history = messages.into_iter()
                    .filter_map(|m| match m.direction.as_str() {
                        "user" => Some(ChatMessage { role: User, ... }),
                        "assistant" => Some(ChatMessage { role: Assistant, ... }),
                        _ => None,
                    })
                    .collect();
            }
            Err(e) => warn!("failed to load session history: {e}"),
        }
    }

    // 4. 寫回記憶體（DashMap write lock 時間極短）
    if should_load && !loaded_history.is_empty() {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.history = loaded_history;
        }
    }
}
```

### 7.6 Continuity Key（跨頻道記憶共享）

```rust
// crates/opencrust-gateway/src/state.rs:197-204
pub fn continuity_key(&self, _user_id: Option<&str>) -> Option<String> {
    if self.config.memory.shared_continuity {
        Some("bus:shared-global".to_string())
    } else {
        None
    }
}
```

當 `shared_continuity = true` 時，所有頻道共用同一個記憶空間。這意味著你在 Telegram 問的問題，agent 在 Discord 中也能記得。

### 7.7 Google OAuth 狀態管理

```rust
// crates/opencrust-gateway/src/state.rs:141-168
pub fn issue_google_oauth_state(&self) -> String {
    let state = Uuid::new_v4().to_string();
    self.google_oauth_states.insert(state.clone(), Instant::now());
    state
}

pub fn consume_google_oauth_state(&self, state: &str, max_age: Duration) -> bool {
    self.google_oauth_states
        .remove(state)
        .map(|(_, created_at)| created_at.elapsed() <= max_age)
        .unwrap_or(false)
}
```

這是一個 **anti-CSRF** 模式：
1. 發起 OAuth → 生成 UUID state token → 存入 DashMap
2. OAuth callback → 用 `consume` 取出並刪除 token
3. 驗證 token 未過期 → 完成 OAuth flow

DashMap 的 `remove()` 是原子的，確保每個 state token 只能被使用一次。

### 7.8 Background Task 模式

```rust
// crates/opencrust-gateway/src/state.rs:434-443
pub fn spawn_session_cleanup(self: &Arc<Self>) {
    let state = Arc::clone(self);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            state.cleanup_expired_sessions();
        }
    });
}
```

注意 `self: &Arc<Self>` — 這是 Rust 的 **arbitrary self types** 語法，讓方法可以在 `Arc<AppState>` 上呼叫。`Arc::clone` 增加引用計數，spawn 的 task 持有獨立的 Arc 指標。

### 7.9 測試策略

```rust
// crates/opencrust-gateway/src/state.rs:478-583
#[cfg(test)]
mod tests {
    fn test_state() -> AppState {
        AppState::new(
            AppConfig::default(),
            AgentRuntime::new(),
            ChannelRegistry::new(),
        )
    }

    #[test]
    fn create_session_returns_unique_ids() { ... }
    #[test]
    fn disconnect_and_resume_session_round_trip() { ... }
    #[test]
    fn resume_nonexistent_session_returns_false() { ... }
    #[test]
    fn cleanup_expired_sessions_removes_only_disconnected_expired() {
        let state = test_state();
        let active_id = state.create_session();
        let expired_id = state.create_session();

        state.disconnect_session(&expired_id);
        // 模擬過期：回溯 last_active 2 小時
        if let Some(mut session) = state.sessions.get_mut(&expired_id) {
            session.last_active = Instant::now() - Duration::from_secs(7200);
        }

        let removed = state.cleanup_expired_sessions();
        assert_eq!(removed, 1);
        assert!(state.sessions.contains_key(&active_id));
        assert!(!state.sessions.contains_key(&expired_id));
    }
    #[test]
    fn cleanup_does_not_remove_connected_sessions() { ... }
    #[test]
    fn continuity_key_with_shared_continuity_enabled() { ... }
    #[test]
    fn continuity_key_with_shared_continuity_disabled() { ... }
}
```

### 7.10 Clawtex 實作建議

clawtex-core 目前的共用狀態分散在多個獨立的 Arc 中。建議統一為 AppState 模式：

```rust
// src/app_state.rs（建議新增）
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

pub struct AppState {
    // 不可變
    pub config: crate::config::AgentsConfig,
    pub agent_runtime: crate::agent_runtime::AgentRuntime,

    // 並發安全
    pub sessions: DashMap<String, SessionState>,
    pub channel_senders: DashMap<String, Arc<dyn crate::channels::ChannelSender>>,
    pub cost_db: Arc<Mutex<rusqlite::Connection>>,
    pub memory_db: Arc<Mutex<rusqlite::Connection>>,

    // 熱重載
    config_rx: Option<watch::Receiver<crate::config::AgentsConfig>>,
}

pub struct SessionState {
    pub id: String,
    pub agent_name: String,
    pub history: Vec<crate::types::Message>,
    pub connected: bool,
    pub last_active: std::time::Instant,
}

impl AppState {
    pub fn current_config(&self) -> crate::config::AgentsConfig {
        if let Some(rx) = &self.config_rx {
            rx.borrow().clone()
        } else {
            self.config.clone()
        }
    }

    pub fn spawn_session_cleanup(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                state.sessions.retain(|_, s| {
                    s.connected || s.last_active.elapsed() < Duration::from_secs(3600)
                });
            }
        });
    }
}

pub type SharedState = Arc<AppState>;
```

---

## 8. AgentRuntime 與工具循環

**檔案**: `crates/opencrust-agents/src/runtime.rs`

### 8.1 結構定義

```rust
pub struct AgentRuntime {
    providers: RwLock<Vec<Arc<dyn LlmProvider>>>,
    default_provider: RwLock<Option<String>>,
    memory: Option<Arc<dyn MemoryProvider>>,
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
    tools: Vec<Box<dyn Tool>>,
    system_prompt: Option<String>,
    dna_content: RwLock<Option<String>>,     // 可熱重載
    max_tokens: Option<u32>,
    max_context_tokens: Option<usize>,
    recall_limit: usize,                     // 預設 10
    summarization_enabled: bool,             // 預設 true
}
```

### 8.2 RwLock 使用策略

為什麼 `providers` 和 `dna_content` 使用 `RwLock` 而不是 `Mutex`？

```rust
// providers 需要在 Arc 後動態添加
pub fn register_provider(&self, provider: Arc<dyn LlmProvider>) {
    let id = provider.provider_id().to_string();
    { self.default_provider.write().unwrap(); }  // 短暫寫鎖
    self.providers.write().unwrap().push(provider);
}

// dna_content 需要在 Arc 後熱重載
pub fn set_dna_content(&self, content: Option<String>) {
    *self.dna_content.write().unwrap() = content;
}

pub fn dna_content(&self) -> Option<String> {
    self.dna_content.read().unwrap().clone();  // 讀鎖，多個讀者可並行
}
```

### 8.3 核心對話循環

```rust
// process_message_impl() 核心循環（簡化版）
async fn process_message_impl(&self, ...) -> Result<String> {
    let provider = self.default_provider()?;

    // 1. 記憶體召回
    let memory_context = self.recall_context(memory_text, session_id, ...).await;

    // 2. 組裝 system prompt
    let system = build_system_prompt(dna, &self.system_prompt, memory_context, None);

    // 3. 組裝對話歷史
    let mut messages = conversation_history.to_vec();
    messages.push(ChatMessage { role: User, content: user_content });

    // 4. 裁切歷史以符合 context window
    trim_messages_to_budget(&mut messages, &system, &tool_defs, max_ctx);

    // 5. 工具呼叫循環（最多 10 次）
    for _iteration in 0..MAX_TOOL_ITERATIONS {
        let response = provider.complete(&request).await?;

        if !has_tool_use(&response) {
            // 沒有工具呼叫 → 儲存記憶 → 返回結果
            self.remember_turn(session_id, ..., &final_text).await;
            return Ok(final_text);
        }

        // 執行工具
        messages.push(ChatMessage { role: Assistant, content: response.content });
        let tool_results = execute_tools(&response.content, &self.tools, heartbeat_depth);
        messages.push(ChatMessage { role: User, content: tool_results });
    }

    Err("tool loop exceeded maximum iterations")
}
```

### 8.4 Streaming 變體

```rust
// process_message_streaming_impl() — 透過 mpsc channel 傳送 delta
async fn process_message_streaming_impl(
    &self,
    delta_tx: mpsc::Sender<String>,
    ...
) -> Result<String> {
    // 同樣的 recall → system prompt → messages 組裝
    // 但使用 stream_complete() 而非 complete()

    let stream = provider.stream_complete(&request).await?;
    let mut accumulated = String::new();

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(text) => {
                accumulated.push_str(&text);
                let _ = delta_tx.send(text).await;
            }
            StreamEvent::ToolUseStart { .. } => { /* 開始工具呼叫 */ }
            // ...
        }
    }

    // 工具呼叫循環同非 streaming 版本
}
```

### 8.5 Rolling Summarization

```rust
// compact_messages() — 75% context window 時觸發
async fn compact_messages(
    messages: &mut Vec<ChatMessage>,
    system: &Option<String>,
    tool_defs: &[ToolDefinition],
    max_ctx: usize,
    provider: &dyn LlmProvider,
    existing_summary: Option<&str>,
    summarization_enabled: bool,
) -> Option<String> {
    let current_tokens = estimate_tokens(messages, system, tool_defs);
    let threshold = (max_ctx as f64 * 0.75) as usize;

    if current_tokens <= threshold || !summarization_enabled {
        return None;
    }

    // 找到可以摘要的訊息範圍（保留最近的 25%）
    // 用 LLM 生成摘要
    // 替換舊訊息為摘要
}
```

---

## 9. WebSocket 閘道安全設計

**檔案**: `crates/opencrust-gateway/src/ws.rs` (458 行)

### 9.1 常數時間 Token 比較

```rust
// crates/opencrust-gateway/src/ws.rs:48-57
let valid = match token {
    Some(t) if t.len() == configured_key.len() => {
        t.bytes()
            .zip(configured_key.bytes())
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
    _ => false,
};
```

這避免了 **計時攻擊**（timing attack）：標準的 `==` 比較會在第一個不匹配的字元處提前返回，攻擊者可以通過測量回應時間逐字推測 API key。

XOR + OR fold 確保：
- 無論哪個位置不匹配，整個迴圈都會執行完
- 長度不匹配時直接返回 false（不洩漏長度資訊以外的資訊）

### 9.2 Rate Limiting

```rust
// Per-WebSocket 滑動視窗 rate limiter
const WS_RATE_LIMIT_MAX: u32 = 30;           // 每分鐘最多 30 條
const WS_RATE_LIMIT_WINDOW: Duration = 60s;

// 在主循環中
let mut msg_timestamps: VecDeque<Instant> = VecDeque::new();

// 每條訊息
msg_timestamps.retain(|ts| now.duration_since(*ts) < WS_RATE_LIMIT_WINDOW);
if msg_timestamps.len() >= WS_RATE_LIMIT_MAX as usize {
    // 回傳 rate_limited error
    continue;
}
msg_timestamps.push_back(now);
```

### 9.3 Heartbeat / Pong 超時

```rust
const HEARTBEAT_INTERVAL: Duration = 30s;   // ping 間隔
const HEARTBEAT_TIMEOUT: Duration = 90s;     // pong 超時

// 主循環中
tokio::select! {
    _ = heartbeat.tick() => {
        if last_pong.elapsed() > HEARTBEAT_TIMEOUT {
            warn!("heartbeat timeout: session={}", session_id);
            break;
        }
        sender.send(Message::Ping(vec![].into())).await;
    }
    msg = receiver.next() => {
        match msg {
            Some(Ok(Message::Pong(_))) => {
                last_pong = Instant::now();
            }
            // ...
        }
    }
}
```

### 9.4 訊息大小限制

```rust
const MAX_WS_FRAME_BYTES: usize = 64 * 1024;    // 64 KB per frame
const MAX_WS_MESSAGE_BYTES: usize = 256 * 1024;  // 256 KB total
const MAX_WS_TEXT_BYTES: usize = 32 * 1024;       // 32 KB text content
```

三層防護：frame level → message level → application level。

### 9.5 Session Resume 協定

```
Client → Server: {"type": "resume", "session_id": "abc-123"}
Server → Client: {"type": "resumed", "session_id": "abc-123", "history_length": 42}

OR (if expired):
Server → Client: {"type": "connected", "session_id": "new-456", "note": "previous session expired"}
```

### 9.6 Clawtex 實作建議

clawtex-core 的 Hub 已有 Bearer token 認證，但缺少：

```rust
// src/cluster_hub.rs 建議加入

// 1. 常數時間 token 比較
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// 2. Per-connection rate limiter
struct RateLimiter {
    timestamps: std::collections::VecDeque<std::time::Instant>,
    max_per_window: u32,
    window: Duration,
}

impl RateLimiter {
    fn check(&mut self) -> bool {
        let now = std::time::Instant::now();
        self.timestamps.retain(|ts| now.duration_since(*ts) < self.window);
        if self.timestamps.len() >= self.max_per_window as usize {
            return false;
        }
        self.timestamps.push_back(now);
        true
    }
}
```

---

## 10. 輸入驗證與 Prompt Injection 偵測

**檔案**: `crates/opencrust-security/src/validation.rs` (114 行)

```rust
pub struct InputValidator;

impl InputValidator {
    pub fn check_prompt_injection(input: &str) -> bool {
        let patterns = [
            "ignore previous instructions",
            "ignore all previous",
            "disregard your instructions",
            "you are now",
            "new instructions:",
            "system prompt:",
            "forget everything",
            "override your",
            "act as if",
            "pretend you are",
            "do not follow",
            "bypass your",
            "reveal your system",
            "what is your system prompt",
        ];
        let lower = input.to_lowercase();
        patterns.iter().any(|p| lower.contains(p))
    }

    pub fn sanitize(input: &str) -> String {
        input.chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect()
    }

    pub fn validate_channel_id(id: &str) -> Result<()> {
        if id.is_empty() { return Err(Error::Security("channel ID cannot be empty")); }
        if id.len() > 256 { return Err(Error::Security("channel ID too long")); }
        Ok(())
    }
}
```

注意：這是基於字串匹配的簡單偵測，無法防禦進階的 prompt injection（如 Unicode 混淆、間接注入）。但作為第一道防線是合理的。

---

## 11. 日誌 Redaction 機制

**檔案**: `crates/opencrust-security/src/redaction.rs` (86 行)

```rust
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W: std::io::Write> std::io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let original = String::from_utf8_lossy(buf);
        let redacted = redact_secrets(&original);
        self.inner.write_all(redacted.as_bytes())?;
        Ok(buf.len())  // 回傳原始長度，避免呼叫者 retry
    }
}

pub fn redact_secrets(input: &str) -> String {
    static PATTERNS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?x)
            sk-ant-api\S{10,}           # Anthropic API keys
          | sk-\S{20,}                  # OpenAI-style keys
          | xoxb-\S{10,}               # Slack bot tokens
          | xapp-\S{10,}               # Slack app tokens
          | xoxp-\S{10,}               # Slack user tokens
          | Bot\s+[A-Za-z0-9_\-]{30,}  # Discord bot tokens
        ").expect("redaction regex should compile")
    });
    PATTERNS.replace_all(input, "[REDACTED]").into_owned()
}
```

巧妙之處：
- `LazyLock` 確保 regex 只編譯一次
- `MakeWriter` trait 實作讓它可以直接注入 tracing-subscriber
- `Ok(buf.len())` 回傳原始長度而非 redacted 長度，避免上層 writer 認為寫入不完整而 retry

### Clawtex 實作建議

```rust
// src/security/redaction.rs（建議新增）
use once_cell::sync::Lazy;
use regex::Regex;

static REDACTION_PATTERNS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)
        sk-ant-api\S{10,}
      | sk-\S{20,}
      | enc2:\S{10,}        # clawtex 加密 prefix
      | Bearer\s+\S{10,}    # Bearer tokens
    ").unwrap()
});

pub fn redact(input: &str) -> String {
    REDACTION_PATTERNS.replace_all(input, "[REDACTED]").into_owned()
}
```

---

## 12. 向量搜尋與記憶體系統

**檔案**: `crates/opencrust-db/src/vector_store.rs` (268 行)

### 12.1 sqlite-vec 整合

```rust
static SQLITE_VEC_INIT: Once = Once::new();
static mut SQLITE_VEC_LOADED: bool = false;

fn ensure_sqlite_vec_registered() -> bool {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        let func = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        sqlite3_auto_extension(Some(func));
        SQLITE_VEC_LOADED = true;
    });
    unsafe { SQLITE_VEC_LOADED }
}
```

- 使用 `Once` 確保全程序只註冊一次
- `sqlite3_auto_extension` 讓所有新建的 SQLite 連線自動載入 vec 擴展
- 如果 vec 不可用，gracefully fallback 到 in-Rust cosine similarity

### 12.2 KNN 搜尋

```rust
pub fn search_nearest(
    &self,
    query: &[f32],
    dimensions: usize,
    limit: usize,
) -> Result<Vec<(String, f64)>> {
    let sql = format!(
        "SELECT m.entry_id, v.distance
         FROM [{table_name}] v
         JOIN vec_id_map m ON m.rowid = v.rowid
         WHERE v.embedding MATCH ? AND k = ?"
    );
    // ...
}
```

使用 vec0 的 `MATCH` + `k` 語法進行 KNN 查詢，結果按距離升序排列。

### 12.3 ID 映射

vec0 virtual table 只支援 integer rowid，但記憶體系統使用 UUID。`vec_id_map` 表提供映射：

```sql
CREATE TABLE vec_id_map (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id TEXT NOT NULL UNIQUE
);
```

---

## 13. Context Window 管理與 Rolling Summarization

AgentRuntime 實現了兩層 context 管理：

1. **trim_messages_to_budget** — 硬裁切，從歷史訊息頭部開始刪除
2. **compact_messages** — 軟壓縮，使用 LLM 生成摘要替換舊訊息

觸發條件：`current_tokens > max_context_tokens * 0.75`

這與 OpenHands 的 Condenser Pipeline 類似，但更簡單。clawtex-core 的 `context_optimizer.rs` 已有類似實作。

---

## 14. DNA 個性化系統

首次對話時，agent 自動提問並建立個性化設定：
- 寫入 `~/.opencrust/dna.md`
- 包含使用者名稱、溝通風格、指導原則
- 支援熱重載（即時生效）
- 與 system_prompt 合併注入 LLM 請求

DNA 內容透過 `RwLock<Option<String>>` 存在 `AgentRuntime` 中，允許在 `Arc` 後修改：

```rust
pub fn set_dna_content(&self, content: Option<String>) {
    *self.dna_content.write().unwrap() = content;
}
```

---

## 15. Provider 系統

### 15.1 原生 Provider

| Provider | 模型預設 | 特色 |
|----------|---------|------|
| Anthropic | claude-sonnet-4-5-20250929 | SSE streaming、Vision、tool_use |
| OpenAI | gpt-4o | SyntheticEvent 多事件處理 |
| Ollama | llama3.1 | JSONL streaming、本地推理 |

### 15.2 SyntheticEvent 模式

```rust
// OpenAI SSE 的一個 chunk 可能產生多個事件
struct SyntheticEvent(StreamEvent);
// 序列化後推回 buffer 前端
for extra in iter.rev() {
    let json = serde_json::to_string(&SyntheticEvent(extra)).unwrap();
    buffer = format!("data: {json}\n\n{buffer}");
}
```

### 15.3 OpenAI-Compatible Provider

透過 `OpenAiProvider::with_name()` 實現 11 個相容 provider：
Sansa, DeepSeek, Mistral, Gemini, Falcon, Jais, Qwen, Yi, Cohere, MiniMax, Moonshot

### 15.4 API Key 解析優先順序

```
1. 加密金鑰保險庫 (vault)
2. 設定檔 (config.yml)
3. 環境變數
```

---

## 16. MCP 工具橋接

**檔案**: `crates/opencrust-agents/src/mcp/`

- 使用 `rmcp` 1.1 crate 實作 stdio 傳輸
- 工具以 `server_name.tool_name` 格式命名
- 支援 `resources/` 和 `prompts/` 端點
- 健康監控與自動重連
- MCP server instructions 自動注入 system prompt

---

## 17. 排程系統（Heartbeat）

### 17.1 三種排程模式

- **Cron**: 標準 cron 表達式
- **Interval**: 固定間隔
- **One-shot**: 一次性定時任務

### 17.2 排程執行流程

```
scheduler loop (every 5s)
    │
    poll_due_tasks() ← SQLite
    │
    for each task:
    │
    ├── persist system message to history
    ├── hydrate session history
    ├── process_heartbeat() → LLM response
    ├── persist assistant response
    ├── best-effort channel delivery (via ChannelSender)
    ├── complete_task()
    └── reschedule_recurring_task()
```

### 17.3 Cross-channel Delivery

```rust
// crates/opencrust-gateway/src/server.rs:437-450
let delivery_channel = if let Some(ref override_ch) = task.deliver_to_channel {
    if state.channel_senders.contains_key(override_ch.as_str()) {
        override_ch.as_str()  // 使用 override
    } else {
        &task.channel_id      // fallback 到原始 channel
    }
} else {
    &task.channel_id
};
```

### 17.4 heartbeat_depth 機制

`heartbeat_depth` 限制遞迴排程：
- 0 = 正常使用者訊息
- 1 = 排程任務
- 2, 3 = 巢狀排程

最多到 3 層，防止無限遞迴。

---

## 18. 測試架構

### 18.1 單元測試模式

每個 crate 都有 `#[cfg(test)] mod tests`，以下是覆蓋的關鍵場景：

| 模組 | 測試數量 | 測試重點 |
|------|---------|---------|
| credentials.rs | 4 | round-trip、wrong passphrase、remove、cache refresh |
| state.rs | 7 | session lifecycle、cleanup、continuity key |
| ws.rs | 6 | message size、resume parsing、user message parsing |
| validation.rs | 3 | prompt injection、sanitize、channel ID |
| redaction.rs | 4 | API key patterns、normal text |
| vector_store.rs | 2 | table creation、vec lifecycle |
| config model.rs | 2 | defaults、YAML parsing |

### 18.2 Mock Server 模式

```rust
// 使用 axum 建立 mock server
async fn run_mock_server() -> (String, oneshot::Sender<()>) {
    let app = Router::new()
        .route("/api/tags", get(|| async { Json(mock_models) }))
        .route("/api/chat", post(|payload| async move { mock_response }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, app));
    (url, stop_tx)
}
```

### 18.3 Integration Tests

使用 `wiremock` + `tokio-tungstenite` 測試 WebSocket：
- `health_endpoint_returns_ok`
- `ws_connect_receives_welcome_with_session_id`
- `ws_session_resume_returns_resumed_type`
- `ws_prompt_injection_rejected`
- `status_endpoint_returns_session_count`

---

## 19. 與 clawtex-core 的全面對比

### 19.1 架構對比

| 面向 | OpenCrust | clawtex-core |
|------|-----------|--------------|
| **Crate 組織** | 11 個獨立 crate (workspace) | 單一 crate (src/modules) |
| **頻道** | 5 (Telegram, Discord, Slack, WhatsApp, iMessage) | 1 (Telegram) |
| **Provider** | 14 (3 原生 + 11 相容) | 12+ (各自獨立實作) |
| **工具數量** | 8 內建 + MCP 橋接 | 24 內建 + MCP |
| **安全** | AES-256-GCM vault + OS Keychain | ChaCha20-Poly1305 |
| **排程** | SQLite-backed heartbeat (cron/interval/one-shot) | Cron + JobAction::Hand |
| **記憶體** | SQLite + sqlite-vec 向量搜尋 | SQLite + semantic memory |
| **設定** | YAML/TOML + 熱重載 | TOML (agents.toml)，需重啟 |
| **外掛** | WASM 沙箱 (wasmtime) | 無 |
| **多 Agent** | NamedAgentConfig (計畫中) | Hands 工作流引擎 |
| **叢集** | 無 | Hub + Worker 叢集系統 |
| **共用狀態** | AppState + DashMap | 分散的 Arc 們 |
| **Config 熱重載** | notify + watch channel | 無 |
| **日誌安全** | RedactingWriter | 無 |

### 19.2 clawtex-core 可借鑑的具體項目（按優先度排序）

#### P0（立即受益）

1. **Config 熱重載** — 加入 `notify` + `tokio::sync::watch`，修改 `agents.toml` 後不需重啟
2. **ChannelSender 分離** — 為未來多頻道擴展奠基，讓 Hands/cron 可持有輕量 sender

#### P1（中期改進）

3. **AppState 統一** — 用 `DashMap` 替代分散的 `Arc<Mutex>` session 管理
4. **常數時間 token 比較** — 防禦 Hub auth 的計時攻擊
5. **日誌 Redaction** — 避免 API key 洩漏到日誌
6. **OS Keychain 整合** — 用 `keyring` crate 自動管理加密密鑰

#### P2（長期方向）

7. **Workspace 拆分** — 將 `providers/`、`tools/`、`hands/` 拆成獨立 crate
8. **向量搜尋 KNN** — 用 sqlite-vec 的 `MATCH` + `k` 替代暴力搜尋
9. **DNA 個性化** — 首次對話自動建立使用者偏好設定檔

### 19.3 clawtex-core 的優勢（OpenCrust 沒有的）

| clawtex-core 獨有 | 說明 |
|-------------------|------|
| Hands 工作流引擎 | 多階段、可鏈接的自動化工作流 |
| 叢集系統 | Hub + Worker 分散式架構 |
| Smart Routing | 請求分類器 + 複雜度路由 |
| Revenue Pipeline | 完整的收入追蹤 + SaaS 自動化 |
| SoT 引擎 | Skeleton-of-Thought 並行生成 |
| 24 個工具 | 遠超 OpenCrust 的 8 個 |
| ChatGPT Backend | Codex CLI subprocess 存取 |
| Approval Gate | Telegram 人工審核閘門 |
| E-Stop | 緊急停止機制 |
| Cost/Revenue Tracking | SQLite 成本和收入追蹤 |
| Self-Evolution | Felix-style 每夜 1% 複合改進 |
| Context Optimizer | 獨立的上下文壓縮引擎 |

### 19.4 總結

OpenCrust 是一個設計精良的 **個人 AI 助手平台**，其核心價值在於：

1. **Config 熱重載** — 三層管道（notify → debounce → watch channel），覆蓋 editor atomic write
2. **AES-256-GCM 金鑰保險庫** — PBKDF2 600K + OS Keychain + fingerprint 快取，零用戶負擔加密
3. **Channel Lifecycle/Sender 分離** — 解決 `&mut self` vs `Arc<dyn>` 的所有權衝突，排程器可安全發送
4. **Arc-safe AppState** — DashMap 分片鎖 + AtomicBool + RwLock + watch::Receiver 的多策略並發安全
5. **安全優先** — 常數時間比較、rate limiting、prompt injection 偵測、日誌 redaction

clawtex-core 在功能豐富度、自動化流程、叢集支援方面遠超 OpenCrust，但在以下工程品質面向可以從 OpenCrust 汲取經驗：

- **可觀測性** — 日誌 redaction 防洩漏
- **操作便利性** — 設定熱重載免重啟
- **抽象品質** — trait 分離讓多頻道/多 task 共用自然
- **安全深度** — OS Keychain 整合讓加密對用戶透明

---

*分析完成。本文件深度解剖 OpenCrust v0.1.21 的 11 個 crate，重點覆蓋 Config 熱重載（96 行 watcher.rs 逐行分析）、AES-256-GCM 金鑰保險庫（597 行 credentials.rs 完整資料流）、Channel Lifecycle/Sender 分離（60 行 traits.rs + 所有權證明）、Arc-safe AppState（583 行 state.rs + 並發策略表）。每個核心模式都包含具體的 clawtex-core Rust 實作建議。*
