# experimental-hermes-providers

**狀態：** experimental（實驗性）。預設為關閉（Default OFF）。
**Cargo feature（Cargo 功能旗標）：** `experimental-hermes-providers`（不引入新的 crate（套件）相依）。
**發佈時間：** 2026-05-15 週末衝刺（PR #25）。

## 功能說明

新增四個與 OpenAI 相容的 chat-completions（對話補全）provider（供應商）adapter（轉接器）：

| `provider_type` | 顯示名稱          | 預設 base URL（基底網址）                  | 預設模型                                        | 環境變數           |
|-----------------|-------------------|-------------------------------------------|------------------------------------------------|--------------------|
| `mistral`       | Mistral AI        | `https://api.mistral.ai`                  | `mistral-small-latest`                         | `MISTRAL_API_KEY`  |
| `xai`           | xAI Grok          | `https://api.x.ai`                        | `grok-4`                                       | `XAI_API_KEY`      |
| `together`      | Together AI       | `https://api.together.xyz`                | `meta-llama/Llama-3.3-70B-Instruct-Turbo`      | `TOGETHER_API_KEY` |
| `fireworks`     | Fireworks AI      | `https://api.fireworks.ai/inference`      | `accounts/fireworks/models/llama-v3p3-70b-instruct` | `FIREWORKS_API_KEY` |

傳輸格式（wire format）為與 OpenAI 相容的 `/v1/chat/completions` —— Fireworks
是特例（`/inference/v1/...`）；它的 `DEFAULT_BASE_URL` 已將
`/inference` 前綴內建寫死。

每個模組（module）對外提供：
- `PROVIDER_ID: &'static str` —— 供 `ProviderEntry.provider_type` 使用的穩定 id（識別碼）。
- `DEFAULT_BASE_URL: &'static str`、`DEFAULT_MODEL: &'static str`。
- `streaming_url(&ProviderEntry) -> String` —— endpoint（端點）URL 建構器。
- `auth_header(api_key) -> Result<(HeaderName, HeaderValue), _>` —— bearer（持有者）標頭
  建構器，會拒絕金鑰中的 CRLF（換行字元，header-injection 標頭注入防護）。
- `INFO: ProviderInfo` —— 靜態 metadata（中繼資料）。

## 如何啟用

```toml
phantom-mesh = { path = "core", features = ["experimental-hermes-providers"] }
```

```toml
# agents.toml
[providers.mistral]
type = "mistral"
api_key_env = "MISTRAL_API_KEY"
default_model = "mistral-small-latest"
```

## 快速體驗

```rust,ignore
use phantom_mesh::providers::{mistral, xai, together, fireworks};
use phantom_mesh::config::ProviderEntry;

let p = ProviderEntry { provider_type: mistral::PROVIDER_ID.into(), ..Default::default() };
assert_eq!(mistral::streaming_url(&p), "https://api.mistral.ai/v1/chat/completions");

let (name, value) = mistral::auth_header("sk-abc")?;
assert_eq!(value.to_str()?, "Bearer sk-abc");
```

## 執行範例

```bash
CARGO_TARGET_DIR=D:/tmp/hermes-docs-target \
  cargo run -p phantom-mesh \
    --example experimental_hermes_providers_example \
    --features experimental-hermes-providers
```

預期的最後一行：`experimental-hermes-providers OK`。離開碼（exit code）為 0。

## 原始碼

- `core/src/providers/mistral.rs`
- `core/src/providers/xai.rs`
- `core/src/providers/together.rs`
- `core/src/providers/fireworks.rs`
- `core/src/providers/mod.rs` —— `display_name()` 與 URL-fallback（網址後備）偵測。

## 備註

- 四個 adapter（轉接器）皆重用 `core/src/streaming.rs` 中的 OpenAI streaming（串流）程式碼路徑；此 feature flag（功能旗標）只會引入 metadata（中繼資料）與 auth（驗證）輔助函式。
- `auth_header()` 會拒絕含有 `\r` 或 `\n` 的金鑰，因此遭洩漏且含換行字元的金鑰無法偷夾帶（smuggle）標頭至下游。
