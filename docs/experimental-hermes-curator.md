# experimental-hermes-curator

**狀態（Status）：** 實驗性（experimental）。預設關閉（Default OFF）。
**Cargo feature（Cargo 功能旗標）：** `experimental-hermes-curator`（會一併引入 `serde_yaml`）。
**出貨（Shipped）：** 2026-05-15 週末衝刺（PRs #29 + #33）。

## 它的功能

單一 feature gate（功能開關）後面有兩個彼此協作的子系統：

1. **Curator（策展器，H1）** — 呼叫一個小型 Claude 模型（「haiku」），依照一份凍結的評分準則（rubric，評分量表）對某個
   `EvolveCheckpoint`（演化檢查點）的 transcript（對話記錄）打分（0–10），
   然後把這個裁決（verdict）持久化（persist）寫回該檢查點。已接線到
   `phantom evolve --judge`。

2. **Skill Document parser（技能文件解析器，H2）** — 針對 Hermes 風格的技能檔案（YAML frontmatter（前置設定區塊）+ Markdown 內文）提供帶型別（typed）的解析與可往返（round-trip）的序列化器（serializer），讓
   Curator 與未來的 agent（代理）可以讀取、修改、再重新輸出技能，
   而不會遺失結構。Schema（綱要）位於
   `docs/hermes-skill-schema.json`；範例位於
   `docs/hermes-skills/sample-skill.md`。

## 如何啟用

```toml
# Cargo.toml
[dependencies]
phantom-mesh = { path = "core", features = ["experimental-hermes-curator"] }
```

或透過 CLI：

```bash
cargo build --features experimental-hermes-curator
```

## 快速體驗

```rust,ignore
use phantom_mesh::hermes::{build_judge_user_prompt, parse_judge_reply, RUBRIC_VERSION};
use phantom_mesh::hermes::skill::{parse_str, serialize};
use phantom_mesh::evolve_checkpoint::EvolveCheckpoint;

// Curator: build the prompt the judge sees
let cp = EvolveCheckpoint::new("fix the lint", "check", "test-node");
let prompt = build_judge_user_prompt(&cp);
assert!(prompt.contains(RUBRIC_VERSION));

// Curator: parse what a judge replied
let (score, why) = parse_judge_reply(r#"{"score": 8, "rationale": "ok"}"#).unwrap();
assert_eq!(score, 8);

// Skill: parse + round-trip a skill document
let src = "---\nname: x\nversion: 0.1.0\ndescription: y\ntriggers: [t]\n---\nbody\n";
let doc = parse_str(src).unwrap();
let back = serialize(&doc).unwrap();
assert_eq!(parse_str(&back).unwrap(), doc);
```

## 執行範例

```bash
CARGO_TARGET_DIR=D:/tmp/hermes-docs-target \
  cargo run -p phantom-mesh \
    --example experimental_hermes_curator_example \
    --features experimental-hermes-curator
```

預期的最後一行：`experimental-hermes-curator OK`。結束碼（Exit code）0。

## 原始碼

- `core/src/hermes/curator.rs` — Curator、`build_judge_user_prompt`、`parse_judge_reply`、`verdict_from_parsed`、`RUBRIC_VERSION`、`DEFAULT_JUDGE_MODEL`。
- `core/src/hermes/skill.rs` — `SkillDocument`、`SkillFrontmatter`、`parse_str`、`serialize`、`SkillError`。
- `core/src/hermes/mod.rs` — re-exports（重新匯出）。

## 備註

- 裁判（judge）模型預設為 `claude-haiku-4-5-20251001`（便宜；我們是在評分，不是在生成）。
- `RUBRIC_VERSION = "h1-v1"`。只有在評分量表（scoring scale）發生實質改變時才遞增（bump）。
- Skill parser（技能解析器）同時接受 LF 與 CRLF 輸入，並把內文正規化（normalize）為剛好一個結尾換行字元，使往返（round-trip）結果保持穩定。

## V2：多裁判集成（multi-judge ensemble，T28）

`phantom evolve --judge --ensemble N`（N >= 2）會把同一個 `EvolveCheckpoint`
並行送給 N 個獨立的 LLM 裁判（透過 `tokio::task::JoinSet`），
以母體中位數（population median）彙整各裁決，並對結果分類：

- **unanimous（全體一致）** — 母體標準差（population stddev）== 0
- **consensus（共識）** — 母體標準差落在 (0, 2.0]
- **needs_human_review（需人工複審）** — 母體標準差 > 2.0，**或** 成功的裁判少於 2 個

彙整後的 `JudgeVerdict` 會存到檢查點的 `judge_ensemble`
（與 V1 的 `judge_score` 欄位並存 — V2 並不會取代 V1）。各裁判的個別
結果保留在 `judge_ensemble.individual[*]`，讓複審者可以精確看到
是哪個 provider（供應商）回傳了什麼。

### 裁判選用

集成（ensemble）會依下列順序從環境變數組裝，直到湊滿 N
個裁判為止：

1. `ANTHROPIC_API_KEY`   → Anthropic Messages API（V1 預設裁判模型）
2. `MISTRAL_API_KEY`     → mistral，經由 `api.mistral.ai`
3. `XAI_API_KEY`         → xai，經由 `api.x.ai`
4. `TOGETHER_API_KEY`    → together，經由 `api.together.xyz`
5. `FIREWORKS_API_KEY`   → fireworks，經由 `api.fireworks.ai/inference`

若只設定了 `ANTHROPIC_API_KEY` 且 `N > 1`，則會用重複的 Anthropic 實例
來填補（自我一致性模式（self-consistency mode）— 同金鑰、同模型、獨立
取樣（sampling））。若無法組出至少 2 個裁判，該命令會以
帶說明的訊息報錯結束。

### Schema 強制檢查

每個裁判的回覆都用 `parse_judge_reply_strict` 解析，它在 `{score, rationale}`
結構上套用 `serde(deny_unknown_fields)`。任何未知的
頂層欄位、缺漏的 rationale（理由），或非整數的 score（分數），都會導致該裁判
被排除在中位數之外（計入 `judges_attempted` 但不計入
`judges_succeeded`），並在
`EnsembleOutcome::per_judge_results` 紀錄中以 `JudgeError::Schema` 形式呈現。

### 建置旗標

V2 與 V1 同樣位於 `experimental-hermes-curator` cargo feature 後面。
預設建置不含任何 V2 程式碼。預設建置維持位元組相同（binary-identical），
差別僅在於新增的 `EvolveCheckpoint::judge_ensemble: Option<...>` 欄位
（serde 預設為 `None`，對既有的檢查點讀取者透明無感）。

### V2 原始碼

- `core/src/hermes/curator_ensemble.rs` — `JudgeProvider`、`AnthropicJudge`、`OpenAICompatJudge`、`EnsembleCurator`、`aggregate`、`median_score`、`population_stddev`。
- `core/src/hermes/curator.rs` — `parse_judge_reply_strict`（與 V1 的 `parse_judge_reply` 並存的新增項）。
- `core/src/evolve_checkpoint.rs` — `AgreementClass`、`EnsembleVerdict`、`judge_ensemble` 欄位、`record_ensemble_verdict`。
