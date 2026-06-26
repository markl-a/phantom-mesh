//! The process-wide tool gate — the single permission/trust enforcement
//! chokepoint installed into [`crate::tools::execute`]. Lives in the LIB (not a
//! binary) so EVERY entrypoint installs the same gate: both the `phantom` and
//! `phantom-mesh` binaries, before any agent / HTTP / daemon surface runs.
//!
//! Policy is loaded HOME-ONLY ([`crate::config::AgentsConfig::load_home_only`])
//! so a malicious `cwd/agents.toml` can't weaken it. `interactive=false`
//! (daemons, `exec`, `serve`, cluster) is fail-closed — a profile/trust `Ask`
//! becomes `Deny`, since no one is at a terminal. `interactive=true` (the REPL)
//! prompts y/n/a/A. Escape hatch: `PHANTOM_TRUST_ALL=1` (or a home
//! `profile = "developer-full"`).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::permission::{Decision, Engine};
use crate::permission_profiles::Profile;
use crate::project_trust::{apply_trust, TrustPolicy, TrustStore};
use crate::util::term::colored;

/// One-shot plan-mode approval flag — the REPL sets it (via [`set_plan_approved`])
/// when the user types "go"; the gate denies all tools until then.
static GATE_PLAN_APPROVED: AtomicBool = AtomicBool::new(false);

/// Set/clear the one-shot plan-mode approval (the REPL toggles this per turn).
pub fn set_plan_approved(v: bool) {
    GATE_PLAN_APPROVED.store(v, Ordering::Relaxed);
}

/// Session "always allow" cache — populated by the interactive prompt ('a'/'A')
/// and the REPL `/perm` command. Process-global so it's shared by the gate + UI.
pub fn allowlist() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static A: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Build a permission [`Engine`] from a `[permissions]` block: explicit
/// deny/ask/allow rules win; else the named `profile`; else an empty engine
/// (legacy allow-all). Mirrors the precedence `diagnose()` reports.
pub fn engine_from_permissions(pc: &crate::config::PermissionsConfig) -> Engine {
    let deny: Vec<&str> = pc.deny.iter().map(String::as_str).collect();
    let ask: Vec<&str> = pc.ask.iter().map(String::as_str).collect();
    let allow: Vec<&str> = pc.allow.iter().map(String::as_str).collect();
    if deny.is_empty() && ask.is_empty() && allow.is_empty() {
        if let Some(p) = pc.profile.as_deref().and_then(Profile::from_slug) {
            return p.engine();
        }
        return Engine::new(Vec::new());
    }
    // Rules ARE configured but failed to parse → FAIL CLOSED (deny-all), not
    // allow-all: a malformed rule must not silently disable enforcement.
    Engine::from_lists(&deny, &ask, &allow).unwrap_or_else(|err| {
        eprintln!("  permission rule parse error — failing closed (deny-all): {err}");
        Engine::from_lists(&["*"], &[], &[]).expect("`deny *` is valid DSL")
    })
}

/// Interactive y/n/a/A approval for an `Ask` decision. **Fail-closed** when
/// stdin is not a real terminal (a piped/closed stdin must never auto-approve).
fn prompt_tool_approval(name: &str, args: &serde_json::Value) -> Result<(), String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "'{name}' needs approval but stdin is not a terminal (fail-closed). \
             Use a permissive profile, `phantom project trust add`, or PHANTOM_TRUST_ALL=1."
        ));
    }
    let summary = serde_json::to_string(args).unwrap_or_default();
    let summary = if summary.chars().count() > 200 {
        format!("{}…", summary.chars().take(200).collect::<String>())
    } else {
        summary
    };
    eprintln!();
    eprintln!(
        "  {} run {}({}) ?",
        colored("⚠", 33),
        colored(name, 36),
        colored(&summary, 90)
    );
    eprint!(
        "    {}: [y]es / [N]o / [a]lways this tool / [A]lways all tools : ",
        colored("permission", 33)
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut buf = String::new();
    // EOF (Ctrl-D / closed) reads 0 bytes — deny, do NOT treat as "" → yes.
    match std::io::stdin().read_line(&mut buf) {
        Ok(0) | Err(_) => return Err("denied (no input / EOF)".into()),
        _ => {}
    }
    // Default ([N]o): a bare Enter DENIES — matches the codebase's `[y/N]`
    // convention; only an explicit yes/a/A approves. (Reflexively hitting Enter
    // on a surprise prompt must not grant a destructive tool.)
    match buf.trim() {
        "y" | "yes" => Ok(()),
        "a" => {
            if let Ok(mut al) = allowlist().lock() {
                al.insert(name.to_string());
            }
            eprintln!("  {} {} added to always-allow list", colored("◆", 32), name);
            Ok(())
        }
        "A" => {
            if let Ok(mut al) = allowlist().lock() {
                al.insert("*".to_string());
            }
            eprintln!("  {} all tools always-allowed for this session", colored("◆", 32));
            Ok(())
        }
        _ => Err("user denied this tool call".into()),
    }
}

/// Install the process-wide gate. `interactive=false` for daemons/headless
/// (fail-closed); `interactive=true` for the REPL (prompts). Idempotent /
/// replaceable: the REPL calls `install(true)` to upgrade the `install(false)`
/// an early `main` already set.
pub fn install(interactive: bool) {
    let perm_deny = std::env::var("PHANTOM_PERM").as_deref() == Ok("deny");

    // Explicit opt-out for trusted automation (your own cluster workers / CI) —
    // but PHANTOM_PERM=deny (most-restrictive) still wins over it.
    if std::env::var("PHANTOM_TRUST_ALL").as_deref() == Ok("1") && !perm_deny {
        return;
    }

    // SECURITY POLICY FROM HOME ONLY — never the cwd-first find_and_load. A HOME
    // config that EXISTS but is malformed must FAIL CLOSED (a typo must not
    // silently disable the gate); a genuinely absent config is legacy allow-all.
    let home_cfg = match crate::config::AgentsConfig::load_home_only() {
        Some(c) => c,
        None if crate::config::AgentsConfig::home_config_present() => {
            eprintln!(
                "  {} HOME security config is malformed — FAIL-CLOSED: denying all \
                 tools until it parses. Fix it (`phantom doctor`), or set \
                 PHANTOM_TRUST_ALL=1 to bypass. (`phantom permissions`/`trust` still work.)",
                colored("✗", 31)
            );
            let gate = std::sync::Arc::new(|_: &str, _: &serde_json::Value| {
                Err("denied — HOME security config (~/.phantom-mesh/agents.toml) is \
                     malformed; fail-closed until fixed".to_string())
            });
            crate::tools::set_tool_gate(gate);
            return;
        }
        None => crate::config::AgentsConfig::default(),
    };
    let engine = engine_from_permissions(&home_cfg.permissions);
    let policy = home_cfg
        .trust
        .enforcement
        .as_deref()
        .and_then(TrustPolicy::from_slug)
        .unwrap_or_default();

    // ALWAYS install (the only fast-path opt-out is PHANTOM_TRUST_ALL, handled
    // above). Installing even for an empty/allow-all policy is what makes runtime
    // toggles (`/plan`, `/perm deny`, PHANTOM_PERM) take effect on EVERY surface
    // — incl. the default-config TUI, where a missing gate previously made those
    // controls silently inert (a fail-open). An empty-engine + trust-off gate is
    // a cheap pass-through, so the per-call cost is negligible.
    let home = crate::cli_config::resolve_home_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // Load the store once; compute the VERDICT per call from the live cwd (a bare
    // `phantom` can change cwd via the workspace pin AFTER install).
    let store = std::sync::Arc::new(TrustStore::load(&TrustStore::path(&home)));

    if interactive {
        if !engine.is_empty() {
            eprintln!(
                "  {} permission rules active ({})",
                colored("◆", 36),
                engine.rules().len()
            );
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        if policy != TrustPolicy::Off && !store.verdict(&cwd).is_trusted() {
            eprintln!(
                "  {} project trust: untrusted dir — enforcement={} ({}). \
                 `phantom project trust add` to trust it.",
                colored("⚠", 33),
                policy.slug(),
                policy.summary(),
            );
        }
    }

    let engine = std::sync::Arc::new(engine);
    let gate = std::sync::Arc::new(
        move |name: &str, args: &serde_json::Value| -> Result<(), String> {
            // ExecutionContract deny-until-approved gate (T7). OPT-IN via
            // PHANTOM_CONTRACT_GATE=1; a no-op pass-through when off, so this
            // line does NOT change behavior unless the operator engages it.
            crate::contract_gate::check(name, args)?;
            // PHANTOM_PERM=deny: hard global deny, before any Allow.
            if std::env::var("PHANTOM_PERM").as_deref() == Ok("deny") {
                return Err("PHANTOM_PERM=deny — tool execution denied".into());
            }
            // Plan mode: deny all tools until the user approves the turn.
            if std::env::var("PHANTOM_PLAN_MODE").as_deref() == Ok("1")
                && !GATE_PLAN_APPROVED.load(Ordering::Relaxed)
            {
                return Err("Plan mode is active — output the plan as text and stop; \
                            the user approves with 'go' before any tool runs."
                    .into());
            }
            // Per-call trust verdict from the LIVE cwd.
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let verdict = store.verdict(&cwd);
            let base = apply_trust(engine.evaluate(name, args), verdict, policy, name);
            // PHANTOM_PERM=ask|diff forces a prompt even on an engine Allow, on
            // ALL surfaces — interactive prompts; non-interactive then fail-closes
            // (Ask→Deny below). (The diff *preview* was dropped in consolidation,
            // but diff/ask must never silently allow.)
            let decision = if matches!(base, Decision::Allow)
                && matches!(std::env::var("PHANTOM_PERM").as_deref(), Ok("ask") | Ok("diff"))
            {
                Decision::Ask
            } else {
                base
            };
            match decision {
                Decision::Allow => Ok(()),
                Decision::Deny(reason) => Err(reason),
                Decision::Ask => {
                    if !interactive {
                        return Err(format!(
                            "'{name}' needs approval, but this is a non-interactive session \
                             (fail-closed). Use an interactive `phantom repl`, a more \
                             permissive profile, `phantom project trust add` this directory, or \
                             PHANTOM_TRUST_ALL=1 for trusted automation."
                        ));
                    }
                    if let Ok(al) = allowlist().lock() {
                        if al.contains("*") || al.contains(name) {
                            return Ok(());
                        }
                    }
                    prompt_tool_approval(name, args)
                }
            }
        },
    );
    crate::tools::set_tool_gate(gate);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionsConfig;
    use serde_json::json;

    fn pc(profile: Option<&str>, deny: &[&str], ask: &[&str], allow: &[&str]) -> PermissionsConfig {
        PermissionsConfig {
            profile: profile.map(String::from),
            deny: deny.iter().map(|s| s.to_string()).collect(),
            ask: ask.iter().map(|s| s.to_string()).collect(),
            allow: allow.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_permissions_is_allow_all() {
        let e = engine_from_permissions(&pc(None, &[], &[], &[]));
        assert!(e.is_empty(), "no rules/profile → empty engine (allow-all)");
        assert!(matches!(e.evaluate("file_write", &json!({})), Decision::Allow));
    }

    #[test]
    fn profile_expands_to_engine() {
        let e = engine_from_permissions(&pc(Some("observe"), &[], &[], &[]));
        assert!(matches!(e.evaluate("file_read", &json!({"path": "x"})), Decision::Allow));
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Deny(_)));
    }

    #[test]
    fn malformed_dsl_fails_closed_deny_all() {
        // An unparseable explicit rule must FAIL CLOSED (deny-all), not silently
        // collapse to allow-all.
        let e = engine_from_permissions(&pc(None, &[], &[], &["this(is(broken"]));
        assert!(matches!(e.evaluate("file_read", &json!({"path": "x"})), Decision::Deny(_)));
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Deny(_)));
    }

    #[test]
    fn explicit_rules_take_precedence_over_profile() {
        // explicit allow wins; the observe profile is ignored when rules exist.
        let e = engine_from_permissions(&pc(Some("observe"), &[], &[], &["file_write"]));
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Allow));
    }
}
