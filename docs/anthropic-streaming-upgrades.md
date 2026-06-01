# Anthropic 串流升級（F1）

**狀態：** GA（正式上線）。永遠啟用。無 feature flag（功能旗標）。
**上線日期：** 2026-05-15 週末推送（PR #31）。
**程式碼位置：** `core/src/streaming.rs`（搜尋 `// [F1]` 標記）。

## 變更內容

正式環境的 Anthropic 串流程式路徑現在會自動做三件事：

1. **自動 prompt caching（提示快取，GA 2026-02-19）。** 系統提示會被渲染成
   單一文字區塊，並附帶 `cache_control: {type: "ephemeral"}`，因此
   整個系統前綴（system prefix）變成一個可快取的前綴，呼叫端無需任何
   額外處理。早於 prompt caching 的舊版 Claude 模型會優雅地忽略這個
   欄位（不會產生 400 錯誤）。

2. **工具定義快取斷點（cache breakpoint）。** 當有工具存在時，最後一個工具
   定義上也會放置一個 `cache_control: {type: "ephemeral"}` 斷點，因此
   工具加上系統提示會一起被快取（Anthropic 的渲染順序：tools → system）。

3. **`thinking.display = "omitted"`（Anthropic 2026-03-16），適用於 Opus 4.7 以上。**
   對於 Claude Opus 4.7 以及更新的模型，請求主體（request body）現在會包含
   `{"thinking": {"type": "adaptive", "display": "omitted"}}`。模型
   仍然會產生我們可以持久化保存的內部 thinking signature（思考簽章），但不會
   把（通常很大的）思考文字串流回傳給使用者。較舊的
   Claude 模型遇到 `display` 會回傳 400，因此閘控（gate）很嚴格：模型
   ID 的小寫形式必須以 `claude-opus-4-7`、`claude-opus-4-8`、
   `claude-opus-4-9` 或 `claude-opus-5` 開頭。

外加一項清理性變更：

4. **移除舊版 `anthropic-beta: tool-use-2024-04-04` 標頭。** 工具使用（tool use）
   已是 GA。傳送這個舊版 beta 標頭開始在某些
   模型 ID 上造成 400（例如 Haiku 4.5）。現在已不再傳送。

## F1 現在送出的 wire shape（線上資料格式）

對於一個帶有系統提示加上一個工具定義、目標為 `claude-opus-4-7-20260315` 的請求：

```json
{
  "model": "claude-opus-4-7-20260315",
  "max_tokens": 8192,
  "stream": true,
  "messages": [...],
  "system": [
    {"type": "text", "text": "<your system prompt>", "cache_control": {"type": "ephemeral"}}
  ],
  "tools": [
    {"name": "...", "description": "...", "input_schema": {...},
     "cache_control": {"type": "ephemeral"}}
  ],
  "thinking": {"type": "adaptive", "display": "omitted"}
}
```

對於較舊的模型（Sonnet 4.6、Opus 4.6、Haiku 4.5 等），主體完全相同，
唯一差別是 `thinking` 鍵會被完全省略。

## 如何驗證你的建置已啟用 F1

```bash
CARGO_TARGET_DIR=D:/tmp/hermes-docs-target \
  cargo run -p phantom-mesh --example anthropic_streaming_upgrades_example
```

預期的最後一行：`anthropic-streaming-upgrades OK`。離開碼（exit code）為 0。

這個範例會啟動一個本地 TCP mock（模擬伺服器），送出一個與
F1 的 `build_request_body` 為 Opus 4.7 請求所產生內容完全相符的主體，並斷言
線上送出的位元組包含 `"cache_control"`（兩次 — 系統加工具）以及
`"thinking"`（一次 — display=omitted 區塊）。

## 來源

- `core/src/streaming.rs` — 請求主體建構（搜尋 `// [F1]` 標記）。
- `core/src/streaming.rs` — `model_supports_thinking_display_omitted()`
  （私有輔助函式，模型 ID 前綴閘控）。
- `core/src/streaming.rs` — 涵蓋兩種注入的回歸測試（regression tests）。

## 備註

- `build_request_body` 沒有公開 API — 它是一個模組私有的輔助函式。
  透過 `stream_agent` / `stream_agent_full` 走正式環境路徑的呼叫端
  會自動免費取得 F1。
- 舊版 `anthropic-beta: tool-use-2024-04-04` 標頭在同一個 PR 中被移除。
  如果你的供應商設定（provider config）設定了自訂標頭，請確保你沒有
  重新引入它。
