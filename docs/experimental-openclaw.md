# experimental-openclaw

**狀態（Status）:** experimental（實驗性）。預設關閉（Default OFF）。
**Cargo 功能總開關（feature umbrella）:** `experimental-openclaw` (= `experimental-openclaw-whatsapp` + `experimental-openclaw-slack`)。
**發佈日（Shipped）:** 2026-05-15 週末衝刺（PR #26）。

## 它的功用

OpenClaw 是你叢集（cluster）的**遠端遙控（Remote Control）**介面
（[BIG-GOAL.md §P3](superpowers/BIG-GOAL.md)）：每一個你
已經在用的聊天頻道（channel）—— Telegram、Slack、WhatsApp —— 都變成一個遙控器。一則聊天
訊息 = 一條叢集指令；機器人（bot）的回覆是代理（agent）對
*正在工作的叢集* 所做出的回應，而非一段獨立的閒聊。框架由
[`docs/superpowers/specs/_current/E004-openclaw-remote-control.md`](superpowers/specs/_current/E004-openclaw-remote-control.md)
這份史詩（epic）負責定義。

此功能提供 `Channel` trait（特徵）外加兩個僅供編譯的樁（stub，佔位實作）：

- **`WhatsappStub`** —— `send_message` 回傳
  `ChannelError::NotImplemented`。真正的實作延後（Meta
  Business 驗證流程尚未完成）。
- **`SlackStub`** —— 形狀相同，同樣延後（需要 OAuth（開放授權）+ Events API）。

第三個頻道 —— **Telegram** —— 有自己的旗標（flag）`experimental-openclaw-telegram`，
其文件位於 [experimental-openclaw-telegram.md](experimental-openclaw-telegram.md)。
該旗標目前在 `core/Cargo.toml` 中被註解掉，直到 PR #28 合併為止。

## Public API

```rust,ignore
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    fn is_user_allowed(&self, user_id: i64) -> bool;
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), ChannelError>;
}

pub enum ChannelError {
    Transport { channel: &'static str, message: String },
    Upstream  { channel: &'static str, message: String },
    NotImplemented { channel: &'static str, reason: &'static str },
}
```

## 如何啟用

```toml
phantom-mesh = { path = "core", features = ["experimental-openclaw"] }
```

或只挑選單一頻道：

```toml
phantom-mesh = { path = "core", features = ["experimental-openclaw-whatsapp"] }
```

## 快速嚐鮮

```rust,ignore
use phantom_mesh::openclaw::{Channel, ChannelError};
use phantom_mesh::openclaw::whatsapp::WhatsappStub;
use phantom_mesh::openclaw::slack::SlackStub;

let wa = WhatsappStub::with_allowed_users(vec![42]);
assert!(wa.is_user_allowed(42));
assert!(!wa.is_user_allowed(7));
let err = wa.send_message(42, "hello").await.unwrap_err();
assert!(matches!(err, ChannelError::NotImplemented { channel: "whatsapp", .. }));
```

## 執行範例

```bash
CARGO_TARGET_DIR=D:/tmp/hermes-docs-target \
  cargo run -p phantom-mesh \
    --example experimental_openclaw_example \
    --features experimental-openclaw
```

預期最後一行：`experimental-openclaw OK`。退出碼（exit code）為 0。

## 原始碼

- `core/src/openclaw/channel_trait.rs` —— `Channel`、`ChannelError`。
- `core/src/openclaw/whatsapp.rs` —— `WhatsappStub`。
- `core/src/openclaw/slack.rs` —— `SlackStub`。
- `core/src/openclaw/mod.rs` —— 重新匯出（re-exports）。

## 為什麼用樁（stub）

Meta Business（WhatsApp Cloud）與 Slack OAuth 兩者都需要長達數日的
人工驗證流程，無法塞進單一個週末衝刺。這些樁存在的目的，是讓
派發器（dispatcher）+ 路由（routing）接線今天就能被演練；對
`send_message` 的呼叫會以 `NotImplemented` 大聲失敗，讓被錯誤路由的流量
絕不會悄無聲息地消失。
