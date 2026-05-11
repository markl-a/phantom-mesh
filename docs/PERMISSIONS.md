# Permission DSL

phantom-mesh ships a Claude-Code-style **Tool(specifier)** rule grammar
for gating tool execution. Configured via `[permissions]` in
`agents.toml`; enforced by the agent runtime's tool dispatch path.

Source of truth: [`core/src/permission.rs`](../core/src/permission.rs).
6 fuzz tests guard the parser + engine against panic-on-input
(`fuzz_parse_rule_never_panics`, etc).

---

## Quick reference

```toml
[permissions]
deny  = ["Read(./.env)", "Read(./secrets/*)", "WebFetch(domain:badsite.com)"]
ask   = ["Bash", "Edit"]
allow = ["Bash(git status)", "Bash(cargo check)", "Read(./README.md)", "100:Bash(git push origin feature/*)"]
```

Three lists → three actions. Empty/missing block ⇒ allow-all (legacy
default). Any rule present ⇒ unmatched calls fall through to **Ask**.

---

## Syntax

```text
RULE        := [PRIORITY ":"] TOOL [ "(" SPECIFIER ")" ]
PRIORITY    := signed integer (default 0; higher beats lower)
TOOL        := PascalCase name | snake_case name | "*"
SPECIFIER   := tool-specific glob/string (see below)
```

| Form | Example | Effect |
|---|---|---|
| Bare tool name | `Bash` | Matches every shell call |
| Tool + specifier | `Bash(npm run *)` | Matches shell calls whose `command` is glob-equal to `npm run *` |
| Wildcard tool | `*` | Matches every tool call |
| Priority prefix | `100:Bash(git status)` | Same matching as `Bash(git status)` but with priority=100 |

### Tool name aliases (Claude Code parity)

| You write | phantom matches |
|---|---|
| `Bash` or `Shell` | `shell` |
| `Read` | `file_read` |
| `Write` | `file_write` |
| `Edit` | `file_edit`, `file_write`, `multi_file_edit`, `apply_patch` (edit-family collapse) |
| `WebFetch` | `web_fetch` |
| `WebSearch` | `web_search` |
| anything else | passes through verbatim — write `shell` directly if you prefer |

### Specifier shapes per tool

| Tool | Specifier syntax | Matched against |
|---|---|---|
| `Bash` / `Shell` | `cmd-pattern with *` | the `command` argument; `*` matches any chars |
| `Read` / `Write` / `Edit` | `path-glob with *` | the `path` argument |
| `WebFetch` | `domain:host.com` (or just `host.com`) | URL host (subdomain match: `github.com` ⇒ also `api.github.com`) |
| anything else | fallback: substring on `path` / `cmd` / `url` / serialised args | first non-empty wins |

Specifiers are **anchored** (must match the whole arg) — except for
domain matching which is host-suffix-aware.

---

## Evaluation order

Rules are sorted **descending** by `(user_priority, action_precedence)`:

1. **Higher numeric priority wins**. `100:Bash(git status)` beats
   default-priority `Bash`.
2. Among **same priority**: deny > ask > allow.
3. **First match wins** — rule list is scanned top to bottom in the
   sorted order; the first rule that matches the (tool, args) pair
   produces the decision.
4. **No match** ⇒ if the engine has any rules, fall through to
   `Decision::Ask`. If the engine is empty, fall through to
   `Decision::Allow` (preserves legacy `PHANTOM_PERM=allow`).

This matches Claude Code's documented order (deny → ask → allow,
first-match-wins) **plus** an escape-hatch via the priority field.
Without priority you can't allow `git status` while denying every
other shell call — phantom's engine adds priority specifically for
this case.

---

## Bash hardening: redirect/chain auto-downgrade

Bash is the most dangerous tool surface — a single redirect (`>`,
`>>`, `|`, `<`) or chain operator (`;`, `&&`, `||`) turns an "allowed"
command into something that can leak files. phantom's engine
**automatically downgrades** an `Allow` decision to `Ask` when the
matched command contains any of those operators.

```toml
[permissions]
allow = ["Bash(cat *)"]
```

```bash
# Allowed:  cat README.md           → Decision::Allow
# Allowed:  cat ./.env              → Decision::Allow  (path-glob ignored at this level — see Read rules)
# DOWNGRADED: cat secrets > /tmp/x  → Decision::Ask    (redirect detected)
# DOWNGRADED: cat README.md | wc    → Decision::Ask    (pipe detected)
# DOWNGRADED: cat a; rm b           → Decision::Ask    (chain detected)
```

Quoted operators are **respected** — `echo 'a > b'` doesn't trigger.
Implementation: `bash_has_redirect_or_chain()` walks the command
char-by-char honoring single + double quotes and backslash escapes.

If you intentionally want to allow redirects (e.g. `Bash(tee
./logs/*.log)`), use a more specific pattern + raise priority:
```toml
allow = ["100:Bash(tee ./logs/*)"]
```
The redirect-downgrade rule applies to the *matched* command, so
exact patterns can opt back in at high priority.

---

## Statically-denied tools

A `Deny` rule with **no specifier** (e.g. `WebFetch`) means "this tool
is blanket-denied — never callable in any args". The engine surfaces
these via `Engine::statically_denied_tools()`. The agent runtime's
`run_with_callbacks_gated()` filters them OUT of the LLM's tool-list
schema, so the model never proposes a tool it can't run.

```toml
[permissions]
deny = ["WebFetch", "WebSearch"]
```
⇒ The LLM doesn't even know `web_fetch` and `web_search` exist. No
"allow"+"deny" Lambo dance per turn — the model stays in scope.

Conditional denies (e.g. `Bash(rm -rf *)`) do **not** statically
exclude `Bash` — they fire only on matching args, so the tool stays
in the schema.

---

## Examples — common policies

### "Personal dev mode" (most permissive)

```toml
[permissions]
deny = ["Read(./.env)", "Read(./secrets/*)"]
allow = ["*"]
```
Allow everything except touching secrets. No prompts.

### "Production-careful" (always ask before writes)

```toml
[permissions]
deny  = ["Read(./.env)", "Bash(rm -rf *)"]
ask   = ["Edit", "Write", "Bash"]
allow = [
  "Bash(git status)", "Bash(git diff)", "Bash(git log *)",
  "Bash(cargo check)", "Bash(cargo build)", "Bash(cargo test)",
  "Read(./*)",
]
```
Read-only operations + safe git/cargo commands run silently. Anything
that mutates files or runs novel shell prompts.

### "CI auto-deny shell"

```toml
[permissions]
deny  = ["Bash"]
ask   = []
allow = ["Read(./*)", "Edit(./src/**)"]
```
Tightest sandbox: no shell at all, can read source, can edit only
files under `./src/`.

---

## Diagnostics

`phantom doctor` includes a `[permissions]` section showing:
- Number of rules parsed
- Static-deny tool list (the ones the LLM won't see)
- Parse errors per rule (if any)

```
permissions
  ✓ [permissions]: 4 rules parsed (2 deny, 1 ask, 1 allow)
  ✓ statically denied: web_fetch (will be hidden from LLM tool list)
```

If `phantom doctor` shows `parse error: unterminated specifier in rule
"Bash(unclosed"` etc., the offending line is named verbatim so you can
fix `agents.toml` and re-run.

---

## Legacy `PHANTOM_PERM` env var

Pre-DSL behavior is preserved as a fallback when the engine returns
`Decision::Ask` (no rule matched, default state):

| Env value | Effect |
|---|---|
| `allow` (default) | Engine `Ask` ⇒ allow |
| `ask` | Engine `Ask` ⇒ interactive y/n prompt |
| `deny` | Engine `Ask` ⇒ deny |
| `diff` | Engine `Ask` ⇒ render unified diff for file_edit, then prompt |

Once you've got `[permissions]` rules covering your real cases, set
`PHANTOM_PERM=ask` so unmatched calls bring up the prompt instead of
silently allowing — that's where new policy gaps surface.

---

## Implementation pointers

- Parser: `permission::parse_rule(s, action)` → `Vec<Rule>` (multiple
  rules per call when an alias expands, e.g. `Edit(...)` returns 4
  rules for the 4 edit-family tools).
- Engine: `permission::Engine::from_lists(deny, ask, allow)` →
  `Engine`; sorts internally; `engine.evaluate(tool, args)` →
  `Decision`.
- Helpers: `wildcard_match(pat, text)`, `bash_segments(cmd)`,
  `bash_has_redirect_or_chain(cmd)`, `host_matches(host, url)`.
- Tests: 26 unit + 6 fuzz in `core/src/permission.rs`.
