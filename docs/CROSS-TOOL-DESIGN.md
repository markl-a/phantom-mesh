# phantom ↔ Other AI Tools — Context Sharing & Cross-Tool Invocation

**Status**: design / analysis (not implemented).
**Target**: v0.2 (after 5/15 OSS launch).
**Estimated cost**: 4-5 days for the full surface.

The user wants two distinct things:

1. **Context sharing** — phantom can hand over its current state (recent
   conversation, files touched, decisions, TODO list, repo context) to
   other AI tools (Claude Code, Codex, Gemini CLI, Antigravity, Aider…)
   so the next tool picks up where phantom left off.

2. **Cross-tool invocation** — from inside phantom, treat another AI
   CLI as a callable tool. Like `delegate(tool="gemini", prompt="…")`
   — same shape as today's `subagent` tool, but the "agent" is a
   *different product* not a different *machine*.

## TL;DR — feasibility

| Feature | Doable? | Why |
|---|---|---|
| Context export to Claude Code (CLAUDE.md / @-include) | ✅ trivial | already write AGENTS.md-like files |
| Context export to Codex (transcript) | ✅ medium | Codex has its own format; convert from JSONL |
| Context export to Gemini CLI | ✅ trivial | accepts stdin / `@file` |
| Context export to Antigravity | ⚠️ partial | Antigravity's CLI is VS Code-shell (`-d`, `-g`, `--diff`); no LLM-prompt flag. Best we can do: drop a context file into the project that Antigravity's in-IDE chat reads |
| Invoke `gemini` CLI as a tool | ✅ ~half day | spawn subprocess, pipe stdin, read stdout |
| Invoke `codex` CLI as a tool | ✅ ~half day | same shape |
| Invoke `claude` CLI (Claude Code) as a tool | ✅ ~half day | same shape; auth via `ANTHROPIC_API_KEY` |
| Invoke Antigravity as a "do this for me" agent | ❌ today | no headless / prompt-API in current Antigravity build (1.107.0) |
| Bidirectional context (read other tool's state INTO phantom) | ⚠️ tool-by-tool | Claude Code's session JSONL is reasonable to read; Codex similar; Antigravity / Cline / Continue are IDE-side, harder |

**No hard technical blockers**, just plumbing + per-tool adapter work.

## Design

### Feature A — `/share` slash + `phantom share` subcommand

```
/share claude                  # write ~/.phantom-mesh/share/claude-<sid>.md
/share codex                   # write ~/.phantom-mesh/share/codex-<sid>.md
/share gemini                  # write ~/.phantom-mesh/share/gemini-<sid>.md
/share antigravity             # write .antigravity/context.md inside the project
/share AGENTS.md --append      # append "## Recent context" block to project's AGENTS.md
/share --copy                  # put on clipboard (uses `pbcopy` / `xclip` / `clip`)
/share --to <path>             # write anywhere
```

The format converters live in a new `core/src/share.rs` module:

```rust
pub enum ShareFormat {
    ClaudeCode,    // CLAUDE.md-flavored markdown with `@file` includes
    Codex,         // transcript JSON (one message per line, OpenAI-style)
    Gemini,        // plain markdown with code fences
    Antigravity,   // CLAUDE.md but written to .antigravity/context.md
    Generic,       // plain markdown — what /export does today
    Agents,        // AGENTS.md insert (`## Recent context` block)
}

pub fn render(history: &[ChatMessage], format: ShareFormat) -> String { … }
```

`/export` (already shipped) becomes a thin wrapper around `share::render`
with `Generic`. New formats reuse the same data.

### Feature B — `delegate` tool

A new tool registered next to `subagent`:

```toml
# ~/.phantom-mesh/agents.toml — top-level [tools.delegate] section
[tools.delegate]
default_context = true               # auto-include current session's last 6 turns
default_timeout_secs = 120

[tools.delegate.targets.gemini]
cmd       = "gemini"                 # or absolute path
args      = ["-"]                    # `-` = read from stdin
context_via = "stdin_prepend"        # how we ship the context

[tools.delegate.targets.codex]
cmd       = "codex"
args      = ["chat"]
context_via = "context_flag"
context_flag = "--context-file"

[tools.delegate.targets.claude]
cmd       = "claude"
args      = []
context_via = "stdin_prepend"

[tools.delegate.targets.antigravity]
cmd       = "/Applications/Antigravity.app/Contents/Resources/app/bin/antigravity"
mode      = "open_project"          # NOT a prompt; just opens the project
share_to  = ".antigravity/context.md"  # where the context gets written
```

Tool schema (what Claude Code / phantom REPL sees):

```json
{
  "name": "delegate",
  "description": "Delegate the current task to another AI CLI (gemini, codex, claude, …) and return its response.",
  "parameters": {
    "tool":    {"type": "string", "enum": ["gemini","codex","claude","antigravity","aider"]},
    "prompt":  {"type": "string"},
    "with_context": {"type": "boolean", "default": true},
    "context_session": {"type": "string", "description": "Override which session to ship; default = current"}
  }
}
```

Implementation (`core/src/tools/delegate.rs`):

```rust
pub async fn delegate(args: &Value, cfg: &ToolsConfig) -> String {
    let tool   = args.get("tool")?.as_str()?;
    let prompt = args.get("prompt")?.as_str()?;
    let target = cfg.delegate.targets.get(tool)?;

    // 1. Get the context (if requested)
    let context_md = if args.get("with_context").as_bool().unwrap_or(true) {
        let sid = args.get("context_session").as_str().unwrap_or(&CURRENT_SESSION);
        let history = ConversationStore::default().get_history(sid).await;
        share::render(&history, share::format_for(tool))
    } else {
        String::new()
    };

    // 2. Spawn the subprocess
    let mut cmd = std::process::Command::new(&target.cmd);
    cmd.args(&target.args);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    // 3. Ship context based on mode
    match target.context_via.as_str() {
        "stdin_prepend" => {
            let mut child = cmd.spawn()?;
            let stdin = child.stdin.take()?;
            stdin.write_all(format!("{}\n\n---\n\n{}\n", context_md, prompt).as_bytes())?;
            // capture stdout, return
        }
        "context_flag" => {
            let tmp = write_to_tempfile(&context_md)?;
            cmd.args([&target.context_flag, &tmp.path().to_str().unwrap()]);
            cmd.stdin(prompt.as_bytes());
            // capture
        }
        "open_project" => {
            // Antigravity: write context file, spawn IDE detached
            let project_root = workspace::root()?;
            std::fs::write(project_root.join(&target.share_to), context_md)?;
            cmd.args(&[project_root.to_str().unwrap()]);
            cmd.spawn()?.wait();   // user takes over, no stdout to return
            return format!("Opened {} in {}; context dropped at {}", project_root.display(), tool, target.share_to);
        }
        _ => unreachable!()
    }
}
```

### Feature C — read OTHER tools' state into phantom

This is the reverse: phantom resumes from where Claude Code left off.

```
phantom --import claude-code        # read ~/.claude/sessions/<latest>.jsonl
phantom --import codex              # read codex transcript
phantom --import-file <path>        # explicit
```

Convert each format into phantom's `ChatMessage` shape, append to a new
session, run as if continuing.

Lower priority than A+B but trivial to add once the share converters exist
(just run them in reverse).

## What ALREADY works (that the user might not know)

- `/copy all` and `/export` already produce a neutral markdown (not bound
  to any tool's format). User can paste into ANY chat and that tool reads
  it. So the **manual workflow** for "share with another tool" is already
  one slash command + ⌘V away.
- `AGENTS.md` is already auto-loaded. If phantom WRITES to AGENTS.md the
  next time it runs (or any other tool that respects AGENTS.md), state
  carries forward.
- phantom's MCP server lets Claude Code / Codex etc. INVOKE phantom. So
  the inverse of feature B is already done.

## What's tricky

### Auth juggling

Each delegated tool has its own auth:
- `gemini` — `~/.config/google-cloud-sdk` or `GOOGLE_APPLICATION_CREDENTIALS`
- `codex` — `OPENAI_API_KEY`
- `claude` — `ANTHROPIC_API_KEY`
- `aider` — env vars OR `~/.aider/config`
- `antigravity` — its own login

phantom needs to NOT clobber these or expose them between tools. Each
delegate target inherits the parent's env (which is fine — user's shell
already has them set).

### Cost accounting across tools

Today phantom's CostTracker only tracks API calls phantom itself made.
When we delegate to `gemini`, gemini's cost is invisible to us. v0.2
acceptable; v0.3 might add `delegate_cost_usd` field that the target's
final response is parsed for (most CLIs print "tokens used: N").

### Antigravity-specific

Antigravity 1.107.0's CLI is VS-Code-shell shape:
```
antigravity [paths...]                # open files/folders
-d --diff <file> <file>               # diff
-m --merge ...                        # 3-way merge
-g --goto <file:line>                 # open at position
-w --wait                             # block until file closed
```

No `--prompt`, no headless agent invocation. So `delegate(tool="antigravity")`
can ONLY do `mode = "open_project"`: write context to a file Antigravity
will read (e.g. `.antigravity/context.md` or AGENTS.md), open the project,
let the user continue interactively.

If Antigravity later ships a `--ask` or `--prompt` flag, just update the
toml target and we're done.

### Standards landscape

- **MCP** (Anthropic): tool protocol, NOT context sync. We're already an
  MCP server.
- **AGENTS.md**: convention, multiple tools read it. Reasonable lowest
  common denominator.
- **OpenAI Agents API**: not a CLI standard.
- **None of the IDE-resident tools** (Cline, Continue, Antigravity) have
  documented import/export schemas. We adapt as they evolve.

## Delivery plan (after 5/15 OSS launch)

**Day 1** — `core/src/share.rs` with `ShareFormat` + 5 converters.
            `/export --format` flag (currently `/export` defaults to Generic).
            `/share <format>` slash that writes to `~/.phantom-mesh/share/`.
**Day 2** — `core/src/tools/delegate.rs` + agents.toml schema.
            3 backends wired: gemini, codex, claude.
**Day 3** — Auth-passthrough verification + per-target stderr handling
            (capture but don't pollute stdout of delegate's response).
            Antigravity `open_project` mode.
**Day 4** — `phantom --import <tool>` reverse direction.
**Day 5** — Docs + 4 demo scripts:
            - "Code reviewed by 3 tools" (phantom → delegate to gemini → delegate to codex → synthesize)
            - "Continue this in Antigravity" (phantom /share antigravity → switch to GUI)
            - "Resume Claude Code session in phantom" (`phantom --import claude-code`)
            - Cost: how much each tool added.

**Total**: 4-5 days of focused work after launch. None of it is high-risk.

## Risks

- **CLI flag stability**: if `gemini` 2.0 changes flag names, we adapt
  the toml target. No code change.
- **Auth surprises**: delegated tool fails auth → phantom captures stderr,
  surfaces to user with hint to set `ANTHROPIC_API_KEY` etc.
- **Path sensitivity**: tools resolve `cmd` via PATH; if user has multiple
  versions, default `which` order wins. Same as `which gemini` from a
  shell.

## What this enables (user-facing)

```
# Phantom REPL
> 我寫好的 PR diff 在 ~/work/repo, 請 gemini 看看，再給 claude review,
> 兩家結果合在一起寫 review 留言
```

Phantom internally calls:
1. `delegate(tool=gemini, prompt="review the diff at ...", with_context=true)`
2. `delegate(tool=claude, prompt="review the same diff", with_context=true)`
3. (master agent) synthesize the two responses

Or:

```
> 我這個 session 切過去 antigravity 繼續寫
> /share antigravity
> # phantom 寫 .antigravity/context.md 並 spawn antigravity 開 project
```

User switches tool, doesn't lose state.

## Decision needed

**Do we ship this in v0.1.0 (5/1) or v0.2 (5/15+)?**

Recommendation: v0.2. Reasons:
1. v0.1.0's surface is already large (mesh, MCP, evolve, snapshots). Don't
   pile on more.
2. The MANUAL workaround for share (`/export` + paste) is already adequate.
3. The cross-tool delegate is a **growth** feature, not core. Easier to
   sell post-launch as "now phantom orchestrates the rest of your AI
   stack" than to ship as part of "what is phantom?".

But the v0.2 build is independent of broker / billing / Apple Sign In, so
it can run in parallel with the SaaS line.
