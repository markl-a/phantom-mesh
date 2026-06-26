//! System-state diagnostics — the state-machine `phantom doctor` renders (and a
//! future shared `status` overview).
//!
//! Onboarding/CLI design (owner-decided 2026-06-15, option 乙): the CLI is split
//! into four independent, inspectable concerns — **Local Identity**, **Provider
//! Credential**, **Project Trust**, **Execution Permission** — plus the **Mesh**
//! and **Config** plumbing. Every failure state is *named* (a stable `code`) and
//! carries the exact next command to run (`fix`), so a user (or a recruiter
//! watching a demo) never sees a bare error. This module is the single source
//! of truth for the system-state view: `phantom doctor` renders it today (human
//! + `--json`). `phantom status` is a SEPARATE command (the dev-session
//! heartbeat) and does NOT render this yet — a shared `status` overview onto
//! `diagnose()` is a follow-up, not a current claim.
//!
//! This module is deliberately **hermetic**: `diagnose()` takes the home + cwd
//! paths AND an env-lookup closure explicitly, so it can be unit-tested against
//! a temp HOME with a synthetic environment — without touching the real
//! `~/.phantom-mesh` or the real process env. At the call site the closure is
//! `|k| std::env::var(k).ok()`, the exact seam `agent.rs` sources a provider key
//! from (`provider.api_key_env` → env), so `doctor` checks whether a provider's
//! key is *actually present in the environment* — not merely that the config
//! names one. (That live-key check is THE most common onboarding failure;
//! reporting "✓ provider configured" while the env var is empty would be a
//! false-green.) Config-file resolution reuses
//! [`crate::config::AgentsConfig::candidate_config_paths`] — the same list the
//! runtime loader walks — so doctor never points at a different file than the
//! runtime loads.
//!
//! Phase 1a (this module) covers the layers we can detect cheaply + honestly
//! today: Identity, Provider, Permission, Mesh, Config. **Project Trust and the
//! permission *profiles* (observe/suggest/workspace-write/full) are Phase 2** —
//! they are not yet enforced, so they are NOT reported here as if they were
//! (no-overclaim).
//!
//! 中文: 系統狀態診斷 — `phantom doctor` render 的 state machine(status 之後再接)。把 CLI 切成
//! 四個可查的層(身份/供應商/專案信任/執行權限)+ mesh/config;每個失敗狀態有命名 code
//! 與「下一步指令(fix)」。hermetic:吃顯式路徑、直接讀檔,可用 temp HOME 單測。

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The four product layers (+ plumbing) a finding belongs to. Mirrors the
/// onboarding mental model so `status`/`doctor` can group output cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // Phase 2 will add real Project-Trust / permission-profile variants
pub enum Layer {
    Identity,
    Provider,
    /// Project Trust — Phase 2 (gate not yet enforced; reserved).
    Project,
    Permission,
    Mesh,
    Config,
}

impl Layer {
    pub fn label(self) -> &'static str {
        match self {
            Layer::Identity => "Local identity",
            Layer::Provider => "Provider",
            Layer::Project => "Project",
            Layer::Permission => "Permissions",
            Layer::Mesh => "Mesh",
            Layer::Config => "Config",
        }
    }
}

/// Severity of a finding. Ordered: `Ok < Warn < Fail` (see `rank`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

impl Severity {
    pub fn glyph(self) -> &'static str {
        match self {
            Severity::Ok => "✓",
            Severity::Warn => "⚠",
            Severity::Fail => "✗",
        }
    }
}

/// One diagnosed fact about the system. `code` is a stable machine-readable
/// state slug (empty for plain `Ok` rows). `fix`/`inspect` are the exact
/// commands the user should run next, so no failure is a dead end.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub layer: Layer,
    pub severity: Severity,
    pub label: String,
    pub detail: String,
    pub fix: Option<String>,
    pub inspect: Option<String>,
}

impl Finding {
    fn ok(layer: Layer, label: &str, detail: impl Into<String>) -> Self {
        Finding { code: "", layer, severity: Severity::Ok, label: label.into(), detail: detail.into(), fix: None, inspect: None }
    }
    fn warn(code: &'static str, layer: Layer, label: &str, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Finding { code, layer, severity: Severity::Warn, label: label.into(), detail: detail.into(), fix: fix.map(String::from), inspect: None }
    }
    fn fail(code: &'static str, layer: Layer, label: &str, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Finding { code, layer, severity: Severity::Fail, label: label.into(), detail: detail.into(), fix: fix.map(String::from), inspect: None }
    }
}

/// The full diagnosis: an ordered list of findings (grouped by layer) plus the
/// worst severity seen. Callers map `worst` to an exit code or a headline.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub findings: Vec<Finding>,
    pub worst: Severity,
}

impl Diagnosis {
    /// Suggested process exit code: 0 = all ok, 1 = warnings only, 2 = any fail.
    /// Mirrors the existing `doctor --mesh` convention.
    pub fn exit_code(&self) -> i32 {
        match self.worst {
            Severity::Ok => 0,
            Severity::Warn => 1,
            Severity::Fail => 2,
        }
    }
    /// Findings in a given layer (for grouped rendering).
    pub fn in_layer(&self, layer: Layer) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.layer == layer)
    }
}

/// `<home>/.phantom-mesh`.
fn phantom_dir(home: &Path) -> PathBuf {
    home.join(".phantom-mesh")
}

/// Resolve the active config file from the SAME candidate list the runtime
/// loader walks (`AgentsConfig::candidate_config_paths`), so `doctor` can never
/// report a different file than the runtime actually loads (no false `NO_CONFIG`).
fn resolve_config(home: &Path, cwd: &Path) -> Option<PathBuf> {
    crate::config::AgentsConfig::candidate_config_paths(home, cwd)
        .into_iter()
        .find(|p| p.is_file())
}

/// Diagnose the system state from explicit home + cwd paths and an env-lookup
/// closure. File reads + env lookups only — no network, no daemon. The closure
/// is the runtime's key seam (`|k| std::env::var(k).ok()` at the call site;
/// tests inject a synthetic map). (`doctor --mesh` does the live peer ping
/// separately; we only report configured-peer count here so the check stays
/// instant and offline-safe.)
pub fn diagnose(home: &Path, cwd: &Path, env_get: &dyn Fn(&str) -> Option<String>) -> Diagnosis {
    let mut findings: Vec<Finding> = Vec::new();

    // ── Local Identity ──────────────────────────────────────────────────────
    let id_path = phantom_dir(home).join("identity.key");
    if id_path.is_file() {
        findings.push(Finding::ok(Layer::Identity, "device key", "ed25519 identity.key present"));
    } else {
        findings.push(Finding::fail(
            "NO_LOCAL_IDENTITY",
            Layer::Identity,
            "device key",
            "no ~/.phantom-mesh/identity.key on this device",
            Some("phantom setup"),
        ));
    }

    // ── Config + Provider + Permission (all parsed from agents.toml) ─────────
    // Captured from the parsed config (if any) for the Project-trust section,
    // which is emitted after this match (trust applies even with no config).
    let mut trust_enforcement: Option<String> = None;
    match resolve_config(home, cwd) {
        None => {
            findings.push(Finding::fail(
                "NO_CONFIG",
                Layer::Config,
                "agents.toml",
                "not found in cwd or ~/.phantom-mesh",
                Some("phantom onboarding"),
            ));
            // Without a config there is no provider either — make it explicit.
            findings.push(Finding::fail(
                "NO_PROVIDER",
                Layer::Provider,
                "model provider",
                "no provider configured (no agents.toml yet)",
                Some("phantom onboarding"),
            ));
        }
        Some(path) => {
            findings.push(Finding::ok(Layer::Config, "agents.toml", path.display().to_string()));
            match std::fs::read_to_string(&path).ok().and_then(|s| toml::from_str::<toml::Value>(&s).ok()) {
                None => {
                    findings.push(Finding::fail(
                        "CONFIG_PARSE_ERROR",
                        Layer::Config,
                        "agents.toml",
                        "exists but is not valid TOML — fix the syntax, or regenerate it",
                        Some("phantom onboarding"),
                    ));
                    findings.push(Finding::fail(
                        "NO_PROVIDER",
                        Layer::Provider,
                        "model provider",
                        "cannot read providers (config parse error)",
                        Some("phantom provider add"),
                    ));
                }
                Some(val) => {
                    // Provider layer.
                    let n_providers = val
                        .get("providers")
                        .and_then(|v| v.as_table())
                        .map(|t| t.len())
                        .unwrap_or(0);
                    if n_providers == 0 {
                        findings.push(Finding::fail(
                            "NO_PROVIDER",
                            Layer::Provider,
                            "model provider",
                            "agents.toml has no [providers.*] — LLM tasks cannot run",
                            Some("phantom provider add"),
                        ));
                    } else {
                        let tbl = val.get("providers").and_then(|v| v.as_table());
                        let names: Vec<String> =
                            tbl.map(|t| t.keys().cloned().collect()).unwrap_or_default();
                        // No-key types source their credential live (subscription
                        // CLI tokens) or need none (a local server) — usable on
                        // sight. Everything else needs a real key REACHABLE NOW:
                        // an inline api_key, a token_source=auto subscription, OR
                        // an api_key_env whose env var is actually SET (checked via
                        // the injected env closure — the same seam agent.rs uses).
                        // Naming GROQ_API_KEY in config while the var is empty is
                        // the #1 onboarding failure, so it must NOT read as green.
                        const NO_KEY_TYPES: &[&str] = &[
                            "claude_cli", "claude_agent", "codex_oauth", "gemini_oauth",
                            "ollama", "local-ollama", "lmstudio", "lemonade",
                        ];
                        // Classify each provider: usable / env-named-but-unset /
                        // no-mechanism. The worst case across all of them decides
                        // the finding (any usable → ok).
                        let mut any_usable = false;
                        let mut env_unset: Vec<String> = Vec::new(); // "name ($VAR)"
                        if let Some(t) = tbl {
                            for (name, v) in t.iter() {
                                let pt = v
                                    .get("type")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or(name.as_str());
                                let has_inline = v
                                    .get("api_key")
                                    .and_then(|x| x.as_str())
                                    .is_some_and(|s| !s.is_empty());
                                let auto = v.get("token_source").and_then(|x| x.as_str())
                                    == Some("auto");
                                let env_name = v
                                    .get("api_key_env")
                                    .and_then(|x| x.as_str())
                                    .filter(|s| !s.is_empty());
                                let env_set = env_name
                                    .map(|k| env_get(k).is_some_and(|val| !val.is_empty()))
                                    .unwrap_or(false);
                                if has_inline || auto || env_set || NO_KEY_TYPES.contains(&pt) {
                                    any_usable = true;
                                } else if let Some(k) = env_name {
                                    // Mechanism present but the env var is empty/unset.
                                    env_unset.push(format!("{} (${})", name, k));
                                }
                            }
                        }
                        if any_usable {
                            findings.push(Finding::ok(
                                Layer::Provider,
                                "model provider",
                                format!("{} configured ({})", n_providers, names.join(", ")),
                            ));
                        } else if !env_unset.is_empty() {
                            findings.push(Finding::warn(
                                "PROVIDER_KEY_MISSING",
                                Layer::Provider,
                                "model provider",
                                format!(
                                    "{} configured but no key is set in the environment — \
                                     {} expect(s) an env var that is empty/unset; \
                                     LLM tasks will fail until you export it",
                                    n_providers,
                                    env_unset.join(", ")
                                ),
                                Some("phantom provider add"),
                            ));
                        } else {
                            findings.push(Finding::warn(
                                "PROVIDER_NO_KEY",
                                Layer::Provider,
                                "model provider",
                                format!(
                                    "{} configured ({}) but none has a key mechanism \
                                     (api_key / api_key_env) or is a local/subscription type \
                                     — LLM tasks will fail",
                                    n_providers,
                                    names.join(", ")
                                ),
                                Some("phantom provider add"),
                            ));
                        }
                    }

                    // Permission layer. Precedence mirrors the engine builder:
                    // explicit deny/ask/allow rules win; else a named `profile`;
                    // else legacy allow-all.
                    let perms = val.get("permissions").and_then(|v| v.as_table());
                    let count = |key: &str| {
                        perms
                            .and_then(|t| t.get(key))
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0)
                    };
                    let (deny, ask, allow) = (count("deny"), count("ask"), count("allow"));
                    let profile = perms
                        .and_then(|t| t.get("profile"))
                        .and_then(|v| v.as_str());
                    if deny + ask + allow > 0 {
                        findings.push(Finding::ok(
                            Layer::Permission,
                            "[permissions]",
                            format!("{} deny / {} ask / {} allow", deny, ask, allow),
                        ));
                    } else if let Some(prof) = profile {
                        match crate::permission_profiles::Profile::from_slug(prof) {
                            Some(p) => findings.push(Finding::ok(
                                Layer::Permission,
                                "[permissions]",
                                format!("profile: {} — {}", p.slug(), p.summary()),
                            )),
                            None => findings.push(Finding::warn(
                                "UNKNOWN_PROFILE",
                                Layer::Permission,
                                "[permissions]",
                                format!(
                                    "profile {:?} is not recognised → running allow-all. \
                                     Valid: observe / suggest / workspace-write / developer-full",
                                    prof
                                ),
                                Some("phantom permissions set workspace-write"),
                            )),
                        }
                    } else {
                        findings.push(Finding::warn(
                            "PERMISSIONS_ALLOW_ALL",
                            Layer::Permission,
                            "[permissions]",
                            "no rules or profile → allow-all (legacy default). See docs/PERMISSIONS.md",
                            Some("phantom permissions set observe"),
                        ));
                    }

                    // Mesh layer — configured peers only (no live ping here).
                    let n_peers = val
                        .get("cluster")
                        .and_then(|v| v.get("peers"))
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    if n_peers == 0 {
                        findings.push(Finding::ok(Layer::Mesh, "peers", "single-node (no peers configured)"));
                    } else {
                        findings.push(Finding::warn(
                            "MESH_UNVERIFIED",
                            Layer::Mesh,
                            "peers",
                            format!("{} peer(s) configured — liveness unchecked", n_peers),
                            Some("phantom doctor --mesh"),
                        ));
                    }

                    trust_enforcement = val
                        .get("trust")
                        .and_then(|t| t.get("enforcement"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                }
            }
        }
    }

    // ── Project Trust ────────────────────────────────────────────────────────
    // Independent of config validity: the trusted set is its own file and the
    // policy defaults to Off. With enforcement Off an untrusted cwd is purely
    // informational (nothing is restricted); under prompt/observe it's a warning
    // with the fix command.
    {
        use crate::project_trust::{TrustPolicy, TrustStore};
        let policy = trust_enforcement
            .as_deref()
            .and_then(TrustPolicy::from_slug)
            .unwrap_or_default();
        let trusted = TrustStore::load(&TrustStore::path(home)).verdict(cwd).is_trusted();
        if trusted {
            findings.push(Finding::ok(
                Layer::Project,
                "project trust",
                format!("this directory is trusted (enforcement: {})", policy.slug()),
            ));
        } else if policy == TrustPolicy::Off {
            findings.push(Finding::ok(
                Layer::Project,
                "project trust",
                "this directory is untrusted — enforcement off (informational)",
            ));
        } else {
            findings.push(Finding::warn(
                "PROJECT_UNTRUSTED",
                Layer::Project,
                "project trust",
                format!(
                    "this directory is untrusted and enforcement is '{}' ({}) — \
                     some tools are restricted here",
                    policy.slug(),
                    policy.summary()
                ),
                Some("phantom project trust add"),
            ));
        }
    }

    let worst = findings.iter().map(|f| f.severity).max().unwrap_or(Severity::Ok);
    Diagnosis { findings, worst }
}

/// Render a diagnosis as a human-readable, grouped report (the body
/// `phantom doctor` prints). Findings are grouped by layer in a stable order;
/// every Warn/Fail prints its `fix:` (and `inspect:`) command so no state is a
/// dead end. Glyphs are colour-coded via the shared term helper.
pub fn render_human(d: &Diagnosis) -> String {
    use crate::util::term::colored;
    use std::fmt::Write as _;
    let mut out = String::new();
    let order = [
        Layer::Identity,
        Layer::Provider,
        Layer::Project,
        Layer::Permission,
        Layer::Mesh,
        Layer::Config,
    ];
    for layer in order {
        let mut rows = d.in_layer(layer).peekable();
        if rows.peek().is_none() {
            continue;
        }
        let _ = writeln!(out, "\n{}", colored(layer.label(), 35));
        for f in rows {
            let color = match f.severity {
                Severity::Ok => 32,
                Severity::Warn => 33,
                Severity::Fail => 31,
            };
            let code = if f.code.is_empty() {
                String::new()
            } else {
                format!(" [{}]", f.code)
            };
            let _ = writeln!(
                out,
                "  {} {}{}: {}",
                colored(f.severity.glyph(), color),
                f.label,
                code,
                f.detail
            );
            if let Some(fix) = &f.fix {
                let _ = writeln!(out, "      → fix: {}", colored(fix, 36));
            }
            if let Some(inspect) = &f.inspect {
                let _ = writeln!(out, "      → inspect: {}", colored(inspect, 36));
            }
        }
    }
    let summary = match d.worst {
        Severity::Ok => colored("✓ all systems go", 32),
        Severity::Warn => colored("⚠ usable, with warnings above (see fix: lines)", 33),
        Severity::Fail => colored("✗ action needed — run the fix: command(s) above", 31),
    };
    let _ = writeln!(out, "\n{}", summary);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(d: &Diagnosis, code: &str) -> bool {
        d.findings.iter().any(|f| f.code == code)
    }
    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }
    /// No env vars set (the common "key not exported" case).
    fn no_env(_: &str) -> Option<String> {
        None
    }
    /// `GROQ_API_KEY` present in the environment.
    fn groq_set(k: &str) -> Option<String> {
        (k == "GROQ_API_KEY").then(|| "sk-test".to_string())
    }

    #[test]
    fn empty_home_flags_no_identity_and_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(has(&d, "NO_LOCAL_IDENTITY"));
        assert!(has(&d, "NO_CONFIG"));
        assert!(has(&d, "NO_PROVIDER"));
        assert_eq!(d.worst, Severity::Fail);
        assert_eq!(d.exit_code(), 2);
        // every failure carries a fix command — no dead ends
        for f in d.findings.iter().filter(|f| f.severity == Severity::Fail) {
            assert!(f.fix.is_some(), "fail {} has no fix", f.code);
        }
    }

    #[test]
    fn identity_and_provider_present_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        // Mirror what `write_onboarding_config` emits for a free provider: a
        // `type` + an `api_key_env`. It is usable ONLY when that env var is
        // actually set — modelled here by the `groq_set` closure.
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype = \"groq\"\napi_key_env = \"GROQ_API_KEY\"\n\
             default_model = \"llama-3.3-70b-versatile\"\n\n\
             [permissions]\nallow = [\"file_read\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        assert!(!has(&d, "NO_LOCAL_IDENTITY"));
        assert!(!has(&d, "NO_PROVIDER"));
        assert!(!has(&d, "PROVIDER_NO_KEY")); // carries api_key_env
        assert!(!has(&d, "PROVIDER_KEY_MISSING")); // and the env var is set
        assert!(!has(&d, "PERMISSIONS_ALLOW_ALL")); // a rule is present
        assert_eq!(d.worst, Severity::Ok);
        assert_eq!(d.exit_code(), 0);
    }

    #[test]
    fn provider_env_var_unset_warns_key_missing_not_green() {
        // THE #1 onboarding failure: config names GROQ_API_KEY but it is not
        // exported. Must warn (not green) AND must not be a hard Fail.
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype = \"groq\"\napi_key_env = \"GROQ_API_KEY\"\n\n\
             [permissions]\nallow = [\"file_read\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &no_env); // GROQ_API_KEY NOT set
        assert!(has(&d, "PROVIDER_KEY_MISSING"), "unset env key must warn, not green");
        assert!(!has(&d, "PROVIDER_NO_KEY")); // a mechanism IS named, just unset
        assert!(!has(&d, "NO_PROVIDER")); // it IS configured
        assert_eq!(d.worst, Severity::Warn);
        // the warning names the offending var so the user knows what to export
        let f = d.findings.iter().find(|f| f.code == "PROVIDER_KEY_MISSING").unwrap();
        assert!(f.detail.contains("GROQ_API_KEY"), "detail must name the var: {}", f.detail);
    }

    #[test]
    fn config_without_providers_flags_no_provider() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(&tmp.path().join(".phantom-mesh/agents.toml"), "[core]\nport = 7878\n");
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(has(&d, "NO_PROVIDER"));
        assert!(has(&d, "PERMISSIONS_ALLOW_ALL")); // no rules → legacy warn
        assert_eq!(d.worst, Severity::Fail);
    }

    #[test]
    fn cwd_config_takes_precedence_over_home() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        write(&home.path().join(".phantom-mesh/agents.toml"), "[core]\nport=1\n");
        write(&cwd.path().join("agents.toml"), "[providers.groq]\ntype=\"groq\"\n");
        let d = diagnose(home.path(), cwd.path(), &no_env);
        // cwd has a provider → no NO_PROVIDER despite home config lacking one
        assert!(!has(&d, "NO_PROVIDER"));
        let cfg = d.findings.iter().find(|f| f.layer == Layer::Config && f.severity == Severity::Ok).unwrap();
        assert!(cfg.detail.contains(&cwd.path().display().to_string()));
    }

    #[test]
    fn malformed_config_is_flagged_not_panicked() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/agents.toml"), "this is { not valid toml");
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(has(&d, "CONFIG_PARSE_ERROR"));
        assert_eq!(d.worst, Severity::Fail);
    }

    #[test]
    fn render_human_shows_fix_for_failures_and_is_json_serializable() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diagnose(tmp.path(), tmp.path(), &no_env); // empty → failures with fixes
        let human = render_human(&d);
        assert!(human.contains("fix:"), "human render must surface fix commands:\n{human}");
        assert!(human.contains("Local identity"), "grouped by layer:\n{human}");
        // structured output round-trips for `--json`
        let json = serde_json::to_string(&d).expect("Diagnosis serializes");
        assert!(json.contains("NO_LOCAL_IDENTITY"));
        assert!(json.contains("\"worst\""));
    }

    #[test]
    fn keyless_provider_warns_not_false_green() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        // a provider block with a type but NO key mechanism and not a no-key type
        write(&tmp.path().join(".phantom-mesh/agents.toml"), "[providers.foo]\ntype = \"foo\"\n");
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(has(&d, "PROVIDER_NO_KEY"), "keyless provider must warn, not green");
        assert!(!has(&d, "NO_PROVIDER")); // it IS configured, just keyless
        assert_eq!(d.worst, Severity::Warn);
        assert_eq!(d.exit_code(), 1); // warn → exit 1
    }

    #[test]
    fn subscription_provider_is_usable_without_any_env_key() {
        // claude_cli is a no-key subscription type → usable even with NO env set.
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.claude_cli]\ntype = \"claude_cli\"\n\n\
             [permissions]\nallow = [\"file_read\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(!has(&d, "PROVIDER_NO_KEY"), "subscription type must not false-warn");
        assert!(!has(&d, "PROVIDER_KEY_MISSING"));
        assert_eq!(d.worst, Severity::Ok);
    }

    #[test]
    fn env_keyed_provider_is_usable_when_var_is_set() {
        // groq carries api_key_env and the var IS set → usable (the env path).
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype = \"groq\"\napi_key_env = \"GROQ_API_KEY\"\n\n\
             [permissions]\nallow = [\"file_read\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        assert!(!has(&d, "PROVIDER_KEY_MISSING"), "set env var must not warn");
        assert!(!has(&d, "PROVIDER_NO_KEY"));
        assert_eq!(d.worst, Severity::Ok);
    }

    #[test]
    fn phantom_toml_in_cwd_is_a_valid_config() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        write(&home.path().join(".phantom-mesh/identity.key"), "x");
        write(&cwd.path().join("PHANTOM.toml"), "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n");
        let d = diagnose(home.path(), cwd.path(), &no_env);
        assert!(!has(&d, "NO_CONFIG"), "PHANTOM.toml must count as config (shared candidate list)");
        assert!(!has(&d, "NO_PROVIDER"));
    }

    #[test]
    fn severity_orders_ok_warn_fail() {
        assert!(Severity::Ok < Severity::Warn);
        assert!(Severity::Warn < Severity::Fail);
    }

    #[test]
    fn configured_peers_warn_unverified() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\n[cluster]\npeers = [\"http://100.64.0.2:7878\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &no_env);
        assert!(has(&d, "MESH_UNVERIFIED"));
    }

    #[test]
    fn named_profile_is_reported_not_allow_all_warn() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n\n\
             [permissions]\nprofile = \"workspace-write\"\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        assert!(!has(&d, "PERMISSIONS_ALLOW_ALL"), "a profile is not allow-all");
        let perm = d
            .findings
            .iter()
            .find(|f| f.layer == Layer::Permission)
            .unwrap();
        assert_eq!(perm.severity, Severity::Ok);
        assert!(perm.detail.contains("workspace-write"), "detail: {}", perm.detail);
    }

    #[test]
    fn unknown_profile_warns() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n\n\
             [permissions]\nprofile = \"nonsense\"\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        assert!(has(&d, "UNKNOWN_PROFILE"));
        assert_eq!(d.worst, Severity::Warn);
    }

    #[test]
    fn untrusted_dir_with_enforcement_off_is_informational_ok() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        let proj = d.findings.iter().find(|f| f.layer == Layer::Project).unwrap();
        assert_eq!(proj.severity, Severity::Ok, "off → informational, not a warning");
        assert!(!has(&d, "PROJECT_UNTRUSTED"));
    }

    #[test]
    fn trusted_dir_is_reported_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n",
        );
        // trust the cwd
        let mut store = crate::project_trust::TrustStore::default();
        store.add(tmp.path());
        store.save(&crate::project_trust::TrustStore::path(tmp.path())).unwrap();
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        let proj = d.findings.iter().find(|f| f.layer == Layer::Project).unwrap();
        assert_eq!(proj.severity, Severity::Ok);
        assert!(proj.detail.contains("trusted"), "detail: {}", proj.detail);
    }

    #[test]
    fn untrusted_dir_with_enforcement_on_warns() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n\n\
             [trust]\nenforcement = \"prompt\"\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        assert!(has(&d, "PROJECT_UNTRUSTED"));
        assert_eq!(d.worst, Severity::Warn);
    }

    #[test]
    fn explicit_rules_take_precedence_over_profile_in_report() {
        // Mirrors the engine builder: explicit rules win, so doctor reports the
        // rule counts (not the profile) when both are present.
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".phantom-mesh/identity.key"), "x");
        write(
            &tmp.path().join(".phantom-mesh/agents.toml"),
            "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n\n\
             [permissions]\nprofile = \"observe\"\nallow = [\"file_read\"]\n",
        );
        let d = diagnose(tmp.path(), tmp.path(), &groq_set);
        let perm = d
            .findings
            .iter()
            .find(|f| f.layer == Layer::Permission)
            .unwrap();
        assert!(perm.detail.contains("allow"), "should report rule counts: {}", perm.detail);
        assert!(!perm.detail.contains("observe"), "explicit rules win: {}", perm.detail);
    }
}
