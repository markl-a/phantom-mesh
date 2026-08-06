//! Task 7 wiring: run an L0 AI-CLI session under L1 governance for `spectyn
//! govern`. Resolves the spectyn home, opens the task EventStore, builds the real
//! recorder + escalator, starts the L0 session, and drives it under the governor
//! on a blocking thread. The recorder/escalator bridge sync -> async via
//! `Handle::block_on`, which needs a NON-async-worker thread on a multi-thread
//! runtime — `#[tokio::main]` + `spawn_blocking` provide exactly that.

use crate::cli_config;
use crate::cli_session::{self, CliKind, SessionSpec, TurnInput};
use crate::governed_run::escalation::PhoneEscalator;
use crate::governed_run::recorder::EventStoreRecorder;
use crate::governed_run::{GovernPolicy, GovernedFold, RunOutcome, drive_fold};
use crate::notifications::NotificationDispatcher;
use crate::tasks::{EventStore, TaskStore};
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Handle;
use uuid::Uuid;

/// Parse a `--max-wallclock` duration into whole SECONDS for the wall-clock brake.
/// Accepts bare seconds (`"90"`), or a `s` / `m` / `h` suffix (`"30s"`, `"5m"`,
/// `"1h"`). Returns `None` on empty/non-numeric/unknown-unit input or on overflow,
/// so the caller bails with a help line WITHOUT mutating policy. Kept local + tiny
/// (no humantime dep); whole-seconds is exactly what `GovernPolicy::max_wall_secs`
/// is checked against (`now().as_secs() > max`).
fn parse_wallclock_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Split the leading digit run from an optional unit suffix.
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num.parse().ok()?;
    let mult: u64 = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => return None,
    };
    n.checked_mul(mult)
}

/// Inputs for a governed run.
pub struct GovernConfig {
    pub cli: CliKind,
    pub prompt: String,
    pub cwd: PathBuf,
    pub timeout_secs: u64,
    pub model: Option<String>,
    pub workspace_id: String,
    /// Inbox poll interval while awaiting a blocking decision.
    pub poll: Duration,
    /// How long to await a blocking decision before the policy fallback applies.
    pub deadline: Duration,
    pub policy: GovernPolicy,
    /// Override the spectyn home (None = `resolve_home_dir()`). For tests.
    pub home: Option<PathBuf>,
    /// apex-④ dispatch↔govern correlation. When `Some`, this DISPATCH row's
    /// `job_uuid` (from `serve.rs` `rpc_task_assign`) is used AS the govern
    /// `task_id` (instead of a fresh `Uuid::new_v4()`), so the flight-recorder,
    /// the escalator, and the dispatch task row all share one correlation key. An
    /// approval raised mid-run then stamps its `approval_id` onto the dispatch row
    /// live (see `PhoneEscalator`). `None` (default) = a fresh id is minted as
    /// before — ungoverned runs and standalone `spectyn govern` are byte-identical.
    pub dispatch_task_id: Option<Uuid>,
}

impl GovernConfig {
    /// Sensible defaults for a locally-triggered run.
    pub fn new(cli: CliKind, prompt: impl Into<String>) -> Self {
        Self {
            cli,
            prompt: prompt.into(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            timeout_secs: 300,
            model: None,
            workspace_id: "default".to_string(),
            poll: Duration::from_secs(2),
            deadline: Duration::from_secs(300),
            policy: GovernPolicy::default(),
            home: None,
            dispatch_task_id: None,
        }
    }

    /// Apply optional `spectyn govern` brake flags onto the policy (apex ④).
    /// ADDITIVE: an absent flag leaves its `GovernPolicy` field at the exact
    /// default, so a call with an empty iterator is byte-identical to today's
    /// behavior. Recognized flags:
    ///   `--max-wall-secs <u64>`     → `policy.max_wall_secs`
    ///   `--max-wallclock <dur>`     → `policy.max_wall_secs` (human duration: `30s`/`5m`/`1h`/bare-secs)
    ///   `--max-output-tokens <u64>` → `policy.max_output_tokens`
    ///   `--min-battery-pct <u8>`    → `policy.min_battery_pct`
    ///   `--soft-budget`             → `policy.auto_continue_low_risk = true`
    /// (On-stuck has no flag: it is a callback/no separate detector — see the
    /// `min_battery_pct` doc-comment in `governed_run/mod.rs`.)
    /// Returns `Err` with a usage string on a missing/malformed value or an
    /// unknown flag, so the handler can print it and exit without mutating state.
    pub fn apply_flags<I, S>(&mut self, flags: I) -> Result<(), String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut it = flags.into_iter();
        while let Some(tok) = it.next() {
            match tok.as_ref() {
                "--max-wall-secs" => {
                    let v = it.next().ok_or("--max-wall-secs expects a value")?;
                    self.policy.max_wall_secs = Some(
                        v.as_ref()
                            .parse::<u64>()
                            .map_err(|_| "--max-wall-secs expects a u64")?,
                    );
                }
                // apex-④ HARD-BRAKE alias: a human duration (`1s`, `2m`, `1h`, or
                // bare seconds `90`) for the wall-clock deadline. Feeds the SAME
                // `policy.max_wall_secs` the drive loop enforces + records as
                // `budget-brake:max_wall_secs` — so an unattended run aborts safely
                // at a wall-clock deadline ("your hard brakes, not the vendor's
                // invoice").
                "--max-wallclock" => {
                    let v = it.next().ok_or("--max-wallclock expects a duration (e.g. 30s, 5m, 1h)")?;
                    self.policy.max_wall_secs = Some(parse_wallclock_secs(v.as_ref()).ok_or(
                        "--max-wallclock expects a duration like 30s, 5m, 1h, or bare seconds (90)",
                    )?);
                }
                "--max-output-tokens" => {
                    let v = it.next().ok_or("--max-output-tokens expects a value")?;
                    self.policy.max_output_tokens = Some(
                        v.as_ref()
                            .parse::<u64>()
                            .map_err(|_| "--max-output-tokens expects a u64")?,
                    );
                }
                "--min-battery-pct" => {
                    let v = it.next().ok_or("--min-battery-pct expects a value")?;
                    self.policy.min_battery_pct = Some(
                        v.as_ref()
                            .parse::<u8>()
                            .map_err(|_| "--min-battery-pct expects a u8 (0-100)")?,
                    );
                }
                "--soft-budget" => {
                    self.policy.auto_continue_low_risk = true;
                }
                other => return Err(format!("unknown govern flag: {other}")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod govern_flag_tests {
    use super::*;

    /// ADDITIVE invariant: no flags → the policy is byte-identical to the default.
    #[test]
    fn absent_flags_reproduce_default_policy() {
        let mut cfg = GovernConfig::new(CliKind::Claude, "hi");
        let no_flags: [&str; 0] = [];
        cfg.apply_flags(no_flags).expect("empty flags must succeed");
        let d = GovernPolicy::default();
        assert_eq!(cfg.policy.max_wall_secs, d.max_wall_secs);
        assert_eq!(cfg.policy.max_output_tokens, d.max_output_tokens);
        assert_eq!(cfg.policy.min_battery_pct, d.min_battery_pct);
        assert_eq!(cfg.policy.auto_continue_low_risk, d.auto_continue_low_risk);
    }

    /// Each flag populates exactly its GovernPolicy brake.
    #[test]
    fn flags_populate_policy_brakes() {
        let mut cfg = GovernConfig::new(CliKind::Claude, "hi");
        cfg.apply_flags([
            "--max-wall-secs",
            "120",
            "--max-output-tokens",
            "5000",
            "--min-battery-pct",
            "25",
            "--soft-budget",
        ])
        .expect("valid flags must parse");
        assert_eq!(cfg.policy.max_wall_secs, Some(120));
        assert_eq!(cfg.policy.max_output_tokens, Some(5000));
        assert_eq!(cfg.policy.min_battery_pct, Some(25));
        assert!(cfg.policy.auto_continue_low_risk);
    }

    #[test]
    fn malformed_and_unknown_flags_error_without_mutating() {
        let mut cfg = GovernConfig::new(CliKind::Claude, "hi");
        assert!(cfg.apply_flags(["--max-wall-secs", "abc"]).is_err());
        assert!(cfg.apply_flags(["--min-battery-pct"]).is_err()); // missing value
        assert!(cfg.apply_flags(["--nope"]).is_err());
    }

    /// apex-④ HARD-BRAKE `--max-wallclock <dur>`: a human duration (with or
    /// without a unit suffix) populates the SAME wall-clock brake as
    /// `--max-wall-secs`, so an unattended governed run aborts at a wall-clock
    /// deadline. Bare digits = seconds; `s`/`m`/`h` suffixes scale.
    #[test]
    fn max_wallclock_flag_parses_duration_into_wall_secs() {
        // Bare seconds.
        let mut cfg = GovernConfig::new(CliKind::Codex, "hi");
        cfg.apply_flags(["--max-wallclock", "1"]).expect("bare seconds must parse");
        assert_eq!(cfg.policy.max_wall_secs, Some(1), "bare digits = seconds");

        // Seconds suffix.
        let mut cfg = GovernConfig::new(CliKind::Codex, "hi");
        cfg.apply_flags(["--max-wallclock", "90s"]).expect("`s` suffix must parse");
        assert_eq!(cfg.policy.max_wall_secs, Some(90));

        // Minutes suffix.
        let mut cfg = GovernConfig::new(CliKind::Codex, "hi");
        cfg.apply_flags(["--max-wallclock", "2m"]).expect("`m` suffix must parse");
        assert_eq!(cfg.policy.max_wall_secs, Some(120));

        // Hours suffix.
        let mut cfg = GovernConfig::new(CliKind::Codex, "hi");
        cfg.apply_flags(["--max-wallclock", "1h"]).expect("`h` suffix must parse");
        assert_eq!(cfg.policy.max_wall_secs, Some(3600));
    }

    /// `--max-wallclock` rejects a malformed/missing value WITHOUT mutating the
    /// policy (same fail-loud shape as the other brake flags).
    #[test]
    fn max_wallclock_flag_rejects_bad_value() {
        let mut cfg = GovernConfig::new(CliKind::Codex, "hi");
        assert!(cfg.apply_flags(["--max-wallclock", "abc"]).is_err(), "non-numeric must error");
        assert!(cfg.apply_flags(["--max-wallclock", "5x"]).is_err(), "unknown unit must error");
        assert!(cfg.apply_flags(["--max-wallclock"]).is_err(), "missing value must error");
        assert_eq!(cfg.policy.max_wall_secs, None, "a rejected value must not mutate the brake");
    }
}

/// Drive a governed AI-CLI run to completion. Returns the final outcome + the
/// run's task id (for flight-recorder replay: governance events are stored under
/// that id, and the raw stream is in `governed_runs/<task_id>.jsonl`).
pub async fn run_govern(cfg: GovernConfig) -> anyhow::Result<(RunOutcome, Uuid)> {
    let (fold, task_id) = run_govern_folded(cfg).await?;
    Ok((fold.outcome, task_id))
}

/// Like [`run_govern`], but also returns the assistant text/usage the governed CLI
/// produced (folded out of the same event stream the governor consumed). This is
/// the worker path: a dispatched task is governed AND its answer is returned to
/// the dispatcher. `fold.error` carries a CLI `Error` event for the caller to surface.
pub async fn run_govern_folded(cfg: GovernConfig) -> anyhow::Result<(GovernedFold, Uuid)> {
    let home = match cfg.home {
        Some(h) => h,
        None => cli_config::resolve_home_dir()?,
    };
    let dir = cli_config::spectyn_dir_under(&home);
    std::fs::create_dir_all(&dir)?;
    let store = TaskStore::open_at(dir.join("spectyn.db"))?;
    let events = EventStore::from_conn(store.conn());
    // apex-④ dispatch↔govern correlation: when this run is governing a DISPATCHED
    // task, use that dispatch row's `job_uuid` AS the govern task_id (one
    // correlation key for flight-recorder + escalator + the dispatch row). Absent
    // (standalone `spectyn govern`, ungoverned) → a fresh id, behavior unchanged.
    let task_id = cfg.dispatch_task_id.unwrap_or_else(Uuid::new_v4);

    // Escalation surface. Always register the local OS desktop notification.
    // ADDITIONALLY, when a telegram bot is configured the SAME way `serve`
    // decides to enable it (an `[telegram]` block in agents.toml whose
    // `bot_token_env` env var holds a non-empty token, with a resolvable
    // chat_id), register the telegram channel too so a high-risk / budget
    // escalation reaches the operator's PHONE — not just a desktop toast.
    // No token configured => behavior is unchanged (OsChannel only).
    let dispatcher = NotificationDispatcher::new();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    dispatcher
        .add_channel(std::sync::Arc::new(
            crate::notifications::channels::OsChannel,
        ))
        .await;
    if let Some(ch) = resolve_telegram_channel(
        crate::config::AgentsConfig::find_and_load()
            .and_then(|c| c.telegram)
            .as_ref(),
        |name| std::env::var(name).ok(),
    ) {
        dispatcher.add_channel(ch).await;
    }

    let handle = Handle::current();
    let recorder = EventStoreRecorder::new_with_identity_path(
        events,
        task_id,
        handle.clone(),
        dir.join("governed_runs"),
        dir.join("identity.key"),
    )?;
    // The hook subprocess (claude's PreToolUse gate) receives this as SPECTYN_HOME.
    // It MUST be the DATA dir (`dir` = spectyn_dir_under(home)), NOT the OS home:
    // spectyn_dir_under() returns SPECTYN_HOME verbatim when set, so passing the OS
    // home would make the hook write its pending card to `<os-home>/pending` while
    // `spectyn serve` (which reads list_pending via the real data dir) looks in
    // `<data-dir>/pending` — a mismatch that hid every approval card from the phone.
    // Passing `dir` makes the hook's data root identical to the parent's + serve's.
    let home_env = dir.display().to_string();
    let mut escalator = PhoneEscalator::new(
        home,
        dispatcher,
        handle.clone(),
        task_id,
        cfg.workspace_id,
        cfg.poll,
        cfg.deadline,
        // `ApprovalDecision` is no longer `Copy` (apex-④ Redirect carries a
        // `String`); clone so `cfg.policy` stays whole for the move below.
        cfg.policy.timeout_fallback.clone(),
    );
    // apex-④ dispatch↔govern correlation: only when this run is governing a
    // DISPATCHED task (task_id IS the dispatch job_uuid) do we hand the escalator
    // the dispatch row's store so it can stamp the approval_id + AwaitingApproval
    // onto that row at escalation time. Standalone `spectyn govern` (no dispatch
    // id) attaches no store → the escalator behaves byte-identically to before.
    if cfg.dispatch_task_id.is_some() {
        escalator = escalator.with_dispatch_store(store.clone());
    }

    let spec = if cfg.cli == CliKind::Claude {
        // apex-④ TRUE pre-action gate for claude: spawn it headless with a
        // PreToolUse hook (matcher `*`) that calls back into THIS spectyn exe
        // (`spectyn pretooluse-gate`) BEFORE every tool and blocks on the reply. The
        // hook is the SOLE pre-action awaiter; env binds it to this run's identity
        // (SPECTYN_GOVERN_TASK_ID) so the operator's reply correlates and the parent
        // loop only OBSERVES claude (no double-await — see decision.rs).
        // SAFETY (fail-closed): claude FAIL-OPENS if its PreToolUse hook command
        // cannot be spawned — a missing/unspawnable hook makes the tool run UNGATED
        // (verified live, claude 2.1.170). So the hook MUST resolve to a real spectyn
        // exe. Default = THIS running spectyn (`current_exe`, which by definition
        // exists). `SPECTYN_GOVERN_HOOK_CMD` overrides it (cargo-test/operator — must
        // point at a real spectyn). If neither yields a usable command, REFUSE to run
        // claude rather than spawn it ungated.
        let hook_cmd = match std::env::var("SPECTYN_GOVERN_HOOK_CMD") {
            Ok(cmd) if !cmd.trim().is_empty() => cmd,
            _ => {
                let exe = std::env::current_exe().map_err(|e| {
                    anyhow::anyhow!(
                        "cannot resolve the spectyn exe for claude's governance hook ({e}); \
                         refusing to run claude UNGATED — set SPECTYN_GOVERN_HOOK_CMD to a \
                         real `\"<spectyn>\" pretooluse-gate`"
                    )
                })?;
                format!("\"{}\" pretooluse-gate", exe.display())
            }
        };
        // SAFETY (fail-closed, both default + override paths): pre-flight the hook
        // command once. If it can't be spawned or doesn't return a valid hook
        // decision, claude would run tools UNGATED — refuse to run it. (agy review.)
        preflight_hook(&hook_cmd, &home_env, task_id).await?;
        let settings = serde_json::json!({
            "hooks": { "PreToolUse": [ {
                "matcher": "*",
                "hooks": [ { "type": "command", "command": hook_cmd } ],
            } ] },
            // `default` mode routes non-allowlisted tools through the hook (NOT the
            // `auto`/`bypass` modes, which would skip it). `allow: []` clears any
            // inherited user/project allowlist (e.g. `Bash(agy *)`) so EVERY tool is
            // gated by the hook — an allowlisted tool would otherwise skip it (a
            // fail-open). Low-risk tools are still auto-allowed inside the hook.
            "permissions": { "defaultMode": "default", "allow": [] },
        })
        .to_string();
        let mut spec = SessionSpec::new(cfg.cli, cfg.cwd, cfg.timeout_secs, cfg.model);
        // Override the base `--permission-mode dontAsk` (which auto-allows and would
        // SKIP the hook) with `default` so claude routes non-allowlisted tools
        // through the PreToolUse hook. The later flag wins.
        spec.extra_args = vec![
            "--permission-mode".to_string(),
            "default".to_string(),
            "--settings".to_string(),
            settings,
        ];
        spec.env = vec![
            ("SPECTYN_HOME".to_string(), home_env),
            ("SPECTYN_GOVERN_TASK_ID".to_string(), task_id.to_string()),
            ("SPECTYN_GOVERN_CLI".to_string(), "1".to_string()),
        ];
        spec
    } else {
        SessionSpec::new(cfg.cli, cfg.cwd, cfg.timeout_secs, cfg.model)
    };
    let cli = cfg.cli;
    let prompt = cfg.prompt;
    let policy = cfg.policy;

    // The drive loop consumes L0's sync event channel and the recorder/escalator
    // block_on async I/O — both require a NON-async-worker thread. spawn_blocking
    // gives us one (and `#[tokio::main]` is multi-threaded, so block_on won't
    // deadlock against the loop's own thread).
    let fold = tokio::task::spawn_blocking(move || -> anyhow::Result<GovernedFold> {
        let mut recorder = recorder;
        let mut escalator = escalator;
        let mut session = cli_session::start(spec)?;
        let rx = session.turn(TurnInput { prompt })?;
        let fold = drive_fold(cli, rx, &mut recorder, &mut escalator, &policy);
        drop(session); // end the L0 child (best-effort kill on Aborted)
        Ok(fold)
    })
    .await??;

    Ok((fold, task_id))
}

/// Split a hook command string into `(program, args)`. Handles a leading
/// `"quoted path"` (the default form, since an exe path may contain spaces) and a
/// bare `program arg...` form.
fn split_command(cmd: &str) -> (String, Vec<String>) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let program = rest[..end].to_string();
            let args = rest[end + 1..]
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            return (program, args);
        }
    }
    let mut parts = cmd.split_whitespace();
    let program = parts.next().unwrap_or("").to_string();
    let args = parts.map(|s| s.to_string()).collect();
    (program, args)
}

/// Pre-flight the claude governance hook (fail-closed for BOTH the default and the
/// `SPECTYN_GOVERN_HOOK_CMD` override): spawn it once with a LOW-RISK PreToolUse
/// input — which the hook auto-allows WITHOUT escalation (silent + fast) — and
/// confirm it returns a valid `permissionDecision`. If the command can't be spawned
/// or doesn't produce a valid hook decision, claude would run tools UNGATED (claude
/// fail-opens on a broken hook), so the caller refuses to run claude.
async fn preflight_hook(hook_cmd: &str, home_env: &str, task_id: Uuid) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let (program, args) = split_command(hook_cmd);
    if program.is_empty() {
        anyhow::bail!("empty governance hook command");
    }
    let mut child = tokio::process::Command::new(&program)
        .args(&args)
        .env("SPECTYN_HOME", home_env)
        .env("SPECTYN_GOVERN_TASK_ID", task_id.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("cannot spawn hook `{program}`: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin
            .write_all(
                br#"{"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"path":"preflight"}}"#,
            )
            .await;
        // drop closes stdin so the hook's read_to_string returns.
    }
    let out = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("hook pre-flight timed out"))?
        .map_err(|e| anyhow::anyhow!("hook pre-flight io: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|_| anyhow::anyhow!("hook returned non-JSON output: {stdout:?}"))?;
    match v
        .get("hookSpecificOutput")
        .and_then(|h| h.get("permissionDecision"))
        .and_then(|d| d.as_str())
    {
        Some("allow") | Some("deny") | Some("ask") => Ok(()),
        _ => anyhow::bail!("hook did not return a valid permissionDecision: {stdout:?}"),
    }
}

/// Sync wrapper for non-async callers (the gated live test, scripts): builds a
/// multi-thread runtime and drives `run_govern` to completion.
pub fn run_govern_blocking(cfg: GovernConfig) -> anyhow::Result<(RunOutcome, Uuid)> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_govern(cfg))
}

/// Sync wrapper returning the folded assistant text/usage too (the gated live test
/// asserts on claude's narration that a tool was hook-blocked).
pub fn run_govern_folded_blocking(cfg: GovernConfig) -> anyhow::Result<(GovernedFold, Uuid)> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_govern_folded(cfg))
}

/// Build the telegram notification channel for the PhoneEscalator, applying the
/// SAME enable decision `serve` uses (`core/src/main.rs`): a configured
/// `[telegram]` block whose `bot_token_env` env var resolves to a NON-EMPTY
/// token, plus a resolvable `chat_id` (`notification_chat_id`, else the first
/// `allowed_users` entry). Returns `None` (=> OsChannel only, behavior
/// unchanged) when telegram is absent, the token env var is unset/empty, or no
/// chat_id is available.
///
/// `env` is the env-var lookup (`|n| std::env::var(n).ok()` in production); it
/// is a parameter so the unit test can drive registration WITHOUT mutating the
/// real process environment.
pub(crate) fn resolve_telegram_channel(
    tg: Option<&crate::TelegramConfig>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<std::sync::Arc<dyn crate::notifications::channels::NotificationChannel>> {
    let tg = tg?;
    let token = env(&tg.bot_token_env)?;
    if token.is_empty() {
        return None;
    }
    // Same chat_id resolution as serve: explicit notification target, else the
    // first allowlisted user. Without a chat to send to, there is nothing to
    // attach (serve skips it too).
    let chat_id = tg
        .notification_chat_id
        .or_else(|| tg.allowed_users.first().copied())?;
    let bot = std::sync::Arc::new(crate::channels::telegram::TelegramBot::new(
        token,
        tg.allowed_users.clone(),
    ));
    Some(std::sync::Arc::new(
        crate::notifications::channels::TelegramChannel::new(bot, chat_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TelegramConfig;

    #[test]
    fn split_command_handles_quoted_path_and_bare_form() {
        // Default form: a quoted exe path (may contain spaces) + the subcommand arg.
        let (p, a) = split_command("\"C:/Program Files/spectyn.exe\" pretooluse-gate");
        assert_eq!(p, "C:/Program Files/spectyn.exe");
        assert_eq!(a, vec!["pretooluse-gate".to_string()]);
        // Bare form: program + args by whitespace.
        let (p, a) = split_command("spectyn pretooluse-gate");
        assert_eq!(p, "spectyn");
        assert_eq!(a, vec!["pretooluse-gate".to_string()]);
        // Just a program, no args.
        let (p, a) = split_command("  spectyn  ");
        assert_eq!(p, "spectyn");
        assert!(a.is_empty());
    }

    fn tg_cfg(allowed: Vec<i64>, notify: Option<i64>) -> TelegramConfig {
        TelegramConfig {
            bot_token_env: "SPECTYN_TEST_TG_TOKEN".to_string(),
            allowed_users: allowed,
            agent: "master".to_string(),
            notification_chat_id: notify,
        }
    }

    #[test]
    fn registers_telegram_when_token_present() {
        // Token env var resolves to a non-empty value AND a chat_id is available
        // (here via the first allowed_users entry) -> a telegram channel is built.
        let cfg = tg_cfg(vec![424242], None);
        let ch = resolve_telegram_channel(Some(&cfg), |n| {
            (n == "SPECTYN_TEST_TG_TOKEN").then(|| "secret-bot-token".to_string())
        });
        let ch = ch.expect("telegram channel must be registered when token is configured");
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn no_telegram_when_token_absent() {
        // Env var unset -> None -> caller registers OsChannel only (unchanged).
        let cfg = tg_cfg(vec![424242], None);
        assert!(
            resolve_telegram_channel(Some(&cfg), |_| None).is_none(),
            "absent token must yield no telegram channel (OsChannel-only fallback)"
        );
    }

    #[test]
    fn no_telegram_when_token_empty() {
        // An empty token is treated as "not configured" (serve guards on this too).
        let cfg = tg_cfg(vec![424242], None);
        assert!(
            resolve_telegram_channel(Some(&cfg), |_| Some(String::new())).is_none(),
            "empty token must yield no telegram channel"
        );
    }

    #[test]
    fn no_telegram_when_config_absent() {
        // No [telegram] block at all -> None regardless of env.
        assert!(
            resolve_telegram_channel(None, |_| Some("token".to_string())).is_none(),
            "missing telegram config must yield no telegram channel"
        );
    }

    #[test]
    fn no_telegram_when_no_chat_id() {
        // Token present but neither notification_chat_id nor any allowed_users ->
        // nothing to send to, so no channel (mirrors serve's skip).
        let cfg = tg_cfg(vec![], None);
        assert!(
            resolve_telegram_channel(Some(&cfg), |_| Some("token".to_string())).is_none(),
            "no resolvable chat_id must yield no telegram channel"
        );
    }

    #[test]
    fn explicit_notification_chat_id_is_used() {
        // notification_chat_id present even with empty allowlist -> channel built.
        let cfg = tg_cfg(vec![], Some(-100200300));
        let ch = resolve_telegram_channel(Some(&cfg), |_| Some("token".to_string()));
        assert!(
            ch.is_some(),
            "explicit notification_chat_id must allow registration without allowed_users"
        );
        assert_eq!(ch.unwrap().name(), "telegram");
    }
}
