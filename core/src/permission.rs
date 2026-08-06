//! Permission rule engine — Claude-Code-style `Tool(specifier)` DSL.
//!
//! Spectyn previously gated tool execution with three env vars:
//! `SPECTYN_PERM=allow|ask|deny|diff` plus a per-session "always allow"
//! `HashSet<String>`. That's enough for a single-user smoke test, but
//! a real config wants finer grain — e.g. "always allow `git status`
//! and `cargo check`, ask before any other shell command, never run
//! anything that touches `.env`." This module is the data + engine for
//! that DSL; `bin/spectyn.rs` and the TUI gate plug in via
//! [`Engine::evaluate`].
//!
//! ## Syntax
//!
//! ```text
//! Bash               → whole-tool rule, any args
//! Bash(npm run *)    → matches `npm run` followed by anything
//! Read(./.env)       → exact path
//! Read(./secrets/*)  → glob path
//! WebFetch(domain:github.com) → host equals github.com (or *.github.com)
//! *                  → every tool
//! ```
//!
//! Tool names use Claude-Code's PascalCase (`Bash`, `Read`, `Edit`,
//! `Write`, `WebFetch`) but match spectyn's snake_case tool names via
//! [`canonical_tool_name`]: `Bash` → `shell`, `Read` → `file_read`,
//! `Edit` → any of {`file_edit`, `file_write`, `multi_file_edit`,
//! `apply_patch`} (the OpenCode "edit-family collapse"), `WebFetch`
//! → `web_fetch`. Unknown PascalCase names fall through to the snake
//! version, so any tool reachable from spectyn's dispatcher can be
//! gated.
//!
//! ## Evaluation order
//!
//! Sorted descending by `(action_priority, user_priority, source_order)`:
//! * `action_priority`: deny=2, ask=1, allow=0 (deny wins; among ties,
//!   ask wins over allow). Matches Claude Code's documented order.
//! * `user_priority`: numeric `priority` field (default 0). Higher
//!   beats lower, regardless of action — gives users an escape hatch
//!   to upgrade an `allow` over a deny.
//! * `source_order`: insertion order is the final tiebreaker.
//!
//! First match wins. If nothing matches, the result is [`Decision::Ask`]
//! by default — safer than silent allow.
//!
//! ## Bash hardening
//!
//! When a rule with action `Allow` matches a bash invocation that
//! contains a redirect (`>`, `>>`, `|`, `<`) or chained commands (`;`,
//! `&&`, `||`), the decision is automatically downgraded to `Ask`.
//! Modeled on Gemini CLI's redirect-aware shell policy
//! (`packages/core/src/policy/policy-engine.ts`). Without this, an
//! allow-list of `Bash(cat *)` would silently green-light
//! `cat secrets > /tmp/exfil`.

use std::collections::HashSet;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
    Ask,
}

impl Action {
    fn precedence(self) -> i32 {
        match self {
            Action::Deny => 2,
            Action::Ask => 1,
            Action::Allow => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    Ask,
}

#[derive(Debug, Clone)]
pub enum Specifier {
    /// `Bash(npm run *)` — matched against the bash command string with
    /// `*` wildcards. Also triggers segment + redirect inspection.
    BashCommand(String),
    /// `Read(./.env)` / `Read(./secrets/*)` — `*` glob matched against
    /// the tool's `path` argument.
    PathGlob(String),
    /// `WebFetch(domain:github.com)` — host equality + subdomain match.
    Domain(String),
    /// `Tool(literal)` — generic substring/glob fallback for tools we
    /// haven't taught the engine about.
    Generic(String),
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub action: Action,
    /// Canonical spectyn tool name (`shell`, `file_read`, …) or `*`.
    /// Edit-family rules expand to multiple Rule rows at parse time.
    pub tool: String,
    pub specifier: Option<Specifier>,
    /// User-supplied priority; higher beats lower among same-action peers.
    pub priority: i32,
    /// Free-form provenance for diagnostics ("agents.toml:permissions.deny[3]").
    pub source: String,
}

pub struct Engine {
    rules: Vec<Rule>,
}

impl Engine {
    /// Build an engine, sorting rules in evaluation order so
    /// [`evaluate`] can short-circuit on first match.
    ///
    /// Sort key (descending): `(user_priority, action_precedence)`.
    /// Putting `priority` ahead of `action_precedence` is what gives
    /// users the documented "I want to allow `git status` even though
    /// I deny every shell" escape hatch — set `priority=100` on the
    /// allow and it beats any default-priority deny. Among
    /// same-priority peers, deny still wins over ask wins over allow,
    /// matching Claude Code's documented order.
    pub fn new(mut rules: Vec<Rule>) -> Self {
        rules.sort_by_key(|r| std::cmp::Reverse((r.priority, r.action.precedence())));
        Self { rules }
    }

    pub fn from_lists(deny: &[&str], ask: &[&str], allow: &[&str]) -> Result<Self, String> {
        let mut rules = Vec::new();
        for (action, list) in [
            (Action::Deny, deny),
            (Action::Ask, ask),
            (Action::Allow, allow),
        ] {
            for (idx, raw) in list.iter().enumerate() {
                let mut parsed = parse_rule(raw, action)?;
                for r in parsed.iter_mut() {
                    r.source = format!("{:?}[{}]: {}", action, idx, raw);
                }
                rules.extend(parsed);
            }
        }
        Ok(Self::new(rules))
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Evaluate a tool call. Returns the first matching rule's decision,
    /// with bash-redirect downgrade applied. Default (no match) is `Ask`
    /// when the engine has any rules, else `Allow` (preserves
    /// "permissions section omitted ⇒ legacy unrestricted" behaviour).
    pub fn evaluate(&self, tool: &str, args: &Value) -> Decision {
        if self.rules.is_empty() {
            return Decision::Allow;
        }
        for rule in &self.rules {
            if !tool_matches(&rule.tool, tool) {
                continue;
            }
            if !specifier_matches(rule.specifier.as_ref(), tool, args) {
                continue;
            }
            // Bash-specific hardening: if an Allow rule matches but the
            // command has a redirect or chain, downgrade to Ask. This
            // is what stops `Bash(cat *)` from green-lighting
            // `cat secrets > /tmp/exfil`.
            if rule.action == Action::Allow && tool == "shell" {
                let cmd = args
                    .get("cmd")
                    .or_else(|| args.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if bash_has_redirect_or_chain(cmd) {
                    return Decision::Ask;
                }
            }
            return match rule.action {
                Action::Allow => Decision::Allow,
                Action::Deny => Decision::Deny(format!(
                    "denied by rule {} (matched tool='{}')",
                    rule.source, rule.tool
                )),
                Action::Ask => Decision::Ask,
            };
        }
        Decision::Ask
    }

    /// Tools that no rule can ever allow — every Deny rule whose
    /// specifier is `None` (i.e. blanket deny). Caller can drop their
    /// schemas before sending the tool list to the LLM, so the model
    /// doesn't waste a turn proposing them. Modeled on Gemini CLI's
    /// `getExcludedTools()`.
    pub fn statically_denied_tools(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for rule in &self.rules {
            if rule.action == Action::Deny && rule.specifier.is_none() && rule.tool != "*" {
                out.insert(rule.tool.clone());
            }
        }
        out
    }
}

/// Map a Claude-Code-style PascalCase tool name to one or more spectyn
/// snake_case tool names. Returns the input unchanged (single-element)
/// if no alias is known, so user-defined / MCP tools can be gated by
/// their literal names too.
pub fn canonical_tool_name(s: &str) -> Vec<String> {
    match s {
        "Bash" | "Shell" => vec!["shell".into()],
        "Read" => vec!["file_read".into()],
        "Write" => vec!["file_write".into()],
        // Edit-family collapse: one rule covers all 4 mutation tools so
        // a user writing `Edit(./src/**)` doesn't have to remember
        // which of file_edit/file_write/multi_file_edit/apply_patch the
        // model picked.
        "Edit" => vec![
            "file_edit".into(),
            "file_write".into(),
            "multi_file_edit".into(),
            "apply_patch".into(),
        ],
        "WebFetch" => vec!["web_fetch".into()],
        "WebSearch" => vec!["web_search".into()],
        // Already snake_case / explicit spectyn name / wildcard.
        other => vec![other.to_string()],
    }
}

/// Parse one rule string. Returns a `Vec` because edit-family aliases
/// expand to multiple rows — a single `Edit(...)` rule produces one
/// rule per backing tool name.
pub fn parse_rule(s: &str, action: Action) -> Result<Vec<Rule>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty rule".into());
    }
    // Optional priority prefix: `100:Bash(...)` — keeps the common case
    // (no priority) syntactically clean while letting power users
    // override the default.
    let (priority, rest) = match s
        .find(':')
        .and_then(|i| s[..i].parse::<i32>().ok().map(|p| (p, &s[i + 1..])))
    {
        Some((p, r)) => (p, r),
        None => (0, s),
    };
    let rest = rest.trim();
    // Parse `Name(spec)` or `Name`.
    let (name, spec_str) = match rest.find('(') {
        Some(i) if rest.ends_with(')') => (&rest[..i], Some(&rest[i + 1..rest.len() - 1])),
        Some(_) => return Err(format!("unterminated specifier in rule {:?}", s)),
        None => (rest, None),
    };
    let canonical_names = canonical_tool_name(name.trim());
    let specifier = match spec_str {
        None => None,
        Some(raw) => Some(parse_specifier(name.trim(), raw)?),
    };
    Ok(canonical_names
        .into_iter()
        .map(|tool| Rule {
            action,
            tool,
            specifier: specifier.clone(),
            priority,
            source: String::new(),
        })
        .collect())
}

fn parse_specifier(tool_name: &str, raw: &str) -> Result<Specifier, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty specifier".into());
    }
    Ok(match tool_name {
        "Bash" | "Shell" => Specifier::BashCommand(raw.to_string()),
        "Read" | "Write" | "Edit" => Specifier::PathGlob(raw.to_string()),
        "WebFetch" => {
            let host = raw.strip_prefix("domain:").unwrap_or(raw).to_string();
            Specifier::Domain(host)
        }
        _ => Specifier::Generic(raw.to_string()),
    })
}

fn tool_matches(rule_tool: &str, actual_tool: &str) -> bool {
    rule_tool == "*" || rule_tool == actual_tool
}

fn specifier_matches(spec: Option<&Specifier>, tool: &str, args: &Value) -> bool {
    let Some(spec) = spec else {
        return true;
    };
    match spec {
        Specifier::BashCommand(pat) => {
            let cmd = args
                .get("cmd")
                .or_else(|| args.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Match each segment so `Bash(git status)` permits
            // `git status && git diff` to pass *the git-status part*
            // (the chain itself separately triggers the redirect-or-
            // chain downgrade in evaluate()).
            let segments = bash_segments(cmd);
            segments.iter().any(|seg| wildcard_match(pat, seg.trim()))
        }
        Specifier::PathGlob(pat) => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            wildcard_match(pat, path)
        }
        Specifier::Domain(host) => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            host_matches(host, url)
        }
        Specifier::Generic(pat) => {
            // Try common arg names; fall back to JSON serialisation.
            let s = args
                .get("path")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("cmd").and_then(|v| v.as_str()))
                .or_else(|| args.get("url").and_then(|v| v.as_str()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| serde_json::to_string(args).unwrap_or_default());
            let _ = tool;
            wildcard_match(pat, &s)
        }
    }
}

/// Tiny `*` wildcard matcher. Anchored at both ends; `*` matches any
/// run of characters including empty. Avoids pulling in the `glob`
/// crate for what amounts to a 30-line algorithm.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn rec(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some('*'), _) => {
                // Try matching zero, one, two, … chars from `t`.
                if rec(&p[1..], t) {
                    return true;
                }
                if t.is_empty() {
                    return false;
                }
                rec(p, &t[1..])
            }
            (Some(_), None) => false,
            (Some(pc), Some(tc)) => pc == tc && rec(&p[1..], &t[1..]),
        }
    }
    rec(&p, &t)
}

fn host_matches(rule_host: &str, url: &str) -> bool {
    // Pull the host out of the URL without bringing in the `url` crate.
    let host_in_url = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split('@')
        .last()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    host_in_url == rule_host || host_in_url.ends_with(&format!(".{}", rule_host))
}

/// Split a bash command into segments at top-level chain operators.
/// Returns segments without the operators. Quotes and escapes are
/// honoured so a literal `||` inside `'…'` doesn't false-split.
pub fn bash_segments(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            cur.push(c);
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if !in_double && c == '\'' {
            in_single = !in_single;
            cur.push(c);
            i += 1;
            continue;
        }
        if !in_single && c == '"' {
            in_double = !in_double;
            cur.push(c);
            i += 1;
            continue;
        }
        if !in_single && !in_double {
            // Two-char operators first.
            if i + 1 < chars.len() {
                let pair = (chars[i], chars[i + 1]);
                if matches!(pair, ('&', '&') | ('|', '|')) {
                    out.push(std::mem::take(&mut cur));
                    i += 2;
                    continue;
                }
            }
            if matches!(c, ';' | '|') {
                out.push(std::mem::take(&mut cur));
                i += 1;
                continue;
            }
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `true` when the command contains a redirect (`>`, `>>`, `<`) or a
/// chain operator (`|`, `||`, `&&`, `;`) at top level. Allow rules are
/// downgraded to Ask when this is true — closes the
/// `Bash(cat *)` → `cat secrets > /tmp/exfil` exfiltration hole.
pub fn bash_has_redirect_or_chain(cmd: &str) -> bool {
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if !in_double && c == '\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if !in_single && c == '"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double {
            if matches!(c, '>' | '<' | '|' | ';') {
                return true;
            }
            if c == '&' && i + 1 < chars.len() && chars[i + 1] == '&' {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_bare_tool() {
        let r = parse_rule("Bash", Action::Allow).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].tool, "shell");
        assert!(r[0].specifier.is_none());
    }

    #[test]
    fn parse_bash_with_specifier() {
        let r = parse_rule("Bash(npm run *)", Action::Allow).unwrap();
        assert!(matches!(r[0].specifier, Some(Specifier::BashCommand(ref p)) if p == "npm run *"));
    }

    #[test]
    fn parse_edit_family_expands() {
        let r = parse_rule("Edit(./src/**)", Action::Ask).unwrap();
        let names: Vec<_> = r.iter().map(|r| r.tool.as_str()).collect();
        assert!(names.contains(&"file_edit"));
        assert!(names.contains(&"file_write"));
        assert!(names.contains(&"multi_file_edit"));
        assert!(names.contains(&"apply_patch"));
    }

    #[test]
    fn parse_priority_prefix() {
        let r = parse_rule("100:Bash(git status)", Action::Allow).unwrap();
        assert_eq!(r[0].priority, 100);
        assert_eq!(r[0].tool, "shell");
    }

    #[test]
    fn parse_unterminated_specifier_errors() {
        assert!(parse_rule("Bash(unclosed", Action::Allow).is_err());
    }

    #[test]
    fn wildcard_match_basics() {
        assert!(wildcard_match("npm run *", "npm run build"));
        assert!(wildcard_match("git *", "git status"));
        assert!(wildcard_match("./.env", "./.env"));
        assert!(!wildcard_match("npm run *", "yarn run build"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("./secrets/*", "./secrets/key.pem"));
    }

    #[test]
    fn host_matches_subdomains_but_not_unrelated() {
        assert!(host_matches("github.com", "https://github.com/x/y"));
        assert!(host_matches("github.com", "https://api.github.com/x"));
        assert!(!host_matches("github.com", "https://githubXcom/x"));
        assert!(!host_matches("github.com", "https://example.com/"));
    }

    #[test]
    fn bash_segments_split_on_chain_operators() {
        let segs = bash_segments("git status && git diff | head ; ls");
        assert_eq!(segs, vec!["git status", "git diff", "head", "ls"]);
    }

    #[test]
    fn bash_segments_respect_quotes() {
        let segs = bash_segments("echo 'a && b' && echo c");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], "echo 'a && b'");
    }

    #[test]
    fn bash_redirect_detected() {
        assert!(bash_has_redirect_or_chain("cat secrets > /tmp/x"));
        assert!(bash_has_redirect_or_chain("a | b"));
        assert!(bash_has_redirect_or_chain("a && b"));
        assert!(bash_has_redirect_or_chain("a ; b"));
        assert!(!bash_has_redirect_or_chain("git status"));
        assert!(!bash_has_redirect_or_chain("echo 'a > b'"));
    }

    fn engine(deny: &[&str], ask: &[&str], allow: &[&str]) -> Engine {
        Engine::from_lists(deny, ask, allow).expect("rule parse")
    }

    #[test]
    fn empty_engine_allows_everything() {
        let e = Engine::new(Vec::new());
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "rm -rf /"})),
            Decision::Allow
        );
    }

    #[test]
    fn deny_beats_allow_on_same_tool() {
        let e = engine(&["Bash(rm *)"], &[], &["Bash"]);
        let dec = e.evaluate("shell", &json!({"cmd": "rm -rf node_modules"}));
        assert!(matches!(dec, Decision::Deny(_)));
    }

    #[test]
    fn allow_specific_then_ask_fallback() {
        let e = engine(&[], &[], &["Bash(git status)"]);
        // Matched allow rule
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "git status"})),
            Decision::Allow
        );
        // Non-matching command falls through to default Ask.
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "rm -rf /"})),
            Decision::Ask
        );
    }

    #[test]
    fn allow_downgraded_to_ask_when_redirect_present() {
        let e = engine(&[], &[], &["Bash(cat *)"]);
        // Direct cat: allowed.
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "cat README.md"})),
            Decision::Allow
        );
        // Cat with redirect: must downgrade to Ask, even though the
        // first segment matches the allow pattern.
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "cat secrets > /tmp/exfil"})),
            Decision::Ask
        );
    }

    #[test]
    fn read_path_glob() {
        let e = engine(&["Read(./.env)", "Read(./secrets/*)"], &[], &[]);
        assert!(matches!(
            e.evaluate("file_read", &json!({"path": "./.env"})),
            Decision::Deny(_)
        ));
        assert!(matches!(
            e.evaluate("file_read", &json!({"path": "./secrets/key.pem"})),
            Decision::Deny(_)
        ));
        // Unrelated path: default Ask (no allow rule).
        assert_eq!(
            e.evaluate("file_read", &json!({"path": "./README.md"})),
            Decision::Ask
        );
    }

    #[test]
    fn webfetch_domain_match() {
        let e = engine(&[], &[], &["WebFetch(domain:github.com)"]);
        assert_eq!(
            e.evaluate("web_fetch", &json!({"url": "https://github.com/x/y"})),
            Decision::Allow
        );
        assert_eq!(
            e.evaluate("web_fetch", &json!({"url": "https://api.github.com/repos"})),
            Decision::Allow
        );
        assert_eq!(
            e.evaluate("web_fetch", &json!({"url": "https://evil.com/"})),
            Decision::Ask
        );
    }

    #[test]
    fn edit_family_collapses_to_one_rule() {
        let e = engine(&["Edit(./.git/*)"], &[], &[]);
        for tool in &["file_edit", "file_write", "multi_file_edit", "apply_patch"] {
            let dec = e.evaluate(tool, &json!({"path": "./.git/config"}));
            assert!(
                matches!(dec, Decision::Deny(_)),
                "{} should be denied for ./.git/* edit",
                tool
            );
        }
    }

    #[test]
    fn statically_denied_lists_blanket_denies() {
        let e = engine(&["WebFetch", "Bash(rm *)"], &[], &[]);
        let denied = e.statically_denied_tools();
        assert!(
            denied.contains("web_fetch"),
            "blanket WebFetch deny should appear; got {:?}",
            denied
        );
        // Bash(rm *) is conditional on cmd — must NOT appear.
        assert!(
            !denied.contains("shell"),
            "conditional shell deny must not be listed; got {:?}",
            denied
        );
    }

    #[test]
    fn priority_overrides_action_precedence() {
        // Allow with high priority beats a low-priority deny — escape
        // hatch for "I want to allow git status even though I deny
        // every shell-tool by default."
        let mut rules = parse_rule("Bash", Action::Deny).unwrap();
        rules.extend(parse_rule("100:Bash(git status)", Action::Allow).unwrap());
        let e = Engine::new(rules);
        assert_eq!(
            e.evaluate("shell", &json!({"cmd": "git status"})),
            Decision::Allow
        );
    }

    #[test]
    fn wildcard_tool_rule() {
        let e = engine(&["*"], &[], &[]);
        assert!(matches!(
            e.evaluate("anything_at_all", &json!({})),
            Decision::Deny(_)
        ));
    }

    // ── Crash-resistance fuzz ────────────────────────────────────────────
    //
    // The permission engine is a SECURITY surface — a panic in any of its
    // hot paths (parse_rule from agents.toml, Engine::evaluate per
    // tool-call) crashes the agent loop and leaves whatever the LLM was
    // doing in an undefined state. These tests assert: any input string
    // can be passed to the parser, any (tool, args) JSON can be passed to
    // evaluate, and the public helpers (wildcard_match, bash_segments,
    // bash_has_redirect_or_chain, host_matches) all return rather than
    // panic.
    //
    // Parser is allowed to return Err on bad input; that's expected. The
    // contract is just "no panic, no abort, no infinite loop".
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    /// Generate a "weird-but-valid-UTF-8" string of length up to `max_len`.
    /// Mixes ASCII printable, ASCII control, multi-byte BMP, supplementary
    /// (emoji range), and special whitespace (NBSP, ZWSP). The kinds that
    /// have historically broken naive byte-arithmetic in Rust string code.
    fn fuzz_string(rng: &mut StdRng, max_len: usize) -> String {
        let len = rng.gen_range(0..=max_len);
        let mut out = String::with_capacity(len);
        for _ in 0..len {
            let r: f32 = rng.gen();
            let c = if r < 0.5 {
                char::from_u32(rng.gen_range(0x20u32..0x7Fu32)).unwrap()
            } else if r < 0.7 {
                char::from_u32(rng.gen_range(0x4E00u32..0x9FFFu32)).unwrap()
            } else if r < 0.85 {
                char::from_u32(rng.gen_range(0x1F600u32..0x1F64Fu32)).unwrap()
            } else if r < 0.95 {
                // weird whitespace + zero-widths
                let zws = [
                    '\u{a0}', '\u{2028}', '\u{2029}', '\u{200B}', '\u{FEFF}', '\u{3000}',
                ];
                zws[rng.gen_range(0..zws.len())]
            } else {
                // metacharacters + escapes the parser must handle
                let meta = [
                    '(', ')', '*', '?', ':', '\\', '"', '\'', '|', '>', '<', '&', ';',
                ];
                meta[rng.gen_range(0..meta.len())]
            };
            out.push(c);
        }
        out
    }

    #[test]
    fn fuzz_parse_rule_never_panics() {
        // 5 K random strings × 3 actions each. Parser may return Err for
        // most of them — that's fine; assertion is "no panic".
        let mut rng = StdRng::seed_from_u64(0xDE57_DC57_u64);
        for _ in 0..5_000 {
            let s = fuzz_string(&mut rng, 60);
            for action in [Action::Allow, Action::Deny, Action::Ask] {
                let _ = parse_rule(&s, action);
            }
        }
    }

    #[test]
    fn fuzz_wildcard_match_never_panics() {
        // 10K random (pattern, text) pairs. Pattern-matching with `*`
        // wildcards can recurse deeply on pathological inputs ("**" + 10K
        // chars). We bound recursion via input length; verify no stack
        // overflow within reasonable bounds.
        let mut rng = StdRng::seed_from_u64(0xA77E47_DC57_u64);
        for _ in 0..10_000 {
            let pattern = fuzz_string(&mut rng, 30);
            let text = fuzz_string(&mut rng, 60);
            let _ = wildcard_match(&pattern, &text);
        }
    }

    #[test]
    fn fuzz_bash_segments_never_panics() {
        // bash_segments walks UTF-8 char-by-char to honor quotes; any byte
        // sequence that's valid UTF-8 should be safe. NBSP, embedded
        // emoji, zero-width joiners are all worth throwing at it.
        let mut rng = StdRng::seed_from_u64(0xBA5407E_2u64);
        for _ in 0..5_000 {
            let s = fuzz_string(&mut rng, 200);
            let _ = bash_segments(&s);
            let _ = bash_has_redirect_or_chain(&s);
        }
    }

    #[test]
    fn fuzz_engine_evaluate_never_panics_on_random_args() {
        // Build a small engine, then throw arbitrary tool names + arg
        // shapes at evaluate(). Args include nested objects, numbers,
        // bools, nulls — everything serde_json::Value supports. The
        // matchers must not assume any field is present or any string.
        let e = engine(
            &["Read(./.env)", "Bash(rm *)", "WebFetch(domain:badsite.com)"],
            &["Bash"],
            &["Bash(git status)", "Read(./README.md)"],
        );
        let mut rng = StdRng::seed_from_u64(0xE_4A_70A7Eu64);
        let tool_names = [
            "shell",
            "file_read",
            "file_write",
            "web_fetch",
            "git_status",
            "non_existent_tool",
            "",
            "weird name with spaces",
            "中文",
        ];
        for _ in 0..2_000 {
            let tool = tool_names[rng.gen_range(0..tool_names.len())];
            // Build a random JSON value
            let args = match rng.gen_range(0..6) {
                0 => serde_json::json!({}),
                1 => serde_json::json!({"path": fuzz_string(&mut rng, 80)}),
                2 => serde_json::json!({"command": fuzz_string(&mut rng, 100)}),
                3 => serde_json::json!({"url": fuzz_string(&mut rng, 80)}),
                4 => serde_json::json!({
                    "path": fuzz_string(&mut rng, 60),
                    "extra": rng.gen::<u32>(),
                    "nested": {"k": fuzz_string(&mut rng, 30)},
                }),
                _ => serde_json::Value::Null, // exercise the null-args path
            };
            let _ = e.evaluate(tool, &args);
        }
    }

    #[test]
    fn fuzz_engine_constructor_never_panics() {
        // Try to build engines from arrays of weird strings — should
        // either succeed (all parsed) or return Err — never panic.
        let mut rng = StdRng::seed_from_u64(0xC0_D5_7_DC57_u64);
        for _ in 0..200 {
            let n = rng.gen_range(0..6);
            let strs: Vec<String> = (0..n).map(|_| fuzz_string(&mut rng, 40)).collect();
            let refs: Vec<&str> = strs.iter().map(String::as_str).collect();
            let _ = Engine::from_lists(&refs, &[], &[]);
        }
    }

    #[test]
    fn fuzz_host_matches_handles_garbage_urls() {
        // host_matches parses a URL by hand (no `url` crate) — make sure
        // it doesn't trip on weird characters or empty hosts.
        let mut rng = StdRng::seed_from_u64(0xF0_57_2_E4257_u64);
        let valid_hosts = ["example.com", "github.com", "localhost"];
        for _ in 0..2_000 {
            let host = valid_hosts[rng.gen_range(0..valid_hosts.len())];
            let url = fuzz_string(&mut rng, 100);
            let _ = host_matches(host, &url);
        }
    }
}
