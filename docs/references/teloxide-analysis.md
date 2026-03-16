# Teloxide 深度技術分析 v2

> 分析時間：2026-03-13
> 分析目標：`LLM-Cluster-Project/references/teloxide/`
> 目的：為 clawtex-core Telegram 介面提供架構升級藍圖
> 版本：v2（深度翻倍重寫）

---

## 目錄

1. [專案結構與 Crate 分層](#1-專案結構與-crate-分層)
2. [Bot 結構體與型別安全 API](#2-bot-結構體與型別安全-api)
3. [Requester Trait 與適配器鏈](#3-requester-trait-與適配器鏈)
4. [Dispatcher 核心：Worker 模型與分發策略](#4-dispatcher-核心worker-模型與分發策略)
5. [dptree Handler Chain 與依賴注入系統](#5-dptree-handler-chain-與依賴注入系統)
6. [Dialogue 狀態機深度剖析](#6-dialogue-狀態機深度剖析)
7. [Storage Trait 與四種後端](#7-storage-trait-與四種後端)
8. [Throttle 速率限制器深度剖析](#8-throttle-速率限制器深度剖析)
9. [Long Polling 狀態機與 PollingStream](#9-long-polling-狀態機與-pollingstream)
10. [BotCommands 巨集系統](#10-botcommands-巨集系統)
11. [Update 過濾器巨集系統](#11-update-過濾器巨集系統)
12. [錯誤處理架構](#12-錯誤處理架構)
13. [ShutdownToken 與 StopToken 雙層停機](#13-shutdowntoken-與-stoptoken-雙層停機)
14. [與 clawtex telegram.rs 逐行對比](#14-與-clawtex-telegramrs-逐行對比)
15. [效能分析](#15-效能分析)
16. [Gap 對比總表](#16-gap-對比總表)
17. [遷移路線圖](#17-遷移路線圖)

---

## 1. 專案結構與 Crate 分層

### 1.1 Workspace 結構

Teloxide 是嚴格分層的三 crate workspace：

```
teloxide/
  Cargo.toml                      # workspace root
  crates/
    teloxide-core/                # 底層：Bot + API 型別 + HTTP + 適配器
    teloxide-macros/              # 程序巨集：#[derive(BotCommands)]
    teloxide/                     # 高階框架：Dispatcher + Dialogue + 過濾器
```

**分層依賴關係**：

```
teloxide (高階框架)
  ├── teloxide-core (底層 API 封裝)
  ├── teloxide-macros (程序巨集)
  └── dptree (外部: handler chain 框架)
```

### 1.2 各 Crate 職責範圍

**teloxide-core** (`crates/teloxide-core/src/`):
- `bot.rs` (L1-320) — Bot 結構體，三欄位：`token: Arc<str>`, `api_url: Arc<Url>`, `client: Client`
- `bot/api.rs` — 所有 Telegram API 方法的型別安全封裝（100+ 方法，自動生成）
- `bot/download.rs` — Download trait 實作（AsyncWrite + Stream 兩種模式）
- `errors.rs` — RequestError / ApiError / DownloadError 完整錯誤層級
- `net/` — HTTP 請求層（request_json / request_multipart / download_file）
- `payloads/` — 每個 API 方法一個 Payload 結構體（100+ 個）
- `adaptors/` — Throttle / CacheMe / Erased / Trace / ParseMode 適配器

**teloxide-macros** (`crates/teloxide-macros/src/`):
- `lib.rs` — `#[derive(BotCommands)]` 入口
- `bot_commands.rs` — BotCommands trait 實作的程式碼生成
- `command.rs` / `command_attr.rs` / `command_enum.rs` — 指令屬性解析
- `fields_parse.rs` — 參數解析器生成
- `rename_rules.rs` — 重命名規則（lowercase, camelCase, snake_case 等）

**teloxide** (`crates/teloxide/src/`):
- `dispatching/dispatcher.rs` (L1-720) — Dispatcher + DispatcherBuilder + Worker
- `dispatching/dialogue.rs` (L1-261) — Dialogue<D,S> 對話狀態管理
- `dispatching/dialogue/storage.rs` — Storage trait + InMem/SQLite/Redis/Postgres
- `dispatching/filter_ext.rs` (L1-187) — UpdateFilterExt + MessageFilterExt 巨集
- `dispatching/handler_ext.rs` (L1-296) — filter_command + enter_dialogue
- `dispatching/distribution.rs` (L1-10) — DefaultKey(ChatId) 分發鍵
- `update_listeners/polling.rs` (L1-522) — PollingBuilder + PollingStream
- `error_handlers.rs` (L1-213) — ErrorHandler trait + 三種實作
- `backoff.rs` (L1-33) — 指數退避策略
- `stop.rs` (L1-59) — StopToken/StopFlag（基於 AbortHandle）

### 1.3 Clawtex 實作建議

clawtex-core 目前將所有 Telegram 邏輯壓縮在單一 `src/telegram.rs`（562 行）。建議按 teloxide 的分層思路重構：

```rust
// 建議的模組結構
src/
  telegram/
    mod.rs          // TelegramChannel 主結構體 + Channel trait impl
    types.rs        // Telegram API 回應型別（或直接用 teloxide-core 的型別）
    commands.rs     // 指令 enum + 解析（或直接用 BotCommands 巨集）
    throttle.rs     // 速率限制（或直接用 Throttle 適配器）
    polling.rs      // Long polling 邏輯（含退避策略）
    streaming.rs    // 串流訊息編輯邏輯
```

---

## 2. Bot 結構體與型別安全 API

### 2.1 Bot 的記憶體佈局

```rust
// crates/teloxide-core/src/bot.rs L57-61
#[derive(Debug, Clone)]
pub struct Bot {
    token: Arc<str>,              // 8 bytes (pointer) — 共享，零複製
    api_url: Arc<reqwest::Url>,   // 8 bytes (pointer) — 支援自訂 TBA server
    client: Client,               // ~16 bytes — reqwest Client 內部也是 Arc
}
// 總計 ~32 bytes，Clone 成本極低（三個指標複製）
```

**關鍵設計決策**：Bot 的 Clone 成本等同於三個 Arc clone（原子遞增計數器），因此官方建議**直接 clone Bot 而非包裝 `Arc<Bot>`**。這在 teloxide-core/src/bot.rs L49-53 的文件中明確說明。

### 2.2 四種建構方式

```rust
// crates/teloxide-core/src/bot.rs L71-201

// 1. 直接指定 token（建立預設 reqwest::Client）
let bot = Bot::new("TOKEN");

// 2. 自訂 HTTP client
let bot = Bot::with_client("TOKEN", custom_client);

// 3. 從環境變數（TELOXIDE_TOKEN + TELOXIDE_API_URL + TELOXIDE_PROXY）
let bot = Bot::from_env();

// 4. 自訂 API URL（用於本地 Telegram Bot API Server）
let bot = Bot::new("TOKEN").set_api_url(url);
```

### 2.3 JSON vs Multipart 雙軌執行

Bot 內部有兩條請求路徑：

```rust
// crates/teloxide-core/src/bot.rs L226-255
pub(crate) fn execute_json<P>(&self, payload: &P)
    -> impl Future<Output = ResponseResult<P::Output>>
where
    P: Payload + Serialize,
    P::Output: DeserializeOwned,
{
    let client = self.client.clone();
    let token = Arc::clone(&self.token);
    let api_url = Arc::clone(&self.api_url);
    // 注意：使用 stacker::maybe_grow 防止深度序列化時 stack overflow
    let params = stacker::maybe_grow(256 * 1024, 1024 * 1024, || serde_json::to_vec(payload))
        .expect("serialization of request to be infallible");
    async move {
        net::request_json(&client, token.as_ref(), /* ... */).await
    }
}

// crates/teloxide-core/src/bot.rs L257-286
pub(crate) fn execute_multipart<P>(&self, payload: &mut P)
    -> impl Future<Output = ResponseResult<P::Output>>
where
    P: MultipartPayload + Serialize,
{
    // 用於含檔案上傳的請求（send_photo, send_document 等）
    let params = serde_multipart::to_form(payload);
    // ...
}
```

**stack 保護**：teloxide 使用 `stacker::maybe_grow(256KB, 1MB, ...)` 來防止大型 Payload 序列化時的 stack overflow。這是在 Rust 異步環境中的重要安全措施。

### 2.4 與 clawtex 的對比

```rust
// clawtex-core/src/telegram.rs L77-82
pub struct TelegramChannel {
    bot_token: String,           // 不是 Arc<str>，每次 clone 都是完整複製
    allowed_users: Vec<String>,  // 每次 clone 也是完整複製
    client: Client,              // 只有這個是低成本 clone
    offset: Arc<RwLock<i64>>,    // 需要 Arc+RwLock 因為跨 task 共享
}
```

**差距**：
1. `bot_token: String` 每次 clone 是 O(n) 而非 O(1)
2. 沒有型別安全的 API 方法封裝——所有 API 呼叫都是手動建構 JSON
3. 沒有 multipart 支援——無法透過 Telegram 上傳檔案
4. `offset` 使用 `Arc<RwLock<i64>>` 而 teloxide 直接在 PollingStream 內部管理

### 2.5 Clawtex 實作建議

```rust
/// 最小改動方案：將 bot_token 改為 Arc<str>
pub struct TelegramChannel {
    bot_token: Arc<str>,         // O(1) clone
    allowed_users: Arc<[String]>,// O(1) clone，不可變
    client: Client,
    offset: Arc<RwLock<i64>>,
}

/// 或者直接引入 teloxide-core：
use teloxide_core::Bot;
use teloxide_core::adaptors::throttle::Throttle;

pub struct TelegramChannel {
    bot: Throttle<Bot>,          // 型別安全 API + 自動限速
    allowed_users: Arc<[String]>,
    // offset 由 PollingStream 內部管理，不再需要
}
```

---

## 3. Requester Trait 與適配器鏈

### 3.1 Requester Trait 概覽

`Requester` 是 teloxide-core 的核心 trait，為每個 Telegram API 方法定義一個關聯類型和方法：

```rust
// teloxide-core 中的 Requester trait（簡化）
pub trait Requester {
    type Err: ...;
    type GetUpdates: Request<Payload = GetUpdates, Err = Self::Err>;
    type SendMessage: Request<Payload = SendMessage, Err = Self::Err>;
    type SendPhoto: Request<Payload = SendPhoto, Err = Self::Err>;
    // ... 100+ 方法

    fn get_updates(&self) -> Self::GetUpdates;
    fn send_message<C, T>(&self, chat_id: C, text: T) -> Self::SendMessage
    where C: Into<Recipient>, T: Into<String>;
    fn send_photo<C>(&self, chat_id: C, photo: InputFile) -> Self::SendPhoto
    where C: Into<Recipient>;
    // ...
}
```

### 3.2 適配器鏈模式（Decorator Chain）

teloxide 使用適配器鏈模式包裝 Bot，每一層透明地加入功能：

```rust
// 推薦的適配器堆疊順序
let bot = Bot::new("TOKEN")
    .throttle(Limits::default())  // 最內層：速率限制
    .cache_me()                   // 快取 getMe 結果
    .parse_mode(ParseMode::Html); // 預設 HTML 格式

// 型別：DefaultParseMode<CacheMe<Throttle<Bot>>>
// 每一層都實作 Requester，透明代理未修改的方法
```

### 3.3 Clawtex 實作建議

clawtex 目前沒有適配器鏈概念。可以用 newtype 模式實現類似效果：

```rust
/// Throttled wrapper for TelegramChannel
pub struct ThrottledTelegram {
    inner: TelegramChannel,
    limiter: RateLimiter,  // 例如 governor crate
}

#[async_trait]
impl Channel for ThrottledTelegram {
    async fn send(&self, chat_id: &str, text: &str) -> Result<()> {
        self.limiter.until_ready().await;
        self.inner.send(chat_id, text).await
    }
    // ...
}
```

---

## 4. Dispatcher 核心：Worker 模型與分發策略

### 4.1 Dispatcher 結構體完整分析

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L274-293
pub struct Dispatcher<R, Err, Key> {
    bot: R,                                           // Bot 實例
    dependencies: DependencyMap,                      // DI 容器

    handler: Arc<UpdateHandler<Err>>,                 // dptree handler chain
    default_handler: DefaultHandler,                  // 未匹配 update 的回呼

    distribution_f: fn(&Update) -> Option<Key>,       // 分發函式
    worker_queue_size: usize,                         // Worker 佇列大小（預設 64）
    current_number_of_active_workers: Arc<AtomicU32>, // 當前活躍 worker 數
    max_number_of_active_workers: Arc<AtomicU32>,     // 歷史最大活躍數（用於清理）
    workers: HashMap<Key, Worker>,                    // 按 Key 分組的 worker
    default_worker: Option<Worker>,                   // 無分發鍵的預設 worker

    error_handler: Arc<dyn ErrorHandler<Err> + Send + Sync>,
    state: ShutdownToken,                             // 三態停機控制
}
```

### 4.2 Worker 結構與雙軌處理

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L295-299
struct Worker {
    tx: tokio::sync::mpsc::Sender<Update>,  // update 發送端
    handle: tokio::task::JoinHandle<()>,     // tokio task 控制代碼
    is_waiting: Arc<AtomicBool>,            // 是否閒置（用於清理判斷）
}
```

**雙軌 Worker 模型**：

**1. 分組 Worker（`spawn_worker`）— 序列處理同 Key 的 update：**

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L602-641
fn spawn_worker<Err>(
    deps: DependencyMap,
    handler: Arc<UpdateHandler<Err>>,
    default_handler: DefaultHandler,
    error_handler: Arc<dyn ErrorHandler<Err> + Send + Sync>,
    current_number_of_active_workers: Arc<AtomicU32>,
    max_number_of_active_workers: Arc<AtomicU32>,
    queue_size: usize,
) -> Worker {
    let (tx, mut rx) = tokio::sync::mpsc::channel(queue_size);
    let is_waiting = Arc::new(AtomicBool::new(true));
    let is_waiting_local = Arc::clone(&is_waiting);
    let deps = Arc::new(deps);

    let handle = tokio::spawn(async move {
        while let Some(update) = rx.recv().await {
            is_waiting_local.store(false, Ordering::Relaxed);
            // 追蹤活躍 worker 數量
            let current = current_number_of_active_workers
                .fetch_add(1, Ordering::Relaxed) + 1;
            max_number_of_active_workers.fetch_max(current, Ordering::Relaxed);

            handle_update(update, deps.clone(), handler.clone(),
                         default_handler.clone(), error_handler.clone()).await;

            current_number_of_active_workers.fetch_sub(1, Ordering::Relaxed);
            is_waiting_local.store(true, Ordering::Relaxed);
        }
    });

    Worker { tx, handle, is_waiting }
}
```

**關鍵**：`while let Some(update) = rx.recv().await` — 同一 Key 的 update 嚴格序列執行，保證狀態一致性。

**2. 預設 Worker（`spawn_default_worker`）— 完全並行處理：**

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L643-667
fn spawn_default_worker<Err>(
    deps: DependencyMap,
    handler: Arc<UpdateHandler<Err>>,
    default_handler: DefaultHandler,
    error_handler: Arc<dyn ErrorHandler<Err> + Send + Sync>,
    queue_size: usize,
) -> Worker {
    let (tx, rx) = tokio::sync::mpsc::channel(queue_size);
    let deps = Arc::new(deps);

    let handle = tokio::spawn(
        ReceiverStream::new(rx).for_each_concurrent(None, move |update| {
            // None = 無併發限制，所有 update 完全並行
            handle_update(update, deps.clone(), handler.clone(),
                         default_handler.clone(), error_handler.clone())
        })
    );

    Worker { tx, handle, is_waiting: Arc::new(AtomicBool::new(true)) }
}
```

### 4.3 分發函式與 Worker 路由

```rust
// crates/teloxide/src/dispatching/distribution.rs L1-10
pub struct DefaultKey(ChatId);

pub(crate) fn default_distribution_function(update: &Update) -> Option<DefaultKey> {
    update.chat().map(|c| c.id).map(DefaultKey)
}
```

**Update 分發流程**：

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L473-528
async fn process_update(&mut self, update: Result<Update, LErr>, err_handler: &Arc<LErrHandler>) {
    match update {
        Ok(upd) => {
            // 1. 解析錯誤的 Update 直接丟棄（teloxide-core bug 報告）
            if let UpdateKind::Error(err) = upd.kind { return; }

            // 2. 用分發函式決定路由
            let worker = match (self.distribution_f)(&upd) {
                Some(key) => self.workers.entry(key).or_insert_with(|| {
                    // 按需建立分組 Worker（懶惰初始化）
                    spawn_worker(/* ... */)
                }),
                None => self.default_worker.get_or_insert_with(|| {
                    // 無 Key 的 update 路由到預設 Worker
                    spawn_default_worker(/* ... */)
                }),
            };

            // 3. 發送到 Worker 的 channel
            worker.tx.send(upd).await.expect("TX is dead");
        }
        Err(err) => err_handler.clone().handle_error(err).await,
    }
}
```

### 4.4 Worker 清理機制

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L530-570
async fn remove_inactive_workers_if_needed(&mut self) {
    let workers = self.workers.len();
    let max = self.max_number_of_active_workers.load(Ordering::Relaxed) as usize;
    if workers <= max { return; }  // 只在 worker 數超過歷史最大時清理
    self.remove_inactive_workers().await;
}

#[inline(never)]  // 冷函式，避免內聯膨脹熱路徑
async fn remove_inactive_workers(&mut self) {
    let handles = self.workers.iter()
        .filter(|(_, worker)| {
            // 佇列已空 AND worker 正在等待 = 閒置
            worker.tx.capacity() == self.worker_queue_size
                && worker.is_waiting.load(Ordering::Relaxed)
        })
        .map(|(k, _)| k).cloned().collect::<Vec<_>>()
        .into_iter()
        .map(|key| {
            let Worker { tx, handle, .. } = self.workers.remove(&key).unwrap();
            drop(tx);  // 關閉 channel → worker 停止
            handle
        });

    for handle in handles {
        let _ = handle.await;  // 等待 worker 完成
    }
}
```

**設計亮點**：
- `#[inline(never)]` 標記冷路徑，避免污染熱路徑的指令快取
- 使用 `capacity == queue_size` 判斷佇列是否為空（O(1) 而非 O(n)）
- `is_waiting` 原子旗標避免 race condition（worker 可能剛收到 update 還沒設旗標）

### 4.5 handle_update 核心函式

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L669-689
async fn handle_update<Err>(
    update: Update,
    deps: Arc<DependencyMap>,
    handler: Arc<UpdateHandler<Err>>,
    default_handler: DefaultHandler,
    error_handler: Arc<dyn ErrorHandler<Err> + Send + Sync>,
) {
    let mut deps = deps.deref().clone();  // 淺複製 DependencyMap
    deps.insert(update);                  // 注入當前 Update

    match handler.dispatch(deps).await {
        ControlFlow::Break(Ok(())) => {}                          // 成功處理
        ControlFlow::Break(Err(err)) => {                         // handler 錯誤
            error_handler.clone().handle_error(err).await
        }
        ControlFlow::Continue(deps) => {                          // 無 handler 匹配
            let update = deps.get();
            (default_handler)(update).await;
        }
    }
}
```

**ControlFlow 語義**：
- `Break(Ok(()))` — 某個 handler 成功處理了 update
- `Break(Err(err))` — 某個 handler 匹配但執行失敗
- `Continue(deps)` — 沒有 handler 匹配，交給 default_handler

### 4.6 Clawtex 實作建議

clawtex 目前在 `listen()` 中串行處理所有 update：

```rust
// clawtex-core/src/telegram.rs L389-488（現狀）
async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
    loop {
        // ... getUpdates ...
        for update in updates {
            // 串行處理每個 update — 一個慢請求阻擋所有 chat
            if tx.send(channel_msg).await.is_err() { /* ... */ }
        }
    }
}
```

**建議改為 Worker 模型**：

```rust
use std::collections::HashMap;
use tokio::sync::mpsc;

struct ChatWorker {
    tx: mpsc::Sender<ChannelMessage>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct TelegramChannel {
    // ... 現有欄位 ...
    workers: HashMap<i64, ChatWorker>,  // 按 chat_id 分組
}

impl TelegramChannel {
    fn get_or_create_worker(&mut self, chat_id: i64,
        agent_tx: mpsc::Sender<ChannelMessage>) -> &ChatWorker
    {
        self.workers.entry(chat_id).or_insert_with(|| {
            let (tx, mut rx) = mpsc::channel::<ChannelMessage>(64);
            let agent_tx = agent_tx.clone();
            let handle = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    // 同一 chat 序列處理
                    let _ = agent_tx.send(msg).await;
                }
            });
            ChatWorker { tx, handle }
        })
    }
}
```

---

## 5. dptree Handler Chain 與依賴注入系統

### 5.1 Handler Chain 核心概念

dptree 的 handler chain 是一種**職責鏈模式**，每個節點可以：
- **過濾**（filter）：決定是否繼續
- **轉換**（map/filter_map）：提取或注入依賴
- **分支**（branch）：嘗試多條路徑
- **端點**（endpoint）：最終處理函式

```rust
// 典型 handler chain 結構
let handler = dptree::entry()
    .branch(
        Update::filter_message()                    // 1. 過濾 Message 類型
            .branch(
                dptree::filter_command::<Cmd, _>()   // 2.a 過濾指令
                    .branch(case![Cmd::Help].endpoint(help))
                    .branch(case![Cmd::Start].endpoint(start))
            )
            .branch(
                Message::filter_text()               // 2.b 過濾純文字
                    .endpoint(handle_text)
            )
    )
    .branch(
        Update::filter_callback_query()             // 3. 過濾 CallbackQuery
            .endpoint(handle_callback)
    );
```

### 5.2 DependencyMap 依賴注入

Handler 函式的參數不需要手動提取——全部由 DependencyMap 自動注入：

```rust
// 這些參數全部自動注入：
async fn handler(
    bot: Bot,                   // <- Dispatcher 在 dispatch 時注入
    msg: Message,               // <- Update::filter_message() 從 Update 提取
    dialogue: MyDialogue,       // <- dialogue::enter() 建立並注入
    full_name: String,          // <- dptree::case![State::X { full_name }] 解構注入
    cfg: ConfigParameters,      // <- DispatcherBuilder::dependencies() 手動注入
) -> HandlerResult { /* ... */ }
```

**build 時型別檢查**：

```rust
// crates/teloxide/src/dispatching/dispatcher.rs L209-259
pub fn build(self) -> Dispatcher<R, Err, Key> {
    // dptree::type_check 在建置時驗證所有 handler 的依賴是否滿足
    dptree::type_check(
        handler.sig(),           // handler chain 的型別簽名
        &dependencies,           // 使用者提供的依賴
        &[
            dptree::Type::of::<R>(),                          // Bot
            dptree::Type::of::<teloxide_core::types::Update>(), // Update
            dptree::Type::of::<teloxide_core::types::Me>(),    // Me（bot 資訊）
        ],
    );
    // 如果型別檢查失敗，會 panic 並印出清楚的錯誤訊息
}
```

### 5.3 filter_command 的實作

```rust
// crates/teloxide/src/dispatching/handler_ext.rs L110-119
pub fn filter_command<C, Output>() -> Handler<'static, Output, DpHandlerDescription>
where
    C: BotCommands + Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    dptree::filter_map(move |message: Message, me: Me| {
        let bot_name = me.user.username.expect("Bots must have a username");
        // 也支援 caption 中的指令（圖片附帶指令場景）
        message.text()
            .or_else(|| message.caption())
            .and_then(|text| C::parse(text, &bot_name).ok())
    })
}
```

### 5.4 Clawtex 實作建議

clawtex 目前用字串比對處理指令：

```rust
// 當前 clawtex 模式（散佈在 agent_runtime.rs 或 main.rs 中）
if text.starts_with("/estop") { /* ... */ }
else if text.starts_with("/hands") { /* ... */ }
else if text.starts_with("/approve") { /* ... */ }
```

**建議的 enum-based 指令分發**：

```rust
/// 不需要 teloxide 巨集，也可以手動實作類似模式
#[derive(Clone)]
enum ClawtexCommand {
    Help,
    Hand { name: String },
    Hands,
    Estop,
    Resume,
    Costs,
    Revenue,
    Cron { action: String },
    Approve { id: String },
    Deny { id: String },
}

impl ClawtexCommand {
    fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        match parts[0] {
            "/help" | "/h" | "/?" => Some(Self::Help),
            "/hand" => Some(Self::Hand {
                name: parts.get(1).unwrap_or(&"").to_string()
            }),
            "/hands" => Some(Self::Hands),
            "/estop" => Some(Self::Estop),
            "/resume" => Some(Self::Resume),
            "/costs" => Some(Self::Costs),
            "/revenue" => Some(Self::Revenue),
            "/cron" => Some(Self::Cron {
                action: parts.get(1).unwrap_or(&"").to_string()
            }),
            "/approve" => Some(Self::Approve {
                id: parts.get(1).unwrap_or(&"").to_string()
            }),
            "/deny" => Some(Self::Deny {
                id: parts.get(1).unwrap_or(&"").to_string()
            }),
            _ => None,
        }
    }

    fn descriptions() -> &'static str {
        "/help - 顯示說明\n\
         /hand <name> - 執行 hand\n\
         /hands - 列出所有 hands\n\
         /estop - 緊急停止\n\
         /resume - 恢復\n\
         /costs - 費用報告\n\
         /revenue - 收入報告\n\
         /cron <action> - 排程管理\n\
         /approve <id> - 核准\n\
         /deny <id> - 拒絕"
    }
}
```

---

## 6. Dialogue 狀態機深度剖析

### 6.1 Dialogue 結構體

```rust
// crates/teloxide/src/dispatching/dialogue.rs L122-129
pub struct Dialogue<D, S>
where
    S: ?Sized,
{
    storage: Arc<S>,          // 儲存後端（Arc 共享）
    chat_id: ChatId,          // 此對話所屬的 chat
    _phantom: PhantomData<D>, // 對話狀態型別（零大小）
}
// 記憶體佈局：8 + 8 + 0 = 16 bytes
```

**手動 Clone 實作**（避免 derive 要求 D: Clone + S: Clone）：

```rust
// crates/teloxide/src/dispatching/dialogue.rs L133-139
impl<D, S> Clone for Dialogue<D, S> where S: ?Sized {
    fn clone(&self) -> Self {
        Dialogue {
            storage: self.storage.clone(),  // Arc clone: O(1)
            chat_id: self.chat_id,          // Copy: O(1)
            _phantom: PhantomData,
        }
    }
}
```

### 6.2 狀態機模式：enum 累積資料

```rust
// 典型的多步對話狀態
#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    ReceiveFullName,
    ReceiveAge { full_name: String },
    ReceiveLocation { full_name: String, age: u8 },
}
```

**設計精髓**：每個 variant 攜帶前面步驟累積的所有資料。這避免了：
1. 需要額外的「對話上下文」結構
2. 從 storage 多次讀取
3. 資料不一致風險

### 6.3 五個核心操作

```rust
// crates/teloxide/src/dispatching/dialogue.rs L142-206
impl<D, S> Dialogue<D, S> where D: Send + 'static, S: Storage<D> + ?Sized {
    // 建構
    pub fn new(storage: Arc<S>, chat_id: ChatId) -> Self { /* ... */ }

    // 讀取當前狀態
    pub async fn get(&self) -> Result<Option<D>, S::Error> {
        self.storage.clone().get_dialogue(self.chat_id).await
    }

    // 讀取或建立預設狀態
    pub async fn get_or_default(&self) -> Result<D, S::Error>
    where D: Default {
        match self.get().await? {
            Some(d) => Ok(d),
            None => {
                // 沒有現有對話 → 寫入 Default 然後返回
                self.storage.clone()
                    .update_dialogue(self.chat_id, D::default()).await?;
                Ok(D::default())
            }
        }
    }

    // 更新狀態（支援 From<State> 隱式轉換）
    pub async fn update<State>(&self, state: State) -> Result<(), S::Error>
    where D: From<State> {
        let new_dialogue = state.into();
        self.storage.clone()
            .update_dialogue(self.chat_id, new_dialogue).await?;
        Ok(())
    }

    // 重設到預設狀態
    pub async fn reset(&self) -> Result<(), S::Error>
    where D: Default {
        self.update(D::default()).await
    }

    // 退出對話（從 storage 刪除）
    pub async fn exit(&self) -> Result<(), S::Error> {
        self.storage.clone().remove_dialogue(self.chat_id).await
    }
}
```

### 6.4 enter() 函式：進入對話上下文

```rust
// crates/teloxide/src/dispatching/dialogue.rs L226-260
pub fn enter<Upd, S, D, Output>() -> Handler<'static, Output, DpHandlerDescription>
where
    S: Storage<D> + ?Sized + Send + Sync + 'static,
    D: Default + Clone + Send + Sync + 'static,
    Upd: GetChatId + Clone + Send + Sync + 'static,
{
    // 第一層：從 DependencyMap 取出 Storage 和 Update，建立 Dialogue
    dptree::filter_map(|storage: Arc<S>, upd: Upd| {
        let chat_id = upd.chat_id()?;        // 提取 chat_id
        Some(Dialogue::new(storage, chat_id))  // 建立 Dialogue 並注入
    })
    // 第二層：取得對話狀態
    .filter_map_async(|dialogue: Dialogue<D, S>| async move {
        match dialogue.get_or_default().await {
            Ok(dialogue) => Some(dialogue),     // 狀態值注入到後續 handler
            Err(err) => {
                // 根據環境變數決定行為：
                // TELOXIDE_DIALOGUE_BEHAVIOUR=default → 用預設值恢復
                // TELOXIDE_DIALOGUE_BEHAVIOUR=panic  → 記錯誤，跳過此 update
                match std::env::var("TELOXIDE_DIALOGUE_BEHAVIOUR").as_deref() {
                    Ok("default") => {
                        let default = D::default();
                        dialogue.update(default.clone()).await.ok()?;
                        Some(default)
                    }
                    _ => {
                        log::error!("dialogue.get_or_default() failed: {err:?}");
                        None
                    }
                }
            }
        }
    })
}
```

**注意**：`TELOXIDE_DIALOGUE_BEHAVIOUR` 環境變數提供了一個優雅降級機制——當對話狀態 enum 被修改後（例如新增 variant），舊的序列化資料可能無法反序列化。設為 "default" 可以自動重設而非 panic。

### 6.5 搭配 dptree::case! 的狀態路由

```rust
// 完整的對話 handler chain 範例
let handler = dialogue::enter::<Update, InMemStorage<State>, State, _>()
    .branch(
        Update::filter_message()
            .branch(case![State::Start].endpoint(start))
            .branch(case![State::ReceiveFullName].endpoint(receive_full_name))
            .branch(
                case![State::ReceiveAge { full_name }]
                    .endpoint(receive_age)
                // full_name 被自動解構並注入到 receive_age 的參數中
            )
            .branch(
                case![State::ReceiveLocation { full_name, age }]
                    .endpoint(receive_location)
                // full_name 和 age 以 tuple (String, u8) 注入
            )
    );
```

### 6.6 Clawtex 實作建議

clawtex 的 `approval.rs` 使用簡單的 pending request map，沒有多步對話支援。建議新增對話狀態機用於以下場景：

```rust
/// 用於 /setup 指令的多步設定對話
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum SetupDialogue {
    #[default]
    Idle,
    WaitingApiKey { provider: String },
    WaitingConfirmation { provider: String, key: String },
}

/// 用於 approval 的確認對話
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum ApprovalDialogue {
    #[default]
    None,
    PendingReview { request_id: String, description: String },
    WaitingReason { request_id: String, approved: bool },
}

/// 對話管理器（不依賴 teloxide，純 clawtex 實作）
pub struct DialogueManager<D: Default + Clone> {
    states: Arc<RwLock<HashMap<i64, D>>>,  // chat_id -> state
}

impl<D: Default + Clone> DialogueManager<D> {
    pub async fn get_or_default(&self, chat_id: i64) -> D {
        let states = self.states.read().await;
        states.get(&chat_id).cloned().unwrap_or_default()
    }

    pub async fn update(&self, chat_id: i64, state: D) {
        let mut states = self.states.write().await;
        states.insert(chat_id, state);
    }

    pub async fn exit(&self, chat_id: i64) {
        let mut states = self.states.write().await;
        states.remove(&chat_id);
    }
}
```

---

## 7. Storage Trait 與四種後端

### 7.1 Storage Trait 定義

```rust
// crates/teloxide/src/dispatching/dialogue/storage.rs L55-96
pub trait Storage<D> {
    type Error;

    fn remove_dialogue(self: Arc<Self>, chat_id: ChatId)
        -> BoxFuture<'static, Result<(), Self::Error>>
    where D: Send + 'static;

    fn update_dialogue(self: Arc<Self>, chat_id: ChatId, dialogue: D)
        -> BoxFuture<'static, Result<(), Self::Error>>
    where D: Send + 'static;

    fn get_dialogue(self: Arc<Self>, chat_id: ChatId)
        -> BoxFuture<'static, Result<Option<D>, Self::Error>>;

    // 型別擦除：將具體 Error 轉為 Box<dyn Error>
    fn erase(self: Arc<Self>) -> Arc<ErasedStorage<D>>
    where Self: Sized + Send + Sync + 'static,
          Self::Error: std::error::Error + Send + Sync + 'static
    {
        Arc::new(Eraser(self))
    }
}
```

**`self: Arc<Self>` 的設計原因**：Storage 需要在多個 handler 間共享，且生命週期需要是 `'static`（因為 handler 可能在 tokio task 中執行）。使用 `Arc<Self>` 是 Rust 中最自然的解決方案。

### 7.2 InMemStorage

```rust
// 內部就是一個 tokio::sync::Mutex<HashMap<ChatId, D>>
pub struct InMemStorage<D> {
    map: Mutex<HashMap<ChatId, D>>,
}

impl<S> InMemStorage<S> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { map: Mutex::new(HashMap::new()) })
    }
}
```

### 7.3 SqliteStorage

```rust
// 使用 sqlx 連線池 + 可插拔序列化器
pub struct SqliteStorage<S> {
    pool: SqlitePool,
    serializer: S,  // Json 或 Bincode
}

// 自動建表
// CREATE TABLE IF NOT EXISTS teloxide_dialogues (
//     chat_id BIGINT PRIMARY KEY,
//     dialogue BLOB NOT NULL
// );

// 使用 UPSERT 更新
// INSERT INTO teloxide_dialogues (chat_id, dialogue)
// VALUES (?, ?)
// ON CONFLICT (chat_id) DO UPDATE SET dialogue = excluded.dialogue
```

### 7.4 Serializer 抽象

```rust
// 可插拔的序列化器
pub trait Serializer<D> {
    type Error;
    fn serialize(&self, val: &D) -> Result<Vec<u8>, Self::Error>;
    fn deserialize(&self, data: &[u8]) -> Result<D, Self::Error>;
}

// 內建：Json (serde_json) 和 Bincode (bincode)
```

### 7.5 型別擦除：Eraser 包裝器

```rust
// crates/teloxide/src/dispatching/dialogue/storage.rs L98-140
struct Eraser<S>(Arc<S>);

impl<D, S> Storage<D> for Eraser<S>
where
    S: Storage<D> + Send + Sync + 'static,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn remove_dialogue(self: Arc<Self>, chat_id: ChatId)
        -> BoxFuture<'static, Result<(), Self::Error>>
    where D: Send + 'static {
        Box::pin(async move {
            Arc::clone(&self.0).remove_dialogue(chat_id).await
                .map_err(|e| e.into())
        })
    }
    // ... 其他方法類似
}
```

這允許在不知道具體 Storage 型別的情況下使用 `Arc<ErasedStorage<D>>`。

### 7.6 Clawtex 實作建議

clawtex 已有 `memory.db` 和 `core.db`。對話狀態可以直接存入現有 SQLite：

```rust
/// 在 core.db 中新增 dialogues 表
/// CREATE TABLE IF NOT EXISTS dialogues (
///     chat_id INTEGER PRIMARY KEY,
///     state BLOB NOT NULL,
///     updated_at TEXT DEFAULT CURRENT_TIMESTAMP
/// );

pub struct ClawtexDialogueStorage {
    pool: sqlx::SqlitePool,
}

impl ClawtexDialogueStorage {
    pub async fn get<D: serde::de::DeserializeOwned>(
        &self, chat_id: i64
    ) -> Result<Option<D>> {
        let row = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT state FROM dialogues WHERE chat_id = ?"
        )
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(data) => Ok(Some(serde_json::from_slice(&data)?)),
            None => Ok(None),
        }
    }

    pub async fn update<D: serde::Serialize>(
        &self, chat_id: i64, state: &D
    ) -> Result<()> {
        let data = serde_json::to_vec(state)?;
        sqlx::query(
            "INSERT INTO dialogues (chat_id, state) VALUES (?, ?)
             ON CONFLICT (chat_id) DO UPDATE SET state = excluded.state,
             updated_at = CURRENT_TIMESTAMP"
        )
        .bind(chat_id)
        .bind(data)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

---

## 8. Throttle 速率限制器深度剖析

### 8.1 整體架構

Throttle 是一個 Bot 適配器，使用**專用 Worker task** 控制所有請求的發送速率：

```
                ThrottlingRequest                    worker task

   send() ──► (ChatIdHash, Lock) ──► mpsc::channel ──► Worker 演算法
                                                         │
              RequestWaiter.await  ◄── oneshot::channel ◄─┘ unlock()
                    │
                    ▼
              發送實際 API 請求
```

### 8.2 Throttle 結構體

```rust
// crates/teloxide-core/src/adaptors/throttle.rs L73-79
#[derive(Clone, Debug)]
pub struct Throttle<B> {
    bot: B,                                          // 被包裝的 Bot
    queue: mpsc::Sender<(ChatIdHash, RequestLock)>,  // 請求佇列
    info_tx: mpsc::Sender<InfoMessage>,              // 控制通道（get/set limits）
}
```

### 8.3 ChatIdHash：低成本的 Chat 識別

```rust
// crates/teloxide-core/src/adaptors/throttle.rs L181-216
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
enum ChatIdHash {
    Id(ChatId),                   // 數值 ID：直接使用
    ChannelUsernameHash(u64),     // 使用者名稱：雜湊後使用
}

impl ChatIdHash {
    fn is_channel_or_supergroup(&self) -> bool {
        match self {
            &Self::Id(id) => id.is_channel_or_supergroup(),
            Self::ChannelUsernameHash(_) => true,  // 使用者名稱一定是頻道
        }
    }
}
```

**設計取捨**：使用者名稱被雜湊為 u64，使 ChatIdHash 成為 `Copy` 型別（16 bytes）。缺點是無法比較 `ChatId::Id(123)` 和 `ChatId::ChannelUsername("@channel")` 是否指向同一 chat。

### 8.4 Limits 結構與預設值

```rust
// crates/teloxide-core/src/adaptors/throttle/settings.rs L43-55
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Limits {
    pub messages_per_sec_chat: u32,                     // 預設 1
    pub messages_per_min_chat: u32,                     // 預設 20
    pub messages_per_min_channel_or_supergroup: u32,    // 預設 10
    pub messages_per_sec_overall: u32,                  // 預設 30
}
```

這些預設值來自 Telegram 官方文件。

### 8.5 Settings 進階配置

```rust
// crates/teloxide-core/src/adaptors/throttle/settings.rs L26-31
pub struct Settings {
    pub limits: Limits,
    pub on_queue_full: BoxedFnMut<usize, BoxedFuture>,  // 佇列滿時的回呼
    pub retry: bool,                                     // 是否自動重試 RetryAfter
    pub check_slow_mode: bool,                           // 是否檢查 slow mode
}
```

### 8.6 Worker 演算法深度剖析

Worker 是整個 Throttle 的核心，位於 `crates/teloxide-core/src/adaptors/throttle/worker.rs`。

**資料結構**：

```rust
// worker.rs L36-45
#[derive(Default)]
struct RequestsSentToChats {
    per_min: HashMap<ChatIdHash, RequestsSent>,  // 每分鐘計數
    per_sec: HashMap<ChatIdHash, RequestsSent>,  // 每秒計數（每次迭代重建）
}

// 歷史記錄：(ChatIdHash, Instant) 的時間序列
let mut history: VecDeque<(ChatIdHash, Instant)> = VecDeque::new();
```

**演算法主迴圈（worker.rs L95-283）**：

```
每 250ms 迭代一次：

Step 1. 回應 InfoMessage（GetLimits / SetLimits）
Step 2. 從 rx 讀取新請求到 queue（若 queue 為空則阻塞等待）
Step 3. 處理 freeze 事件（RetryAfter 觸發的凍結）
Step 4. 清理過期歷史（> 1 分鐘前的記錄）
Step 5. 計算每秒已用額度：allowed = limit - used_in_last_second
Step 6. 若 allowed == 0 → sleep(250ms) 並 continue
Step 7. 重建 per_sec 計數
Step 8. 遍歷 queue，對每個請求檢查：
        - slow mode 限制
        - per_sec_chat < limit
        - per_min_chat < limit (區分頻道/一般群組)
        若通過 → unlock 請求，更新計數，記錄歷史
        若 allowed 歸零 → 停止
Step 9. 清除 per_sec 計數（下次迭代重建）
Step 10. sleep(250ms)
```

**具體程式碼**：

```rust
// worker.rs L190-283（核心限速邏輯）
let now = Instant::now();
let min_back = now.checked_sub(MINUTE).unwrap_or(now);
let sec_back = now.checked_sub(SECOND).unwrap_or(now);

// Step 4: 清理過期歷史
while let Some((_, time)) = history.front() {
    if time >= &min_back { break; }  // 歷史已排序，找到第一筆未過期的
    if let Some((chat, _)) = history.pop_front() {
        let entry = requests_sent.per_min.entry(chat)
            .and_modify(|count| { *count -= 1; });
        if let Entry::Occupied(entry) = entry {
            if *entry.get() == 0 { entry.remove_entry(); }
        }
    }
}

// Step 5: 計算每秒額度
let used = history.iter().rev()
    .take_while(|(_, time)| time > &sec_back)
    .count() as u32;
let mut allowed = limits.messages_per_sec_overall.saturating_sub(used);

if allowed == 0 {
    requests_sent.per_sec.clear();
    tokio::time::sleep(DELAY).await;
    continue;
}

// Step 8: 遍歷 queue 解鎖請求
let mut queue_removing = queue.removing();  // vecrem 提供的就地移除迭代器
while let Some(entry) = queue_removing.next() {
    let chat = &entry.value().0;

    // 檢查 slow mode
    if let Some(&mut (delay, last)) = slow_mode.as_mut().and_then(|sm| sm.get_mut(chat)) {
        if last + delay > Instant::now() { continue; }
    }

    let per_sec = requests_sent.per_sec.get(chat).copied().unwrap_or(0);
    let per_min = requests_sent.per_min.get(chat).copied().unwrap_or(0);

    let min_limit = if chat.is_channel_or_supergroup() {
        limits.messages_per_min_channel_or_supergroup
    } else {
        limits.messages_per_min_chat
    };

    let limits_ok = per_sec < limits.messages_per_sec_chat
        && per_min < min_limit;

    if limits_ok {
        let chat = *chat;
        let (_, lock) = entry.remove();

        // 只有請求未被 drop 時才計數
        if lock.unlock(retry, freeze_tx.clone()).is_ok() {
            *requests_sent.per_sec.entry(chat).or_insert(0) += 1;
            *requests_sent.per_min.entry(chat).or_insert(0) += 1;
            history.push_back((chat, Instant::now()));

            allowed -= 1;
            if allowed == 0 { break; }
        }
    }
}
```

### 8.7 RequestLock / RequestWaiter 機制

```rust
// crates/teloxide-core/src/adaptors/throttle/request_lock.rs L14-45
pub(super) fn channel() -> (RequestLock, RequestWaiter) {
    let (tx, rx) = oneshot::channel();
    (RequestLock(tx), RequestWaiter(rx))
}

// RequestLock 被 Worker 持有
pub(super) struct RequestLock(Sender<(bool, mpsc::Sender<FreezeUntil>)>);

impl RequestLock {
    pub(super) fn unlock(self, retry: bool, freeze: mpsc::Sender<FreezeUntil>)
        -> Result<(), ()>
    {
        self.0.send((retry, freeze)).map_err(drop)
        // 如果 RequestWaiter 已被 drop（請求被取消），返回 Err
    }
}

// RequestWaiter 被 ThrottlingRequest 持有
pub(super) struct RequestWaiter(Receiver<(bool, mpsc::Sender<FreezeUntil>)>);

impl Future for RequestWaiter {
    type Output = (bool, mpsc::Sender<FreezeUntil>);
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match this.0.poll(cx) {
            Poll::Ready(Ok(ret)) => Poll::Ready(ret),
            Poll::Ready(Err(_)) => panic!("`RequestLock` is dropped by the throttle worker"),
            Poll::Pending => Poll::Pending,
        }
    }
}
```

### 8.8 ThrottlingRequest 的完整流程

```rust
// crates/teloxide-core/src/adaptors/throttle/request.rs L156-224
async fn send<R>(
    mut request: ShareableRequest<R>,
    chat: ChatIdHash,
    worker: mpsc::Sender<(ChatIdHash, RequestLock)>,
) -> Result<Output<R>, R::Err> {
    loop {
        // 1. 建立 lock/waiter pair
        let (lock, wait) = channel();

        // 2. 發送到 worker 佇列
        if worker.send((chat, lock)).await.is_err() {
            // Worker 已死 → 直接發送（fallback）
            return match &mut request {
                ShareableRequest::Shared(shared) => shared.send_ref().await,
                ShareableRequest::Owned(owned) => owned.take().unwrap().await,
            };
        }

        // 3. 等待 worker 解鎖
        let (retry, freeze) = wait.await;

        // 4. 發送實際請求
        let res = match (retry, &mut request) {
            (true, request) => {
                // 若開啟重試，使用 send_ref 保留所有權
                let req = match request {
                    ShareableRequest::Shared(s) => &**s,
                    ShareableRequest::Owned(o) => o.as_ref().unwrap(),
                };
                req.send_ref().await
            }
            (false, ShareableRequest::Owned(owned)) => owned.take().unwrap().await,
            (false, ShareableRequest::Shared(shared)) => shared.send_ref().await,
        };

        // 5. 檢查 RetryAfter 錯誤
        let retry_after = res.as_ref().err().and_then(<_>::retry_after);
        if let Some(retry_after) = retry_after {
            let after = retry_after.duration();
            let until = Instant::now() + after;
            // 通知 worker 凍結
            let _ = freeze.send(FreezeUntil { until, after, chat }).await;
            if retry {
                log::warn!("Freezing, before retrying: {retry_after:?}");
                tokio::time::sleep_until(until.into()).await;
            }
        }

        // 6. 決定是否重試
        match res {
            Err(_) if retry && retry_after.is_some() => continue,
            res => break res,
        };
    }
}
```

### 8.9 Freeze 機制：全域凍結

```rust
// worker.rs L302-356
async fn freeze(
    rx: &mut mpsc::Receiver<FreezeUntil>,
    mut slow_mode: Option<&mut HashMap<ChatIdHash, (Duration, Instant)>>,
    bot: &impl Requester,
    mut imm: Option<FreezeUntil>,
) {
    while let Some(freeze_until) = imm.take().or_else(|| rx.try_recv().ok()) {
        let FreezeUntil { until, after, chat } = freeze_until;

        // 如果開啟 slow mode 檢測，查詢該 chat 的 slow mode 設定
        if let Some(slow_mode) = slow_mode.as_deref_mut() {
            if let hash @ ChatIdHash::Id(id) = chat {
                if let Ok(chat) = bot.get_chat(id).await {
                    match chat.slow_mode_delay() {
                        Some(delay) => {
                            slow_mode.insert(hash, (delay.duration(), Instant::now()));
                        }
                        None => { slow_mode.remove(&hash); }
                    };
                }
            }
        }

        // 如果不是 slow mode 造成的 → 全域凍結
        let is_slow_mode = slow_mode.as_ref()
            .and_then(|m| m.get(&chat).map(|(delay, _)| delay <= &after))
            .unwrap_or(false);

        if !is_slow_mode {
            log::warn!("freezing the bot for ~{after:?} due to RetryAfter");
            tokio::time::sleep_until(until.into()).await;
            log::warn!("unfreezing the bot");
        }
    }
}
```

### 8.10 Clawtex 實作建議

clawtex 目前完全沒有 Telegram API 速率限制保護。建議實作簡化版 Throttle：

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct TelegramThrottle {
    // 每 chat 的請求歷史
    history: Mutex<HashMap<i64, Vec<Instant>>>,
    // 限制
    per_sec_overall: u32,     // 30
    per_sec_chat: u32,        // 1
    per_min_chat: u32,        // 20
}

impl TelegramThrottle {
    pub fn new() -> Self {
        Self {
            history: Mutex::new(HashMap::new()),
            per_sec_overall: 30,
            per_sec_chat: 1,
            per_min_chat: 20,
        }
    }

    /// 等待直到可以發送請求
    pub async fn wait_for_slot(&self, chat_id: i64) {
        loop {
            let now = Instant::now();
            let mut history = self.history.lock().await;

            // 清理過期記錄
            for timestamps in history.values_mut() {
                timestamps.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
            }

            // 檢查全域每秒限制
            let total_last_sec: usize = history.values()
                .flat_map(|v| v.iter())
                .filter(|t| now.duration_since(**t) < Duration::from_secs(1))
                .count();

            if total_last_sec >= self.per_sec_overall as usize {
                drop(history);
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }

            // 檢查每 chat 限制
            let chat_history = history.entry(chat_id).or_default();
            let chat_last_sec = chat_history.iter()
                .filter(|t| now.duration_since(**t) < Duration::from_secs(1))
                .count();
            let chat_last_min = chat_history.len();

            if chat_last_sec >= self.per_sec_chat as usize
                || chat_last_min >= self.per_min_chat as usize
            {
                drop(history);
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }

            // 記錄並放行
            chat_history.push(now);
            break;
        }
    }
}
```

---

## 9. Long Polling 狀態機與 PollingStream

### 9.1 PollingBuilder

```rust
// crates/teloxide/src/update_listeners/polling.rs L31-38
pub struct PollingBuilder<R> {
    pub bot: R,
    pub timeout: Option<Duration>,
    pub limit: Option<u8>,
    pub allowed_updates: Option<Vec<AllowedUpdate>>,
    pub drop_pending_updates: bool,
    pub backoff_strategy: BackoffStrategy,
}
```

### 9.2 PollingStream 狀態機

```rust
// crates/teloxide/src/update_listeners/polling.rs L291-328
pub struct PollingStream<'a, B: Requester> {
    polling: &'a mut Polling<B>,
    drop_pending_updates: bool,
    timeout: Option<u32>,
    allowed_updates: Option<Vec<AllowedUpdate>>,
    offset: i32,              // Telegram update offset
    force_stop: bool,         // 強制停止
    stopping: bool,           // 正在優雅停機
    buffer: vec::IntoIter<Update>,  // 緩衝區
    in_flight: Option<<B::GetUpdates as Request>::Send>,  // 進行中的請求
    flag: StopFlag,           // 停機旗標
    eepy: Option<Sleep>,      // 退避延遲（eepy = sleepy 的可愛寫法）
    error_count: u32,         // 連續錯誤計數
}
```

### 9.3 poll_next 狀態機圖

```
┌──────────────────────────────────────────────────────────────┐
│                    PollingStream::poll_next                    │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  force_stop?  ──Y──► Ready(None)                             │
│       │                                                      │
│       N                                                      │
│       ▼                                                      │
│  buffer 有資料?  ──Y──► Ready(Some(Ok(update)))               │
│       │                                                      │
│       N                                                      │
│       ▼                                                      │
│  stop flag 已觸發?  ──Y──► stopping=true, drop in_flight     │
│       │                                                      │
│       N                                                      │
│       ▼                                                      │
│  in_flight 存在?  ──Y──► poll in_flight                      │
│       │                    │                                 │
│       │                    ├── Ok(updates) + stopping         │
│       │                    │   → Ready(None)                 │
│       │                    ├── Err + stopping                 │
│       │                    │   → force_stop=true             │
│       │                    │     Ready(Some(Err))             │
│       │                    ├── Ok(updates)                    │
│       │                    │   → error_count=0               │
│       │                    │     offset = last.id + 1        │
│       │                    │     buffer = updates            │
│       │                    └── Err(e)                         │
│       │                        ├── RetryAfter(n)             │
│       │                        │   → eepy=sleep(n)           │
│       │                        └── Network error             │
│       │                            → eepy=backoff(count++)   │
│       N                                                      │
│       ▼                                                      │
│  eepy 存在?  ──Y──► poll eepy → 完成後清除                    │
│       │                                                      │
│       N                                                      │
│       ▼                                                      │
│  建立 get_updates 請求：                                      │
│    normal:   (offset, limit, timeout)                         │
│    stopping: (offset, Some(1), Some(0))  ← 確認已處理         │
│    dropping: (-1, Some(1), Some(0))      ← 丟棄待處理         │
│                                                              │
│  in_flight = Some(request.send())                            │
│  cx.waker().wake_by_ref()  ← 立即再次 poll                   │
│  Pending                                                     │
└──────────────────────────────────────────────────────────────┘
```

### 9.4 Graceful Shutdown

```rust
// polling.rs L474-488
let (offset, limit, timeout) = match (this.stopping, this.drop_pending_updates) {
    (false, false) => (*this.offset, this.polling.limit, *this.timeout),
    // 優雅停機：timeout=0, limit=1 → 立即返回 + 確認 offset
    (true, _) => {
        log::trace!("graceful shutdown `get_updates` call");
        (*this.offset, Some(1), Some(0))
    }
    // 丟棄待處理：offset=-1 → 從最新開始
    (_, true) => (-1, Some(1), Some(0)),
};
```

### 9.5 退避策略

```rust
// crates/teloxide/src/backoff.rs L11-14
pub fn exponential_backoff_strategy(error_count: u32) -> Duration {
    Duration::from_secs(1_u64 << error_count.min(6))
    // 1s, 2s, 4s, 8s, 16s, 32s, 64s, 64s, ...
}
```

### 9.6 與 clawtex 的對比

```rust
// clawtex-core/src/telegram.rs L389-488（現狀）
async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
    loop {
        // 問題 1：固定 5 秒重試，沒有指數退避
        Err(e) => {
            warn!("getUpdates failed: {}, retrying in 5s", e);
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        // 問題 2：沒有 RetryAfter 處理
        // 問題 3：沒有 Graceful Shutdown
        // 問題 4：沒有 allowed_updates 最佳化
        // 問題 5：沒有 drop_pending_updates 支援
    }
}
```

### 9.7 Clawtex 實作建議

```rust
/// 改進 clawtex 的 polling loop
async fn listen_improved(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
    let mut error_count: u32 = 0;

    loop {
        let offset = *self.offset.read().await;

        let url = format!(
            "{}?offset={}&timeout={}&allowed_updates=[\"message\",\"callback_query\"]",
            self.api_url("getUpdates"), offset, POLL_TIMEOUT
        );

        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                // 指數退避而非固定 5s
                let delay = Duration::from_secs(1u64 << error_count.min(6));
                warn!("getUpdates failed: {}, retrying in {:?}", e, delay);
                error_count = error_count.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let body: TelegramResponse<Vec<Update>> = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                let delay = Duration::from_secs(1u64 << error_count.min(6));
                warn!("Parse failed: {}, retrying in {:?}", e, delay);
                error_count = error_count.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        // 成功 → 重設錯誤計數
        error_count = 0;

        // 檢查 RetryAfter
        if !body.ok {
            if let Some(desc) = &body.description {
                if desc.contains("retry after") {
                    // 解析延遲秒數
                    if let Some(secs) = desc.split("retry after ")
                        .nth(1)
                        .and_then(|s| s.trim().parse::<u64>().ok())
                    {
                        warn!("Rate limited, sleeping {}s", secs);
                        tokio::time::sleep(Duration::from_secs(secs)).await;
                        continue;
                    }
                }
            }
        }

        // ... 處理 updates ...
    }
}
```

---

## 10. BotCommands 巨集系統

### 10.1 使用範例

```rust
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    /// Display this text.
    #[command(aliases = ["h", "?"])]
    Help,
    /// Handle a username.
    #[command(alias = "u")]
    Username(String),
    /// Handle a username and an age.
    #[command(parse_with = "split", alias = "ua", hide_aliases)]
    UsernameAndAge { username: String, age: u8 },
}
```

### 10.2 巨集生成的 trait

```rust
pub trait BotCommands: Sized {
    fn parse(s: &str, bot_username: &str) -> Result<Self, ParseError>;
    fn descriptions() -> CommandDescriptions<'static>;
    fn bot_commands() -> Vec<BotCommand>;
}
```

### 10.3 列舉層級屬性

| 屬性 | 說明 |
|------|------|
| `rename_rule` | lowercase, UPPERCASE, PascalCase, camelCase, snake_case, kebab-case |
| `prefix` | 指令前綴（預設 "/"） |
| `description` | 全域描述 |
| `parse_with` | default / split / 自訂函式 |
| `separator` | split 解析器的分隔符 |
| `command_separator` | 指令與參數的分隔符 |

### 10.4 變體層級屬性

| 屬性 | 說明 |
|------|------|
| `rename` | 字面重命名 |
| `description` | 指令描述 |
| `parse_with` | 自訂解析函式 |
| `hide` | 從 help 隱藏 |
| `alias` / `aliases` | 別名 |
| `hide_aliases` | 隱藏別名 |

### 10.5 Clawtex 實作建議

不需要引入 teloxide-macros，可以用 `strum` 或手動實作等效功能。見 Section 5.4 的 `ClawtexCommand` enum。

---

## 11. Update 過濾器巨集系統

### 11.1 UpdateFilterExt（23 種 update 過濾器）

```rust
// crates/teloxide/src/dispatching/filter_ext.rs L162-186
define_update_ext! {
    (filter_message, UpdateKind::Message, Message),
    (filter_edited_message, UpdateKind::EditedMessage, EditedMessage),
    (filter_channel_post, UpdateKind::ChannelPost, ChannelPost),
    (filter_callback_query, UpdateKind::CallbackQuery, CallbackQuery),
    (filter_inline_query, UpdateKind::InlineQuery, InlineQuery),
    (filter_shipping_query, UpdateKind::ShippingQuery, ShippingQuery),
    (filter_pre_checkout_query, UpdateKind::PreCheckoutQuery, PreCheckoutQuery),
    (filter_poll, UpdateKind::Poll, Poll),
    (filter_poll_answer, UpdateKind::PollAnswer, PollAnswer),
    (filter_my_chat_member, UpdateKind::MyChatMember, MyChatMember),
    (filter_chat_member, UpdateKind::ChatMember, ChatMember),
    (filter_chat_join_request, UpdateKind::ChatJoinRequest, ChatJoinRequest),
    (filter_chat_boost, UpdateKind::ChatBoost, ChatBoost),
    (filter_removed_chat_boost, UpdateKind::RemovedChatBoost, RemovedChatBoost),
    // ... 以及 Business 相關過濾器
}
```

### 11.2 MessageFilterExt（46+ 種訊息過濾器）

```rust
// crates/teloxide/src/dispatching/filter_ext.rs L75-143
define_message_ext! {
    (filter_text, Message::text),
    (filter_photo, Message::photo),
    (filter_video, Message::video),
    (filter_document, Message::document),
    (filter_audio, Message::audio),
    (filter_voice, Message::voice),
    (filter_sticker, Message::sticker),
    (filter_location, Message::location),
    (filter_contact, Message::contact),
    (filter_dice, Message::dice),
    (filter_animation, Message::animation),
    (filter_invoice, Message::invoice),
    (filter_successful_payment, Message::successful_payment),
    // ... 46+ 種
}
```

### 11.3 DpHandlerDescription 自動追蹤 allowed_updates

每個 filter 攜帶一個 `AllowedUpdate` 標記。Dispatcher build 時自動收集所有使用的 update 類型，傳給 getUpdates 的 `allowed_updates` 參數，減少不必要的網路傳輸。

### 11.4 Clawtex 實作建議

clawtex 目前只訂閱 `["message"]`：

```rust
// clawtex-core/src/telegram.rs L396-397
"{}?offset={}&timeout={}&allowed_updates=[\"message\"]",
```

應該擴展為：

```rust
// 增加 callback_query 支援（用於 inline keyboard 按鈕）
"{}?offset={}&timeout={}&allowed_updates=[\"message\",\"callback_query\"]",
```

---

## 12. 錯誤處理架構

### 12.1 ErrorHandler Trait

```rust
// crates/teloxide/src/error_handlers.rs L10-13
pub trait ErrorHandler<E> {
    fn handle_error(self: Arc<Self>, error: E) -> BoxFuture<'static, ()>;
}

// 閉包自動實作
impl<E, F, Fut> ErrorHandler<E> for F
where
    F: Fn(E) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send,
{
    fn handle_error(self: Arc<Self>, error: E) -> BoxFuture<'static, ()> {
        Box::pin(async move { self(error).await })
    }
}
```

### 12.2 三種內建 ErrorHandler

| 型別 | 行為 |
|------|------|
| `LoggingErrorHandler` | `log::error!("{text}: {:?}", error)` |
| `IgnoringErrorHandler` | 靜默忽略所有錯誤 |
| `IgnoringErrorHandlerSafe` | 僅處理 `Infallible`（永遠不會觸發） |

### 12.3 OnError 擴充 trait

```rust
// 讓 Result 可以直接呼叫 error handler
pub trait OnError<E> {
    fn on_error<'a, Eh>(self, eh: Arc<Eh>) -> BoxFuture<'a, ()>;
    fn log_on_error<'a>(self) -> BoxFuture<'a, ()>;
}

// 使用範例
let result: Result<(), _> = bot.send_message(chat_id, "text").await;
result.log_on_error().await;
```

### 12.4 Clawtex 實作建議

clawtex 目前用 `anyhow::Result` 處理所有錯誤，缺少結構化的 Telegram API 錯誤處理：

```rust
/// 結構化 Telegram API 錯誤
#[derive(Debug)]
pub enum TelegramApiError {
    BotBlocked,
    ChatNotFound,
    MessageNotModified,
    RetryAfter(u64),
    TooManyRequests,
    MessageTooLong,
    InvalidToken,
    NetworkError(reqwest::Error),
    Unknown(String),
}

impl TelegramApiError {
    pub fn from_response(status: u16, body: &str) -> Self {
        if body.contains("bot was blocked") { return Self::BotBlocked; }
        if body.contains("chat not found") { return Self::ChatNotFound; }
        if body.contains("message is not modified") { return Self::MessageNotModified; }
        if body.contains("retry after") {
            if let Some(secs) = body.split("retry after ")
                .nth(1).and_then(|s| s.split('"').next())
                .and_then(|s| s.trim().parse().ok())
            {
                return Self::RetryAfter(secs);
            }
        }
        if body.contains("Too Many Requests") { return Self::TooManyRequests; }
        Self::Unknown(format!("[{}] {}", status, body))
    }
}
```

---

## 13. ShutdownToken 與 StopToken 雙層停機

### 13.1 StopToken/StopFlag（Update Listener 層）

```rust
// crates/teloxide/src/stop.rs L11-58
pub fn mk_stop_token() -> (StopToken, StopFlag) {
    let (handle, reg) = AbortHandle::new_pair();
    (StopToken(handle), StopFlag(Abortable::new(pending(), reg)))
}

// StopToken: 觸發停機
pub struct StopToken(AbortHandle);
impl StopToken {
    pub fn stop(&self) { self.0.abort() }
}

// StopFlag: 等待停機信號（可作為 Future 使用）
pub struct StopFlag(Abortable<Pending<Infallible>>);
impl Future for StopFlag {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<()> {
        self.project().0.poll(cx).map(|_| ())
    }
}
```

**巧妙設計**：`Abortable<Pending<Infallible>>` — 包裝一個永不完成的 future，當 AbortHandle 被 abort 時，Abortable 立即完成。這比使用 `tokio::sync::Notify` 更輕量。

### 13.2 ShutdownToken（Dispatcher 層）

```rust
// crates/teloxide/src/utils/shutdown_token.rs L16-89

#[repr(u8)]
enum ShutdownState {
    Running,       // 正在運行
    ShuttingDown,  // 正在停機
    Idle,          // 閒置
}

pub struct ShutdownToken {
    dispatcher_state: Arc<DispatcherState>,  // AtomicU8 + Notify
    shutdown_notify_back: Arc<Notify>,       // 停機完成通知
}

impl ShutdownToken {
    pub fn shutdown(&self) -> Result<impl Future<Output = ()>, IdleShutdownError> {
        // CAS: Running → ShuttingDown
        match shutdown_inner(&self.dispatcher_state) {
            Ok(()) | Err(Ok(AlreadyShuttingDown)) => Ok(async move {
                self.shutdown_notify_back.notified().await
            }),
            Err(Err(err)) => Err(err),  // Idle 狀態無法停機
        }
    }
}
```

**三態狀態機**：

```
Idle ──start_dispatching()──► Running
Running ──shutdown()──► ShuttingDown
ShuttingDown ──done()──► Idle
```

### 13.3 雙層停機流程

```
1. 外部呼叫 shutdown_token.shutdown()
2. ShutdownToken: Running → ShuttingDown
3. Dispatcher::start_listening 偵測到 ShuttingDown
4. 呼叫 stop_token.stop() → StopFlag 完成
5. PollingStream 進入 stopping 模式
6. 發送最後一個 getUpdates(offset, limit=1, timeout=0)
7. 等待所有 Worker 完成
8. ShutdownToken: done() → Idle
9. 通知 shutdown_notify_back
```

### 13.4 Clawtex 實作建議

clawtex 目前使用 `/estop` 指令觸發 E-Stop，但缺少優雅停機。建議增加：

```rust
/// 在 TelegramChannel 中加入停機支援
pub struct TelegramChannel {
    // ... 現有欄位 ...
    shutdown: Arc<AtomicBool>,
}

impl TelegramChannel {
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

// 在 listen() 迴圈中檢查
async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
    loop {
        if self.shutdown.load(Ordering::Relaxed) {
            // Graceful: 確認 offset
            let url = format!(
                "{}?offset={}&timeout=0&limit=1",
                self.api_url("getUpdates"),
                *self.offset.read().await
            );
            let _ = self.client.get(&url).send().await;
            info!("Telegram polling stopped gracefully");
            return Ok(());
        }
        // ... 正常 polling ...
    }
}
```

---

## 14. 與 clawtex telegram.rs 逐行對比

### 14.1 結構體對比

| 特性 | clawtex `TelegramChannel` | teloxide `Bot` + `Dispatcher` |
|------|--------------------------|-------------------------------|
| **Token 儲存** | `String`（clone O(n)） | `Arc<str>`（clone O(1)） |
| **HTTP Client** | `reqwest::Client` | `reqwest::Client` |
| **API URL** | 硬編碼常數 | `Arc<Url>`（可自訂） |
| **Update offset** | `Arc<RwLock<i64>>` | PollingStream 內部 `i32` |
| **使用者過濾** | `Vec<String>` allowlist | 無（在 handler 層處理） |
| **多 Chat 處理** | 串行 | Worker 並行（按 ChatId 分組） |
| **速率限制** | 無 | Throttle 適配器 |
| **指令解析** | 無 | BotCommands derive 巨集 |
| **對話管理** | 無 | Dialogue + Storage |
| **錯誤處理** | `anyhow::Result` | 結構化 `RequestError` + `ApiError` |
| **退避策略** | 固定 5 秒 | 指數退避 1s→64s |
| **優雅停機** | 無 | StopToken + ShutdownToken |
| **串流回應** | 有（send_streaming） | 無（teloxide 不處理 LLM 串流） |
| **Typing 指示器** | 有（keep_typing） | 需自行實作 |
| **訊息分割** | 有（4096 字元） | 需自行實作 |

### 14.2 API 呼叫方式對比

**clawtex（手動 JSON 建構）**：
```rust
// clawtex-core/src/telegram.rs L344-357
let body = json!({
    "chat_id": chat_id,
    "text": format!("{}{}", chunk, suffix),
    "parse_mode": "Markdown",
    "disable_web_page_preview": true
});

let resp = self.client
    .post(&self.api_url("sendMessage"))
    .json(&body)
    .send()
    .await?;
```

**teloxide（型別安全 API）**：
```rust
// teloxide 等效
bot.send_message(chat_id, format!("{}{}", chunk, suffix))
    .parse_mode(ParseMode::MarkdownV2)
    .disable_web_page_preview(true)
    .await?;
```

### 14.3 錯誤處理對比

**clawtex（字串比對）**：
```rust
// clawtex-core/src/telegram.rs L360-377
if !status.is_success() {
    let err_text = resp.text().await.unwrap_or_default();
    if err_text.contains("can't parse entities") {
        // Markdown 失敗，重試純文字
        debug!("Markdown parse failed, retrying as plain text");
        let plain_body = json!({ /* ... */ });
        self.client.post(&self.api_url("sendMessage"))
            .json(&plain_body).send().await?;
    } else {
        error!("sendMessage failed ({}): {}", status, err_text);
    }
}
```

**teloxide（結構化錯誤）**：
```rust
// teloxide 等效
match bot.send_message(chat_id, text)
    .parse_mode(ParseMode::MarkdownV2)
    .await
{
    Ok(_) => {},
    Err(RequestError::Api(ApiError::CantParseEntities)) => {
        // 型別安全的錯誤匹配
        bot.send_message(chat_id, text).await?;
    }
    Err(RequestError::RetryAfter(secs)) => {
        tokio::time::sleep(secs.duration()).await;
    }
    Err(e) => return Err(e.into()),
}
```

### 14.4 Polling Loop 對比

**clawtex（固定重試）**：
```rust
// clawtex-core/src/telegram.rs L402-418
let resp = match self.client.get(&url).send().await {
    Ok(r) => r,
    Err(e) => {
        warn!("getUpdates failed: {}, retrying in 5s", e);
        tokio::time::sleep(Duration::from_secs(5)).await;  // 固定 5s
        continue;
    }
};
```

**teloxide（指數退避 + RetryAfter）**：
```rust
// crates/teloxide/src/update_listeners/polling.rs L440-465
Err(err) => {
    let delay = match err.retry_after() {
        Some(seconds) => {
            *this.error_count = 0;
            seconds.duration()  // Telegram 指定的延遲
        }
        None => {
            let delay = (this.polling.backoff_strategy)(*this.error_count);
            *this.error_count = this.error_count.saturating_add(1);
            delay  // 1s, 2s, 4s, 8s, ..., 64s
        }
    };
    this.eepy.set(Some(sleep(delay)));
    return Ready(Some(Err(err)));
}
```

### 14.5 clawtex 獨有優勢

teloxide 沒有但 clawtex 有的功能：

1. **串流回應編輯**（`send_streaming`）：使用 editMessageText 漸進更新，含遊標指示器 `▌`
2. **Typing 保持**（`keep_typing`）：每 4 秒發送 typing action，使用 RAII guard
3. **智慧訊息分割**（`chunk_message`）：在換行符附近分割，避免截斷文字
4. **使用者白名單**（`is_user_allowed`）：支援 user ID、username、`@username`、`*` 萬用字元
5. **Markdown fallback**：Markdown 解析失敗時自動重試純文字

這些功能在遷移到 teloxide 時應該保留。

---

## 15. 效能分析

### 15.1 記憶體效能

| 元件 | clawtex | teloxide | 差異 |
|------|---------|----------|------|
| Bot token 儲存 | `String` (~48B + len) | `Arc<str>` (8B 共享) | teloxide 在多 clone 時更優 |
| Update 解析 | 自訂簡化結構 | 完整 Telegram 型別 | clawtex 更小，teloxide 更完整 |
| Worker per chat | 無 | mpsc channel(64) | teloxide 每 chat ~2KB |
| Throttle 歷史 | 無 | VecDeque + 2 HashMap | teloxide 按請求量線性成長 |

### 15.2 CPU 效能

| 操作 | clawtex | teloxide |
|------|---------|----------|
| API 呼叫 | 手動 JSON 建構（快但易錯） | Payload 序列化（型別安全但稍慢） |
| Update 路由 | 串行 if-else | dptree handler chain（函式指標跳轉） |
| 速率限制 | 無（可能被封鎖） | 250ms 間隔 worker 計算 |
| 指令解析 | 字串 starts_with | 巨集生成的 match 分支 |

### 15.3 併發效能

| 場景 | clawtex | teloxide |
|------|---------|----------|
| 10 chat 同時活躍 | 串行處理，延遲 10x | 10 worker 並行，延遲 1x |
| 100 update/s burst | 可能觸發 RetryAfter | Throttle 自動排隊，不超限 |
| 網路斷線恢復 | 固定 5s 後重連 | 指數退避，最大 64s |

---

## 16. Gap 對比總表

| # | 功能 | clawtex 現狀 | teloxide 參考 | 優先級 | 預估工時 |
|---|------|-------------|--------------|--------|---------|
| 1 | 速率限制 | 無 | Throttle 適配器 | **P0** | 2h |
| 2 | 指數退避 | 固定 5s | 1s→64s 退避 | **P0** | 30min |
| 3 | RetryAfter 處理 | 無 | 自動解析+等待 | **P0** | 1h |
| 4 | 多 Chat 並行 | 串行 | Worker 分組 | **P1** | 3h |
| 5 | 結構化錯誤 | anyhow | RequestError/ApiError | **P1** | 2h |
| 6 | 指令 enum | 字串比對 | BotCommands 巨集 | **P1** | 2h |
| 7 | 優雅停機 | 無 | ShutdownToken | **P1** | 1h |
| 8 | 對話狀態機 | 無 | Dialogue + Storage | **P2** | 4h |
| 9 | callback_query | 不支援 | UpdateFilterExt | **P2** | 2h |
| 10 | 型別安全 API | 手動 JSON | Requester trait | **P3** | 8h |
| 11 | allowed_updates 自動偵測 | 手動硬編碼 | DpHandlerDescription | **P3** | 1h |
| 12 | Webhook 支援 | 無 | Axum 整合 | **P3** | 4h |

---

## 17. 遷移路線圖

### Phase 1：最小改動，最大收益（4 小時）

不引入任何新依賴，僅改善現有 `telegram.rs`：

1. **指數退避**（30 min）：替換固定 5s 為 `1 << min(count, 6)` 秒
2. **RetryAfter 解析**（1h）：從回應中解析 retry_after 參數
3. **簡化速率限制**（2h）：實作 Section 8.10 的 `TelegramThrottle`
4. **優雅停機**（30 min）：實作 Section 13.4 的 shutdown 旗標

### Phase 2：結構化改進（8 小時）

1. **指令 enum**（2h）：實作 Section 5.4 的 `ClawtexCommand`
2. **Worker 分組**（3h）：實作 Section 4.6 的 `ChatWorker`
3. **結構化錯誤**（2h）：實作 Section 12.4 的 `TelegramApiError`
4. **callback_query**（1h）：擴展 Update 處理支援 inline keyboard 回應

### Phase 3：進階功能（12 小時）

1. **對話狀態機**（4h）：實作 Section 6.6 的 `DialogueManager`
2. **引入 teloxide-core**（4h）：替換手動 API 呼叫為型別安全方法
3. **Webhook 支援**（4h）：在 daemon 的 Axum server 中加入 webhook endpoint

### Phase 4：完整遷移（16 小時）

1. **引入完整 teloxide**（8h）：使用 Dispatcher + handler chain
2. **重構現有 handler**（4h）：將 agent_runtime 邏輯轉為 teloxide handler
3. **持久化對話**（4h）：使用 SqliteStorage 存入 core.db

---

## 附錄：關鍵檔案索引

| 檔案 | 行數 | 說明 |
|------|------|------|
| `crates/teloxide/src/dispatching/dispatcher.rs` | 720 | Dispatcher 核心（Worker 模型、分發邏輯） |
| `crates/teloxide/src/dispatching/dialogue.rs` | 261 | Dialogue 狀態機（5 個操作 + enter） |
| `crates/teloxide/src/dispatching/dialogue/storage.rs` | 157 | Storage trait + 型別擦除 |
| `crates/teloxide/src/dispatching/filter_ext.rs` | 187 | 23 Update + 46 Message 過濾器巨集 |
| `crates/teloxide/src/dispatching/handler_ext.rs` | 296 | filter_command + enter_dialogue |
| `crates/teloxide/src/dispatching/distribution.rs` | 10 | DefaultKey(ChatId) 分發鍵 |
| `crates/teloxide/src/update_listeners/polling.rs` | 522 | PollingStream 狀態機 |
| `crates/teloxide/src/error_handlers.rs` | 213 | ErrorHandler trait + 三種實作 |
| `crates/teloxide/src/backoff.rs` | 33 | 指數退避策略 |
| `crates/teloxide/src/stop.rs` | 59 | StopToken/StopFlag（AbortHandle） |
| `crates/teloxide/src/utils/shutdown_token.rs` | 168 | ShutdownToken 三態狀態機 |
| `crates/teloxide-core/src/bot.rs` | 321 | Bot 結構體（JSON/multipart 雙軌） |
| `crates/teloxide-core/src/adaptors/throttle.rs` | 217 | Throttle 適配器主結構 |
| `crates/teloxide-core/src/adaptors/throttle/worker.rs` | 402 | Worker 限速演算法 |
| `crates/teloxide-core/src/adaptors/throttle/settings.rs` | 111 | Limits + Settings |
| `crates/teloxide-core/src/adaptors/throttle/request.rs` | 225 | ThrottlingRequest 流程 |
| `crates/teloxide-core/src/adaptors/throttle/request_lock.rs` | 46 | RequestLock/RequestWaiter |
| `clawtex-core/src/telegram.rs` | 562 | clawtex 現有 Telegram 實作 |

所有 teloxide 檔案路徑相對於 `LLM-Cluster-Project/references/teloxide/`。
