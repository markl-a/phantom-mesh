# Self-iteration: phantom modifying its own code

`phantom evolve` is the autonomous development loop. Given a goal, the agent
reads the relevant files, makes minimal edits, runs `cargo check` /
`cargo test` to verify, and reports.

## First successful self-fix (2026-04-27)

**Goal**:
> Fix the 'method persist is never used' warning in core/src/cost.rs. Read
> the file with file_read first to understand context, then use file_edit
> to either prefix the function name with underscore or annotate it with
> `#[allow(dead_code)]`. Do NOT delete the function. After editing, run
> cargo_check via shell to confirm the warning is gone. Do not commit.

**Run** (single command, ~ 60s, $0 cost):

```bash
phantom evolve "Fix the 'method persist is never used' warning in core/src/cost.rs ..." \
  --max-rounds 4 --agent coder
```

**What phantom actually did** (one round, three tool calls):

```
── Round 1/4 ───────────────────────────────────────
  ⟳ file_read   {"path":"core/src/cost.rs"}
  ✓ file_read   use std::collections::HashMap; …
  ⟳ file_edit   {"new_string":"    #[allow(dead_code)]\n    fn persist(&self, inner: &Co…
  ✓ file_edit   Edited /Users/marklight/.../core/src/cost.rs successfully
  ⟳ shell       {"command":"cargo check","cwd":"core","timeout_secs":300}
  ✓ shell       STDERR: Checking hyper-rustls v0.27.9 …  Finished `dev` profile
```

**Resulting diff** (the actual change applied by the agent):

```diff
@@ impl CostTracker {
+    #[allow(dead_code)]
     fn persist(&self, inner: &CostTrackerInner) {
         if let Some(parent) = self.path.parent() {
             let _ = std::fs::create_dir_all(parent);
```

**Verification** (manual, after the run):

```bash
cd core && cargo build --release --bin phantom 2>&1 | grep warning
# → no output (0 warnings — was 1 before)
```

## What was wired so this works

1. **Provider chain works without a paid Claude key.** The `coder` agent uses
   `groq` (`llama-3.3-70b-versatile`) as primary. The `opencode` provider in
   the same config falls through to `minimax-m2.5-free` (free tier on the
   opencode.ai zen gateway). Both reliably emit OpenAI-format `tool_calls`
   when given an explicit `tools = [...]` block.
2. **Per-agent tool list is mandatory.** Without `tools = [...]` the agent
   runtime sends zero tool definitions to the LLM and the model hallucinates
   tool calls instead of invoking them. The `coder` agent in
   `~/.phantom-mesh/agents.toml` lists ~25 essential tools (file_*,
   content_search, glob_search, shell, git_*, cargo_check, cargo_test, etc.).
3. **`max_tokens` bumped from 256 to 4096.** Reasoning-style models (minimax,
   nemotron) consume their budget in the thinking phase before content can
   be emitted with the smaller cap. 4096 leaves room for both.
4. **Streaming + visible tool calls in the REPL.** Each round prints
   `⟳ tool_name(args)` on start and `✓ tool_name preview` on completion, so
   the loop is observable as it runs.
5. **`/show <n>` for full output.** Tool results are truncated to 5 lines in
   the live display; `/show 1` (etc.) dumps the complete output.

## Distributed self-evolution (not yet verified)

`phantom evolve --distributed` would split the goal into sub-tasks and
dispatch them to cluster peers (yoyogood / ayaneo / laptop). The wiring
exists; verification is on the validation ladder.

## Cost

$0 on the free Groq tier (and $0 on opencode `*-free` models). The whole
"agent fixes its own warning" loop above cost nothing.
