# experimental-tools

**狀態（Status）：** 實驗性（experimental）。預設關閉（Default OFF）。
**Cargo feature（Cargo 功能旗標）：** `experimental-tools`（會引入 `chrono` 與 `url`）。
**發布（Shipped）：** 2026-05-15 週末推進（PR #27）。

## 功能說明

一份包含 10 個確定性（deterministic，相同輸入必得相同輸出）、無副作用（side-effect-free）工具的目錄（catalog），讓技能型
代理（agent）可以在不需 shell-out（外呼系統指令）或網路存取的情況下呼叫。每個工具都是
一個 `SkillTool` trait（特徵）實作；每個工具都會輸出一份 OpenAI 風格的
`{"type":"function","function":{...}}` schema（綱要），接受一個 JSON `Value`
引數，並回傳一個 JSON `Value` 結果（或一個結構化的 `ToolError`）。

| # | 工具名稱                  | 用途                                                |
|---|--------------------------|----------------------------------------------------|
| 1 | `skill_calculator`      | 安全的算術運算（`+ - * / %`、括號）                  |
| 2 | `skill_datetime`        | UTC 當前時間 / RFC3339 解析 / 以秒為單位的時間差     |
| 3 | `skill_regex_extract`   | 正規表示式（regex）擷取捕獲群組                       |
| 4 | `skill_json_query`      | JSON 路徑查找                                       |
| 5 | `skill_text_stats`      | 字詞 / 行數 / 字元數統計                             |
| 6 | `skill_text_summarize`  | 樸素的擷取式摘要（取首句 + 尾句）                     |
| 7 | `skill_unit_convert`    | 長度 / 質量 / 溫度換算                               |
| 8 | `skill_base64_codec`    | base64 編碼 / 解碼                                  |
| 9 | `skill_url_parse`       | 將 URL 拆解為各組成部分                              |
|10 | `skill_uuid_gen`        | 產生 UUID v4                                        |

## 如何啟用

```toml
spectyn-mesh = { path = "core", features = ["experimental-tools"] }
```

## 快速體驗

```rust,ignore
use spectyn_mesh::skillbank::tools::catalog;
use serde_json::json;

let cat = catalog();
assert_eq!(cat.len(), 10);

let calc = cat.iter().find(|t| t.name() == "skill_calculator").unwrap();
let out = calc.call(&json!({"expression": "2 + 3 * 4"})).await.unwrap();
assert_eq!(out["result"], 14.0);
```

## 執行範例

```bash
CARGO_TARGET_DIR=D:/tmp/skillbank-docs-target \
  cargo run -p spectyn-mesh \
    --example experimental_tools_example \
    --features experimental-tools
```

預期的最後一行：`experimental-tools OK`。結束代碼（exit code）為 0。

## 原始碼

- `core/src/skillbank/tools/mod.rs` — `SkillTool` trait、`ToolError`、`ToolResult`、`catalog()`。
- 每個工具各一個檔案：`core/src/skillbank/tools/{calculator,datetime,regex_extract,json_query,text_stats,text_summarize,unit_convert,base64_codec,url_parse,uuid_gen}.rs`。

## 概念來源歸屬

工具目錄的概念取自公開的開源 agent-tooling 文獻（MIT 授權）。並未
逐字複製任何程式碼；每個工具皆為全新的 Rust 實作。
