# 免費 LLM API 供應商 — Phantom 直接相容（drop-in），2026 年 5 月

> **狀態**：研究快照；額度（quota）會變動，依賴前請重新查證。
> **產生時間**：2026-05-02（多代理人研究工作階段，起因為
> OpenCode `hy3-preview-free` 在測試套件中途觸發速率限制（rate-limiting））。
>
> 除非另有明確標註，所有列出的服務皆支援 OpenAI 相容（OpenAI-compatible）的
> `/v1/chat/completions`，因此可直接套用 phantom 現有的
> `[providers.openai_compat]` 區塊，設定 `type = "openai_compat"` 即可——
> phantom 端零程式碼改動。

## 比較表

| 服務 | OpenAI 相容端點（endpoint） | 註冊 | 免費額度 | 免費模型（`default_model` 用的 id） | 工具 | 串流 | 429 格式 | 備註 |
|---|---|---|---|---|---|---|---|---|
| **Groq Cloud** | `https://api.groq.com/openai/v1` | groq.com | 30 RPM、6K TPM、1K RPD | `llama-3.3-70b-versatile`, `mixtral-8x7b-32768` | ✅ | ✅ | `429 insufficient_quota` | 不需信用卡（CC）；約 700 tok/s；RPD 是主要限制 |
| **Cerebras Inference** | `https://api.cerebras.ai/v1` | cloud.cerebras.ai | 30 RPM、100K TPM、每日 1M tokens | `llama-3.3-70b`, `qwen-3-32b`, `qwen-3-235b` | ✅ | ✅ | `429 insufficient_quota` | 不需信用卡；晶圓級（wafer-scale），快 20 倍；免費層 context 上限 8K |
| **OpenRouter** | `https://openrouter.ai/api/v1` | openrouter.ai | 每日 50 次請求、20 RPM | `openrouter/free`（自動路由）、`llama-3.1-8b-free`, `mistral-7b-free` | ✅（視模型而定） | ✅ | `429 insufficient_quota` | 儲值 $10 → 每日 1000 次請求；失敗請求也計入 |
| **Mistral La Plateforme** | `https://api.mistral.ai/v1` | console.mistral.ai | 2 RPM、500K TPM、每月 1B tokens | `mistral-small`, `mistral-large`, `pixtral-12b` | ✅ | ✅ | `429 rate_limit_exceeded` | RPM 緊；每月 token 額度寬鬆 |
| **Together AI** | `https://api.together.xyz/v1` | together.ai | $25-100 免費額度、20 RPM | `meta-llama/llama-3-70b-chat-hf`, `mistral-7b-instruct-v0.1`、200+ | ✅ | ✅ | `429 rate_limit_exceeded` | 新手額度寬鬆；儲值 $5 → Build Tier 2 |
| **NVIDIA NIM** | `https://api.nims.nvidia.com/v1` | build.nvidia.com | 1K-5K 點數、每模型約 40 RPM | `nemotron-3-super`, `deepseek-r1`, `glm-5-chinese`、100+ | ✅ | ✅ | `429 rate_limit` | 點數制；需加入 Developer Program |
| **DeepSeek API** | `https://api.deepseek.com/v1` | platform.deepseek.com | 5M 免費 tokens（約 3.5K 次呼叫 @ 1K tokens） | `deepseek-chat`, `deepseek-coder` | ✅ | ✅ | `429` 並附原因 | 無速率限制；付費模型最便宜 |
| **GitHub Models** | `https://models.githubcopilot.com/v1` | GitHub 帳號 | 每日約 150 次請求 | `gpt-4o`, `phi-3.5-mini`, `claude-3.5-sonnet` | 受限 | ✅ | `429 rate_limit_exceeded` | 非商用；以美國為主 |
| **Cloudflare Workers AI** | `https://api.cloudflare.com/.../ai/` | dash.cloudflare.com | 每日 10K Neurons（約 5-10K 次請求） | `@cf/meta/llama-3-8b-instruct`, `@cf/mistral/mistral-7b-instruct` | 受限 | ✅ | `429`（達上限即硬性失敗） | 永久免費方案；無超量計費 |
| **HuggingFace Inference** | （非 OpenAI 相容） | huggingface.co | 每月 100K 點數 | <10B 參數、約 30 秒冷啟動 | 受限 | 透過 SDK | N/A | 非直接相容；需 HF 原生用戶端 |
| **Google Gemini API** | （非原生 OpenAI 相容） | ai.google.dev | 5-15 RPM、100-1K RPD | Gemini 3 Flash, 3.1 Flash-Lite | ✅（原生 API） | ✅ | `429 RESOURCE_EXHAUSTED` | 需代理（proxy）或原生用戶端 |

## 排除項目
- **OpenAI**：截至 2026 年 5 月無免費層；$5 試用已於 2025 年中停止。
- **Anthropic**：無免費 API 層；僅 Claude.ai 網頁版或付費 API。

## 為 phantom 推薦的前三名

排序依據：(a) 額度寬鬆且支援工具呼叫（tool calling）、(b) 可靠度、(c) OpenAI 相容即插即用。

### 1. Groq Cloud ⭐⭐⭐
每日 1K 次請求、完整工具呼叫、約 700 tok/s、可直接替換。

```toml
[providers.groq]
type          = "openai_compat"
base_url      = "https://api.groq.com/openai/v1"
api_key_env   = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"
```
注意：6K TPM 可能在長時間執行時形成瓶頸；RPD 才是真正的每日上限。

### 2. Cerebras Inference
每日 1M tokens（約為 Groq token 量的 1000 倍）、30 RPM、支援工具呼叫。

```toml
[providers.cerebras]
type          = "openai_compat"
base_url      = "https://api.cerebras.ai/v1"
api_key_env   = "CEREBRAS_API_KEY"
default_model = "llama-3.3-70b"
```
注意：免費層 context 上限 8K——請讓提示（prompt）保持簡短。

### 3. NVIDIA NIM
100+ 個模型，含 DeepSeek-R1，點數制。

```toml
[providers.nvidia]
type          = "openai_compat"
base_url      = "https://api.nims.nvidia.com/v1"
api_key_env   = "NVIDIA_NIM_API_KEY"
default_model = "nemotron-3-super"
```
注意：點數成本因模型而異；預估每 1K 輸入 tokens 約 0.02 點數。

## 容錯切換（failover）順序建議

針對 phantom 的測試套件（避免套件中途的速率限制抖動）：

1. **主要**：Cerebras（token 量最多，但 context 僅 8K）
2. **次要**：Groq（可靠、快速、RPD 上限可預期）
3. **第三**：NVIDIA NIM（模型多樣性）
4. **最後手段**：DeepSeek（無速率限制，但較不穩定）

四者皆為 `type = "openai_compat"`——phantom 零程式碼改動。

## 如何在 phantom 中使用（現行做法）

1. 註冊 Groq（5 分鐘，不需信用卡）→ 取得 `GROQ_API_KEY`
2. `[Environment]::SetEnvironmentVariable('GROQ_API_KEY', '<key>', 'User')`
3. 編輯 `~/.phantom-mesh/agents.toml`，加入上方的 `[providers.groq]` 區塊
4. 切換 `[agent.master].provider = "groq"` 與 `model = "llama-3.3-70b-versatile"`
5. `phantom doctor` → 確認顯示 `✓ Groq: env`
6. `phantom repl --agent master -c "ping"` → 應在 2 秒內回應

## 429 處理（適用任何供應商）

OpenAI 相容服務會回傳類似的 429 與 JSON：
```json
{"error": {"message": "...", "type": "rate_limit_exceeded", "code": "rate_limit_exceeded"}}
```

當某個供應商出錯時，phantom 現有的後備鏈（fallback chain）已會跨供應商重試；
針對 429 的退避（backoff-on-429）邏輯，請參見 `core/src/streaming.rs` 的重試
邏輯。測試框架情境 `scripts/phantom-test/scenarios/25-agent-anti-hallucination.sh`
會偵測 stdout 中的 429 並以 77（跳過）退出——這是測試平台（testbed）的慣用模式。

## 來源

- [TokenMix – Groq 免費層 2026](https://tokenmix.ai/blog/groq-free-tier-limits-2026)
- [Cerebras API 金鑰與速率限制](https://tokenmix.ai/blog/cerebras-api-key-rate-limits-free-tier-2026)
- [Mistral 文件 – 速率限制](https://docs.mistral.ai/deployment/ai-studio/tier)
- [NVIDIA NIM Developer Program](https://developer.nvidia.com/blog/access-to-nvidia-nim-now-available-free-to-developer-program-members/)
- [GitHub Models 免費層](https://medium.com/@dan.avila7/free-ai-models-with-github-models-api-0464c4ae7f16)
- [OpenRouter 定價](https://openrouter.ai/pricing)
- [DeepSeek API 定價](https://api-docs.deepseek.com/quick_start/pricing)
- [Cloudflare Workers AI 定價](https://developers.cloudflare.com/workers-ai/platform/pricing/)
- [OpenAI 速率限制手冊](https://developers.openai.com/cookbook/examples/how_to_handle_rate_limits)
