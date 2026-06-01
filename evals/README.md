# Phantom Mesh — agent-behavior evals (promptfoo)

LLM/agent-behavior evaluation for the parts of phantom-mesh that unit tests
can't judge — does the Coach give **one concrete, shame-free next action**?
Uses [promptfoo](https://github.com/promptfoo/promptfoo) (MIT, runs locally).

This complements the deterministic suites (Rust `cargo test`, `vitest`) — those
check logic/wire shapes; these check *model output quality* against rubrics.

## Run

```bash
cd evals

# Offline harness self-test (no network, no LLM, no key) — deterministic asserts only:
npx promptfoo@latest eval -c promptfooconfig.offline.yaml

# Live eval (model-graded llm-rubric) — needs a provider:
#   - local:   start `ollama serve` (config defaults to ollama:chat:llama3.1), OR
#   - hosted:  uncomment a provider in promptfooconfig.yaml + export its key
#              (OPENAI_API_KEY / GEMINI_API_KEY / ANTHROPIC_API_KEY)
npx promptfoo@latest eval

npx promptfoo@latest view   # open the results UI
```

Or via the npm scripts here: `npm run eval` / `npm run eval:offline` / `npm run view`.

## What it checks

| Layer | Assert | Runs offline? |
|---|---|---|
| Shame-free | output contains none of `你又 / 你終於 / 你居然 / 你怎麼又 / 還不` (mirrors `core/src/life_node/coach_prompts/lint.rs` `SHAME_PATTERNS`) | ✅ deterministic |
| Conciseness | one action, 1..800 chars (not a wall of text) | ✅ deterministic |
| Quality | `llm-rubric`: exactly one concrete doable action, supportive, no blame — incl. the **bad-day** and **empty-day** cases | ❌ needs a provider |

## Notes / caveats

- **On this WSL dev box**, hosted providers (Groq/Gemini) are quota-limited and
  no Ollama is running, so the **live** `llm-rubric` evals won't run here without
  setup; the **offline** config runs anywhere and proves the harness + asserts.
- The coach prompt in `promptfooconfig.yaml` mirrors
  `core/src/life_node/coach_prompts/templates.rs` — keep them in sync. A future
  improvement is an `exec` provider that drives `phantom coach review` directly
  (needs `phantom serve` + an LLM key) so the eval exercises the real pipeline,
  not just the prompt.
- Files: `promptfooconfig.yaml` (live), `promptfooconfig.offline.yaml` (CI/offline),
  `providers/coach-mock.js` (offline mock provider).
