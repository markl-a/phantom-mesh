// SPEC-14 §7 — LLM provider wire types (request / response surface +
// per-provider metadata + 3-D routing + fallback chain that every
// LlmProvider impl in core/src/providers/ shares).
//
// Stage 3 (real impl — config-side live + 3 frontier providers + ping live;
// 8 secondary providers still TODO): the structural helpers
// `load_fallback_chain`, `filter_chain_by_class_latency`, `provider_alive`,
// `resolve_model_to_provider_type`, `compute_cost_usd` are backed by real
// `toml` + `std::fs` parsing against `~/.spectyn-mesh/agents.toml`. The
// three flagship `complete_*` callers (Groq / Anthropic / Gemini) are now
// real HTTP via async `reqwest` 0.12 (json + rustls-tls features already in
// core/Cargo.toml) wrapped in a small `block_on_async` helper so the
// outer sync `complete()` signature stays unchanged. `provider_ping` is
// likewise live (HEAD/GET on the resolved base URL). Now ALSO real HTTP:
// openai / ollama / cerebras / llamacpp (OpenAI-compat `/v1/chat/completions`)
// + cloudflare (Workers AI `/accounts/<id>/ai/run/<model>`, `result.response`
// envelope). The remaining 3 (opencode / claude / codex) keep their `Stage 4`
// markers — all CLI shell-outs driven via the cli_session substrate, not HTTP.
//
// 中文: 對應 SPEC-14 §7（資料模型）+ §9（API contracts）的 wire 介面。enum
// variant 一律 snake_case 對齊 OSS LLM 上游習慣；upstream API（Anthropic /
// OpenAI / Gemini）的 PascalCase / camelCase 轉換由各 provider adapter 負責。
// 設定面已是真實 toml 解析；Groq / Anthropic / Gemini / OpenAI / Ollama /
// Cerebras / Llamacpp / Cloudflare + `provider_ping` 皆為真實 reqwest HTTP；
// 剩 3 個維持 Stage 4（opencode/claude/codex = CLI shell-out 走 cli_session）。
//
// TODO Stage 4: real impl for the remaining 3 providers (opencode / claude_cli /
// codex_cli) — all CLI shell-outs via process spawn / cli_session;
// per-provider wire impl in `core/src/providers/<each>.rs`; map
// ProviderError to legacy `core::providers::traits::ProviderError`
// 6-variant catalog; resolve `api_key_ref` via SPEC-13 age vault
// (currently read from env var `SPECTYN_MESH_<UPPER>_API_KEY`).

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use ts_rs::TS;

// ─── §7.1 ProviderType — exhaustive 11+ provider enum ────────────────────────

/// Exhaustive list of LLM provider backends recognised by spectyn-mesh.
/// Each variant maps to one entry in `core/src/providers/` (some share
/// `openai_compat.rs` — see SPEC-14 §6.2). Adding a variant is a wire-break:
/// also update SPEC-14 §7.1 Part C 11-provider × 6-metadata table.
///
/// 中文: 認得的 11+ provider 列舉；新增必須同步改 SPEC-14 §7.1 Part C 對照表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Groq Cloud — Llama 3.1 / Whisper，commodity class，TTFT 極低（< 200 ms）。
    Groq,
    /// OpenAI — GPT-5.5 frontier，JSON mode + tool_calls wire。
    Openai,
    /// Anthropic — Claude Opus / Sonnet，XmlTags prompt + tool_use wire。
    Anthropic,
    /// Google Gemini 3.1 Pro — Qa prompt + functionCall wire。
    Gemini,
    /// Cerebras — Llama 3.1 wafer-scale inference，commodity。
    Cerebras,
    /// OpenCode local provider — free model via opencode.ai CLI。
    Opencode,
    /// Cloudflare Workers AI — edge inference，commodity class。
    Cloudflare,
    /// Ollama — 本機 OnDevice，desktop only（iOS / Android 不可用）。
    Ollama,
    /// Claude CLI — 走 `~/.claude/` shell，frontier + MCP tool wire。
    Claude,
    /// Codex CLI — OpenAI Codex shell adapter，desktop only。
    Codex,
    /// llama.cpp local runtime — 直接呼叫 `llama-server`，OnDevice。
    Llamacpp,
}

impl ProviderType {
    /// Stable lowercase slug used in `agents.toml` + `ProviderConfig.slug` +
    /// metric labels (`spectyn.llm.<slug>.calls`). Round-trips with
    /// `FromStr` — the snake_case serde rename matches this slug exactly.
    ///
    /// 中文: 穩定的 lowercase slug，跟 agents.toml + metric 標籤一致；與
    /// `FromStr` round-trip 對稱。
    pub const fn slug(self) -> &'static str {
        match self {
            ProviderType::Groq => "groq",
            ProviderType::Openai => "openai",
            ProviderType::Anthropic => "anthropic",
            ProviderType::Gemini => "gemini",
            ProviderType::Cerebras => "cerebras",
            ProviderType::Opencode => "opencode",
            ProviderType::Cloudflare => "cloudflare",
            ProviderType::Ollama => "ollama",
            ProviderType::Claude => "claude",
            ProviderType::Codex => "codex",
            ProviderType::Llamacpp => "llamacpp",
        }
    }
}

impl FromStr for ProviderType {
    type Err = ProviderError;

    /// Parse a provider slug into the typed enum. Case-insensitive on input
    /// (callers often get the slug from agents.toml where the user may have
    /// typed `Groq` or `GROQ`); always returns the canonical lowercase form
    /// via `ProviderType::slug()`.
    ///
    /// 中文: 把 slug 字串解析成型別。輸入大小寫不敏感；無對應則回
    /// `ProviderError::ModelNotFound`（沿用「找不到上游」語意）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "groq" => Ok(ProviderType::Groq),
            "openai" => Ok(ProviderType::Openai),
            "anthropic" => Ok(ProviderType::Anthropic),
            "gemini" => Ok(ProviderType::Gemini),
            "cerebras" => Ok(ProviderType::Cerebras),
            "opencode" => Ok(ProviderType::Opencode),
            "cloudflare" => Ok(ProviderType::Cloudflare),
            "ollama" => Ok(ProviderType::Ollama),
            "claude" => Ok(ProviderType::Claude),
            "codex" => Ok(ProviderType::Codex),
            "llamacpp" => Ok(ProviderType::Llamacpp),
            other => Err(ProviderError::ModelNotFound {
                detail: format!("unknown provider slug: {}", other),
            }),
        }
    }
}

// ─── §7.1 ProviderClass — frontier / commodity / on-device ───────────────────

/// Provider upstream class — drives routing priority + cost expectations.
/// `frontier` = expensive smart (Opus / GPT-5.5 / Gemini 3.1 Pro);
/// `commodity` = cheap workhorse (Llama 3.1 / DeepSeek);
/// `local` = on-device (ollama / llama.cpp / Apple Foundation Model, desktop only).
///
/// 中文: 上游類別（class）— 影響成本與 routing 優先序。`local` 對應 SPEC-14
/// §7 的 `on-device`，這裡用 `local` 是與 ProviderConfig 的命名習慣對齊；
/// mobile 平台會自動 filter 掉 `local`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "snake_case")]
pub enum ProviderClass {
    /// 前沿（frontier）— Opus / GPT-5.5 / Gemini 3.1 Pro。貴、強、留給 reasoning。
    Frontier,
    /// 通用（commodity）— Llama 3.1 / DeepSeek / Mistral。便宜、interactive 主力。
    Commodity,
    /// 在地（local / on-device）— ollama / llama.cpp。0 元、context 小、僅桌面。
    Local,
}

// ─── §7.1 LatencyClass — interactive / background ───────────────────────────

/// Request-side latency budget — caller hint to resolver.
/// `interactive` = chat / UI in-loop (p50 < 3s targets);
/// `background` = async / queued job (p50 < 30s OK).
///
/// 中文: caller 端標的延遲類別（latency class）。SPEC-14 §7 定義三類：
/// `interactive` / `background` / `reasoning`。`reasoning`（深思）對應無延遲
/// 上限的 chain-of-thought（思維鏈）任務 — resolver 會偏向 frontier（前沿）
/// provider，並可搭配 `ReasoningEffort`（深思強度）細部欄位。
///
/// RECONCILE NOTE (T-PROV-01): the `Reasoning` variant was DROPPED in a
/// parallel-module fork; re-introduced here to match SPEC-14 §7 (three
/// latency classes, not two). Routing-side consumption (filter + score) is
/// being reconnected — see `score()` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    /// p50 < 3s — chat 即時回應、UI in-loop。
    Interactive,
    /// p50 < 30s — async / queued job、batch 任務。
    Background,
    /// 無延遲上限（unbounded）— chain-of-thought（思維鏈）深思任務；resolver
    /// 偏向 frontier（前沿）provider。SPEC-14 §7 第三類。
    Reasoning,
}

// ─── §7.1 PromptStyle — 5-way prompt-construction branch (RECONCILE) ─────────
//
// RECONCILE NOTE (T-PROV-01, SPEC-14 §7): `PromptStyle`（提示風格）was DROPPED
// in a parallel-module fork. Re-introduced to restore the 5-way agent-layer
// `build_prompt_for()` branch (SPEC-14 §9.2 / G5). Without this enum the agent
// layer cannot emit provider-correct prompts (Claude XML tags vs GPT JSON-mode
// vs Gemini Q&A etc.), which the spec proved is NOT over-engineering
// (§17 alt-3: OpenAI-style prompt to Claude drops quality ~15-25%).

/// Prompt 結構偏好（prompt style，提示風格）— 對應 provider 端 build prompt
/// 的 5 種分支。agent layer 的 `build_prompt_for()` 依此選結構。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
pub enum PromptStyle {
    /// Claude — `<task>...</task> <context>...</context>` XML 標籤包裹。
    XmlTags,
    /// GPT-5.5 — strict JSON schema mode（嚴格 JSON 結構模式）。
    JsonMode,
    /// Gemini — `Q: ... A: ...` 問答對話風（Q&A）。
    Qa,
    /// Llama / Mistral — 1-3 個 example（範例）引導 few-shot。
    FewShot,
    /// 本機小 model（on-device）— 單一指令、無修飾。
    Simple,
}

// ─── §7.1 SystemPlacement — where the system prompt sits (RECONCILE) ─────────
//
// RECONCILE NOTE (T-PROV-01, SPEC-14 §7): re-introduced auxiliary enum dropped
// in the fork. Drives per-provider system-prompt placement at adapter time.

/// System prompt（系統提示）在 messages 陣列中的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
pub enum SystemPlacement {
    /// Anthropic — request body 的獨立 `system: "..."` 參數（separate param）。
    SeparateParam,
    /// OpenAI / 多數 provider — `messages[0].role = "system"`。
    RoleSystem,
    /// 本機 model（on-device）— 無 system role，內嵌進使用者 turn（輪）。
    EmbedInUserTurn,
}

/// Map a provider name (and optional model id) to its [`SystemPlacement`].
///
/// SPEC-14 §7.1: where the system prompt（系統提示）physically sits differs by
/// upstream family. Anthropic takes it as a separate `system:` request param
/// (`SeparateParam`); most chat providers (OpenAI / Groq / Gemini-chat) take it
/// as `messages[0].role = "system"` (`RoleSystem`, the common default); local /
/// on-device small models often have no system role at all, so it must be
/// prepended into the first user turn (`EmbedInUserTurn`).
///
/// Pure function — provider/model strings in, enum out. The provider name is
/// matched first, then the model id, then a sensible default (`RoleSystem`).
///
/// 中文: 把 provider 名（與選用 model id）對應到 system 提示的擺放方式。
pub fn system_placement_for_provider(provider_name: &str, model: Option<&str>) -> SystemPlacement {
    let p = provider_name.to_ascii_lowercase();
    let m = model.unwrap_or("").to_ascii_lowercase();

    // Anthropic / Claude → separate `system:` request param.
    if p.contains("anthropic") || p.contains("claude") || m.contains("claude") {
        return SystemPlacement::SeparateParam;
    }
    // On-device / local small runtimes → no system role; embed into user turn.
    if p.contains("ollama") || p.contains("llamacpp") || p.contains("llamafile") || p == "local" {
        return SystemPlacement::EmbedInUserTurn;
    }
    // OpenAI / Groq / Gemini-chat / unknown → messages[0].role = "system".
    SystemPlacement::RoleSystem
}

// ─── §7.1 ToolCallFormat — upstream tool-call wire → MCP shape (RECONCILE) ────
//
// RECONCILE NOTE (T-PROV-01, SPEC-14 §7 + P3.mcp): the MCP（Model Context
// Protocol，模型上下文協議）adapter classification was DROPPED in the fork.
// Re-introduced so each provider impl can normalise its native tool-call wire
// into the unified MCP-shape `ToolCall`. The actual `into_mcp_toolcalls()`
// conversion lives on the `LlmProvider` trait (core/src/providers/); this enum
// is the wire-side tag the resolver + adapter dispatch on.

/// Tool-call（工具呼叫）上游 wire 格式 — provider impl 統一轉成 MCP-shape。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
pub enum ToolCallFormat {
    /// OpenAI — `{ tool_calls: [{ function: { name, arguments }}] }`。
    OpenAiToolCalls,
    /// Anthropic — `content` 中的 `{ type: "tool_use", name, input }` block。
    AnthropicToolUse,
    /// Gemini — `{ functionCall: { name, args }}`。
    GeminiFunctionCall,
    /// 已是 JSON-RPC 2.0 MCP shape — pass-through（直通，免轉換）。
    Mcp,
}

// ─── §7.1 Modality — routing input #3 (RECONCILE) ────────────────────────────

/// Modality（媒介類型）— routing 三輸入之一（class / latency / modality），
/// 也是 5-factor score 的 modality factor。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    /// 文字（text）— 預設。
    Text,
    /// 圖片（image）。
    Image,
    /// 音訊（audio）。
    Audio,
    /// 影片（video）。
    Video,
    /// 嵌入向量（embedding）。
    Embedding,
}

// ─── §7.1 ReasoningEffort — depth hint for `reasoning` latency (RECONCILE) ────

/// ReasoningEffort（深思強度）— 搭配 `LatencyClass::Reasoning` 給上游
/// o-series / thinking model 的 effort 提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

// ─── §9.2 ProviderScoreFactors — the 5 resolver scoring factors (RECONCILE) ──
//
// RECONCILE NOTE (T-PROV-01, SPEC-14 §9.2 / G4): the 5-factor weighted
// scoring was DROPPED in the parallel-module fork (which kept only a
// priority-order walk in `select_provider`). Re-introduced here as an
// explicit struct so the 5 factors are named, type-checked, and asserted by
// `score_has_five_factors` test below. SPEC-14 §9.2 formula:
//   score = w_class * class_match + w_lat * latency_match + w_mod * modality_match
//         - w_cost * cost_per_input_token - w_ttft * (ttft_ms / 1000)
//
// 中文: SPEC-14 §9.2 的 5 個評分因子（scoring factor）— class（類別）/
// latency（延遲）/ modality（媒介）為加分項，cost（成本）/ ttft（首 token
// 延遲）為扣分項。fork 把它砍成單純 priority 排序，這裡先把 5 因子結構補回，
// 真正權重調校 + 接進 `select_provider` 留後續 reconcile（見 blockers）。

/// 一個 provider 對某 routing 請求的 5 維評分因子（normalized 0.0-1.0）。
/// `score()` 把這 5 個因子加權成單一 f64 分數（higher = better）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProviderScoreFactors {
    /// Factor 1 — class match（類別匹配）：請求 class 與 provider class 吻合度。
    pub class_match: f64,
    /// Factor 2 — latency match（延遲匹配）：latency_class 與 provider TTFT 吻合度。
    pub latency_match: f64,
    /// Factor 3 — modality match（媒介匹配）：provider 是否支援請求的 modality。
    pub modality_match: f64,
    /// Factor 4 — cost（成本）：cost_per_input_token（USD/token），扣分項。
    pub cost: f64,
    /// Factor 5 — ttft（首 token 延遲，毫秒/1000 → 秒），扣分項。
    pub ttft: f64,
}

impl ProviderScoreFactors {
    /// The 5 SPEC-14 §9.2 scoring-factor field names, for self-test / audit.
    /// RECONCILE canary: if a factor is renamed/dropped, this drifts from the
    /// struct and `score_has_five_factors` fails.
    pub const FACTOR_NAMES: [&'static str; 5] =
        ["class_match", "latency_match", "modality_match", "cost", "ttft"];
}

/// 5 維加權 score function（評分函式）— higher = better。
///
/// TODO (T-PROV-01 reconcile, follow-up): weights (`w_*`) are placeholders;
/// real values + the live wiring of this into `select_provider` (replacing the
/// current priority-only walk) are the next reconcile step. The shape here is
/// SPEC-14 §9.2-faithful so callers can start depending on the signature.
///
/// 中文: 把 5 個因子依 SPEC-14 §9.2 公式加權成單一分數。權重目前為佔位值，
/// 真正接進 resolver（取代現行純 priority 排序）留下一步 reconcile。
pub fn score(f: &ProviderScoreFactors) -> f64 {
    // TODO: load weights from agents.toml routing-policy section (Stage 3).
    const W_CLASS: f64 = 0.40;
    const W_LATENCY: f64 = 0.25;
    const W_MODALITY: f64 = 0.20;
    const W_COST: f64 = 0.10;
    const W_TTFT: f64 = 0.05;

    W_CLASS * f.class_match
        + W_LATENCY * f.latency_match
        + W_MODALITY * f.modality_match
        - W_COST * f.cost
        - W_TTFT * f.ttft
}

// ─── §7.1 ResponseFormat — plain_text / json / structured ───────────────────

/// Response output format hint sent with the request. `structured` is for
/// providers that support JSON-schema-constrained generation (GPT-5.5 strict
/// mode, Gemini responseSchema); `json` is loose JSON mode without schema;
/// `plain_text` is the default free-form text.
///
/// 中文: 回應格式提示。`structured` 對應 schema 嚴格約束（GPT strict mode）；
/// `json` 是寬鬆 JSON；`plain_text` 是預設純文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    /// 預設純文字 — free-form completion。
    PlainText,
    /// 寬鬆 JSON mode — 上游保證 valid JSON 但無 schema 約束。
    Json,
    /// Schema 嚴格約束 — GPT-5.5 strict mode / Gemini responseSchema。
    Structured,
}

// ─── §7.1 MessageRole — system / user / assistant ───────────────────────────

/// Chat message role in the OpenAI-shape conversation array. `tool` role is
/// not included here — tool results flow through the `ToolCall` round-trip
/// in `ProviderResponse` (Stage 2 adds the result-back wire).
///
/// 中文: chat 對話中的角色。`tool` role 不在此版 — 工具結果走 ProviderResponse
/// 的 tool_calls 反向 channel，Stage 2 補完。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// system prompt（系統提示）— 行為 / 角色設定。
    System,
    /// user turn（使用者輪）— 來自 caller 的輸入。
    User,
    /// assistant turn（助手輪）— 上游 LLM 的回應。
    Assistant,
}

// ─── §7.1 MessageImage — inline image multimodal part (SPEC-20 T-FOOD-02) ────

/// One inline image attachment on a `Message` (SPEC-20 multimodal food
/// capture). Carries the raw image bytes base64-encoded (`data_b64`) plus the
/// IANA MIME type (`mime`, e.g. `"image/jpeg"`). Per-provider adapters render
/// this into their native inline-image wire: Gemini `parts[].inlineData
/// {mimeType,data}`, Anthropic `content[].image.source {type:base64,...}`,
/// OpenAI `image_url` data-URL. Today only the Gemini adapter consumes it
/// (`complete_gemini`); other providers ignore the field (text still flows).
///
/// 中文: `Message` 上的 inline 圖片附件（SPEC-20 多模態 (multimodal) 食物拍照）。
/// `data_b64` 是圖片 bytes 的 base64（基底 64 編碼）字串，`mime` 是 MIME
/// type（如 `"image/jpeg"`）。各 provider adapter 自行轉成上游 inline-image
/// wire；目前只有 Gemini adapter 真的吃這個欄位，其餘 provider 暫時略過（純
/// 文字仍正常送出）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct MessageImage {
    /// IANA MIME type of the image — `"image/jpeg"` / `"image/png"` / etc.
    pub mime: String,
    /// Raw image bytes, base64-encoded (no `data:` URL prefix). Adapters add
    /// any provider-specific wrapper (data-URL / source object) themselves.
    pub data_b64: String,
}

// ─── §7.1 Message — single chat turn ─────────────────────────────────────────

/// One chat message turn. The OpenAI-shape `messages` array is built from a
/// `Vec<Message>`; per-provider adapter (e.g. AnthropicProvider) may flatten
/// or restructure (Anthropic puts system out-of-band as `SeparateParam` —
/// see SPEC-14 §7.1 SystemPlacement).
///
/// `images` carries optional inline multimodal image parts (SPEC-20 T-FOOD-02).
/// It is `#[serde(default)]` so legacy text-only wire payloads (no `images`
/// key) still deserialize, and a `Message::text(role, content)` constructor
/// keeps text-only call sites concise. Providers that don't support images
/// ignore the field; only `complete_gemini` renders it today.
///
/// 中文: 單一輪對話。`Vec<Message>` 是 caller-side 的中性表達；各 provider
/// adapter 自行轉成上游 wire（如 Anthropic 把 system 拉出 messages 陣列）。
/// `images` 欄位（SPEC-20 T-FOOD-02）攜帶可選的 inline 圖片多模態 part；標了
/// `#[serde(default)]` 所以舊的純文字 wire（沒有 `images` key）仍能解析，向後
/// 相容（backward compatible）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// 角色 — system / user / assistant。
    pub role: MessageRole,
    /// 內容文字 — 純文字主體。
    pub content: String,
    /// 可選 inline 圖片多模態 part（SPEC-20）。空 Vec = 純文字 turn（預設）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<MessageImage>,
}

impl Message {
    /// Construct a text-only message (no inline images). Backward-compatible
    /// shorthand for the common `Message { role, content, images: vec![] }`.
    ///
    /// 中文: 建立純文字 message（無 inline 圖片）— 向後相容的便捷建構子。
    pub fn text(role: MessageRole, content: impl Into<String>) -> Self {
        Message {
            role,
            content: content.into(),
            images: Vec::new(),
        }
    }

    /// Construct a message carrying one inline image part plus text.
    ///
    /// 中文: 建立帶單張 inline 圖片 part 的 message。
    pub fn with_image(role: MessageRole, content: impl Into<String>, image: MessageImage) -> Self {
        Message {
            role,
            content: content.into(),
            images: vec![image],
        }
    }
}

// ─── §7.1 ProviderConfig — agents.toml-side provider record ─────────────────

/// Mirrors one `[providers.X]` section in `~/.spectyn-mesh/agents.toml`.
/// `api_key_ref` points into the SPEC-13 age vault — raw key bytes are NEVER
/// stored here (Stage 2 resolves the ref at dispatch time).
///
/// 中文: 對應 agents.toml 的 `[providers.X]`；`api_key_ref` 不存 key 本體。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Lower-kebab slug, matches `ProviderType::slug()` (e.g. `"groq"`).
    pub slug: String,
    /// Pointer into SPEC-13 age vault (e.g. `"secrets.age#providers.groq.api_key"`).
    /// NEVER the raw key bytes.
    pub api_key_ref: String,
    /// Default model id (e.g. `"llama-3.1-8b-instant"`). Per-request override
    /// via `ProviderRequest.model`.
    pub default_model: String,
    /// Optional custom endpoint base URL (override for self-hosted or proxy).
    /// `None` = use provider-builtin default.
    pub base_url: Option<String>,
    /// Per-request timeout in milliseconds. Resolver still applies an outer
    /// fallback-chain budget — this is just the single-call cap.
    pub timeout_ms: u64,
}

// ─── §7.1 ToolDef — caller-side tool/function definition ────────────────────

/// One callable tool/function the model may invoke. Provider-neutral shape:
/// `name` (the function id the model emits in `tool_calls`), `description`
/// (natural-language guidance the model uses to decide when to call), and
/// `parameters` (a JSON-Schema object describing the arguments). Each
/// per-provider adapter renders this into its native outbound wire:
///   - Groq / OpenAI-compat → `{type:"function", function:{name,description,parameters}}`
///   - Anthropic            → `{name, description, input_schema:parameters}`
///   - Gemini               → `tools:[{functionDeclarations:[{name,description,parameters}]}]`
/// Mirrors the canonical OpenAI-style tool envelope already used by
/// `providers/resolver.rs`. The reverse channel (the model's `tool_calls`)
/// flows back through `ToolCallFormat` / `ProviderResponse` unchanged.
///
/// 中文: 一個可被模型呼叫的工具/函式定義。provider-neutral 形狀：`name`
/// 是模型在 `tool_calls` 回傳的函式 id，`description` 給模型判斷何時呼叫，
/// `parameters` 是描述參數的 JSON-Schema 物件。各 provider adapter 自行轉成
/// 上游原生 wire（OpenAI `tools` / Anthropic `input_schema` / Gemini
/// `functionDeclarations`）。在此之前 `ProviderRequest` 根本沒有 tools 欄位，
/// 所以即使 caller 想傳工具也被整個丟掉 — 本型別補上這條 outbound 通道。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    /// Function id the model emits when it decides to call this tool. MUST
    /// match the name the caller dispatches on when handling `tool_calls`.
    pub name: String,
    /// Natural-language description the model uses to decide when/whether to
    /// call. Empty string is allowed but strongly discouraged.
    pub description: String,
    /// JSON-Schema object for the arguments (typically
    /// `{"type":"object","properties":{...},"required":[...]}`). Passed
    /// through to each provider's native schema slot verbatim. ts-rs has no
    /// `TS` impl for `serde_json::Value`, so the emitted TS type is `unknown`.
    #[ts(type = "unknown")]
    pub parameters: serde_json::Value,
}

// ─── §7.1 ProviderRequest — caller → provider wire ──────────────────────────

/// Caller-side request. Resolver picks `ProviderType` via `select_provider`
/// first; caller builds `ProviderRequest` and calls `complete()` which
/// dispatches to the right impl.
///
/// 中文: caller-side 請求；resolver 先選 provider，caller 再組 request 派工。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequest {
    /// 上游 model id（與 `ProviderConfig.default_model` 對齊或覆寫）。
    pub model: String,
    /// System prompt — 可選（OnDevice 小 model 沒 system role 時 caller 留 None）。
    pub system_prompt: Option<String>,
    /// 對話陣列 — 至少 1 個 user turn；adapter 端負責轉上游 wire。
    pub messages: Vec<Message>,
    /// 最大生成 token 數；`None` 走 provider 預設。
    pub max_tokens: Option<u32>,
    /// Sampling 溫度 0.0 - 2.0；`None` 走 provider 預設（多為 0.7 / 1.0）。
    pub temperature: Option<f32>,
    /// 回應格式提示 — `plain_text` / `json` / `structured`。
    pub response_format: ResponseFormat,
    /// Tool/function definitions the model may call. Empty = text-only
    /// request (the legacy behaviour). Each live adapter (Groq / Anthropic /
    /// Gemini) renders these into its native tools wire so `tool_calls` can
    /// come back. `#[serde(default)]` keeps older tool-less wire payloads
    /// deserializing; `skip_serializing_if` keeps the on-wire JSON identical
    /// to before when no tools are passed.
    ///
    /// 中文: 模型可呼叫的工具定義。空 Vec = 純文字請求（舊行為）；`default`
    /// 讓舊 wire 仍可解析，`skip_serializing_if` 確保無工具時 JSON 與舊版一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
}

// ─── §7.1 ProviderResponse — provider → caller wire ─────────────────────────

/// Unified response — all 11 backends collapse their native wire (Anthropic
/// content blocks / OpenAI choices / Gemini candidates) into this single
/// shape so agent layer never branches on provider type at the response side.
///
/// 中文: 統一回應；11 provider 原生 wire 都收斂到此 shape，agent layer 不必分支。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct ProviderResponse {
    /// 完整 completion 文字 — concatenated content blocks。
    pub text: String,
    /// 實際使用的 model id（fallback 後可能與 request 不同）。
    pub model_used: String,
    /// 輸入 token 數（含 system + messages）。
    pub tokens_in: u32,
    /// 輸出 token 數（completion 部分）。
    pub tokens_out: u32,
    /// 端到端延遲（毫秒）— from request send 到 final byte。
    pub latency_ms: u64,
    /// USD cost — `None` 表 provider 不回報（local / on-device 多為 None）。
    pub cost_usd: Option<f64>,
}

// ─── §7.1 FallbackChain — ordered provider slug list ────────────────────────

/// Ordered fallback chain — resolver walks this in priority order until one
/// provider returns a non-retryable result. Slugs MUST match
/// `ProviderType::slug()` outputs; unknown slugs surface as
/// `ProviderError::ModelNotFound` at validate time.
///
/// 中文: 有序 fallback chain — resolver 由前往後試到第一個非可重試結果。
/// slug 必須對應 `ProviderType::slug()`；未知 slug 在 validate 階段就會擋掉。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(rename_all = "camelCase")]
pub struct FallbackChain {
    /// 優先序 provider slug 列表（index 0 = 最高優先）。
    pub providers: Vec<String>,
}

// ─── §11 ProviderError — wire error catalog ─────────────────────────────────

/// Wire-facing provider error variants per SPEC-14 §11 (mirrors the 6 + 3
/// additional codes table). Stage 2 maps these to the legacy
/// `core::providers::traits::ProviderError` enum so existing call sites keep
/// working — this fresh mirror keeps the wire module dependency-light.
///
/// 中文: SPEC-14 §11 error 目錄的 wire 鏡像。原本
/// `core::providers::traits::ProviderError` 不動，Stage 2 加 mapping。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/providers/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProviderError {
    /// 429 / quota exceeded — Stage 2 retry chain 應換下一個 provider。
    #[error("provider.rate_limit: {detail}")]
    RateLimit { detail: String },
    /// 401 / 403 — API key 失效或無權限。
    #[error("provider.auth_error: {detail}")]
    AuthError { detail: String },
    /// 連線錯誤 / DNS 解析失敗 / TLS handshake fail。
    #[error("provider.network_error: {detail}")]
    NetworkError { detail: String },
    /// 上游回 404 model 或 slug 找不到對應 ProviderType。
    #[error("provider.model_not_found: {detail}")]
    ModelNotFound { detail: String },
    /// 請求超過 model max_context_tokens。
    #[error("provider.context_too_long: tokens={tokens} limit={limit}")]
    ContextTooLong { tokens: u32, limit: u32 },
    /// resolver chain 全部失敗 — caller 應顯示「請稍後再試」。
    #[error("provider.fallback_exhausted: {detail}")]
    FallbackExhausted { detail: String },
    /// request 要 `local` class 但裝置無在地 provider（mobile）。
    #[error("provider.no_match_class: {detail}")]
    NoMatchClass { detail: String },
    /// (OoS2 future) 成本預算超出 — v0.7.0+。
    #[error("provider.cost_budget_exceeded: {detail}")]
    CostBudgetExceeded { detail: String },
    /// 其他未分類錯誤 — 含上游回應解析失敗。
    #[error("provider.unknown: {detail}")]
    Unknown { detail: String },
}

// ─── Stage 2 helpers — pseudocode bodies (Stage 3 fills inner _pseudo fns) ───
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md:
//   Stage 2 = function body shows what it WILL do via comments + nested
//   unimplemented!() inner helpers. Reader can audit the algorithm flow
//   without trusting any I/O or HTTP implementation. Stage 3 swaps each
//   `_pseudo` helper for real `toml` / `std::fs` / `reqwest` / `serde_json`
//   calls (added then) into Cargo.toml at that point.
//
// 中文: Stage 2 用 `// Step N` 註解 + 巢狀 `_pseudo` 內層 helper 把演算法骨架
// 揭露出來；讀者可審計流程，但任何真實 I/O / HTTP 都還沒接。Stage 3 才把
// `_pseudo` 換成 toml / reqwest / serde_json 真正呼叫。

/// Pick the highest-scoring provider slug for the given routing inputs by
/// walking the fallback chain per SPEC-14 §6.x. Stage 2 surfaces the
/// 5-factor weighted score (class, latency, modality, ttft, cost) defined
/// in §9.2 `score()` as a `// Step N` algorithm skeleton.
///
/// 中文: 依 `(class, latency)` 從 fallback chain（備援鏈）挑最高分 provider slug。
/// Stage 2 把演算法骨架攤平成 // Step 註解；真正 toml 解析與 5-factor 算分
/// 等 Stage 3 接 `load_fallback_chain_pseudo` 後一起補。
pub fn select_provider(
    class: ProviderClass,
    latency: LatencyClass,
) -> Result<String, ProviderError> {
    // Step 1: load ordered fallback chain from agents.toml (SPEC-14 §9.2
    //         input) — Stage 3 reads `~/.spectyn-mesh/agents.toml` via
    //         std::fs + toml crate. Returns `FallbackChain` with slugs in
    //         priority order (index 0 = highest preference).
    let chain: FallbackChain = load_fallback_chain()?;

    // Step 2: filter slugs by requested `class` + `latency` budget — drops
    //         e.g. `local` on mobile, or `frontier` when caller asked
    //         `commodity`. Skip silently; the chain may still have
    //         eligible candidates further down.
    let filtered: Vec<String> = filter_chain_by_class_latency(&chain, class, latency);

    // Step 3 (T-PROV-02, SPEC-14 §9.2 / G4): rank the filtered candidates by
    //         the reconciled 5-factor `score()` (class / latency / modality /
    //         cost / ttft) instead of the old priority-only "index 0 wins"
    //         walk. We pair each slug with its chain index and use a STABLE
    //         sort keyed on (-score, chain_index): higher score first, and on
    //         a score tie the lower chain index (original priority) wins. This
    //         preserves the prior behaviour exactly when every candidate scores
    //         equally — no regression — while letting a genuinely better-fit
    //         provider jump ahead of a higher-priority-but-worse-fit one.
    //         Empty filtered chain → caller asked for a class+latency combo no
    //         provider in agents.toml can serve.
    if filtered.is_empty() {
        return Err(ProviderError::NoMatchClass {
            detail: format!(
                "no provider in fallback chain matches class={:?} latency={:?}",
                class, latency
            ),
        });
    }
    let mut ranked: Vec<(usize, String)> = filtered.into_iter().enumerate().collect();
    // Stable sort by descending score; equal scores keep enumerate() order
    // (= original priority order) because `sort_by` is stable.
    ranked.sort_by(|(_, a), (_, b)| {
        let sa = score(&score_factors_for(a, class, latency));
        let sb = score(&score_factors_for(b, class, latency));
        // Descending: higher score is "less" so it sorts first. NaN-safe via
        // total_cmp (factors are finite, but stay defensive).
        sb.total_cmp(&sa)
    });

    // Step 4: probe live-ness via `provider_alive` — Stage 3 consults an
    //         in-memory circuit-breaker cache. Walk the now score-ranked list;
    //         on dead, fall through to the next-best candidate. All-dead →
    //         FallbackExhausted.
    for (_idx, slug) in ranked {
        if provider_alive(&slug) {
            return Ok(slug);
        }
    }
    Err(ProviderError::FallbackExhausted {
        detail: "all providers in fallback chain unreachable".to_string(),
    })
}

/// Derive the 5 SPEC-14 §9.2 scoring factors for one candidate provider
/// `slug` given the requested routing `(class, latency)`. Returns normalized
/// 0.0-1.0 add-factors (class / latency / modality) plus the subtractive
/// cost / ttft penalties, ready for `score()`.
///
/// 中文: 依請求的 `(class, latency)` 為某 provider slug 推導 SPEC-14 §9.2 的 5
/// 個評分因子。class / latency / modality 為 0.0-1.0 加分項，cost / ttft 為
/// 扣分項。本 helper 用 class-level 啟發式（heuristic）填值 — provider 等級
/// 的 cost / ttft 區間先用粗估，per-model 精確值待 Stage 4 接 pricing /
/// typical_ttft_ms metadata 後再細化（見 select_provider TODO）。
// ─── T-PROV-03 — static per-model cost + TTFT table for intra-class ranking ──
//
// T-PROV-02 wired `score()` into `select_provider`, but within one
// `ProviderClass` every candidate scored identically (cost / ttft were a flat
// per-class constant), so ranking always fell back to chain priority. This
// table gives `score_factors_for` REAL per-model inputs so a cheaper / faster
// model in the same class outranks a pricier / slower one.
//
// `cost_tier` is a normalized 0.0-1.0 penalty (higher = more expensive),
// `typical_ttft_ms` is a representative measured first-token latency. Both are
// derived from the model's well-known public pricing / latency profile; they
// are deliberately coarse (tiers, not live pricing) — the live per-1k pricing
// table for billing already lives in agents.toml `[pricing]` (see
// `compute_cost_usd`). This table is the routing-side hint only.
//
// 中文: T-PROV-03 — 為 well-known model 補一張靜態「成本 tier + 典型 ttft_ms」
// 表，餵進 `score_factors_for`，讓同 class 內較便宜 / 較快的 model 排在前面，
// 不再全部同分退回 priority。未知 model 用 class-level 預設（neutral，無回歸）。

/// Representative routing profile for one model: a normalized cost tier
/// (0.0 cheapest … 1.0 priciest) and a typical first-token latency in ms.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelProfile {
    /// Normalized cost penalty 0.0-1.0 (higher = pricier). Subtractive in `score()`.
    cost_tier: f64,
    /// Typical time-to-first-token in milliseconds (lower = snappier).
    typical_ttft_ms: u32,
}

/// TTFT (毫秒) normaliser: maps a raw `typical_ttft_ms` into the 0.0-1.0
/// penalty range `score()` expects. 1500 ms is treated as the "1.0" ceiling
/// (a sluggish frontier model); anything faster scales down linearly. Keeps
/// the ttft factor commensurate with the existing class heuristics
/// (commodity ~0.3 ≈ 450 ms, frontier ~1.0 ≈ 1500 ms).
fn normalize_ttft_ms(ttft_ms: u32) -> f64 {
    const TTFT_CEILING_MS: f64 = 1500.0;
    (ttft_ms as f64 / TTFT_CEILING_MS).clamp(0.0, 1.0)
}

/// Static per-model routing profile for the well-known providers/models. Match
/// is case-insensitive on a normalized model id and prefix-based so versioned
/// ids (e.g. `gpt-5.5-2026-xx`) still resolve. Returns `None` for unknown
/// models so the caller can fall back to the neutral class heuristic — this is
/// what guarantees "no regression for unknown models".
///
/// 中文: well-known model 的靜態 profile（cost tier + ttft）。prefix 比對 +
/// 大小寫不敏感，帶版號的 id 也認得。未知 model 回 `None`，由呼叫端退回 class
/// 預設 → 不造成回歸。
fn model_profile(model: &str) -> Option<ModelProfile> {
    let m = model.to_ascii_lowercase();
    // Ordered most-specific-prefix first. (cost_tier, typical_ttft_ms)
    // Commodity tier — cheap + fast; ranked among themselves by these values.
    const TABLE: &[(&str, f64, u32)] = &[
        // ── commodity: Llama / Mixtral / Qwen on fast-inference backends ──
        ("llama-3.1-8b", 0.05, 180),   // Groq 8B — cheapest + fastest commodity
        ("llama-3.3-70b", 0.20, 350),  // bigger commodity → pricier + slower
        ("llama-3.1-70b", 0.20, 350),
        ("llama-3.1", 0.10, 250),      // generic 3.1 family default
        ("mixtral", 0.18, 400),
        ("qwen", 0.15, 320),
        // ── frontier: GPT / Claude / Gemini ──
        ("gpt-5.5", 0.95, 1200),       // priciest frontier
        ("gpt-4o-mini", 0.30, 600),    // cheap frontier-ish
        ("gpt-4o", 0.70, 900),
        ("gpt-4", 0.85, 1100),
        ("claude-opus", 1.00, 1400),   // priciest + slowest frontier
        ("claude-sonnet", 0.55, 800),  // mid frontier — cheaper + faster than opus
        ("claude-haiku", 0.20, 400),   // cheap + fast frontier
        ("claude-3-5-sonnet", 0.55, 800),
        ("claude", 0.70, 1000),        // generic claude default
        ("gemini-2.5-pro", 0.80, 1000),
        ("gemini-2.5-flash", 0.25, 450), // cheap + fast frontier
        ("gemini-1.5-pro", 0.75, 950),
        ("gemini-1.5-flash", 0.22, 420),
        ("gemini", 0.50, 700),         // generic gemini default
        // ── local / on-device — free, latency dominated by hardware ──
        ("ollama", 0.0, 200),
        ("llama-server", 0.0, 200),
    ];
    for (prefix, cost_tier, ttft) in TABLE {
        if m.starts_with(prefix) {
            return Some(ModelProfile {
                cost_tier: *cost_tier,
                typical_ttft_ms: *ttft,
            });
        }
    }
    None
}

/// Representative model id for a provider slug — used when ranking by
/// provider (the resolver picks providers, not models, in `select_provider`).
/// Stage 4 will read the live `[providers.X].default_model` from agents.toml;
/// for routing-side intra-class differentiation we use the well-known flagship
/// model each provider is fronted by today.
///
/// 中文: provider slug 的代表 model（resolver 是挑 provider 不是 model），用來
/// 在同 class 內比較。Stage 4 會改讀 agents.toml 的 default_model。
fn representative_model(pt: ProviderType) -> &'static str {
    match pt {
        ProviderType::Groq => "llama-3.1-8b-instant",
        ProviderType::Cerebras => "llama-3.3-70b",
        ProviderType::Cloudflare => "llama-3.1-8b",
        ProviderType::Opencode => "qwen",
        ProviderType::Openai => "gpt-5.5",
        ProviderType::Anthropic => "claude-opus",
        ProviderType::Gemini => "gemini-2.5-pro",
        ProviderType::Claude => "claude-opus",
        ProviderType::Codex => "gpt-5.5",
        ProviderType::Ollama => "ollama",
        ProviderType::Llamacpp => "llama-server",
    }
}

fn score_factors_for(
    slug: &str,
    class: ProviderClass,
    latency: LatencyClass,
) -> ProviderScoreFactors {
    // Resolve slug → ProviderType so we can read its class. Unknown slug
    // (shouldn't reach here post-filter) gets a neutral all-zero profile so
    // it ranks last without panicking.
    let Ok(pt) = ProviderType::from_str(slug) else {
        return ProviderScoreFactors {
            class_match: 0.0,
            latency_match: 0.0,
            modality_match: 0.0,
            cost: 0.0,
            ttft: 0.0,
        };
    };
    // Rank by the provider's representative model so two same-class providers
    // fronted by different-cost/-speed models no longer tie.
    score_factors_for_model(pt, representative_model(pt), class, latency)
}

/// Failover-side score for a chain `slug`, used by `walk_fallback_chain` to
/// pre-rank the eligible chain exactly like `select_provider` ranks its
/// filtered candidates. Unlike `select_provider` the failover path has no
/// caller-supplied routing `(class, latency)` — the chain is the request — so
/// we score each slug against ITS OWN provider class plus a neutral
/// `Interactive` latency budget. That keeps `class_match == 1.0` for every
/// slug (no cross-class distortion) while letting the per-model cost / ttft
/// factors differentiate, so a cheaper / faster provider deeper in the chain
/// can be tried first. Reuses the same `score()` + `score_factors_for_model()`
/// machinery — no new scoring semantics.
///
/// 中文: 給 `walk_fallback_chain` 用的 slug 評分。failover 沒有 caller 的
/// `(class, latency)`，故用 slug 自己的 class + 中性 `Interactive` latency 評分，
/// class_match 恆為 1.0，靠 per-model cost / ttft 區分；沿用既有 `score()`，不改語意。
fn score_chain_slug(slug: &str) -> f64 {
    let Ok(pt) = ProviderType::from_str(slug) else {
        // Unknown slug → neutral lowest score so it sinks to the back without
        // panicking (mirrors score_factors_for's all-zero fallback).
        return f64::NEG_INFINITY;
    };
    let own_class = classify(pt);
    score(&score_factors_for_model(
        pt,
        representative_model(pt),
        own_class,
        LatencyClass::Interactive,
    ))
}

/// Model-aware variant of `score_factors_for`: derive the 5 scoring factors
/// for a concrete `(ProviderType, model)` pair. This is where T-PROV-03 feeds
/// the per-model cost / ttft table in. Unknown models fall back to the
/// per-class heuristic (identical to the pre-T-PROV-03 behaviour → no
/// regression).
///
/// 中文: `score_factors_for` 的 model 感知版本 — 用 per-model 表算 cost / ttft；
/// 未知 model 退回 class 預設（與舊行為一致，無回歸）。
fn score_factors_for_model(
    pt: ProviderType,
    model: &str,
    class: ProviderClass,
    latency: LatencyClass,
) -> ProviderScoreFactors {
    let pclass = classify(pt);

    // Factor 1 — class_match: 1.0 when the provider's class equals the
    // requested class (the filter already enforces this, but keep the signal
    // explicit so the factor is meaningful if filtering is relaxed later).
    let class_match = if pclass == class { 1.0 } else { 0.0 };

    // Factors 4 + 5 — cost & ttft penalties. PREFER the per-model table
    // (T-PROV-03) so same-class candidates differentiate; fall back to the
    // per-class heuristic for unknown models (unchanged legacy behaviour →
    // no regression). Lower is better; both are subtractive in `score()`.
    let (cost, ttft) = match model_profile(model) {
        Some(p) => (p.cost_tier, normalize_ttft_ms(p.typical_ttft_ms)),
        None => match pclass {
            ProviderClass::Frontier => (1.0, 1.0),
            ProviderClass::Commodity => (0.3, 0.3),
            ProviderClass::Local => (0.0, 0.1),
        },
    };

    // Factor 2 — latency_match: how well the provider's responsiveness fits
    // the caller's latency budget. Interactive wants fast TTFT, so faster
    // (lower ttft) providers match better; reasoning is unbounded so any
    // provider matches; background tolerates slower providers.
    let latency_match = match latency {
        LatencyClass::Interactive => 1.0 - ttft, // fast provider → high match
        LatencyClass::Background => 1.0,          // any provider fits a queue
        LatencyClass::Reasoning => match pclass {
            // Reasoning prefers frontier brains regardless of latency.
            ProviderClass::Frontier => 1.0,
            ProviderClass::Commodity => 0.5,
            ProviderClass::Local => 0.3,
        },
    };

    // Factor 3 — modality_match: every backend here serves text today; the
    // per-modality capability tag (image / audio / embedding) lands in
    // Stage 4. Until then text routing always matches.
    let modality_match = 1.0;

    ProviderScoreFactors {
        class_match,
        latency_match,
        modality_match,
        cost,
        ttft,
    }
}

/// Dispatch a `ProviderRequest` to the right `ProviderType` impl in
/// `core/src/providers/` and collapse the response into the unified
/// `ProviderResponse` shape. Stage 2 spells out the per-variant branch
/// table; Stage 3 collapses most branches behind one shared reqwest client
/// (openai-compat providers — Groq / Cerebras / Cloudflare / Opencode — all
/// share `complete_openai_compat_pseudo`).
///
/// 中文: 把 ProviderRequest 派工到對應 provider impl，回收成統一
/// ProviderResponse。Stage 2 列出 11 個 ProviderType variant 的分支表；
/// Stage 3 用 reqwest + serde_json 真接 HTTP，並把 openai-compat 家族
/// (Groq / Cerebras / Cloudflare / Opencode) 收斂到單一 helper。
pub fn complete(req: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    // Step 1: resolve the request's model string to a `ProviderType` so we
    //         know which adapter to call. agents.toml maps model → provider;
    //         Stage 3 uses the same toml loader as `select_provider`.
    let provider_type: ProviderType = resolve_model_to_provider_type(&req.model)?;

    // Step 2-5: dispatch per ProviderType variant. Each branch calls its
    //           own `complete_<provider>_pseudo` helper so a Stage 3 reader
    //           can audit which variants share an HTTP adapter and which
    //           need bespoke wire (Anthropic XmlTags / Gemini functionCall).
    let raw: ProviderResponse = match provider_type {
        // Step 2: Groq — OpenAI-compat wire, Llama 3.1 commodity, TTFT < 200 ms.
        ProviderType::Groq => complete_groq(&req)?,
        // Step 3: OpenAI — native GPT-5.5 frontier, JSON mode + tool_calls.
        ProviderType::Openai => complete_openai_pseudo(&req)?,
        // Step 4: Anthropic — Claude Opus / Sonnet, XmlTags prompt + tool_use.
        ProviderType::Anthropic => complete_anthropic(&req)?,
        // Step 5: Gemini 3.1 Pro — Qa prompt + functionCall wire.
        ProviderType::Gemini => complete_gemini(&req)?,
        // Step 5a: Cerebras — Llama 3.1 wafer-scale, openai-compat.
        ProviderType::Cerebras => complete_cerebras_pseudo(&req)?,
        // Step 5b: Opencode — free model via opencode.ai CLI shell.
        ProviderType::Opencode => complete_opencode_pseudo(&req)?,
        // Step 5c: Cloudflare Workers AI — edge inference, openai-compat.
        ProviderType::Cloudflare => complete_cloudflare_pseudo(&req)?,
        // Step 5d: Ollama — local OnDevice, desktop-only HTTP on :11434.
        ProviderType::Ollama => complete_ollama_pseudo(&req)?,
        // Step 5e: Claude CLI — `~/.claude/` shell, frontier + MCP tool wire.
        ProviderType::Claude => complete_claude_cli_pseudo(&req)?,
        // Step 5f: Codex CLI — OpenAI Codex shell adapter, desktop-only.
        ProviderType::Codex => complete_codex_cli_pseudo(&req)?,
        // Step 5g: llama.cpp local runtime — calls `llama-server` HTTP.
        ProviderType::Llamacpp => complete_llamacpp_pseudo(&req)?,
    };

    // Step 6: cost_usd is computed from (tokens_in, tokens_out) × per-1k
    //         price table loaded from agents.toml (Stage 3). Local /
    //         OnDevice providers stay `None`. We re-stamp `raw.cost_usd`
    //         here so adapters can leave it unset and the resolver layer
    //         owns the pricing policy.
    let cost_usd: Option<f64> = compute_cost_usd(
        provider_type,
        &raw.model_used,
        raw.tokens_in,
        raw.tokens_out,
    );
    Ok(ProviderResponse { cost_usd, ..raw })
}

/// SPEC-14 §6 — walk the agents.toml `[routing].fallback_chain` in priority
/// order, attempting `complete()` against each provider's `default_model`
/// until one returns `Ok`. The first success is returned verbatim; if EVERY
/// provider errors (or the chain is empty / all slugs unusable) this returns
/// `ProviderError::FallbackExhausted` carrying the last underlying error.
///
/// This is the orchestration layer the single-provider `complete()` lacks: it
/// is what makes "all providers failed" a concrete, observable outcome. The
/// coach degraded-review path (SPEC-23 §11.1) keys off exactly this — an
/// `Err(FallbackExhausted)` here is what triggers the stats-only degrade.
///
/// `template` supplies the system prompt, messages, and sampling knobs; only
/// `template.model` is overridden per-slug (each provider runs its own
/// `default_model`). A slug with no `[providers.X]` section, an empty
/// `default_model`, or a tripped circuit-breaker (`provider_alive == false`)
/// is skipped silently and the walk continues down the chain.
///
/// 中文: 依 agents.toml `[routing].fallback_chain` 順序逐一試 `complete()`，
/// 第一個成功就回傳；全部失敗 → `FallbackExhausted`（coach degraded 路徑的
/// 觸發點）。每個 slug 用自己的 `default_model`，跳過無 section / 無 model /
/// 被熔斷器標記為 down 的 provider。
pub fn complete_with_fallback(template: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let view = read_agents_toml()?;
    // Production drives each chosen slug through the real `complete()` HTTP
    // dispatcher. The chain-walk policy (skip-unconfigured, breaker, soft-error
    // advance, last-error capture, FallbackExhausted synthesis) lives in the
    // pure `walk_fallback_chain` so it is unit-testable without real network —
    // the failover behaviour is the same whether `attempt` is `complete()` or
    // a test double.
    walk_fallback_chain(
        &view.routing.fallback_chain,
        |slug| {
            view.providers
                .get(slug)
                .filter(|sec| !sec.default_model.is_empty())
                .map(|sec| sec.default_model.clone())
        },
        provider_alive,
        record_provider_failure,
        // T-PROV-04: pre-rank the eligible chain by the SAME 5-factor score()
        // select_provider uses, so failover honours scoring (cheaper/faster
        // provider tried first) instead of raw chain order.
        score_chain_slug,
        |req| complete(req),
        &template,
    )
}

/// Map a wire `ProviderError` to "is this transient?" by routing it through the
/// core `FailureKind` classifier. Keeps the breaker's count gated to genuinely
/// transient failures (network / rate-limit) and lets permanent ones fail over
/// without tripping it. Pure — testable without HTTP.
fn wire_failure_is_transient(err: &ProviderError) -> bool {
    use crate::providers::circuit_breaker::{classify_failure, FailureKind};
    use crate::providers::traits::ProviderError as CoreErr;
    let core = match err {
        ProviderError::RateLimit { .. } => CoreErr::RateLimit,
        ProviderError::AuthError { .. } => CoreErr::AuthError,
        ProviderError::NetworkError { .. } => CoreErr::NetworkError,
        ProviderError::ModelNotFound { .. } => CoreErr::ModelNotFound,
        ProviderError::ContextTooLong { .. } => CoreErr::ContextTooLong,
        // FallbackExhausted / NoMatchClass / CostBudgetExceeded / Unknown all
        // map to Unknown → Failover (don't open the breaker on these).
        _ => CoreErr::Unknown(err.to_string()),
    };
    classify_failure(&core) == FailureKind::Retry
}

/// Pure failover-chain policy (SPEC-14 §6) extracted from
/// [`complete_with_fallback`] so it is testable without real HTTP. Walks
/// `chain` in priority order:
/// - `model_for(slug)` returning `None` ⇒ slug has no usable model ⇒ skipped.
/// - `is_alive(slug) == false` ⇒ circuit-breaker tripped ⇒ skipped.
/// - `attempt(req)` `Ok` ⇒ returned immediately (first success wins).
/// - `attempt(req)` `Err` ⇒ `mark_down(slug)` is recorded and the walk
///   advances to the next slug, keeping the last error.
///
/// Returns `FallbackExhausted` (carrying the last underlying error) when every
/// eligible provider failed, or when the chain is empty / fully skipped — the
/// concrete "all providers failed" signal the coach degraded path keys off.
fn walk_fallback_chain(
    chain: &[String],
    model_for: impl Fn(&str) -> Option<String>,
    is_alive: impl Fn(&str) -> bool,
    mut mark_down: impl FnMut(&str),
    score_for: impl Fn(&str) -> f64,
    mut attempt: impl FnMut(ProviderRequest) -> Result<ProviderResponse, ProviderError>,
    template: &ProviderRequest,
) -> Result<ProviderResponse, ProviderError> {
    if chain.is_empty() {
        return Err(ProviderError::FallbackExhausted {
            detail: "agents.toml `[routing].fallback_chain` is empty or missing".to_string(),
        });
    }

    // T-PROV-04: pre-rank the chain by the same 5-factor `score()` that
    // `select_provider` uses, with a STABLE tie-break on the original chain
    // index (exactly like select_provider's `(-score, chain_index)` sort).
    // Higher score is attempted first; on a score tie the lower (original
    // priority) index wins, so an all-equal chain walks in its declared order
    // — no regression. Skip/breaker/attempt policy below is unchanged; only the
    // ORDER the eligible slugs are visited now honours scoring instead of raw
    // chain order.
    let mut ranked: Vec<(usize, &String)> = chain.iter().enumerate().collect();
    ranked.sort_by(|(_, a), (_, b)| {
        let sa = score_for(a);
        let sb = score_for(b);
        // Descending: higher score sorts first. total_cmp is NaN-safe and a
        // total order, so the sort stays stable on ties → chain index breaks them.
        sb.total_cmp(&sa)
    });

    let mut last_err: Option<ProviderError> = None;
    for (_idx, slug) in ranked {
        // Resolve this slug's default model; skip slugs with no section /
        // no model so a partially-configured chain still makes progress.
        let model = match model_for(slug) {
            Some(m) => m,
            None => continue,
        };
        // Honour the per-process circuit breaker — don't re-hit an upstream
        // that failed within the last BREAKER_TTL.
        if !is_alive(slug) {
            continue;
        }
        let req = ProviderRequest {
            model,
            ..template.clone()
        };
        match attempt(req) {
            Ok(resp) => {
                // P0-5: a success revives the slug's breaker (Closed + count 0)
                // so a provider that recovered is immediately reusable.
                record_provider_success(slug);
                return Ok(resp);
            }
            Err(e) => {
                // P0-5: only TRANSIENT failures count toward opening the
                // breaker. A permanent error (bad key / wrong model) fails over
                // to the next slug but must NOT circuit-break a healthy upstream.
                if wire_failure_is_transient(&e) {
                    mark_down(slug);
                }
                last_err = Some(e);
            }
        }
    }

    Err(ProviderError::FallbackExhausted {
        detail: match last_err {
            Some(e) => format!("all providers in fallback chain failed; last: {}", e),
            None => "no usable provider in fallback chain (all slugs skipped)".to_string(),
        },
    })
}

/// Validate a `ProviderConfig` — schema shape check + reachability ping
/// (HTTP HEAD/GET on `base_url` or provider default) with the resolved
/// `api_key_ref`. Returns `Ok(())` only when both checks pass.
///
/// 中文: 驗 ProviderConfig — (1) 欄位 schema 形狀 + (2) 用 base_url（或
/// provider 預設）做連線測試。兩者都通過才回 Ok。Stage 3 接 reqwest HEAD ping。
pub fn validate_config(config: &ProviderConfig) -> Result<(), ProviderError> {
    // Step 1: schema check — every required field must be non-empty so a
    //         typo'd agents.toml entry fails loudly here rather than at
    //         dispatch time (where it would surface as a 401 with useless
    //         error text from the upstream).
    if config.slug.trim().is_empty() {
        return Err(ProviderError::Unknown {
            detail: "ProviderConfig.slug must not be empty".to_string(),
        });
    }
    if config.api_key_ref.trim().is_empty() {
        return Err(ProviderError::AuthError {
            detail: format!("provider `{}` missing api_key_ref", config.slug),
        });
    }
    if config.default_model.trim().is_empty() {
        return Err(ProviderError::ModelNotFound {
            detail: format!("provider `{}` missing default_model", config.slug),
        });
    }
    if config.timeout_ms == 0 {
        return Err(ProviderError::Unknown {
            detail: format!("provider `{}` timeout_ms must be > 0", config.slug),
        });
    }

    // Step 2: HEAD/GET reachability ping — Stage 3 uses reqwest with the
    //         resolved api_key (via SPEC-13 age vault) in the Authorization
    //         header. base_url defaults to provider-builtin per
    //         ProviderType::default_base_url() (added Stage 3).
    provider_ping(config)?;

    // Step 3: both checks passed.
    Ok(())
}

// ─── Stage 3 inner helpers — real config-side + 3 frontier providers ────────
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md the Stage 2 `_pseudo`
// panicking stubs are replaced wholesale for the structural / config-side
// helpers AND for the three flagship per-provider HTTP callers (Groq /
// Anthropic / Gemini) via async `reqwest` 0.12 driven through a small
// `block_on_async` sync→async bridge. The other 8 provider variants keep
// their `Stage 4` markers (each is a ~30 LOC variant of the same pattern).
//
// Real (Stage 3):
//   • `load_fallback_chain`             — toml + std::fs read of agents.toml
//   • `filter_chain_by_class_latency`   — slug-set filter (pure logic)
//   • `provider_alive`                  — in-mem circuit-breaker
//   • `resolve_model_to_provider_type`  — toml + std::fs scan
//   • `compute_cost_usd`                — toml pricing table lookup
//   • `complete_groq`                   — POST api.groq.com/openai/v1/chat/...
//   • `complete_anthropic`              — POST api.anthropic.com/v1/messages
//   • `complete_gemini`                 — POST generativelanguage.googleapis...
//   • `provider_ping`                   — HEAD/GET reachability check
//
// Deferred (Stage 4 — ~30 LOC each, same pattern as the 3 promoted above):
//   • `complete_openai`     — OpenAI-native /v1/chat/completions
//   • `complete_cerebras`   — openai-compat
//   • `complete_opencode`   — std::process::Command shell-out
//   • `complete_cloudflare` — openai-compat / cloudflare account-scoped
//   • `complete_ollama`     — localhost:11434 (desktop-only)
//   • `complete_claude_cli` — std::process::Command shell-out
//   • `complete_codex_cli`  — std::process::Command shell-out
//   • `complete_llamacpp`   — localhost:8080 llama-server

/// Path to the on-disk agents.toml file. Resolved once via `dirs::home_dir`
/// equivalent (std::env::var "HOME") so unit tests can override via the
/// `SPECTYN_MESH_AGENTS_TOML` env var without booting a fs sandbox.
/// Resolve a provider's API base URL, honouring a per-slug env override. When
/// `SPECTYN_MESH_<SLUG>_BASE_URL` is set + non-empty it wins; otherwise the
/// built-in production default is returned. This is the test seam that lets
/// wiremock point a provider call at a local mock server — production behaviour
/// is byte-identical when the env var is unset.
///
/// 中文: 解析 provider 的 API base URL，可被 `SPECTYN_MESH_<SLUG>_BASE_URL`
/// 環境變數覆蓋（測試用 wiremock 攔截）；未設時回 production 預設，行為不變。
fn provider_base_url(slug: &str, default_base: &str) -> String {
    let key = format!("SPECTYN_MESH_{}_BASE_URL", slug.to_ascii_uppercase());
    std::env::var(&key)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_base.to_string())
}

fn agents_toml_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("SPECTYN_MESH_AGENTS_TOML") {
        return std::path::PathBuf::from(p);
    }
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".spectyn-mesh"))
        .join("agents.toml")
}

/// Parsed view of `~/.spectyn-mesh/agents.toml`. Only the subset of fields
/// this module needs is modelled; unknown sections (e.g. `[agent.X]`) are
/// silently ignored thanks to `#[serde(default)]` + `toml::Value` catch-all.
#[derive(Debug, Default, Deserialize)]
struct AgentsTomlView {
    #[serde(default)]
    routing: RoutingTable,
    #[serde(default)]
    providers: std::collections::HashMap<String, ProviderTomlSection>,
    #[serde(default)]
    pricing: std::collections::HashMap<String, std::collections::HashMap<String, ModelPriceRow>>,
}

#[derive(Debug, Default, Deserialize)]
struct RoutingTable {
    #[serde(default)]
    fallback_chain: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProviderTomlSection {
    #[serde(default)]
    default_model: String,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
struct ModelPriceRow {
    #[serde(default)]
    input_per_1k_usd: f64,
    #[serde(default)]
    output_per_1k_usd: f64,
}

fn read_agents_toml() -> Result<AgentsTomlView, ProviderError> {
    let path = agents_toml_path();
    let raw = std::fs::read_to_string(&path).map_err(|e| ProviderError::Unknown {
        detail: format!("agents.toml read failed at {}: {}", path.display(), e),
    })?;
    toml::from_str::<AgentsTomlView>(&raw).map_err(|e| ProviderError::Unknown {
        detail: format!("agents.toml parse failed: {}", e),
    })
}

fn load_fallback_chain() -> Result<FallbackChain, ProviderError> {
    let v = read_agents_toml()?;
    if v.routing.fallback_chain.is_empty() {
        return Err(ProviderError::Unknown {
            detail: "agents.toml `[routing].fallback_chain` is empty or missing".to_string(),
        });
    }
    Ok(FallbackChain {
        providers: v.routing.fallback_chain,
    })
}

fn filter_chain_by_class_latency(
    chain: &FallbackChain,
    class: ProviderClass,
    _latency: LatencyClass,
) -> Vec<String> {
    // Pure-logic filter: drop slugs whose ProviderType class doesn't match
    // the requested `class`. `_latency` is reserved for Stage 4 when each
    // provider gets a measured-TTFT capability tag; today every backend
    // claims to serve both `interactive` and `background`, so latency
    // is a no-op until per-provider TTFT history lands.
    chain
        .providers
        .iter()
        .filter(|slug| {
            ProviderType::from_str(slug)
                .map(|pt| classify(pt) == class)
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Map each `ProviderType` to its SPEC-14 §7.1 `ProviderClass` tag.
/// Frontier = expensive smart; Commodity = cheap workhorse;
/// Local = on-device / desktop-only.
fn classify(pt: ProviderType) -> ProviderClass {
    match pt {
        ProviderType::Openai | ProviderType::Anthropic | ProviderType::Gemini => {
            ProviderClass::Frontier
        }
        ProviderType::Groq
        | ProviderType::Cerebras
        | ProviderType::Cloudflare
        | ProviderType::Opencode => ProviderClass::Commodity,
        ProviderType::Ollama | ProviderType::Llamacpp => ProviderClass::Local,
        // CLI-shell providers (`claude` / `codex`) tunnel into a frontier
        // upstream — keep classified as frontier so cost / quality routing
        // doesn't mistake them for free local backends.
        ProviderType::Claude | ProviderType::Codex => ProviderClass::Frontier,
    }
}

/// Per-process deterministic circuit breaker (P0-5). Replaces the old
/// `slug -> Instant` TTL cache with the `Closed → Open(after N) → HalfOpen(after
/// cooldown) → Closed` state machine in `providers::circuit_breaker`. Driven by
/// the real `SystemClock` in production; the unit tests for the machine itself
/// use a `MockClock` (see `providers/circuit_breaker.rs`).
fn breaker() -> &'static crate::providers::circuit_breaker::CircuitBreaker {
    use crate::providers::circuit_breaker::{BreakerConfig, CircuitBreaker};
    static CB: std::sync::OnceLock<CircuitBreaker> = std::sync::OnceLock::new();
    CB.get_or_init(|| CircuitBreaker::new(BreakerConfig::default()))
}

/// Is `slug` allowed right now (Closed or HalfOpen)? An Open breaker still in
/// its cooldown returns `false` so the chain walk skips to the next slug.
fn provider_alive(slug: &str) -> bool {
    breaker().allow(slug, &crate::clock::SystemClock)
}

/// Record a TRANSIENT failure against `slug`. The chain walk calls this only
/// for `FailureKind::Retry` errors (network / rate-limit / overload); permanent
/// errors (auth / model-not-found / context-too-long / unknown) fail over
/// WITHOUT tripping the breaker (see `walk_fallback_chain`). Marked
/// `#[allow(dead_code)]` until the Stage-4 live `complete_*` callers invoke it.
#[allow(dead_code)]
fn record_provider_failure(slug: &str) {
    breaker().on_failure(
        slug,
        crate::providers::circuit_breaker::FailureKind::Retry,
        &crate::clock::SystemClock,
    );
}

/// Record a success against `slug` — closes the breaker and clears its
/// consecutive-failure count. Called on the `Ok` arm of the chain walk so a
/// recovered provider is immediately reusable.
fn record_provider_success(slug: &str) {
    breaker().on_success(slug, &crate::clock::SystemClock);
}

/// Infer a provider slug from a well-known model-name prefix — the fallback for
/// when a model id isn't explicitly listed in any `[providers.X]` section of
/// agents.toml. Pure + hermetically testable. The API key is still required
/// downstream (`resolve_api_key`), so a guessed provider can't reach one the
/// user hasn't configured a key for.
fn infer_provider_from_model_prefix(model: &str) -> Option<&'static str> {
    let m = model.to_ascii_lowercase();
    if m.starts_with("gemini") {
        Some("gemini")
    } else if m.starts_with("claude") {
        Some("anthropic")
    } else if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        Some("openai")
    } else if m.starts_with("llama") || m.starts_with("mixtral") || m.starts_with("qwen") || m.starts_with("groq") {
        Some("groq")
    } else {
        None
    }
}

fn resolve_model_to_provider_type(model: &str) -> Result<ProviderType, ProviderError> {
    let v = read_agents_toml()?;
    for (slug, section) in &v.providers {
        if section.default_model == model || section.models.iter().any(|m| m == model) {
            return ProviderType::from_str(slug);
        }
    }
    // Fallback: infer from the model-name family so callers can use a standard
    // model id (e.g. "gemini-2.5-flash") without declaring it in agents.toml.
    if let Some(slug) = infer_provider_from_model_prefix(model) {
        return ProviderType::from_str(slug);
    }
    Err(ProviderError::ModelNotFound {
        detail: format!("no `[providers.X]` section advertises model `{}`", model),
    })
}

// ─── Stage 3 — sync→async bridge + shared HTTP helpers ─────────────────────
//
// `complete()` keeps its sync signature so the SPEC-14 §9 wire contract
// doesn't drift; the real per-provider helpers below are async (reqwest 0.12
// has no blocking feature enabled in core/Cargo.toml). `block_on_async`
// resolves the runtime situation by either (a) using the in-flight tokio
// runtime via `block_in_place` + `Handle::block_on` when we're inside one,
// or (b) spinning a one-shot current-thread runtime when called from a
// plain sync context (e.g. CLI / tests).
//
// 中文: `complete()` 對外維持 sync；底下三個真實 helper 是 async（reqwest
// 0.12 在 core/Cargo.toml 沒開 blocking feature）。`block_on_async` 視
// runtime 狀況切換：(a) 已在 tokio runtime 內 → `block_in_place`；(b) 不在 →
// 起一個 current-thread runtime 跑完即丟。
pub(crate) fn block_on_async<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // We're inside an existing tokio runtime. `block_in_place` parks
            // the current worker thread so the future can drive itself
            // without deadlocking the scheduler.
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        Err(_) => {
            // No ambient runtime — spin a fresh single-threaded one. Cheap
            // (~10 ms) for the rare sync-from-CLI / test path.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime build")
                .block_on(fut)
        }
    }
}

/// Resolve an API key for the given provider slug. Stage 3 fallback chain:
/// (1) env var `SPECTYN_MESH_<UPPER>_API_KEY` (test + CLI friendly);
/// (2) legacy env `<UPPER>_API_KEY` (e.g. `GROQ_API_KEY` per common shell
/// dotfile habit). Stage 4 will resolve `ProviderConfig.api_key_ref` against
/// the SPEC-13 age vault — this env-var path is the bridge so the live HTTP
/// callers below are usable today without booting the vault.
fn resolve_api_key(slug: &str) -> Result<String, ProviderError> {
    let upper = slug.to_ascii_uppercase();
    let candidates = [
        format!("SPECTYN_MESH_{}_API_KEY", upper),
        format!("{}_API_KEY", upper),
    ];
    for k in &candidates {
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
    }
    Err(ProviderError::AuthError {
        detail: format!(
            "no api key for `{}` (tried env vars: {})",
            slug,
            candidates.join(", ")
        ),
    })
}

/// Build a `reqwest::Client` with the provider's `timeout_ms` budget. Reused
/// by every per-provider helper so retry / connect / TLS settings stay
/// consistent. Defaults to 30 s if no agents.toml entry sets a per-provider
/// timeout — matches SPEC-14 §6.3 background-class budget.
fn http_client(timeout_ms: u64) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms.max(1000)))
        .build()
        .map_err(|e| ProviderError::NetworkError {
            detail: format!("reqwest client build failed: {}", e),
        })
}

/// Map an HTTP response status to a `ProviderError` per SPEC-14 §11. 2xx
/// returns Ok with the body bytes already buffered by the caller. 401/403 →
/// AuthError, 404 → ModelNotFound, 429 → RateLimit, 5xx → NetworkError.
fn map_http_status(slug: &str, status: u16, body: &str) -> ProviderError {
    let snippet = body.chars().take(200).collect::<String>();
    match status {
        401 | 403 => ProviderError::AuthError {
            detail: format!("{} {}: {}", slug, status, snippet),
        },
        404 => ProviderError::ModelNotFound {
            detail: format!("{} 404: {}", slug, snippet),
        },
        429 => ProviderError::RateLimit {
            detail: format!("{} 429: {}", slug, snippet),
        },
        500..=599 => ProviderError::NetworkError {
            detail: format!("{} {}: {}", slug, status, snippet),
        },
        _ => ProviderError::Unknown {
            detail: format!("{} {}: {}", slug, status, snippet),
        },
    }
}

/// Lookup default timeout from agents.toml for a provider slug, falling back
/// to 30 s when no `[providers.X].timeout_ms` is configured. Keeps the
/// per-provider HTTP callers terse — they don't need to thread a Config arg.
fn timeout_for(slug: &str) -> u64 {
    // For Stage 3 we use a static default; Stage 4 reads `[providers.X]
    // .timeout_ms` from agents.toml. This keeps the live helpers usable in
    // tests + scripts that don't ship a full agents.toml.
    let _ = slug;
    30_000
}

// ─── §7.1 Tool-def serializers — neutral ToolDef → per-provider wire ────────
//
// Each live adapter (Groq / Anthropic / Gemini) renders the provider-neutral
// `Vec<ToolDef>` into its own native tools schema. Extracted as pure helpers
// (no I/O) so the outbound JSON is unit-testable WITHOUT a live HTTP call.
// Before this, `ProviderRequest` had no `tools` field at all, so every adapter
// built a text-only body and `tool_calls` could never come back.

/// Groq / OpenAI-compat tools array: `[{type:"function", function:{name,
/// description, parameters}}]`. Mirrors the canonical envelope used by
/// `providers/resolver.rs`. Returns `None` when no tools (so the adapter omits
/// the `tools` body key entirely, keeping the no-tools wire byte-identical).
fn groq_tools_json(tools: &[ToolDef]) -> Option<serde_json::Value> {
    if tools.is_empty() {
        return None;
    }
    let arr: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                },
            })
        })
        .collect();
    Some(serde_json::Value::Array(arr))
}

/// Anthropic tools array: `[{name, description, input_schema}]`. Anthropic
/// names the schema slot `input_schema` (not `parameters`) and has no outer
/// `{type:"function"}` envelope. Returns `None` when no tools.
fn anthropic_tools_json(tools: &[ToolDef]) -> Option<serde_json::Value> {
    if tools.is_empty() {
        return None;
    }
    let arr: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect();
    Some(serde_json::Value::Array(arr))
}

/// Gemini tools wire: a single-element `tools` array whose entry carries a
/// `functionDeclarations` array of `{name, description, parameters}`. Gemini
/// nests every declaration under one `functionDeclarations` block. Returns
/// `None` when no tools.
fn gemini_tools_json(tools: &[ToolDef]) -> Option<serde_json::Value> {
    if tools.is_empty() {
        return None;
    }
    let decls: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();
    Some(serde_json::json!([{ "functionDeclarations": decls }]))
}

/// SPEC-14 §9.2 — POST groq.com/openai/v1/chat/completions. Groq exposes the
/// OpenAI-compat wire shape (model + messages + temperature + max_tokens +
/// optional response_format) so the body is a thin map. Returns the
/// unified ProviderResponse with text / tokens / latency / cost (cost is
/// re-stamped by the outer `complete()` from the pricing table).
///
/// 中文: 真實 reqwest 呼叫 Groq Cloud；OpenAI-compat wire；上游 5xx / 429 /
/// 401 / 404 都按 SPEC-14 §11 對應到 ProviderError variant。
fn complete_groq(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "groq";
    let api_key = resolve_api_key(slug)?;
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/openai/v1/chat/completions",
        provider_base_url(slug, "https://api.groq.com")
    );

    // Build OpenAI-shape messages array: prepend system if present, then
    // verbatim user/assistant turns. Groq does NOT accept `tool` role at
    // Stage 3 — caller-side tool-call wire is Stage 4 work.
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    // Wire caller-supplied tool defs through as the OpenAI-style `tools`
    // array so the model can emit `tool_calls`. Omitted entirely when empty.
    if let Some(tools) = groq_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async {
        client
            .post(url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
    })
    .map_err(|e| ProviderError::NetworkError {
        detail: format!("groq send failed: {}", e),
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("groq body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("groq response parse failed: {}", e),
        })?;
    let completion = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None, // outer complete() re-stamps from pricing table
    })
}

/// Stage-4 stub guard — a genuinely-unimplemented provider arm must surface
/// as a typed `Err` (NEVER `unimplemented!()`): `complete()` is dispatched
/// mid-chain by `walk_fallback_chain`, so a panic here would abort the whole
/// process instead of recording the failure and falling through to the next
/// provider (SPEC-14 §6 failover). Mirrors the identity_wire keystore-stub
/// convention of returning a typed unavailability error. Reuses the existing
/// `Unknown` catch-all variant — no parallel error type.
///
/// 中文: Stage 4 尚未實作的 provider 分支必須回傳 typed Err（絕不能
/// `unimplemented!()` panic）——鏈中失敗要能 fail over 到下一個 provider，
/// 不能讓整個 process 掛掉。
fn stage4_unimplemented(slug: &str, todo: &str) -> ProviderError {
    ProviderError::Unknown {
        detail: format!("provider `{slug}` wire not implemented (Stage 4): {todo}"),
    }
}

/// SPEC-14 §9.2 — POST api.openai.com/v1/chat/completions. OpenAI is the
/// canonical OpenAI-compat wire: same messages / temperature / max_tokens /
/// response_format body builder as Groq, just a different host + Bearer key,
/// and it natively returns `choices[0].message.tool_calls` when caller-supplied
/// `tools` are exercised. We lift `message.content` into `ProviderResponse.text`
/// (which can be empty on a pure tool-call turn) and the `usage` block into the
/// token counts — the parse must survive a `tool_calls`-bearing response.
///
/// 中文: 真實 reqwest 呼叫 OpenAI Chat Completions；body 與 Groq 同形，僅 host +
/// Bearer 不同；回應原生帶 `tool_calls`，解析仍取 `message.content` + `usage`。
fn complete_openai_pseudo(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "openai";
    let api_key = resolve_api_key(slug)?;
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/v1/chat/completions",
        provider_base_url(slug, "https://api.openai.com")
    );

    // OpenAI-shape messages: prepend system if present, then verbatim turns.
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    // Native tool_calls: caller-supplied tool defs render through the same
    // OpenAI-compat `tools` envelope as Groq so the model can emit tool_calls.
    if let Some(tools) = groq_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async {
        client.post(url).bearer_auth(&api_key).json(&body).send().await
    })
    .map_err(|e| ProviderError::NetworkError {
        detail: format!("openai send failed: {}", e),
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("openai body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("openai response parse failed: {}", e),
        })?;
    // `message.content` may be JSON null on a pure tool-call turn — treat that
    // as an empty completion string (the tool_calls live alongside it; the
    // unified ProviderResponse carries text only at this stage).
    let completion = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None, // outer complete() re-stamps from pricing table
    })
}

/// SPEC-14 §9.2 — POST api.anthropic.com/v1/messages. Anthropic's native
/// wire pulls `system` out of the messages array (XmlTags convention) and
/// returns content as an array of typed blocks; we flatten the first
/// `text` block into `ProviderResponse.text`. Requires the
/// `anthropic-version: 2023-06-01` header per the public API contract.
///
/// 中文: 真實 reqwest 呼叫 Anthropic Messages API；system 走 out-of-band
/// 欄位，content 是 typed block 陣列（取第一個 `text` block 攤平）。
fn complete_anthropic(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "anthropic";
    let api_key = resolve_api_key(slug)?;
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/v1/messages",
        provider_base_url(slug, "https://api.anthropic.com")
    );

    // Anthropic-shape messages: system out-of-band; messages array only has
    // user / assistant. We drop any system-role turns from the messages
    // array — they get folded into `system_prompt` per SPEC-14 §7.1
    // SystemPlacement::SeparateParam convention.
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| !matches!(m.role, MessageRole::System))
        .map(|m| {
            let role = match m.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "user", // already filtered above
            };
            serde_json::json!({"role": role, "content": m.content})
        })
        .collect();

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        // Anthropic requires max_tokens — default to 1024 when caller omitted.
        "max_tokens": req.max_tokens.unwrap_or(1024),
    });
    if let Some(sys) = &req.system_prompt {
        body["system"] = serde_json::json!(sys);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    // Anthropic native tools block: `tools:[{name,description,input_schema}]`.
    // Omitted entirely when the caller passed no tools.
    if let Some(tools) = anthropic_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async {
        client
            .post(url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
    })
    .map_err(|e| ProviderError::NetworkError {
        detail: format!("anthropic send failed: {}", e),
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("anthropic body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("anthropic response parse failed: {}", e),
        })?;
    // Walk content[] and concatenate every block with `type == "text"`.
    let completion = json
        .get("content")
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None,
    })
}

/// SPEC-14 §9.2 — POST generativelanguage.googleapis.com Gemini v1beta.
/// Gemini's wire shape uses `contents[]` with `parts[]` per turn; system
/// prompt goes into a top-level `systemInstruction` object. Returns the
/// concatenated text parts of the first candidate. Auth is by `?key=` query
/// param rather than a header per Google's convention.
///
/// 中文: 真實 reqwest 呼叫 Gemini API；wire 用 contents[]/parts[]，system 走
/// `systemInstruction`；auth 走 query string `?key=`。
fn complete_gemini(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "gemini";
    let api_key = resolve_api_key(slug)?;
    let client = http_client(timeout_for(slug))?;
    // Gemini auth: ?key=<api_key> (not Bearer header).
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        provider_base_url(slug, "https://generativelanguage.googleapis.com"),
        req.model,
        api_key
    );

    // Map MessageRole → Gemini role names: user stays "user", assistant
    // becomes "model"; system turns are folded into systemInstruction.
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| !matches!(m.role, MessageRole::System))
        .map(|m| {
            let role = match m.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "model",
                MessageRole::System => "user", // filtered above
            };
            // Gemini `parts` array: the text part, then one `inlineData` part
            // per attached image (SPEC-20 T-FOOD-02 multimodal). inlineData
            // carries `{mimeType, data}` where `data` is the raw base64 (no
            // `data:` URL prefix) — exactly what `MessageImage.data_b64` holds.
            let mut parts: Vec<serde_json::Value> =
                vec![serde_json::json!({"text": m.content})];
            for img in &m.images {
                parts.push(serde_json::json!({
                    "inlineData": {
                        "mimeType": img.mime,
                        "data": img.data_b64,
                    }
                }));
            }
            serde_json::json!({
                "role": role,
                "parts": parts,
            })
        })
        .collect();

    let mut body = serde_json::json!({"contents": contents});
    if let Some(sys) = &req.system_prompt {
        body["systemInstruction"] = serde_json::json!({
            "parts": [{"text": sys}],
        });
    }
    let mut gen_cfg = serde_json::Map::new();
    if let Some(t) = req.temperature {
        gen_cfg.insert("temperature".into(), serde_json::json!(t));
    }
    if let Some(mt) = req.max_tokens {
        gen_cfg.insert("maxOutputTokens".into(), serde_json::json!(mt));
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        gen_cfg.insert(
            "responseMimeType".into(),
            serde_json::json!("application/json"),
        );
    }
    if !gen_cfg.is_empty() {
        body["generationConfig"] = serde_json::Value::Object(gen_cfg);
    }
    // Gemini native tools wire: `tools:[{functionDeclarations:[...]}]`.
    // Omitted entirely when the caller passed no tools.
    if let Some(tools) = gemini_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async { client.post(&url).json(&body).send().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("gemini send failed: {}", e),
        }
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("gemini body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("gemini response parse failed: {}", e),
        })?;
    let completion = json
        .pointer("/candidates/0/content/parts")
        .and_then(|v| v.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let tokens_in = json
        .pointer("/usageMetadata/promptTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usageMetadata/candidatesTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used: req.model.clone(),
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None,
    })
}

/// SPEC-14 §9.2 — POST Cerebras' OpenAI-compatible `/v1/chat/completions`
/// (`https://api.cerebras.ai`, Bearer auth). Same `choices[]` / `usage`
/// envelope as OpenAI/Groq, so the body builder + response parser are
/// identical — only the host + the api-key slug differ.
fn complete_cerebras_pseudo(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "cerebras";
    let api_key = resolve_api_key(slug)?;
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/v1/chat/completions",
        provider_base_url(slug, "https://api.cerebras.ai")
    );

    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    if let Some(tools) = groq_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async {
        client.post(url).bearer_auth(&api_key).json(&body).send().await
    })
    .map_err(|e| ProviderError::NetworkError {
        detail: format!("cerebras send failed: {}", e),
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("cerebras body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("cerebras response parse failed: {}", e),
        })?;
    let completion = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None,
    })
}

fn complete_opencode_pseudo(_req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    // TODO Stage 4: ~30 LOC std::process::Command shell-out — no HTTP. Spawn
    // `opencode chat --json` with stdin JSON, parse stdout JSON. Mirrors the
    // claude/codex CLI providers below.
    Err(stage4_unimplemented(
        "opencode",
        "~30 LOC std::process::Command shell-out to `opencode` CLI; parse stdout JSON",
    ))
}

/// SPEC-14 §9.2 — POST Cloudflare Workers AI. UNLIKE the openai-compat backends,
/// Workers AI puts the model in the PATH (`/client/v4/accounts/<account_id>/ai/
/// run/<model>`), Bearer-auths with the API token, and wraps the reply in a
/// `{"result": {"response": "...", "usage": {...}}, "success": ...}` envelope
/// (not `choices[]`). The account id comes from `SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID`.
fn complete_cloudflare_pseudo(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "cloudflare";
    let api_key = resolve_api_key(slug)?;
    let account_id = resolve_cloudflare_account_id()?;
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/client/v4/accounts/{}/ai/run/{}",
        provider_base_url(slug, "https://api.cloudflare.com"),
        account_id,
        req.model
    );

    // Workers AI chat models accept the OpenAI-style `messages` array.
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({ "messages": messages });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }

    let start = std::time::Instant::now();
    let resp = block_on_async(async {
        client.post(url).bearer_auth(&api_key).json(&body).send().await
    })
    .map_err(|e| ProviderError::NetworkError {
        detail: format!("cloudflare send failed: {}", e),
    })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("cloudflare body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("cloudflare response parse failed: {}", e),
        })?;
    // Workers AI wraps the completion under `result.response` (NOT choices[]).
    let completion = json
        .pointer("/result/response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tokens_in = json
        .pointer("/result/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/result/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        // Workers AI does not echo the model in the body — report the requested one.
        text: completion,
        model_used: req.model.clone(),
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None,
    })
}

/// Resolve the Cloudflare account id (Workers AI URLs are account-scoped) from
/// `SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID` / `CLOUDFLARE_ACCOUNT_ID`. Mirrors
/// `resolve_api_key`'s env-bridge shape; missing → `AuthError`.
fn resolve_cloudflare_account_id() -> Result<String, ProviderError> {
    let candidates = ["SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_ACCOUNT_ID"];
    for k in candidates {
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
    }
    Err(ProviderError::AuthError {
        detail: format!(
            "no cloudflare account id (tried env vars: {})",
            candidates.join(", ")
        ),
    })
}

/// SPEC-14 §9.2 — POST a LOCAL Ollama server's OpenAI-compatible
/// `/v1/chat/completions` endpoint (default host `http://localhost:11434`).
/// Desktop-only Local-class provider: NO api key and NO Bearer header (mobile
/// platforms are filtered out by ProviderClass::Local at routing time, see
/// filter_chain_by_class_latency). The OpenAI-compat shim returns the same
/// `choices[]` / `usage` envelope as Groq/OpenAI, so the body builder and the
/// response parser are identical — only the host + the absence of auth differ.
///
/// 中文: 真實 reqwest 呼叫本機 Ollama 的 OpenAI-compat `/v1/chat/completions`；
/// Local-class、不需 api key、不送 Bearer header；body 與回應 envelope 同 Groq。
fn complete_ollama_pseudo(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "ollama";
    // No resolve_api_key — Ollama is keyless. NEVER send a Bearer header.
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/v1/chat/completions",
        provider_base_url(slug, "http://localhost:11434")
    );

    // OpenAI-shape messages: prepend system if present, then verbatim turns.
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    if let Some(tools) = groq_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    // No `.bearer_auth(...)` — Ollama serves locally without credentials.
    let resp = block_on_async(async { client.post(url).json(&body).send().await })
        .map_err(|e| ProviderError::NetworkError {
            detail: format!("ollama send failed: {}", e),
        })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("ollama body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("ollama response parse failed: {}", e),
        })?;
    let completion = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None, // Local provider — no cost reported.
    })
}

fn complete_claude_cli_pseudo(_req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    // TODO Stage 4: ~30 LOC std::process::Command shell-out — spawn
    // ~/.claude/local/claude with stdin JSON; parse stdout JSON; MCP tool
    // wire piggybacks the same channel.
    Err(stage4_unimplemented(
        "claude",
        "~30 LOC std::process::Command shell-out to `~/.claude/local/claude`",
    ))
}

fn complete_codex_cli_pseudo(_req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    // TODO Stage 4: ~30 LOC std::process::Command shell-out — spawn `codex`
    // CLI with stdin JSON; parse OpenAI-shape JSON from stdout.
    Err(stage4_unimplemented(
        "codex",
        "~30 LOC std::process::Command shell-out to `codex` CLI",
    ))
}

/// SPEC-14 §9.2 — POST a LOCAL llama.cpp server's OpenAI-compatible
/// `/v1/chat/completions` endpoint (default `http://localhost:8080`). Like
/// Ollama: Local-class, keyless, NO Bearer header. llama-server exposes the
/// same `choices[]` / `usage` OpenAI-compat envelope, so the body builder +
/// parser match Ollama exactly — only the default host differs. (We use the
/// OpenAI-compat route, not the native `/completion`, for one parser shared
/// across every openai-shape backend.)
fn complete_llamacpp_pseudo(req: &ProviderRequest) -> Result<ProviderResponse, ProviderError> {
    let slug = "llamacpp";
    // No resolve_api_key — llama-server is keyless. NEVER send a Bearer header.
    let client = http_client(timeout_for(slug))?;
    let url = format!(
        "{}/v1/chat/completions",
        provider_base_url(slug, "http://localhost:8080")
    );

    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        messages.push(serde_json::json!({"role": role, "content": m.content}));
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(mt) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(mt);
    }
    if matches!(req.response_format, ResponseFormat::Json | ResponseFormat::Structured) {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    if let Some(tools) = groq_tools_json(&req.tools) {
        body["tools"] = tools;
    }

    let start = std::time::Instant::now();
    // No `.bearer_auth(...)` — llama-server serves locally without credentials.
    let resp = block_on_async(async { client.post(url).json(&body).send().await })
        .map_err(|e| ProviderError::NetworkError {
            detail: format!("llamacpp send failed: {}", e),
        })?;

    let status = resp.status().as_u16();
    let text = block_on_async(async { resp.text().await }).map_err(|e| {
        ProviderError::NetworkError {
            detail: format!("llamacpp body read failed: {}", e),
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(map_http_status(slug, status, &text));
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProviderError::Unknown {
            detail: format!("llamacpp response parse failed: {}", e),
        })?;
    let completion = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_used = json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.model)
        .to_string();
    let tokens_in = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let tokens_out = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Ok(ProviderResponse {
        text: completion,
        model_used,
        tokens_in,
        tokens_out,
        latency_ms: start.elapsed().as_millis() as u64,
        cost_usd: None, // Local provider — no cost reported.
    })
}

fn compute_cost_usd(
    provider: ProviderType,
    model: &str,
    tokens_in: u32,
    tokens_out: u32,
) -> Option<f64> {
    // Local backends never report cost (free / on-device).
    if classify(provider) == ProviderClass::Local {
        return None;
    }
    let v = read_agents_toml().ok()?;
    let by_model = v.pricing.get(provider.slug())?;
    let row = by_model.get(model)?;
    let in_usd = (tokens_in as f64) * row.input_per_1k_usd / 1000.0;
    let out_usd = (tokens_out as f64) * row.output_per_1k_usd / 1000.0;
    Some(in_usd + out_usd)
}

/// SPEC-14 §9 reachability ping. Sends a HEAD (falls back to GET on 405)
/// to `config.base_url` or the provider's built-in default. 2xx / 3xx / 401
/// / 403 / 404 all count as "endpoint up" (the upstream is responding even
/// if our key is wrong / model missing — those surface at dispatch time).
/// Only network errors (DNS / TLS / timeout) and 5xx are treated as down.
///
/// 中文: 連線測試 — HEAD/GET 上游 endpoint；2xx-4xx 都算「連得到」（401/404
/// 算 auth/model 問題、不是連線問題）；只有 5xx / DNS / TLS / timeout 才算
/// down。base_url 若未設定就用 provider 內建預設。
fn provider_ping(config: &ProviderConfig) -> Result<(), ProviderError> {
    let slug = config.slug.as_str();
    let pt = ProviderType::from_str(slug)?;
    let url = config
        .base_url
        .clone()
        .unwrap_or_else(|| default_base_url_for(pt).to_string());
    let client = http_client(config.timeout_ms.max(1000))?;
    let resp = block_on_async(async { client.head(&url).send().await });
    let status = match resp {
        Ok(r) => r.status().as_u16(),
        Err(e) => {
            // Some hosts disallow HEAD entirely — retry once with GET.
            let retry = block_on_async(async { client.get(&url).send().await });
            match retry {
                Ok(r) => r.status().as_u16(),
                Err(_) => {
                    return Err(ProviderError::NetworkError {
                        detail: format!("{} ping failed: {}", slug, e),
                    });
                }
            }
        }
    };
    // 5xx is down; everything else is "endpoint reachable" (auth/model
    // mismatches show up at dispatch time, not at ping time).
    if (500..600).contains(&status) {
        return Err(ProviderError::NetworkError {
            detail: format!("{} ping returned {}", slug, status),
        });
    }
    Ok(())
}

/// Built-in default base URL per provider — used when ProviderConfig.base_url
/// is None. Mirrors the URL constants in the live `complete_*` helpers
/// above so a single source-of-truth change moves both.
fn default_base_url_for(pt: ProviderType) -> &'static str {
    match pt {
        ProviderType::Groq => "https://api.groq.com/openai/v1/chat/completions",
        ProviderType::Openai => "https://api.openai.com/v1/chat/completions",
        ProviderType::Anthropic => "https://api.anthropic.com/v1/messages",
        ProviderType::Gemini => "https://generativelanguage.googleapis.com/v1beta/models",
        ProviderType::Cerebras => "https://api.cerebras.ai/v1/chat/completions",
        ProviderType::Opencode => "https://opencode.ai",
        ProviderType::Cloudflare => "https://api.cloudflare.com/client/v4",
        ProviderType::Ollama => "http://localhost:11434",
        ProviderType::Claude => "http://localhost:0", // CLI shell, never pinged
        ProviderType::Codex => "http://localhost:0",  // CLI shell, never pinged
        ProviderType::Llamacpp => "http://localhost:8080",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_response_round_trip_smoke() {
        // §7.1 invariant: JSON encode → wire → JSON decode preserves the
        // unified response shape. Stage 1 sanity-checks serde + ts-rs derive
        // alignment; deeper invariants (e.g. tokens_in + tokens_out matches
        // upstream usage block) come in Stage 2 with real provider impls.
        let r = ProviderResponse {
            text: "hello world".to_string(),
            model_used: "llama-3.1-8b-instant".to_string(),
            tokens_in: 42,
            tokens_out: 17,
            latency_ms: 350,
            cost_usd: Some(0.00012),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: ProviderResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(r.text, back.text);
        assert_eq!(r.model_used, back.model_used);
        assert_eq!(r.tokens_in, back.tokens_in);
        assert_eq!(r.tokens_out, back.tokens_out);
        assert_eq!(r.latency_ms, back.latency_ms);
        assert_eq!(r.cost_usd, back.cost_usd);
    }

    #[test]
    fn score_has_five_factors() {
        // RECONCILE invariant (T-PROV-01, SPEC-14 §9.2 / G4): the resolver
        // MUST score on exactly 5 factors — class, latency, modality, cost,
        // ttft. The parallel-module fork dropped this to a priority-only walk;
        // this canary asserts all 5 factors are present + named, so a future
        // edit that silently re-drops one breaks the build.
        assert_eq!(
            ProviderScoreFactors::FACTOR_NAMES.len(),
            5,
            "SPEC-14 §9.2 mandates exactly 5 scoring factors"
        );
        assert_eq!(
            ProviderScoreFactors::FACTOR_NAMES,
            ["class_match", "latency_match", "modality_match", "cost", "ttft"]
        );

        // Construct via all 5 fields — fails to compile if any field is dropped.
        let f = ProviderScoreFactors {
            class_match: 1.0,
            latency_match: 1.0,
            modality_match: 1.0,
            cost: 0.0,
            ttft: 0.0,
        };
        // Perfect-match, zero-cost/ttft provider scores the sum of the
        // positive weights (0.40 + 0.25 + 0.20 = 0.85).
        let s = score(&f);
        assert!((s - 0.85).abs() < 1e-9, "perfect-match score = {}", s);

        // Cost + ttft penalise (factors 4 + 5 are subtractive).
        let penalised = score(&ProviderScoreFactors { cost: 1.0, ttft: 1.0, ..f });
        assert!(penalised < s, "cost+ttft must reduce score: {} !< {}", penalised, s);
    }

    #[test]
    fn same_class_models_rank_by_cost_and_ttft() {
        // T-PROV-03 invariant: within ONE ProviderClass, the per-model cost +
        // ttft table must let a cheaper/faster model outrank a pricier/slower
        // one — so ranking no longer ties and falls back to chain priority.
        //
        // Two FRONTIER models: claude-haiku (cheap + fast) vs claude-opus
        // (pricey + slow). Same class, same latency request → score must
        // differ, with the cheaper/faster model strictly higher.
        let class = ProviderClass::Frontier;
        let latency = LatencyClass::Interactive;

        let cheap_fast = score(&score_factors_for_model(
            ProviderType::Anthropic,
            "claude-haiku",
            class,
            latency,
        ));
        let pricey_slow = score(&score_factors_for_model(
            ProviderType::Anthropic,
            "claude-opus",
            class,
            latency,
        ));
        assert!(
            cheap_fast > pricey_slow,
            "cheaper/faster model must outrank pricier/slower in same class: \
             haiku={} !> opus={}",
            cheap_fast,
            pricey_slow
        );

        // Two COMMODITY models on Groq: 8B (cheapest+fastest) vs 70B.
        let small = score(&score_factors_for_model(
            ProviderType::Groq,
            "llama-3.1-8b-instant",
            ProviderClass::Commodity,
            latency,
        ));
        let big = score(&score_factors_for_model(
            ProviderType::Groq,
            "llama-3.3-70b-versatile",
            ProviderClass::Commodity,
            latency,
        ));
        assert!(
            small > big,
            "cheaper/faster commodity model must rank higher: 8b={} !> 70b={}",
            small,
            big
        );

        // No-regression: an UNKNOWN model must fall back to the class
        // heuristic (cost=1.0, ttft=1.0 for frontier) — identical to the
        // pre-T-PROV-03 behaviour, so unknown models never crash or skew.
        let unknown = score_factors_for_model(
            ProviderType::Anthropic,
            "some-unreleased-model-xyz",
            class,
            latency,
        );
        assert!((unknown.cost - 1.0).abs() < 1e-9, "unknown frontier cost = {}", unknown.cost);
        assert!((unknown.ttft - 1.0).abs() < 1e-9, "unknown frontier ttft = {}", unknown.ttft);
    }

    #[test]
    fn latency_class_reasoning_round_trips() {
        // RECONCILE invariant (T-PROV-01): the `Reasoning` third latency class
        // (dropped in the fork) is restored and serialises as snake_case.
        let j = serde_json::to_string(&LatencyClass::Reasoning).unwrap();
        assert_eq!(j, "\"reasoning\"");
        let back: LatencyClass = serde_json::from_str(&j).unwrap();
        assert_eq!(back, LatencyClass::Reasoning);
    }

    #[test]
    fn reconciled_enums_round_trip() {
        // RECONCILE invariant (T-PROV-01): PromptStyle / SystemPlacement /
        // ToolCallFormat / Modality / ReasoningEffort were dropped in the fork.
        // Smoke-check serde round-trip on a representative variant of each.
        let style: PromptStyle =
            serde_json::from_str(&serde_json::to_string(&PromptStyle::XmlTags).unwrap()).unwrap();
        assert_eq!(style, PromptStyle::XmlTags);

        let placement: SystemPlacement =
            serde_json::from_str(&serde_json::to_string(&SystemPlacement::SeparateParam).unwrap())
                .unwrap();
        assert_eq!(placement, SystemPlacement::SeparateParam);

        let fmt: ToolCallFormat =
            serde_json::from_str(&serde_json::to_string(&ToolCallFormat::Mcp).unwrap()).unwrap();
        assert_eq!(fmt, ToolCallFormat::Mcp);

        // Modality + ReasoningEffort are lowercase on the wire.
        assert_eq!(serde_json::to_string(&Modality::Image).unwrap(), "\"image\"");
        assert_eq!(serde_json::to_string(&ReasoningEffort::High).unwrap(), "\"high\"");
    }

    #[test]
    fn system_placement_for_provider_branches() {
        // SPEC-14 §7.1: Anthropic / Claude → separate `system:` param.
        assert_eq!(
            system_placement_for_provider("anthropic", None),
            SystemPlacement::SeparateParam
        );
        assert_eq!(
            system_placement_for_provider("opencode", Some("claude-sonnet-4-6")),
            SystemPlacement::SeparateParam
        );
        // On-device / local → embed into the first user turn.
        assert_eq!(
            system_placement_for_provider("ollama", None),
            SystemPlacement::EmbedInUserTurn
        );
        assert_eq!(
            system_placement_for_provider("llamacpp", None),
            SystemPlacement::EmbedInUserTurn
        );
        // OpenAI / Groq / Gemini-chat / unknown → messages[0].role = "system".
        assert_eq!(
            system_placement_for_provider("openai", Some("gpt-5.5")),
            SystemPlacement::RoleSystem
        );
        assert_eq!(
            system_placement_for_provider("groq", Some("llama-3.1-8b-instant")),
            SystemPlacement::RoleSystem
        );
        assert_eq!(
            system_placement_for_provider("some-new-provider", None),
            SystemPlacement::RoleSystem
        );
    }

    #[test]
    fn provider_type_from_str_groq() {
        // §7.1 invariant: `ProviderType::from_str("groq")` round-trips via
        // `slug()`. Case-insensitive on input — `Groq` / `GROQ` / `groq` all
        // parse to the same variant.
        let p = ProviderType::from_str("groq").expect("groq must parse");
        assert_eq!(p, ProviderType::Groq);
        assert_eq!(p.slug(), "groq");

        // Case-insensitivity smoke (agents.toml may have user typos).
        assert_eq!(ProviderType::from_str("GROQ").unwrap(), ProviderType::Groq);
        assert_eq!(ProviderType::from_str("Groq").unwrap(), ProviderType::Groq);

        // Unknown slug surfaces as ModelNotFound (carries the bad input
        // for debug logging — caller MUST NOT echo this to UI verbatim).
        let err = ProviderType::from_str("totally-fake-provider").unwrap_err();
        match err {
            ProviderError::ModelNotFound { detail } => {
                assert!(detail.contains("totally-fake-provider"), "detail: {}", detail);
            }
            other => panic!("expected ModelNotFound, got {:?}", other),
        }
    }

    #[test]
    fn provider_type_slug_round_trip_all_variants() {
        // §7.1 invariant: every variant's slug round-trips through FromStr.
        // Adding a variant without updating FromStr breaks this canary.
        use ProviderType::*;
        for p in [Groq, Openai, Anthropic, Gemini, Cerebras, Opencode,
                  Cloudflare, Ollama, Claude, Codex, Llamacpp] {
            let s = p.slug();
            assert_eq!(p, ProviderType::from_str(s).expect("parse"), "rt: {}", s);
        }
    }

    #[test]
    fn provider_error_serializes_with_code_tag() {
        // §11 invariant: error wire shape uses `{"code": "..."}` tag so the
        // UI can dispatch on the machine-readable code string (mirrors
        // identity_wire.rs KeyDerivationError convention).
        let e = ProviderError::RateLimit {
            detail: "429 from upstream".to_string(),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("rate_limit"), "wire shape: {}", j);
        assert!(j.contains("429 from upstream"), "payload preserved: {}", j);

        let e2 = ProviderError::ContextTooLong {
            tokens: 200_000,
            limit: 128_000,
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("context_too_long"), "wire shape: {}", j2);
    }

    #[test]
    fn message_role_snake_case_wire() {
        // §7.1 invariant: MessageRole serialises as snake_case lowercase to
        // match the OpenAI-shape `role` field in upstream wire.
        let m = Message::text(MessageRole::Assistant, "ok");
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"assistant\""), "wire shape: {}", j);
        // Backward-compat: a text-only Message omits the `images` key entirely
        // (skip_serializing_if), so legacy consumers see the same wire shape.
        assert!(!j.contains("images"), "text-only wire stays legacy: {}", j);
    }

    #[test]
    fn fallback_chain_round_trip() {
        // §7.1 invariant: FallbackChain is a thin Vec<String> wrapper —
        // verify it round-trips and preserves order (resolver iterates in
        // index order, so any reordering breaks priority semantics).
        let c = FallbackChain {
            providers: vec!["groq".to_string(), "openai".to_string(), "ollama".to_string()],
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: FallbackChain = serde_json::from_str(&j).unwrap();
        assert_eq!(c.providers, back.providers);
        assert_eq!(back.providers[0], "groq", "priority-0 must stay first");
    }

    // ─── Stage 3 KAT (known-answer-test) tests ───────────────────────────
    //
    // These replace the Stage 2 `#[should_panic(expected = "Stage 3")]`
    // markers. They exercise the now-real config-side helpers against
    // hand-rolled agents.toml fixtures and assert the structural invariants
    // (fallback chain order preserved, class filter drops mismatches,
    // model → ProviderType resolution, pricing computation, circuit-
    // breaker default-alive behaviour).

    /// Test-only helper: write a temp agents.toml + point the env var at it
    /// for the duration of one test. Cleaned up by RAII on drop.
    struct TempAgentsToml {
        _tmp: tempfile::TempDir,
        // Held for the guard's lifetime so concurrent tests don't clobber the
        // process-global SPECTYN_MESH_AGENTS_TOML env var. Drop order: the
        // explicit Drop impl below runs first (removes the var), then fields
        // drop in declaration order (_tmp, then _lock) — so the var is removed
        // while the lock is still held.
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl TempAgentsToml {
        fn new(body: &str) -> Self {
            let lock = crate::sandbox::test_lock();
            let tmp = tempfile::tempdir().expect("tempdir");
            let p = tmp.path().join("agents.toml");
            std::fs::write(&p, body).expect("write agents.toml");
            std::env::set_var("SPECTYN_MESH_AGENTS_TOML", &p);
            Self { _tmp: tmp, _lock: lock }
        }
    }
    impl Drop for TempAgentsToml {
        fn drop(&mut self) {
            std::env::remove_var("SPECTYN_MESH_AGENTS_TOML");
        }
    }

    #[test]
    fn load_fallback_chain_reads_routing_table() {
        // §9.2 invariant: load_fallback_chain returns the toml fallback_chain
        // in declaration order. Index 0 = highest priority — the resolver
        // walks the chain in order so any reordering breaks routing.
        let _guard = TempAgentsToml::new(
            r#"
[routing]
fallback_chain = ["groq", "openai", "ollama"]
"#,
        );
        let chain = load_fallback_chain().expect("load");
        assert_eq!(chain.providers, vec!["groq", "openai", "ollama"]);
    }

    #[test]
    fn filter_chain_keeps_only_matching_class() {
        // §7.1 invariant: filter drops slugs whose ProviderClass != requested.
        // groq+cerebras are Commodity; openai+anthropic are Frontier;
        // ollama+llamacpp are Local. Asking for Commodity must drop the
        // frontier + local entries while preserving chain order.
        let chain = FallbackChain {
            providers: vec![
                "openai".to_string(),
                "groq".to_string(),
                "ollama".to_string(),
                "cerebras".to_string(),
            ],
        };
        let kept = filter_chain_by_class_latency(
            &chain,
            ProviderClass::Commodity,
            LatencyClass::Interactive,
        );
        assert_eq!(kept, vec!["groq", "cerebras"], "commodity filter dropped frontier+local");
    }

    #[test]
    fn score_factors_rank_better_fit_above_worse_fit() {
        // T-PROV-02 (SPEC-14 §9.2 / G4): the reconciled 5-factor score() must
        // now drive selection. Cross-class proof — for an Interactive request,
        // a Local provider (free, fast TTFT) out-scores a Frontier provider
        // (pricey, slow TTFT) once cost+ttft penalties + latency_match apply.
        // class_match is held equal at 0.0 vs 0.0 here by asking each provider
        // for ITS OWN class so the differentiator is the cost/latency factors.
        let ollama = score(&score_factors_for(
            "ollama",
            ProviderClass::Local,
            LatencyClass::Interactive,
        ));
        let openai = score(&score_factors_for(
            "openai",
            ProviderClass::Frontier,
            LatencyClass::Interactive,
        ));
        assert!(
            ollama > openai,
            "interactive: cheap+fast local ({}) must out-score pricey+slow frontier ({})",
            ollama,
            openai
        );

        // Same two providers for a Reasoning request flip: frontier brains win
        // because latency_match favours frontier and cost matters less.
        let ollama_r = score(&score_factors_for(
            "ollama",
            ProviderClass::Local,
            LatencyClass::Reasoning,
        ));
        let openai_r = score(&score_factors_for(
            "openai",
            ProviderClass::Frontier,
            LatencyClass::Reasoning,
        ));
        assert!(
            openai_r > ollama_r,
            "reasoning: frontier ({}) must out-score local ({})",
            openai_r,
            ollama_r
        );
    }

    #[test]
    fn select_provider_ranks_by_score_then_priority() {
        // T-PROV-02: select_provider now sorts the filtered chain by score()
        // (descending) with a STABLE tie-break on chain order. Within one
        // class every candidate scores equally today, so the highest-priority
        // (index-0) slug must still win — proving no regression vs the old
        // priority-only walk.
        let _guard = TempAgentsToml::new(
            r#"
[routing]
fallback_chain = ["groq", "cerebras", "openai"]
"#,
        );
        let picked = select_provider(
            ProviderClass::Commodity,
            LatencyClass::Interactive,
        )
        .expect("commodity request resolves");
        assert_eq!(
            picked, "groq",
            "score tie within a class must preserve priority order (index 0)"
        );
    }

    #[test]
    fn resolve_model_to_provider_type_scans_models_list() {
        // §7.1 invariant: resolver scans both `default_model` and `models[]`
        // lists per `[providers.X]` section. A model declared in `models`
        // must resolve to that section's provider type.
        let _guard = TempAgentsToml::new(
            r#"
[providers.groq]
default_model = "llama-3.1-8b-instant"
models = ["llama-3.1-70b-versatile", "mixtral-8x7b-32768"]

[providers.openai]
default_model = "gpt-4o"
"#,
        );
        let p = resolve_model_to_provider_type("llama-3.1-70b-versatile").expect("resolve");
        assert_eq!(p, ProviderType::Groq);
        let q = resolve_model_to_provider_type("gpt-4o").expect("resolve");
        assert_eq!(q, ProviderType::Openai);

        // Unknown model surfaces as ModelNotFound (carries the bad input
        // for debug logs). "imaginary-model-v9" matches no family prefix.
        let err = resolve_model_to_provider_type("imaginary-model-v9").unwrap_err();
        assert!(matches!(err, ProviderError::ModelNotFound { .. }));
    }

    #[test]
    fn infer_provider_from_model_prefix_maps_known_families() {
        assert_eq!(infer_provider_from_model_prefix("gemini-2.5-flash"), Some("gemini"));
        assert_eq!(infer_provider_from_model_prefix("claude-opus-4.7"), Some("anthropic"));
        assert_eq!(infer_provider_from_model_prefix("gpt-5.5"), Some("openai"));
        assert_eq!(infer_provider_from_model_prefix("o1-preview"), Some("openai"));
        assert_eq!(infer_provider_from_model_prefix("llama-3.1-8b-instant"), Some("groq"));
        assert_eq!(infer_provider_from_model_prefix("totally-unknown-xyz"), None);
    }

    #[test]
    fn resolve_model_falls_back_to_prefix_when_undeclared() {
        // A model not in any [providers.X] section still resolves via the
        // family prefix, so callers can use a standard id without declaring it.
        let _guard = TempAgentsToml::new(
            r#"
[providers.groq]
default_model = "llama-3.1-8b-instant"
"#,
        );
        let p = resolve_model_to_provider_type("gemini-2.5-flash").expect("prefix fallback resolves");
        assert_eq!(p, ProviderType::Gemini);
    }

    #[test]
    fn compute_cost_usd_reads_pricing_table() {
        // §7.1 invariant: cost = (tokens_in * in_rate + tokens_out * out_rate)
        // / 1000. The pricing table is keyed by provider slug + model id;
        // missing entries return None (caller must not block on cost).
        let _guard = TempAgentsToml::new(
            r#"
[pricing.groq."llama-3.1-8b-instant"]
input_per_1k_usd = 0.05
output_per_1k_usd = 0.08
"#,
        );
        let cost = compute_cost_usd(
            ProviderType::Groq,
            "llama-3.1-8b-instant",
            1000,
            2000,
        )
        .expect("priced");
        // 1000 * 0.05/1000 + 2000 * 0.08/1000 = 0.05 + 0.16 = 0.21
        assert!((cost - 0.21).abs() < 1e-9, "cost: {}", cost);

        // Local provider always returns None regardless of pricing table.
        let none = compute_cost_usd(ProviderType::Ollama, "llama3", 1000, 2000);
        assert_eq!(none, None, "local providers never report cost");

        // Unknown model returns None (graceful, not error).
        let unk = compute_cost_usd(ProviderType::Groq, "ghost-model", 100, 100);
        assert_eq!(unk, None);
    }

    #[test]
    fn provider_alive_defaults_to_true_for_unseen_slug() {
        // §9.2 invariant: a slug never marked as failed is alive. The
        // circuit-breaker cache is opt-in pessimism — absence means "go".
        // We use a unique slug per test to avoid cross-test contamination
        // of the process-wide static cache.
        assert!(provider_alive("__test_unseen_provider_slug__"));
    }

    #[test]
    fn wire_breaker_opens_after_threshold_then_success_revives() {
        // Unique slug to avoid cross-test contamination of the process static.
        let slug = "__p0_5_wire_breaker_slug__";
        // Below threshold: still alive.
        record_provider_failure(slug);
        record_provider_failure(slug);
        assert!(provider_alive(slug), "2 transient failures < threshold 3");
        // Reaching threshold trips it.
        record_provider_failure(slug);
        assert!(!provider_alive(slug), "3rd transient failure opens breaker");
        // A success revives it immediately (count reset + Closed).
        record_provider_success(slug);
        assert!(provider_alive(slug), "success closes the breaker");
    }

    #[test]
    fn walk_fallback_chain_empty_is_fallback_exhausted() {
        let template = ProviderRequest {
            model: String::new(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: vec![],
        };
        let err = walk_fallback_chain(
            &[],                                   // empty chain
            |_| Some("m".to_string()),
            |_| true,
            |_| {},
            |_| 0.0,
            |_| Err(ProviderError::Unknown { detail: "unreached".into() }),
            &template,
        )
        .unwrap_err();
        assert!(matches!(err, ProviderError::FallbackExhausted { .. }));
    }

    #[test]
    fn walk_fallback_chain_all_fail_is_fallback_exhausted_with_last_error() {
        let template = ProviderRequest {
            model: String::new(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: vec![],
        };
        let mut downs = 0;
        let err = walk_fallback_chain(
            &["groq".to_string(), "openai".to_string()],
            |_| Some("m".to_string()),
            |_| true,                              // both alive
            |_| downs += 1,                        // both marked down
            |_| 0.0,
            |_| Err(ProviderError::NetworkError { detail: "down".into() }),
            &template,
        )
        .unwrap_err();
        assert_eq!(downs, 2, "both transient failures marked down");
        match err {
            ProviderError::FallbackExhausted { detail } => {
                assert!(detail.contains("network_error"), "carries last error: {detail}");
            }
            other => panic!("expected FallbackExhausted, got {other:?}"),
        }
    }

    // ─── Stage 4 marker tests ────────────────────────────────────────────
    //
    // Real HTTP now: groq/anthropic/gemini (Stage 3) + openai/ollama/cerebras/
    // llamacpp/cloudflare (Stage 4). The 3 remaining helpers (opencode /
    // claude_cli / codex_cli) are still pseudocode — all CLI shell-outs (driven
    // via the cli_session substrate, not an HTTP completion) — but each surfaces
    // a typed Err (NOT a panic) so chain dispatch can fail over. One marker
    // canary keeps the staging contract auditable: when the canary's provider
    // gets promoted, this test flips to fail and the next stub must take over.

    #[test]
    fn complete_returns_typed_err_at_stage_4_for_opencode() {
        // Stage 4 marker — proves a still-unpromoted stub surfaces its
        // "Stage 4: ..." TODO as a typed Err (NOT a panic) through `complete()`.
        // Real HTTP now: groq/anthropic/gemini + openai/ollama/cerebras/llamacpp
        // + cloudflare (Workers AI). The 3 remaining stubs are CLI shell-outs
        // (opencode/claude/codex — driven via the cli_session substrate, not an
        // HTTP completion); opencode anchors this staging-contract canary. When
        // it gets a real impl, this test flips and the next stub takes over.
        let _guard = TempAgentsToml::new(
            r#"
[providers.opencode]
default_model = "opencode-model"
"#,
        );
        let err = complete(ProviderRequest {
            model: "opencode-model".to_string(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        })
        .expect_err("opencode is still a Stage-4 stub — complete() must Err");
        match err {
            ProviderError::Unknown { detail } => assert!(
                detail.contains("Stage 4"),
                "stub Err must carry the Stage-4 TODO marker: {detail}"
            ),
            other => panic!("expected Unknown stage-4 stub error, got {other:?}"),
        }
    }

    #[test]
    fn resolve_api_key_reads_env_var() {
        // §9.2 Stage 3 helper — confirms env-var path works as the bridge
        // before SPEC-13 age vault lands. Uses a unique slug to avoid
        // colliding with any developer's real env.
        std::env::set_var("SPECTYN_MESH_TESTSLUG_API_KEY", "test-key-xyz");
        let k = resolve_api_key("testslug").expect("env lookup");
        assert_eq!(k, "test-key-xyz");
        std::env::remove_var("SPECTYN_MESH_TESTSLUG_API_KEY");

        // Missing env → AuthError carrying the candidate names so the
        // operator can see what to set.
        let err = resolve_api_key("nonexistentslug").unwrap_err();
        match err {
            ProviderError::AuthError { detail } => {
                assert!(detail.contains("SPECTYN_MESH_NONEXISTENTSLUG_API_KEY"), "detail: {}", detail);
            }
            other => panic!("expected AuthError, got {:?}", other),
        }
    }

    #[test]
    fn default_base_url_for_covers_all_variants() {
        // §9 invariant: every ProviderType MUST have a default base URL so
        // provider_ping can fall back when ProviderConfig.base_url is None.
        use ProviderType::*;
        for p in [Groq, Openai, Anthropic, Gemini, Cerebras, Opencode,
                  Cloudflare, Ollama, Claude, Codex, Llamacpp] {
            let u = default_base_url_for(p);
            assert!(!u.is_empty(), "missing default base URL for {:?}", p);
        }
    }

    // ─── SPEC-14 §6 / SPEC-23 §11.1 — end-to-end coach failover proof ─────────
    //
    // The coach engine (`coach_wire::call_providers_complete`) drives every
    // tomorrow-action LLM call through `complete_with_fallback`, mapping a
    // `FallbackExhausted` to `CoachError::LlmAllProvidersFailed` → the
    // SPEC-23 §11.1 degraded review. Until now NO test exercised the failover
    // LOOP itself; only the pure config helpers were covered. This test stands
    // up real wiremock HTTP backends (groq / gemini / anthropic all honour
    // `SPECTYN_MESH_<SLUG>_BASE_URL`, see `provider_base_url`) and proves the
    // two outcomes the coach degraded-path keys off:
    //   1. 429 on the first provider → failover to the next → SUCCESS.
    //   2. 429 on EVERY eligible provider → graceful `FallbackExhausted`
    //      (the concrete "all providers failed" signal coach degrades on),
    //      never a panic / hang / silent empty success.
    // Helper: build a minimal ProviderResponse tagged with which slug produced
    // it, so a test can assert WHICH provider in the chain actually won.
    fn resp_from(model: &str) -> ProviderResponse {
        ProviderResponse {
            text: "ok".to_string(),
            model_used: model.to_string(),
            tokens_in: 1,
            tokens_out: 1,
            latency_ms: 1,
            cost_usd: None,
        }
    }

    #[test]
    fn fallback_429_then_success_failover_keeps_walking_to_a_live_provider() {
        // CORE coach-failover guard: the FIRST provider returns a 429
        // (`RateLimit`), so the chain must FAIL OVER to the next provider and
        // return ITS success — never propagate the 429, never stop at slug 0.
        // This is the exact path coach_wire::call_providers_complete relies on
        // before it would otherwise have to degrade.
        let chain = vec!["groq".to_string(), "anthropic".to_string()];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: Some("coach".into()),
            messages: vec![Message::text(MessageRole::User, "tomorrow action?")],
            max_tokens: Some(16),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };

        // Record every slug we attempt so we can prove the walk order + that the
        // breaker was told about the 429'd provider.
        let attempted = std::cell::RefCell::new(Vec::<String>::new());
        let marked_down = std::cell::RefCell::new(Vec::<String>::new());

        let out = walk_fallback_chain(
            &chain,
            |slug| Some(format!("{slug}-model")), // every slug has a usable model
            |_slug| true,                          // breaker: all alive
            |slug| marked_down.borrow_mut().push(slug.to_string()),
            |_| 0.0, // neutral score → preserve chain order (tests failover, not ranking)
            |req| {
                attempted.borrow_mut().push(req.model.clone());
                if req.model == "groq-model" {
                    // First provider is rate-limited (429 → RateLimit).
                    Err(ProviderError::RateLimit {
                        detail: "groq 429: too many requests".into(),
                    })
                } else {
                    Ok(resp_from(&req.model))
                }
            },
            &template,
        )
        .expect("a 429 on the first provider must FAIL OVER, not propagate");

        assert_eq!(
            out.model_used, "anthropic-model",
            "failover must reach the 2nd (live) provider, not stop at the 429'd one"
        );
        assert_eq!(
            *attempted.borrow(),
            vec!["groq-model".to_string(), "anthropic-model".to_string()],
            "the walk must try groq (429) THEN anthropic (ok), in chain order"
        );
        assert_eq!(
            *marked_down.borrow(),
            vec!["groq".to_string()],
            "only the 429'd provider should be recorded as failed (breaker)"
        );
    }

    #[test]
    fn walk_fallback_chain_attempts_better_scored_slug_before_chain_order() {
        // T-PROV-04 GATE: failover must HONOUR score(), not raw chain order.
        // The chain is ["low-priority-first", "high-score-second"]; the slug
        // that appears SECOND (lower chain priority) is given a strictly BETTER
        // score. walk_fallback_chain must therefore pre-rank by score and
        // attempt the better-scored (2nd-in-chain) slug FIRST — mirroring
        // select_provider's (-score, chain_index) stable sort. Both providers
        // succeed, so the FIRST one attempted wins; we assert it is the
        // better-scored one, not the chain-leading one.
        let chain = vec!["chainfirst".to_string(), "betterscore".to_string()];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };

        let attempted = std::cell::RefCell::new(Vec::<String>::new());
        let out = walk_fallback_chain(
            &chain,
            |slug| Some(format!("{slug}-model")), // both usable
            |_slug| true,                          // both alive
            |_slug| {},
            // Lower-priority "betterscore" slug scores STRICTLY higher than the
            // chain-leading "chainfirst" slug. If walk_fallback_chain ignored
            // score() (old dead-code behaviour), it would attempt "chainfirst"
            // first and this test would fail.
            |slug| if slug == "betterscore" { 1.0 } else { 0.0 },
            |req| {
                attempted.borrow_mut().push(req.model.clone());
                Ok(resp_from(&req.model))
            },
            &template,
        )
        .expect("both providers succeed; the better-scored one is tried first");

        assert_eq!(
            out.model_used, "betterscore-model",
            "failover must try the BETTER-SCORED slug first, not chain order"
        );
        assert_eq!(
            attempted.borrow()[0],
            "betterscore-model".to_string(),
            "the better-scored (lower-priority-in-chain) slug must be ATTEMPTED FIRST"
        );
    }

    #[test]
    fn walk_fallback_chain_equal_scores_preserve_chain_priority_order() {
        // No-regression companion to the GATE: when every slug scores equally
        // the STABLE tie-break on the original chain index must keep the
        // declared priority order (chainfirst then chainsecond) — proving the
        // pre-sort only reorders on a genuine score difference.
        let chain = vec!["chainfirst".to_string(), "chainsecond".to_string()];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let attempted = std::cell::RefCell::new(Vec::<String>::new());
        let out = walk_fallback_chain(
            &chain,
            |slug| Some(format!("{slug}-model")),
            |_slug| true,
            |_slug| {},
            |_slug| 0.5, // every slug scores identically → tie-break on index
            |req| {
                attempted.borrow_mut().push(req.model.clone());
                Ok(resp_from(&req.model))
            },
            &template,
        )
        .expect("first slug succeeds");
        assert_eq!(out.model_used, "chainfirst-model");
        assert_eq!(
            attempted.borrow()[0],
            "chainfirst-model".to_string(),
            "score tie must preserve original chain priority (index 0 first)"
        );
    }

    #[test]
    fn fallback_all_providers_429_degrades_gracefully_with_last_error() {
        // NEGATIVE / load-bearing: EVERY eligible provider returns a 429. The
        // walk must NOT hang, NOT panic, NOT return a bogus success — it must
        // surface `FallbackExhausted` carrying the LAST underlying RateLimit.
        // This is the exact `Err` coach maps to LlmAllProvidersFailed →
        // ReviewStatus::Degraded { reason: "all_providers_failed" } (SPEC-23
        // §11.1). If this regressed, coach would either crash or silently emit
        // an empty review.
        let chain = vec!["groq".to_string(), "anthropic".to_string(), "gemini".to_string()];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };

        let attempted = std::cell::RefCell::new(0usize);
        let err = walk_fallback_chain(
            &chain,
            |slug| Some(format!("{slug}-model")),
            |_slug| true,
            |_slug| {},
            |_| 0.0,
            |req| {
                *attempted.borrow_mut() += 1;
                Err(ProviderError::RateLimit {
                    detail: format!("{} 429: rate_limit", req.model),
                })
            },
            &template,
        )
        .expect_err("every provider 429 must yield an error, not a bogus success");

        assert_eq!(*attempted.borrow(), 3, "all 3 providers must be tried before giving up");
        match err {
            ProviderError::FallbackExhausted { detail } => {
                assert!(
                    detail.contains("all providers in fallback chain failed"),
                    "exhaustion must name the all-fail cause: {detail}"
                );
                // The LAST attempted provider's 429 detail must be preserved so
                // operators can diagnose (no silent swallow).
                assert!(
                    detail.contains("gemini-model 429: rate_limit"),
                    "last underlying RateLimit must be carried through: {detail}"
                );
            }
            other => panic!(
                "all-providers-429 must be FallbackExhausted (coach degraded trigger), got {other:?}"
            ),
        }
    }

    #[test]
    fn fallback_skips_unconfigured_and_breaker_tripped_slugs() {
        // The walk must SKIP a slug with no usable model AND a slug whose
        // circuit-breaker is tripped, landing on the first eligible+live one —
        // a partially-broken chain still makes progress instead of hard-failing.
        let chain = vec![
            "noModel".to_string(),   // skipped: model_for -> None
            "breakerDown".to_string(), // skipped: is_alive -> false
            "good".to_string(),      // wins
        ];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let attempted = std::cell::RefCell::new(Vec::<String>::new());
        let out = walk_fallback_chain(
            &chain,
            |slug| if slug == "noModel" { None } else { Some(format!("{slug}-model")) },
            |slug| slug != "breakerDown",
            |_slug| {},
            |_| 0.0,
            |req| {
                attempted.borrow_mut().push(req.model.clone());
                Ok(resp_from(&req.model))
            },
            &template,
        )
        .expect("a configured + live provider exists, so the walk must succeed");
        assert_eq!(out.model_used, "good-model");
        assert_eq!(
            *attempted.borrow(),
            vec!["good-model".to_string()],
            "only the eligible+live slug should be attempted (others skipped pre-attempt)"
        );
    }

    #[test]
    fn fallback_empty_chain_is_graceful_exhaustion_not_panic() {
        // Defensive: an empty chain (misconfigured agents.toml) must degrade to
        // FallbackExhausted, never panic — coach then shows the degraded card.
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let err = walk_fallback_chain(
            &[],
            |_| Some("m".into()),
            |_| true,
            |_| {},
            |_| 0.0,
            |_req| Ok(resp_from("never")),
            &template,
        )
        .expect_err("empty chain must be a graceful error");
        assert!(matches!(err, ProviderError::FallbackExhausted { .. }));
    }

    #[test]
    fn complete_with_fallback_wires_real_complete_and_degrades_on_empty_chain() {
        // End-to-end seam check: the PUBLIC `complete_with_fallback` reads the
        // real agents.toml and runs the real `complete()` through the same
        // policy. With an explicitly empty fallback_chain it must surface
        // FallbackExhausted — proving the public entrypoint is wired to the
        // pure policy (no network needed for this deterministic branch).
        // NOTE: `TempAgentsToml::new` already acquires `sandbox::test_lock()`;
        // taking it again here would deadlock the non-reentrant mutex.
        let _agents = TempAgentsToml::new(
            r#"
[routing]
fallback_chain = []
"#,
        );
        let template = ProviderRequest {
            model: "llama-3.1-8b-instant".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let err = complete_with_fallback(template)
            .expect_err("empty chain must surface FallbackExhausted via the public entrypoint");
        assert!(matches!(err, ProviderError::FallbackExhausted { .. }));
    }

    #[test]
    fn unimplemented_stub_arms_return_typed_err_not_panic() {
        // Regression (panic→Err): every genuinely-unimplemented Stage-4 stub
        // must return a typed `ProviderError` instead of aborting via
        // `unimplemented!()`. Direct calls — no catch_unwind: if any stub
        // still panics, the test harness reports the panic as a failure.
        let req = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        // NOTE: openai + ollama (Stage 4) then cerebras + llamacpp were promoted
        // to real HTTP (each round-trips a live wiremock 200 — see the
        // `*_round_trips_chat_completion_over_wiremock` gates), so they are NO
        // LONGER in this still-stub walk. The remaining 3 stay typed-Err stubs —
        // all CLI shell-outs (opencode/claude/codex — driven via the cli_session
        // substrate, not an HTTP completion). (cloudflare is now real HTTP too.)
        type StubFn = fn(&ProviderRequest) -> Result<ProviderResponse, ProviderError>;
        let stubs: [(&str, StubFn); 3] = [
            ("opencode", complete_opencode_pseudo),
            ("claude", complete_claude_cli_pseudo),
            ("codex", complete_codex_cli_pseudo),
        ];
        for (slug, stub) in stubs {
            let err = stub(&req)
                .expect_err("a Stage-4 stub must return Err, never Ok");
            match err {
                ProviderError::Unknown { detail } => {
                    assert!(
                        detail.contains(slug) && detail.contains("Stage 4"),
                        "stub error must name the provider + the Stage-4 TODO: {detail}"
                    );
                }
                other => panic!(
                    "stub `{slug}` must surface ProviderError::Unknown, got {other:?}"
                ),
            }
        }
    }

    #[test]
    fn fallback_chain_survives_unimplemented_provider_arm() {
        // Regression (panic→Err): a chain whose FIRST slug dispatches —
        // through the REAL `complete()` — to a genuinely-unimplemented
        // provider arm (opencode is still a Stage-4 stub; openai/ollama/cerebras/
        // llamacpp/cloudflare have since been promoted) must fail over to the
        // next provider, not abort the process. Before the fix the stub body was
        // `unimplemented!()`, so this walk panicked at slug 0 instead of
        // returning the mock arm's success.
        let _agents = TempAgentsToml::new(
            r#"
[providers.opencode]
default_model = "opencode-model"
"#,
        );
        let chain = vec!["opencode".to_string(), "mock".to_string()];
        let template = ProviderRequest {
            model: "seed".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "x")],
            max_tokens: Some(8),
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let marked_down = std::cell::RefCell::new(Vec::<String>::new());
        let out = walk_fallback_chain(
            &chain,
            |slug| match slug {
                "opencode" => Some("opencode-model".to_string()),
                other => Some(format!("{other}-model")),
            },
            |_slug| true,
            |slug| marked_down.borrow_mut().push(slug.to_string()),
            |_| 0.0, // neutral score → preserve chain order (opencode first, mock second)
            |req| {
                if req.model == "opencode-model" {
                    // Drive the real dispatcher into the unimplemented
                    // opencode arm — the exact call that used to panic.
                    complete(req)
                } else {
                    Ok(resp_from(&req.model))
                }
            },
            &template,
        )
        .expect("an unimplemented first arm must fail over, not panic the walk");
        assert_eq!(
            out.model_used, "mock-model",
            "the walk must continue past the unimplemented arm to the working one"
        );
        // P0-5: the Stage-4 opencode stub surfaces a `ProviderError::Unknown`
        // (the "Stage 4: ..." TODO marker). Unknown classifies as `Failover`
        // (permanent-for-this-provider), so the walk fails over to the next
        // slug WITHOUT tripping the breaker — a not-yet-implemented arm is not
        // a reason to circuit-break a slug. Before P0-5 every error marked the
        // slug down; the breaker now only counts genuinely TRANSIENT failures
        // (network / rate-limit). The failover itself (→ "mock-model") is
        // unchanged; only the breaker-marking is now classification-gated.
        assert!(
            marked_down.borrow().is_empty(),
            "an Unknown (Stage-4 stub) error fails over WITHOUT tripping the breaker; got {:?}",
            *marked_down.borrow()
        );
    }

    // ─── feat/providers-tooldefs-live — outbound tool-def serialization ──────
    //
    // Audit fix: before `ProviderRequest.tools` existed, every live adapter
    // built a text-only body, so `tool_calls` could never come back. These
    // tests pin the per-provider native tools wire (asserted on the pure
    // body-builder helpers — no live HTTP) so the schema can't silently
    // regress: OpenAI-style `tools` for Groq, Anthropic `input_schema` block,
    // Gemini `functionDeclarations`.

    fn sample_tool() -> ToolDef {
        ToolDef {
            name: "get_weather".to_string(),
            description: "Get the current weather for a city".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string", "description": "City name" }
                },
                "required": ["city"]
            }),
        }
    }

    #[test]
    fn groq_tools_json_emits_openai_function_envelope() {
        // Groq is OpenAI-compat: tools is an array of
        // {type:"function", function:{name,description,parameters}}.
        let out = groq_tools_json(&[sample_tool()]).expect("non-empty tools → Some");
        let arr = out.as_array().expect("tools must be a JSON array");
        assert_eq!(arr.len(), 1, "one tool in → one entry out");
        let t = &arr[0];
        assert_eq!(t["type"], "function", "OpenAI envelope tags each tool type=function");
        assert_eq!(t["function"]["name"], "get_weather");
        assert_eq!(
            t["function"]["description"],
            "Get the current weather for a city"
        );
        // The JSON-Schema object is carried verbatim under `parameters`.
        assert_eq!(
            t["function"]["parameters"]["properties"]["city"]["type"],
            "string"
        );
        assert_eq!(t["function"]["parameters"]["required"][0], "city");
        // Empty tools → None so the adapter omits the body key entirely.
        assert!(
            groq_tools_json(&[]).is_none(),
            "no tools must omit the `tools` key (byte-identical no-tools wire)"
        );
    }

    #[test]
    fn anthropic_tools_json_emits_input_schema_block() {
        // Anthropic names the schema slot `input_schema` (NOT `parameters`)
        // and has no outer {type:"function"} envelope.
        let out = anthropic_tools_json(&[sample_tool()]).expect("non-empty tools → Some");
        let arr = out.as_array().expect("tools must be a JSON array");
        assert_eq!(arr.len(), 1);
        let t = &arr[0];
        assert_eq!(t["name"], "get_weather");
        assert_eq!(t["description"], "Get the current weather for a city");
        assert!(
            t.get("type").is_none(),
            "Anthropic tools have no `type` envelope key: {}",
            t
        );
        assert!(
            t.get("parameters").is_none(),
            "Anthropic uses `input_schema`, not `parameters`: {}",
            t
        );
        assert_eq!(
            t["input_schema"]["properties"]["city"]["type"],
            "string"
        );
        assert_eq!(t["input_schema"]["required"][0], "city");
        assert!(anthropic_tools_json(&[]).is_none());
    }

    #[test]
    fn gemini_tools_json_emits_function_declarations() {
        // Gemini nests every declaration under a single-element `tools` array
        // whose entry carries a `functionDeclarations` array.
        let out = gemini_tools_json(&[sample_tool()]).expect("non-empty tools → Some");
        let arr = out.as_array().expect("tools must be a JSON array");
        assert_eq!(arr.len(), 1, "single tools entry wrapping all declarations");
        let decls = arr[0]["functionDeclarations"]
            .as_array()
            .expect("functionDeclarations must be a JSON array");
        assert_eq!(decls.len(), 1, "one declaration per ToolDef");
        let d = &decls[0];
        assert_eq!(d["name"], "get_weather");
        assert_eq!(d["description"], "Get the current weather for a city");
        assert_eq!(d["parameters"]["properties"]["city"]["type"], "string");
        assert_eq!(d["parameters"]["required"][0], "city");
        assert!(gemini_tools_json(&[]).is_none());
    }

    #[test]
    fn multiple_tooldefs_serialize_in_order_per_provider() {
        // Order + count must be preserved across all three live providers so
        // the model sees every tool exactly once.
        let two = vec![
            ToolDef {
                name: "alpha".into(),
                description: "first".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
            ToolDef {
                name: "beta".into(),
                description: "second".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ];

        let groq = groq_tools_json(&two).unwrap();
        let g = groq.as_array().unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0]["function"]["name"], "alpha");
        assert_eq!(g[1]["function"]["name"], "beta");

        let anth = anthropic_tools_json(&two).unwrap();
        let a = anth.as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0]["name"], "alpha");
        assert_eq!(a[1]["name"], "beta");

        let gem = gemini_tools_json(&two).unwrap();
        let decls = gem.as_array().unwrap()[0]["functionDeclarations"]
            .as_array()
            .unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0]["name"], "alpha");
        assert_eq!(decls[1]["name"], "beta");
    }

    #[test]
    fn provider_request_tools_round_trip_and_default() {
        // `tools` is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`:
        //   - a tool-less request serializes WITHOUT a `tools` key (byte-compat
        //     with the pre-fix wire), and
        //   - legacy payloads with no `tools` key still deserialize (default).
        let no_tools = ProviderRequest {
            model: "llama-3.1-8b-instant".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let j = serde_json::to_string(&no_tools).unwrap();
        assert!(
            !j.contains("\"tools\""),
            "empty tools must be skipped on the wire: {}",
            j
        );

        // Legacy wire (no `tools` key) deserializes to an empty Vec.
        let legacy: ProviderRequest = serde_json::from_str(
            r#"{"model":"m","systemPrompt":null,"messages":[],"maxTokens":null,"temperature":null,"responseFormat":"plain_text"}"#,
        )
        .expect("legacy tool-less wire must still deserialize");
        assert!(legacy.tools.is_empty(), "missing tools key → default empty");

        // A request WITH tools round-trips and carries the camelCase `tools`.
        let with_tools = ProviderRequest {
            tools: vec![sample_tool()],
            ..no_tools
        };
        let j2 = serde_json::to_string(&with_tools).unwrap();
        assert!(j2.contains("\"tools\""), "tools present must serialize: {}", j2);
        let back: ProviderRequest = serde_json::from_str(&j2).unwrap();
        assert_eq!(back.tools.len(), 1);
        assert_eq!(back.tools[0].name, "get_weather");
    }

    // ─── Stage 4 promotion — real-HTTP round-trip gates (openai + ollama) ────
    //
    // These two providers were the highest-value of the 8 Stage-4 stubs:
    // OpenAI (frontier, native tool_calls) + Ollama (local, no auth). Each
    // test stands up a real wiremock HTTP server, points the adapter at it via
    // `SPECTYN_MESH_<SLUG>_BASE_URL` (see `provider_base_url`), mounts a 200
    // chat-completions body, and proves the adapter ROUND-TRIPS it into a
    // `ProviderResponse` with the expected text + token counts — i.e. the real
    // request-build → HTTP → response-parse path, NOT a mocked-away stub. The
    // adapters are sync (`block_on_async`), so the async MockServer is driven
    // through the same bridge the production helper uses.

    // Multi-thread tokio runtime: `block_on_async` (the sync→async bridge the
    // production adapter uses) detects this ambient runtime and drives the
    // reqwest call via `block_in_place` + `Handle::block_on` — keeping the
    // wiremock MockServer's background task and the HTTP client on the SAME
    // runtime. (A plain `#[test]` would make `block_on_async` spin a fresh
    // current-thread runtime per call, orphaning the server's timer task.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_openai_round_trips_chat_completion_over_wiremock() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // sandbox lock keeps the per-process env vars (BASE_URL + API key)
        // from racing other tests in this module.
        let _lock = crate::sandbox::test_lock();

        // OpenAI chat-completions success envelope: `choices[0].message`
        // carries BOTH text content AND a native `tool_calls` array (proves
        // the parse survives a tool-call response), plus a `usage` block the
        // adapter must lift into tokens_in / tokens_out.
        let body = serde_json::json!({
            "id": "chatcmpl-xyz",
            "object": "chat.completion",
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Sunny, 24C.",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"Taipei\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 31, "completion_tokens": 7, "total_tokens": 38 }
        });

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        std::env::set_var("SPECTYN_MESH_OPENAI_BASE_URL", server.uri());
        std::env::set_var("SPECTYN_MESH_OPENAI_API_KEY", "test-openai-key");

        let req = ProviderRequest {
            model: "gpt-4o".into(),
            system_prompt: Some("be terse".into()),
            messages: vec![Message::text(MessageRole::User, "weather in Taipei?")],
            max_tokens: Some(64),
            temperature: Some(0.0),
            response_format: ResponseFormat::PlainText,
            tools: vec![sample_tool()],
        };
        // The adapter is sync and bridges via `block_on_async`; from this async
        // test we run it under `block_in_place` so its internal
        // `Handle::block_on` is permitted on the multi-thread runtime.
        let resp = tokio::task::block_in_place(|| complete_openai_pseudo(&req))
            .expect("openai adapter must round-trip the 200 mock");

        std::env::remove_var("SPECTYN_MESH_OPENAI_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_OPENAI_API_KEY");

        assert_eq!(resp.text, "Sunny, 24C.", "text lifted from choices[0].message.content");
        assert_eq!(resp.model_used, "gpt-4o-2024-08-06", "model_used echoes upstream model");
        assert_eq!(resp.tokens_in, 31, "tokens_in from usage.prompt_tokens");
        assert_eq!(resp.tokens_out, 7, "tokens_out from usage.completion_tokens");
        assert_eq!(resp.cost_usd, None, "adapter leaves cost None; outer complete() re-stamps");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_ollama_round_trips_chat_completion_over_wiremock() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let _lock = crate::sandbox::test_lock();

        // Ollama OpenAI-compat `/v1/chat/completions` success envelope — same
        // choices[]/usage shape as OpenAI, but NO api key is required (the
        // adapter must NOT send Bearer auth / must not fail when no key env is
        // set). We deliberately leave SPECTYN_MESH_OLLAMA_API_KEY unset.
        std::env::remove_var("SPECTYN_MESH_OLLAMA_API_KEY");
        std::env::remove_var("OLLAMA_API_KEY");

        let body = serde_json::json!({
            "id": "chatcmpl-local",
            "object": "chat.completion",
            "model": "llama3.2",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "42 is the answer." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17 }
        });

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        std::env::set_var("SPECTYN_MESH_OLLAMA_BASE_URL", server.uri());

        let req = ProviderRequest {
            model: "llama3.2".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "what is 6*7?")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let resp = tokio::task::block_in_place(|| complete_ollama_pseudo(&req))
            .expect("ollama adapter must round-trip the 200 mock (no auth)");

        std::env::remove_var("SPECTYN_MESH_OLLAMA_BASE_URL");

        assert_eq!(resp.text, "42 is the answer.", "text lifted from choices[0].message.content");
        assert_eq!(resp.model_used, "llama3.2", "model_used echoes upstream model");
        assert_eq!(resp.tokens_in, 12, "tokens_in from usage.prompt_tokens");
        assert_eq!(resp.tokens_out, 5, "tokens_out from usage.completion_tokens");
        assert_eq!(resp.cost_usd, None, "local provider reports no cost");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_cerebras_round_trips_chat_completion_over_wiremock() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let _lock = crate::sandbox::test_lock();

        // OpenAI-compat envelope; cerebras is Bearer-authed, so the mock also
        // asserts the Authorization header is present (the adapter must send it).
        let body = serde_json::json!({
            "id": "chatcmpl-cb",
            "object": "chat.completion",
            "model": "llama3.1-8b",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "fast answer." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12 }
        });

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-cerebras-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        std::env::set_var("SPECTYN_MESH_CEREBRAS_BASE_URL", server.uri());
        std::env::set_var("SPECTYN_MESH_CEREBRAS_API_KEY", "test-cerebras-key");

        let req = ProviderRequest {
            model: "llama3.1-8b".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "ping")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let resp = tokio::task::block_in_place(|| complete_cerebras_pseudo(&req))
            .expect("cerebras adapter must round-trip the 200 mock (with Bearer auth)");

        std::env::remove_var("SPECTYN_MESH_CEREBRAS_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_CEREBRAS_API_KEY");

        assert_eq!(resp.text, "fast answer.");
        assert_eq!(resp.model_used, "llama3.1-8b");
        assert_eq!(resp.tokens_in, 9);
        assert_eq!(resp.tokens_out, 3);
        assert_eq!(resp.cost_usd, None, "adapter leaves cost None; outer complete() re-stamps");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_llamacpp_round_trips_chat_completion_over_wiremock() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let _lock = crate::sandbox::test_lock();

        // Local llama-server OpenAI-compat envelope — keyless, NO Bearer header.
        std::env::remove_var("SPECTYN_MESH_LLAMACPP_API_KEY");

        let body = serde_json::json!({
            "id": "chatcmpl-lc",
            "object": "chat.completion",
            "model": "local-gguf",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "on-device reply." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12 }
        });

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        std::env::set_var("SPECTYN_MESH_LLAMACPP_BASE_URL", server.uri());

        let req = ProviderRequest {
            model: "local-gguf".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let resp = tokio::task::block_in_place(|| complete_llamacpp_pseudo(&req))
            .expect("llamacpp adapter must round-trip the 200 mock (no auth)");

        std::env::remove_var("SPECTYN_MESH_LLAMACPP_BASE_URL");

        assert_eq!(resp.text, "on-device reply.");
        assert_eq!(resp.model_used, "local-gguf");
        assert_eq!(resp.tokens_in, 8);
        assert_eq!(resp.tokens_out, 4);
        assert_eq!(resp.cost_usd, None, "local provider reports no cost");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_cloudflare_round_trips_workers_ai_over_wiremock() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let _lock = crate::sandbox::test_lock();

        // Workers AI envelope: completion under `result.response` (NOT choices[]),
        // account-scoped path, Bearer auth. The mock pins the exact account/model
        // path + the Authorization header so a wrong path or a missing token fails.
        let body = serde_json::json!({
            "result": {
                "response": "cf workers reply.",
                "usage": { "prompt_tokens": 6, "completion_tokens": 2 }
            },
            "success": true,
            "errors": [],
            "messages": []
        });

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/client/v4/accounts/acct-123/ai/run/@cf/meta/llama"))
            .and(header("authorization", "Bearer test-cf-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        std::env::set_var("SPECTYN_MESH_CLOUDFLARE_BASE_URL", server.uri());
        std::env::set_var("SPECTYN_MESH_CLOUDFLARE_API_KEY", "test-cf-token");
        std::env::set_var("SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID", "acct-123");

        let req = ProviderRequest {
            model: "@cf/meta/llama".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        };
        let resp = tokio::task::block_in_place(|| complete_cloudflare_pseudo(&req))
            .expect("cloudflare adapter must round-trip the 200 Workers-AI mock");

        std::env::remove_var("SPECTYN_MESH_CLOUDFLARE_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_CLOUDFLARE_API_KEY");
        std::env::remove_var("SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID");

        assert_eq!(resp.text, "cf workers reply.", "text lifted from result.response");
        assert_eq!(resp.model_used, "@cf/meta/llama", "model_used echoes the requested model");
        assert_eq!(resp.tokens_in, 6, "tokens_in from result.usage.prompt_tokens");
        assert_eq!(resp.tokens_out, 2, "tokens_out from result.usage.completion_tokens");
        assert_eq!(resp.cost_usd, None);
    }

    #[test]
    fn cloudflare_missing_account_id_is_auth_error() {
        // The account id is required (Workers AI URLs are account-scoped); absent
        // → AuthError naming the env vars, never a silent wrong-URL request.
        let _lock = crate::sandbox::test_lock();
        std::env::remove_var("SPECTYN_MESH_CLOUDFLARE_ACCOUNT_ID");
        std::env::remove_var("CLOUDFLARE_ACCOUNT_ID");
        std::env::set_var("SPECTYN_MESH_CLOUDFLARE_API_KEY", "test-cf-token");
        let err = complete_cloudflare_pseudo(&ProviderRequest {
            model: "@cf/meta/llama".into(),
            system_prompt: None,
            messages: vec![Message::text(MessageRole::User, "hi")],
            max_tokens: None,
            temperature: None,
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        })
        .expect_err("missing account id must error before any network call");
        std::env::remove_var("SPECTYN_MESH_CLOUDFLARE_API_KEY");
        match err {
            ProviderError::AuthError { detail } => assert!(
                detail.contains("CLOUDFLARE_ACCOUNT_ID"),
                "AuthError must name the account-id env var: {detail}"
            ),
            other => panic!("expected AuthError for missing account id, got {other:?}"),
        }
    }
}
