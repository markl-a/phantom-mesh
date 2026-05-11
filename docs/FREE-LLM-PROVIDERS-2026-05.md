# Free LLM API Providers — Phantom drop-in compatibility, May 2026

> **Status**: Research snapshot; quotas drift, re-verify before relying.
> **Generated**: 2026-05-02 (multi-agent research session triggered by
> OpenCode `hy3-preview-free` rate-limiting mid-test-suite).
>
> All listed services support OpenAI-compatible `/v1/chat/completions`
> unless explicitly noted, so they drop into phantom's existing
> `[providers.openai_compat]` block with `type = "openai_compat"` —
> zero phantom-side code changes.

## Comparison table

| Service | OpenAI-compat endpoint | Signup | Free quota | Free models (id for `default_model`) | Tools | Stream | 429 format | Notes |
|---|---|---|---|---|---|---|---|---|
| **Groq Cloud** | `https://api.groq.com/openai/v1` | groq.com | 30 RPM, 6K TPM, 1K RPD | `llama-3.3-70b-versatile`, `mixtral-8x7b-32768` | ✅ | ✅ | `429 insufficient_quota` | No CC; ~700 tok/s; RPD is binding constraint |
| **Cerebras Inference** | `https://api.cerebras.ai/v1` | cloud.cerebras.ai | 30 RPM, 100K TPM, 1M tokens/day | `llama-3.3-70b`, `qwen-3-32b`, `qwen-3-235b` | ✅ | ✅ | `429 insufficient_quota` | No CC; wafer-scale, 20× faster; 8K context cap on free tier |
| **OpenRouter** | `https://openrouter.ai/api/v1` | openrouter.ai | 50 req/day, 20 RPM | `openrouter/free` (auto-routes), `llama-3.1-8b-free`, `mistral-7b-free` | ✅ (model-dep) | ✅ | `429 insufficient_quota` | $10 spend → 1000 req/day; failed requests count |
| **Mistral La Plateforme** | `https://api.mistral.ai/v1` | console.mistral.ai | 2 RPM, 500K TPM, 1B tokens/month | `mistral-small`, `mistral-large`, `pixtral-12b` | ✅ | ✅ | `429 rate_limit_exceeded` | Tight RPM; generous monthly tokens |
| **Together AI** | `https://api.together.xyz/v1` | together.ai | $25-100 free credits, 20 RPM | `meta-llama/llama-3-70b-chat-hf`, `mistral-7b-instruct-v0.1`, 200+ | ✅ | ✅ | `429 rate_limit_exceeded` | Generous startup credits; $5 spend → Build Tier 2 |
| **NVIDIA NIM** | `https://api.nims.nvidia.com/v1` | build.nvidia.com | 1K-5K credits, ~40 RPM/model | `nemotron-3-super`, `deepseek-r1`, `glm-5-chinese`, 100+ | ✅ | ✅ | `429 rate_limit` | Credit-based; requires Developer Program |
| **DeepSeek API** | `https://api.deepseek.com/v1` | platform.deepseek.com | 5M free tokens (~3.5K calls @ 1K tokens) | `deepseek-chat`, `deepseek-coder` | ✅ | ✅ | `429` with reason | No rate limits; cheapest paid models |
| **GitHub Models** | `https://models.githubcopilot.com/v1` | GitHub account | ~150 req/day | `gpt-4o`, `phi-3.5-mini`, `claude-3.5-sonnet` | Limited | ✅ | `429 rate_limit_exceeded` | Non-commercial; US-primary |
| **Cloudflare Workers AI** | `https://api.cloudflare.com/.../ai/` | dash.cloudflare.com | 10K Neurons/day (~5-10K req) | `@cf/meta/llama-3-8b-instruct`, `@cf/mistral/mistral-7b-instruct` | Limited | ✅ | `429` (hard fail at cap) | Free forever plan; no overage |
| **HuggingFace Inference** | (NOT OpenAI-compat) | huggingface.co | 100K credits/month | <10B params, ~30s cold-start | Limited | via SDK | N/A | Not drop-in; needs HF native client |
| **Google Gemini API** | (NOT natively OpenAI-compat) | ai.google.dev | 5-15 RPM, 100-1K RPD | Gemini 3 Flash, 3.1 Flash-Lite | ✅ (native API) | ✅ | `429 RESOURCE_EXHAUSTED` | Needs proxy or native client |

## Excluded
- **OpenAI**: No free tier as of May 2026; $5 trial discontinued mid-2025.
- **Anthropic**: No free API tier; Claude.ai web only or paid API.

## Recommended top 3 for phantom

Ranked by: (a) generous quota with tool calling, (b) reliability, (c) OpenAI-compat plug-and-play.

### 1. Groq Cloud ⭐⭐⭐
1K req/day, full tool calling, ~700 tok/s, drop-in replacement.

```toml
[providers.groq]
type          = "openai_compat"
base_url      = "https://api.groq.com/openai/v1"
api_key_env   = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"
```
Caveat: 6K TPM can bottleneck long runs; RPD is the real daily limit.

### 2. Cerebras Inference
1M tokens/day (≈1000× Groq's tokens), 30 RPM, tool calling.

```toml
[providers.cerebras]
type          = "openai_compat"
base_url      = "https://api.cerebras.ai/v1"
api_key_env   = "CEREBRAS_API_KEY"
default_model = "llama-3.3-70b"
```
Caveat: 8K context cap on free tier — keep prompts short.

### 3. NVIDIA NIM
100+ models including DeepSeek-R1, credits-based.

```toml
[providers.nvidia]
type          = "openai_compat"
base_url      = "https://api.nims.nvidia.com/v1"
api_key_env   = "NVIDIA_NIM_API_KEY"
default_model = "nemotron-3-super"
```
Caveat: credit costs vary per model; budget ~0.02 credits / 1K input tokens.

## Failover order suggestion

For phantom's test suite (avoid mid-suite rate-limit thrash):

1. **Primary**: Cerebras (most token volume, but 8K context)
2. **Secondary**: Groq (reliable, fast, predictable RPD limit)
3. **Tertiary**: NVIDIA NIM (model diversity)
4. **Last resort**: DeepSeek (no rate limit, less stable)

All four are `type = "openai_compat"` — zero phantom code changes.

## How to use in phantom (today)

1. Sign up for Groq (5 min, no CC) → get `GROQ_API_KEY`
2. `[Environment]::SetEnvironmentVariable('GROQ_API_KEY', '<key>', 'User')`
3. Edit `~/.phantom-mesh/agents.toml` to add the `[providers.groq]` block above
4. Switch `[agent.master].provider = "groq"` and `model = "llama-3.3-70b-versatile"`
5. `phantom doctor` → confirm `✓ Groq: env`
6. `phantom repl --agent master -c "ping"` → should respond < 2s

## 429 handling (for any provider)

OpenAI-compat services return similar 429 + JSON:
```json
{"error": {"message": "...", "type": "rate_limit_exceeded", "code": "rate_limit_exceeded"}}
```

Phantom's existing fallback chain already retries across providers when one
errors; for backoff-on-429 specifically, see `core/src/streaming.rs` retry
logic. The harness scenario `scripts/phantom-test/scenarios/25-agent-anti-hallucination.sh`
detects 429 in stdout and exits 77 (skip) — this is the testbed pattern.

## Sources

- [TokenMix – Groq Free Tier 2026](https://tokenmix.ai/blog/groq-free-tier-limits-2026)
- [Cerebras API Key & Rate Limits](https://tokenmix.ai/blog/cerebras-api-key-rate-limits-free-tier-2026)
- [Mistral Docs – Rate Limits](https://docs.mistral.ai/deployment/ai-studio/tier)
- [NVIDIA NIM Developer Program](https://developer.nvidia.com/blog/access-to-nvidia-nim-now-available-free-to-developer-program-members/)
- [GitHub Models Free Tier](https://medium.com/@dan.avila7/free-ai-models-with-github-models-api-0464c4ae7f16)
- [OpenRouter Pricing](https://openrouter.ai/pricing)
- [DeepSeek API Pricing](https://api-docs.deepseek.com/quick_start/pricing)
- [Cloudflare Workers AI Pricing](https://developers.cloudflare.com/workers-ai/platform/pricing/)
- [OpenAI Rate Limit Handbook](https://developers.openai.com/cookbook/examples/how_to_handle_rate_limits)
