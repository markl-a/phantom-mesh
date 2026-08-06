# experimental-remote-control-telegram

**狀態：** experimental（實驗性）— 等待 PR #28（`wt/remote-control-telegram`）合併。
**Cargo feature（功能旗標）：** `experimental-remote-control-telegram`（會拉入 `teloxide`）。
**程式碼狀態：** 模組檔 `core/src/remote_control/telegram.rs` 存在於
`wt/remote-control-telegram` 分支，但尚未進入 `main`。`core/Cargo.toml` 中的
feature（功能）那一行目前是被註解掉的。

## 它的功能（PR #28 合併之後）

遠端控制（remote control）的 Telegram channel adapter（頻道轉接器）。實作了：

- 透過 [teloxide](https://docs.rs/teloxide) 的 long-polling（長輪詢）迴圈。
- 啟動時做 bot 身分探測（`getMe`），讓錯誤的 token（權杖）能立即明顯地失敗。
- Allowlist（白名單）門檻控管 — 空白名單 = 全部允許；有內容 = 嚴格控管。
- 通用錯誤轉譯 — 內部 dispatcher（派發器）的錯誤會被轉譯成
  面向使用者的「internal error」回覆，因此 stack trace（堆疊追蹤）絕不會外洩。
- 多位元組安全的訊息切塊 — 在不切斷 UTF-8 字元的前提下，
  遵守 Telegram 的 4096 位元組上限。
- Token 遮蔽 — `Debug` 實作會將 `bot_token` 印成 `<redacted>`，
  錯誤日誌中任何 token 子字串也會被剝除。

## 公開 API 介面（預覽）

```rust,ignore
pub trait RemoteTelegramDispatcher: Send + Sync {
    async fn dispatch(&self, user_text: String) -> Result<String, String>;
}

pub struct EchoDispatcher;

pub struct RemoteTelegramConfig {
    pub bot_token: String,           // never logged
    pub allowed_user_ids: Vec<i64>,  // empty = allow all
}

pub struct RemoteTelegramBot { /* opaque */ }

impl RemoteTelegramBot {
    pub fn new(config: RemoteTelegramConfig, dispatcher: Arc<dyn RemoteTelegramDispatcher>) -> Self;
    pub fn config(&self) -> &RemoteTelegramConfig;
    pub async fn handle_text(&self, user_id: i64, text: String) -> Option<String>;
}

pub async fn run_round_trip(bot: Arc<RemoteTelegramBot>) -> Result<(), String>;
```

## 如何啟用（PR #28 合併之後）

1. 取消註解 `core/Cargo.toml` 的第 34 行：
   ```toml
   experimental-remote-control-telegram = ["dep:teloxide"]
   ```
2. 設定 bot token（權杖）：`spectyn keys set telegram_bot <token>`。此 CLI 會把
   該值寫入環境變數 `TELEGRAM_BOT_API_KEY`。
3. 建置：
   ```bash
   cargo build --features experimental-remote-control-telegram
   ```

## 快速試味（離線 — 不發出任何網路呼叫）

```rust,ignore
use std::sync::Arc;
use spectyn_mesh::remote_control::telegram::{
    EchoDispatcher, RemoteTelegramBot, RemoteTelegramConfig,
};

let cfg = RemoteTelegramConfig {
    bot_token: "fake-token".into(),
    allowed_user_ids: vec![42],
};
assert!(cfg.is_user_allowed(42));
assert!(!cfg.is_user_allowed(7));

// Debug never leaks the token
let s = format!("{:?}", cfg);
assert!(s.contains("<redacted>"));
assert!(!s.contains("fake-token"));

// handle_text routes through the dispatcher; allowlist is enforced.
let bot = RemoteTelegramBot::new(cfg, Arc::new(EchoDispatcher));
let r = bot.handle_text(42, "ping".into()).await;
assert_eq!(r, Some("spectyn-mesh echo: ping".into()));
```

## 執行範例（PR #28 合併之後）

```bash
CARGO_TARGET_DIR=D:/tmp/skillbank-docs-target \
  cargo run -p spectyn-mesh \
    --example experimental_remote_control_telegram_example \
    --features experimental-remote-control-telegram
```

預期的最後一行：`experimental-remote-control-telegram OK`。離開碼（exit code）為 0。

（此範例並不會聯絡 api.telegram.org — 它只演練 API 的離線部分：
config 的 debug 遮蔽、allowlist（白名單）門檻控管，以及
`handle_text` dispatcher（派發器）路徑。）

## 原始碼（在 `wt/remote-control-telegram` 分支上，直到 PR #28 合併為止）

- `core/src/remote_control/telegram.rs`

## 備註

- `run_round_trip` 是唯一真正會聯絡 Telegram 的函式。
  其餘所有介面都可在離線狀態下演練。
- 另請參閱：[experimental-remote-control.md](experimental-remote-control.md)，了解
  共用同一傘式架構的 channel-trait（頻道 trait）以及 WhatsApp/Slack stub（樁程式）。

## 為何本文件引用尚未合併的 PR

這份三件式文件是在 T12 週末文件整理工作的一環中撰寫的。Telegram 程式碼
位於 PR #28（`wt/remote-control-telegram`），在撰寫這些文件時尚未合併。範例檔
已被提交，但尚未在 `core/Cargo.toml` 的 `[[example]]` 區塊中註冊 —
Cargo 會拒絕，因為該 feature（功能）旗標被註解掉了。在 PR #28 落地並
取消註解 `experimental-remote-control-telegram = ["dep:teloxide"]` 之後，
把下方的 `[[example]]` 區塊附加到 `core/Cargo.toml`，即可讓此範例可執行：

```toml
[[example]]
name = "experimental_remote_control_telegram_example"
path = "examples/experimental_remote_control_telegram_example.rs"
required-features = ["experimental-remote-control-telegram"]
```
