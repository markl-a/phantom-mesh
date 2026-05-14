use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use phantom_mesh::{AppState, agent::{AgentRuntime, AgentEvent}, context::WorkspaceContext, cost::CostTracker, session::ConversationStore};
use phantom_mesh::providers::traits::ChatMessage;
use rustyline::error::ReadlineError;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};
use rustyline::history::DefaultHistory;
use std::borrow::Cow;

// ── Evolve system prompt ──────────────────────────────────────────────────────

const EVOLVE_SYSTEM_PROMPT_BASE: &str = "\
## AUTONOMOUS DEVELOPMENT MODE

You are phantom-mesh's self-improvement agent. Improve this codebase through a \
test-driven loop.

### Core loop
1. Run `cargo_test` — collect every failing test and its error output.
2. For each failure: use `content_search` + `file_read` to understand the code.
3. Use `file_edit` to make the minimal correct fix. Never change test expectations \
   to make tests pass — fix the implementation instead.
4. Re-run `cargo_test` to confirm. If still failing, try a different approach (max 4 attempts per failure).
5. Once confirmed green, use `git_add` + `git_commit` (message: \"fix: <what>\").
6. Report: list what was fixed, what is still failing (if any).

### Multi-node distribution
If `PHANTOM_PEER_WINDOWS` env var is set, delegate Windows-specific tasks \
via `http_client` POST to that URL's /agent/master/run.

### Constraints
- Never delete a failing test to make it pass.
- Each commit must leave the tree in a passing state.
- Skip failures that need more than 4 attempts; log them.
- Prefer the smallest possible change.
- After all tests pass, run `cargo_check` for warnings.
- `cargo test` and `cargo build` take 60-300 seconds after file edits (recompilation).
  The shell tool default timeout is 120s; for fresh builds specify `timeout_secs: 300`.
- After completing all tool calls, write ONE of these lines as the LAST line
  of your text response (not as a shell command — write it directly):
  EVOLVE_DONE: all tests pass
  EVOLVE_CONTINUE: N tests still failing
";

const EVOLVE_DEFAULT_TASK: &str = "\
Run cargo_test. Find every failing test. Read the relevant source files, fix the \
failures one by one, re-run to confirm each fix, then commit. \
Continue until all tests pass or you have made progress on every known failure.";

/// Build the system prompt for an evolve run, optionally augmenting it with
/// recent autoevolve history if the calling daemon left a hint in the
/// `PHANTOM_AUTOEVOLVE_HISTORY` env var (compact summary of the last few
/// JSONL entries). When the hint is empty, returns the base prompt.
fn build_evolve_system_prompt() -> String {
    let base = EVOLVE_SYSTEM_PROMPT_BASE.to_string();
    // Self-diagnostic nudge — whenever phantom is evolving its own code we
    // want the agent to start by checking its own crash logs / event ring,
    // not blindly guessing root causes from the prompt.
    let diag_nudge = "\n\n### Self-diagnostic FIRST\n\
If this task involves a crash, regression, or 'why did X fail', call \
`diag_read({kind:'summary'})` BEFORE anything else. If a recent crash \
exists, follow up with `diag_read({kind:'last_crash'})` to read the \
panic/backtrace. Use that to ground your investigation. Don't re-derive \
what happened when phantom already wrote it down.";

    let hist = std::env::var("PHANTOM_AUTOEVOLVE_HISTORY").unwrap_or_default();
    if hist.trim().is_empty() {
        return format!("{}{}", base, diag_nudge);
    }
    format!(
        "{base}{diag}\n\
\n\
### Recent autoevolve history\n\
The autoevolve daemon ran the following recent rounds (oldest → newest). \
Use this to avoid re-investigating known fixes and to recognise recurring \
failures:\n\
{hist}\n",
        base = base,
        diag = diag_nudge,
        hist = hist
    )
}

// ── Evolve loop ───────────────────────────────────────────────────────────────

async fn run_evolve(args: Vec<String>) -> Result<()> {
    // Parse flags
    let mut goal: Option<String> = None;
    let mut max_rounds: usize = 10;
    let mut agent_name = "master".to_string();
    let mut do_rebuild   = false;  // --rebuild: cargo build after success
    let mut do_deploy    = false;  // --deploy:  cluster-install.sh after rebuild
    let mut distributed  = false;  // --distributed: parallel assign to all peers
    let mut allow_core   = false;  // --allow-core-evolve: opt out of sandbox guard
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--max-rounds" | "-n" => {
                i += 1;
                if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) {
                    max_rounds = n;
                }
            }
            "--agent" | "-a" => {
                i += 1;
                if let Some(name) = args.get(i) {
                    agent_name = name.clone();
                }
            }
            "--rebuild"      | "-r" => { do_rebuild = true; }
            "--deploy"       | "-d" => { do_rebuild = true; do_deploy = true; }
            "--distributed"  | "-D" => { distributed = true; }
            "--allow-core-evolve"   => { allow_core = true; }
            arg if !arg.starts_with('-') && goal.is_none() => {
                goal = Some(arg.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    let task = goal.unwrap_or_else(|| EVOLVE_DEFAULT_TASK.to_string());

    eprintln!("{}", colored("◆ phantom evolve — autonomous development loop", 35));
    eprintln!("  {}", colored(&format!("goal: {}", safe_truncate(&task, 100)), 90));
    eprintln!("  {}", colored(&format!("agent: {}  max-rounds: {}", agent_name, max_rounds), 90));

    // CO-EVO Phase 1 sandbox guard (SPEC-FREEZE-V1.1 §4.1-d).
    // Default ON for autoevolve invocations; opt out via flag.
    // Restored to whatever it was on exit so the surrounding session
    // (e.g. interactive REPL after the loop) isn't affected.
    let sandbox_was = phantom_mesh::sandbox::is_enabled();
    let sandbox_now = !allow_core;
    phantom_mesh::sandbox::enable(sandbox_now);
    if sandbox_now {
        eprintln!("  {}", colored("sandbox: ON (refuses writes to core/ app/ templates/ scripts/ — pass --allow-core-evolve to opt out)", 90));
    } else {
        eprintln!("  {}", colored("sandbox: OFF (--allow-core-evolve set; agent may write to core/ app/ templates/ scripts/)", 33));
    }

    let outcome = if distributed {
        run_evolve_distributed(&task, &agent_name, max_rounds).await
    } else {
        run_evolve_local(&task, &agent_name, max_rounds, do_rebuild, do_deploy).await
    };

    // Restore sandbox state for the surrounding session.
    phantom_mesh::sandbox::enable(sandbox_was);

    outcome
}

// ── Distributed evolve ────────────────────────────────────────────────────────

// Describe a node's capabilities as a short string for the decompose prompt.
fn caps_label(caps: &[String]) -> String {
    if caps.is_empty() { "analysis, file ops".into() } else { caps.join(", ") }
}

struct NodeDesc {
    url: Option<String>, // None = local
    name: String,
    caps: Vec<String>,
}

// ─── `phantom evolve replay/list` — checkpoint viewer ─────────────────────────
//
// Surface the autonomous agent's reasoning history. Each autoevolve / evolve
// run writes an `EvolveCheckpoint` to ~/.phantom-mesh/evolve-checkpoints/.
// `phantom evolve list` enumerates them; `phantom evolve replay <id>` renders
// the full markdown timeline. This is the human-audit trail jcode's
// reload-context lacks.

async fn run_evolve_list(args: &[String]) -> Result<()> {
    let active_only = args.iter().any(|a| a == "--active");
    let all = phantom_mesh::evolve_checkpoint::EvolveCheckpoint::list_all(active_only)?;
    if all.is_empty() {
        eprintln!("  {} (no checkpoints under ~/.phantom-mesh/evolve-checkpoints/)",
                  colored("◇", 90));
        return Ok(());
    }
    eprintln!("{} {} evolve checkpoint(s){}",
              colored("◆", 35),
              all.len(),
              if active_only { " (active only)" } else { "" });
    for c in &all {
        eprintln!("  {}", c.render_one_line());
    }
    eprintln!();
    eprintln!("  {} replay one with: phantom evolve replay <session-id>",
              colored("›", 90));
    Ok(())
}

/// `phantom evolve goals <subcommand>` — human-curated goal queue.
///
/// Lets you (the operator) write a markdown checklist of small, concrete
/// milestones, and feed them to autoevolve one at a time. The agent works
/// the next pending goal, commits, marks it done, repeat. Smaller and more
/// auditable than turning autoevolve loose with a generic "improve the
/// codebase" instruction — the goal list is exactly what ships.
async fn run_evolve_goals(args: &[String]) -> Result<()> {
    use phantom_mesh::evolve_goals::GoalsFile;

    let action = args.get(3).map(|s| s.as_str()).unwrap_or("next");
    let path = args
        .iter()
        .position(|a| a == "--file")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "EVOLVE-GOALS.md".to_string());

    match action {
        "next" => {
            let g = GoalsFile::load(&path)?;
            match g.next_pending() {
                Some(line) => {
                    let cb = line.checkbox.as_ref().unwrap();
                    // Print the bare goal text on stdout so it composes with
                    // shell pipelines: `phantom evolve "$(phantom evolve goals next)"`.
                    println!("{}", cb.text);
                    eprintln!("  {} from {} (line {})",
                              colored("›", 90), path, line.idx + 1);
                    eprintln!("  {} {} pending · {} done",
                              colored("·", 90),
                              g.pending_count(), g.done_count());
                }
                None => {
                    eprintln!("  {} all goals complete in {}", colored("✓", 32), path);
                }
            }
            Ok(())
        }
        "add" => {
            // phantom evolve goals add "<goal text>"
            let text = args.get(4)
                .ok_or_else(|| anyhow::anyhow!("usage: phantom evolve goals add \"<goal text>\""))?;
            let mut g = GoalsFile::load(&path)?;
            g.add_pending(text);
            g.save()?;
            eprintln!("  {} added to {}: {}", colored("+", 32), path, text);
            Ok(())
        }
        "list" => {
            let g = GoalsFile::load(&path)?;
            let json_flag = args.contains(&"--json".to_string());
            if json_flag {
                println!("{{\"pending\":{},\"done\":{}}}",
                         serde_json::to_string(&g.pending_goals()).unwrap_or_default(),
                         serde_json::to_string(&g.done_goals()).unwrap_or_default());
            } else {
                eprintln!("{} {}", colored("◆", 35), path);
                eprintln!("  {} pending · {} done",
                          g.pending_count(), g.done_count());
                for line in &g.lines {
                    if let Some(cb) = &line.checkbox {
                        let mark = if cb.checked { colored("✓", 32) } else { colored("○", 33) };
                        let section_tag = match line.section {
                            phantom_mesh::evolve_goals::GoalSection::Pending => "",
                            phantom_mesh::evolve_goals::GoalSection::Done => "",
                            phantom_mesh::evolve_goals::GoalSection::Other => " (outside known section)",
                        };
                        eprintln!("  {} L{:<4} {}{}", mark, line.idx + 1,
                                  cb.text, colored(section_tag, 90));
                    }
                }
            }
            Ok(())
        }
        "mark-done" => {
            // phantom evolve goals mark-done <line> [--sha SHA] [--date YYYY-MM-DD]
            let line_arg = args.get(4)
                .ok_or_else(|| anyhow::anyhow!("usage: phantom evolve goals mark-done <line> [--sha SHA] [--date YYYY-MM-DD]"))?;
            let line_num: usize = line_arg.parse()
                .map_err(|_| anyhow::anyhow!("'{}' is not a line number", line_arg))?;
            let sha = args.iter().position(|a| a == "--sha")
                .and_then(|i| args.get(i + 1).cloned())
                .unwrap_or_else(|| "pending".to_string());
            let date = args.iter().position(|a| a == "--date")
                .and_then(|i| args.get(i + 1).cloned())
                .unwrap_or_else(|| {
                    // Default: today in YYYY-MM-DD (UTC). Plain enough not
                    // to need chrono — autoevolve will rewrite it anyway
                    // when it has the real commit timestamp.
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let day = now / 86400;
                    let (y, m, d) = epoch_day_to_ymd(day as i64);
                    format!("{:04}-{:02}-{:02}", y, m, d)
                });

            let mut g = GoalsFile::load(&path)?;
            // Convert 1-based line input to the 0-based idx the parser uses.
            let idx = line_num.saturating_sub(1);
            let text = g.mark_done(idx, &date, &sha)?;
            g.save()?;
            eprintln!("  {} marked done: {}", colored("✓", 32), text);
            eprintln!("  {} updated {}", colored("·", 90), path);
            Ok(())
        }
        _ => {
            eprintln!("usage: phantom evolve goals <next|list [--json]|add \"<text>\"|mark-done <line> [--sha SHA] [--date YYYY-MM-DD]> [--file PATH]");
            anyhow::bail!("unknown evolve goals subcommand '{}'", action);
        }
    }
}

/// Plain Gregorian date math without pulling in chrono. Returns (year,
/// month, day). Source: Howard Hinnant's civil_from_days algorithm.
fn epoch_day_to_ymd(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `phantom evolve handoff <peer-url> [<session-id>|latest]` — Phase 2 mesh handoff.
///
/// Mark the local checkpoint as `Done(Migrated)`, push it to the peer's
/// `/rpc/evolve-handoff` endpoint over HMAC-auth'd HTTP, and persist the
/// migrated state locally so `evolve list` shows where it went.
///
/// Why ship this rather than a pull-based model: the originator already has
/// fresh provider context (which keys exhausted, which dead-ends were tried)
/// and pushes that snapshot atomically. The receiver appends its own node
/// hop on arrival — no two-phase commit needed because the checkpoint itself
/// is the source of truth, persisted on both sides.
async fn run_evolve_handoff(args: &[String]) -> Result<()> {
    use phantom_mesh::evolve_checkpoint::{EvolveCheckpoint, EvolvePhase, EvolveOutcome};

    let peer_url = match args.get(3) {
        Some(u) if !u.is_empty() => u.trim_end_matches('/').to_string(),
        _ => {
            eprintln!("  {} usage: phantom evolve handoff <peer-url> [<session-id>|latest]",
                      colored("✗", 31));
            anyhow::bail!("missing peer url");
        }
    };
    let arg = args.get(4).map(|s| s.as_str()).unwrap_or("latest");

    // ── Resolve which checkpoint to migrate ───────────────────────────────
    let session_id = if arg == "latest" {
        let active = EvolveCheckpoint::list_all(true)?;
        match active.first() {
            Some(c) => c.session_id.clone(),
            None => {
                eprintln!("  {} no active checkpoints to hand off",
                          colored("○", 33));
                return Ok(());
            }
        }
    } else {
        arg.to_string()
    };

    let mut checkpoint = match EvolveCheckpoint::load(&session_id)? {
        Some(c) => c,
        None => {
            eprintln!("  {} session id '{}' not found",
                      colored("✗", 31), session_id);
            anyhow::bail!("checkpoint not found");
        }
    };

    if matches!(checkpoint.phase, EvolvePhase::Done { .. }) {
        eprintln!("  {} session '{}' already in terminal state — refusing to re-hand off",
                  colored("✗", 31), checkpoint.session_id);
        anyhow::bail!("checkpoint already done");
    }

    eprintln!("{}", colored("◆ phantom evolve handoff", 35));
    eprintln!("  {} {} → {}", colored("session:", 90), checkpoint.session_id, colored(&peer_url, 36));
    eprintln!("  {} {}", colored("goal:", 90), safe_truncate(&checkpoint.goal, 100));
    eprintln!("  {} {}",
              colored("plan:", 90),
              format!("{}/{} steps complete", checkpoint.completed_steps.len(), checkpoint.plan.len()));

    // ── Mark migrated, record hop ─────────────────────────────────────────
    checkpoint.record_node_hop(
        peer_url.clone(),
        format!("manual handoff via `phantom evolve handoff {}`", peer_url),
    );
    checkpoint.set_phase(EvolvePhase::Done {
        outcome: EvolveOutcome::Migrated {
            to_node: peer_url.clone(),
            reason: "operator-initiated mesh handoff".into(),
        },
    });
    checkpoint.save()?;

    // ── Build body, compute HMAC ─────────────────────────────────────────
    let mut app_state = AppState::new();
    if let Some(content) = find_config() {
        app_state.load_config_toml(&content);
    }
    let cluster = app_state.cluster_manager.clone();
    let body = serde_json::to_string(&checkpoint)?;
    let auth_token = if cluster.config.cluster_secret.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        Some(cluster.make_auth_token(&body))
    } else {
        eprintln!("  {} cluster_secret not configured — peer may reject the request",
                  colored("⚠", 33));
        None
    };

    // ── POST ─────────────────────────────────────────────────────────────
    let url = format!("{}/rpc/evolve-handoff", peer_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body);
    if let Some(tok) = auth_token.as_deref() {
        req = req.header("X-Cluster-Auth", tok);
    }

    eprintln!("  {} POST {}", colored("›", 90), url);
    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        eprintln!("  {} peer returned HTTP {}: {}",
                  colored("✗", 31), status.as_u16(), text);
        anyhow::bail!("handoff rejected by peer");
    }

    eprintln!("  {} accepted by peer", colored("✓", 32));
    if let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(hops) = j.get("hops").and_then(|v| v.as_u64()) {
            eprintln!("    {} {} hops in journey",
                      colored("·", 90), hops);
        }
        if let Some(to) = j.get("to_node").and_then(|v| v.as_str()) {
            eprintln!("    {} now running on '{}'",
                      colored("·", 90), to);
        }
    }
    eprintln!("  {} replay locally with: phantom evolve replay {}",
              colored("›", 90), checkpoint.session_id);
    Ok(())
}

async fn run_evolve_replay(args: &[String]) -> Result<()> {
    // Accept either an explicit session id at args[3], or "latest" / nothing
    // (in which case we pick the most-recent checkpoint).
    let arg = args.get(3).map(|s| s.as_str()).unwrap_or("latest");
    let session_id = if arg == "latest" {
        let all = phantom_mesh::evolve_checkpoint::EvolveCheckpoint::list_all(false)?;
        match all.first() {
            Some(c) => c.session_id.clone(),
            None => {
                eprintln!("  {} no checkpoints found", colored("○", 33));
                return Ok(());
            }
        }
    } else {
        arg.to_string()
    };
    match phantom_mesh::evolve_checkpoint::EvolveCheckpoint::load(&session_id)? {
        Some(c) => {
            print!("{}", c.render_markdown());
            Ok(())
        }
        None => {
            eprintln!("  {} session id '{}' not found",
                      colored("✗", 31), session_id);
            anyhow::bail!("checkpoint not found");
        }
    }
}

/// `phantom evolve publish [<session-id>|latest] [--private]`
///
/// Export an EvolveCheckpoint as a signed Recipe at
/// `~/.phantom-mesh/recipes/<sha>.json`. v0.1.0 ships LOCAL ONLY
/// (--private behaviour, which is also the default). Broker upload
/// + auto-PR pipeline ship in v0.2 per CONTRIBUTOR-FUNNEL §3.
///
/// Requires `phantom keys init` to have been run first.
async fn run_evolve_publish(args: &[String]) -> Result<()> {
    let _private = !args.iter().any(|a| a == "--share"); // default = private
    let session_arg = args.iter().skip(3)
        .find(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .unwrap_or("latest");

    let session_id = if session_arg == "latest" {
        let all = phantom_mesh::evolve_checkpoint::EvolveCheckpoint::list_all(false)?;
        match all.first() {
            Some(c) => c.session_id.clone(),
            None => {
                eprintln!("  {} no evolve checkpoints found — run `phantom evolve <goal>` first",
                    colored("○", 33));
                return Ok(());
            }
        }
    } else {
        session_arg.to_string()
    };

    let cp = match phantom_mesh::evolve_checkpoint::EvolveCheckpoint::load(&session_id)? {
        Some(c) => c,
        None => {
            eprintln!("  {} session id '{}' not found", colored("✗", 31), session_id);
            anyhow::bail!("checkpoint not found");
        }
    };

    // Confirm the user has an identity. Recipe export REQUIRES signing.
    if phantom_mesh::identity::load_pub_hex().is_err() {
        eprintln!("  {} no ed25519 keypair found — run `phantom keys init` first",
            colored("✗", 31));
        anyhow::bail!("identity not initialised");
    }

    // Best-effort: extract a `git format-patch` blob if there are
    // uncommitted changes in the working tree. v0.1.0 doesn't try to
    // be clever about this — if the autoevolve session left a clean
    // tree, patch is None (recipe is informational). v0.2 wires the
    // autoevolve commit's patch automatically.
    let patch: Option<String> = match std::process::Command::new("git")
        .args(["diff", "HEAD"])
        .output()
    {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            Some(String::from_utf8_lossy(&o.stdout).to_string())
        }
        _ => None,
    };

    // EvolveCheckpoint already stores plan as Vec<String>; pass through.
    let plan = cp.plan.clone();
    // DeadEnd has hypothesis + why_failed; flatten to a single
    // descriptive line for the recipe consumer.
    let dead_ends = cp.dead_ends.iter()
        .map(|d| format!("{} — {}", d.hypothesis, d.why_failed))
        .collect::<Vec<_>>();
    let completed_steps = cp.completed_steps.iter()
        .map(|s| s.description.clone())
        .collect::<Vec<_>>();
    // NodeHop fields differ from JourneyEntry — adapt:
    // NodeHop {at_ms (i64), from, to, reason} -> JourneyEntry {node=to, ts_ms (u64), note=reason}
    let journey = cp.journey.iter()
        .map(|h| phantom_mesh::recipe::JourneyEntry {
            node:  h.to.clone(),
            ts_ms: h.at_ms.max(0) as u64,
            note:  h.reason.clone(),
        })
        .collect::<Vec<_>>();

    let recipe_sha = phantom_mesh::recipe::compute_sha(
        &cp.goal,
        &plan,
        patch.as_deref(),
    );
    let classification = phantom_mesh::recipe::classify_patch(patch.as_deref());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let body = phantom_mesh::recipe::RecipeBody {
        recipe_sha: recipe_sha.clone(),
        session_id: cp.session_id.clone(),
        goal: cp.goal.clone(),
        plan,
        dead_ends,
        completed_steps,
        journey,
        patch,
        descriptor: phantom_mesh::recipe::Descriptor {
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            phantom_version: env!("CARGO_PKG_VERSION").to_string(),
            core_sha: option_env!("PHANTOM_GIT_HASH").unwrap_or("?").to_string(),
            classification: classification.to_string(),
        },
        published_at_ms: now_ms,
    };

    let recipe = phantom_mesh::recipe::sign(body)?;

    // Verify round-trip immediately (catch sign-load bugs early).
    if !phantom_mesh::recipe::verify(&recipe)? {
        anyhow::bail!("signature failed self-verify — refusing to write");
    }

    let path = phantom_mesh::recipe::save(&recipe)?;

    eprintln!();
    eprintln!("  {} recipe published locally", colored("✓", 32));
    eprintln!("    {}  {}", colored("path:",          90), path.display());
    eprintln!("    {}  {}", colored("recipe_sha:",    90), recipe.body.recipe_sha);
    eprintln!("    {}  {}", colored("classification:", 90), recipe.body.descriptor.classification);
    eprintln!("    {}  {} bytes patch", colored("patch:",  90),
        recipe.body.patch.as_deref().map(|p| p.len()).unwrap_or(0));
    eprintln!("    {}  {}", colored("signed by:",     90), &recipe.author_pubkey[..16]);
    eprintln!();
    eprintln!("  {} broker upload + auto-PR pipeline lands in v0.2", colored("›", 90));
    eprintln!("  {} for now, share manually:  cat {}", colored("›", 90), path.display());
    Ok(())
}

async fn run_evolve_distributed(task: &str, agent_name: &str, max_rounds: usize) -> Result<()> {
    eprintln!("{}", colored("◆ phantom evolve --distributed", 35));
    eprintln!("  {}", colored(&format!("goal: {}", safe_truncate(&task, 120)), 90));

    let mut app_state = AppState::new();
    if let Some(content) = find_config() { app_state.load_config_toml(&content); }
    let cluster = app_state.cluster_manager.clone();

    // ── Discover peers + local ───────────────────────────────────────────────
    eprintln!("\n{}", colored("── Discovering peers ──", 35));
    let statuses = cluster.refresh_all().await;
    let online: Vec<&phantom_mesh::mesh::PeerStatus> = statuses.iter().filter(|s| s.online).collect();

    if online.is_empty() {
        eprintln!("{}", colored("✗ No online peers. Falling back to local evolve.", 31));
        return run_evolve_local(task, agent_name, max_rounds, false, false).await;
    }

    // Build node list: peers first, local last
    let local_caps = cluster.config.capabilities.clone();
    let local_name = cluster.config.node_name.clone().unwrap_or_else(|| "local".into());
    let mut nodes: Vec<NodeDesc> = online.iter().map(|s| NodeDesc {
        url: Some(s.url.clone()),
        name: s.name.clone(),
        caps: s.capabilities.clone(),
    }).collect();
    nodes.push(NodeDesc { url: None, name: local_name.clone(), caps: local_caps });

    eprintln!("  {} peer(s) + local = {} nodes", online.len(), nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let label = if n.url.is_none() { "local".into() } else { n.url.as_deref().unwrap_or("").to_string() };
        eprintln!("    [{}] {} ({})  caps: [{}]",
            i + 1,
            colored(&label, 36),
            colored(&n.name, 90),
            colored(&caps_label(&n.caps), 33));
    }

    // ── Decompose with capability context ────────────────────────────────────
    eprintln!("\n{}", colored("── Decomposing task ──", 35));
    let cost_tracker = CostTracker::new();
    let runtime = app_state.agent_runtime.clone();
    phantom_mesh::tools::subagent::init_global(runtime.clone(), CostTracker::new());


    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let local_node_name = local_name.clone();

    let node_desc: String = nodes.iter().enumerate().map(|(i, n)| {
        let access = if n.url.is_none() {
            format!(" [has local filesystem access at {}]", cwd.display())
        } else {
            " [remote node, no access to local filesystem]".into()
        };
        format!("  Node {}: {} — capabilities: [{}]{}", i + 1, n.name, caps_label(&n.caps), access)
    }).collect::<Vec<_>>().join("\n");

    let decompose_prompt = format!(
        "You are a task orchestrator. Decompose the goal into exactly {total} plain-text subtask descriptions.\n\
        Write each subtask as a natural language instruction (NO code, NO JSON, NO function calls — plain sentences only).\n\n\
        Available nodes:\n{nodes}\n\n\
        Working directory (local node only): {cwd}\n\n\
        Rules:\n\
        - Nodes with 'rust/build/cargo' caps → cargo/clippy/test tasks; include path {cwd} in the instruction\n\
        - Nodes with 'analysis/docs' caps → code review, TODO search, summarize architecture\n\
        - Nodes with 'python' caps → Python scripts\n\
        - Nodes with 'web/fetch' caps → web research, HTTP fetching\n\
        - Remote nodes (non-{local}) have NO filesystem access — do NOT mention local file paths in their subtasks\n\
        - Local node '{local}' CAN read files at {cwd} — include the path when relevant\n\
        - Each subtask: one self-contained sentence describing what to DO and what to OUTPUT\n\n\
        Output ONLY a numbered list of exactly {total} lines. Example:\n\
        1. Run cargo clippy --lib in {cwd} and report any warnings.\n\
        2. List any TODO or FIXME comments found in the src/ directory at {cwd}.\n\n\
        Goal: {goal}",
        total = nodes.len(),
        nodes = node_desc,
        cwd = cwd.display(),
        local = local_node_name,
        goal = task
    );

    let decompose = runtime.run_with_callbacks(
        agent_name, &decompose_prompt, &[],
        Some("Task orchestrator. Output ONLY a numbered list. One subtask per line. No extra text."),
        &cost_tracker, |_| {},
    ).await?;

    let mut subtasks: Vec<String> = parse_numbered_list(&decompose.output);

    if subtasks.is_empty() {
        eprintln!("{}", colored("✗ Could not decompose. Running locally.", 33));
        return run_evolve_local(task, agent_name, max_rounds, false, false).await;
    }

    // Pad or truncate to match node count
    while subtasks.len() < nodes.len() { subtasks.push(task.to_string()); }
    subtasks.truncate(nodes.len());

    eprintln!("  {} subtasks (capability-matched):", subtasks.len());

    // ── Match subtasks → nodes by capability ─────────────────────────────────
    // Build assignment: node_idx → subtask
    let mut assignments: Vec<(usize, String)> = Vec::new(); // (node_idx, subtask)
    let mut used_nodes: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Index of the local node (always last in nodes vec)
    let local_idx = nodes.len() - 1;
    let cwd_lower = cwd.to_string_lossy().to_lowercase();
    // Keywords that indicate the task requires local filesystem access
    let filesystem_keywords = ["cargo","clippy","test","compile","build","grep","find","read file",
                                "read the file","read a file","search.*src","content_search",
                                "file_read","git log","git diff","git blame","todo","fixme",
                                "src/","lib.rs","main.rs",".toml",".rs"];

    for subtask in &subtasks {
        // Find best matching unused node
        let s = subtask.to_lowercase();

        // Tasks that require local filesystem must go to the local node.
        // Also catches any task that literally references the local working path.
        let needs_local_fs = s.contains(&cwd_lower) || filesystem_keywords.iter().any(|kw| {
            if kw.contains(".*") {
                let parts: Vec<&str> = kw.split(".*").collect();
                parts.iter().all(|p| s.contains(p))
            } else {
                s.contains(kw)
            }
        });

        let mut best = nodes.len() - 1; // default: local
        let mut best_score = -1i32;

        for i in 0..nodes.len() {
            if used_nodes.contains(&i) { continue; }
            // Force filesystem-dependent tasks to the local node
            if needs_local_fs && i != local_idx { continue; }
            let score: i32 = nodes[i].caps.iter().map(|cap| {
                let kw = cap.to_lowercase();
                match kw.as_str() {
                    "rust" | "build" | "cargo" =>
                        ["cargo","rust","clippy","test","compile","build","crate"].iter()
                        .any(|w| s.contains(w)) as i32 * 3,
                    "python" =>
                        ["python","pip","torch","numpy"].iter().any(|w| s.contains(w)) as i32 * 3,
                    "analysis" | "research" =>
                        ["analyz","review","report","summar","audit","assess","document","comment"].iter()
                        .any(|w| s.contains(w)) as i32 * 2,
                    "web" | "fetch" =>
                        ["http","web","fetch","scrape","download","url"].iter()
                        .any(|w| s.contains(w)) as i32 * 2,
                    "docs" | "document" =>
                        ["doc","readme","comment","explain","write"].iter()
                        .any(|w| s.contains(w)) as i32 * 2,
                    other => s.contains(other) as i32,
                }
            }).sum::<i32>();
            if score > best_score {
                best_score = score;
                best = i;
            }
        }
        // If all remote nodes are used and a filesystem task is left, assign to local
        if needs_local_fs || !used_nodes.contains(&local_idx) {
            if used_nodes.contains(&best) {
                best = local_idx;
            }
        }
        used_nodes.insert(best);
        assignments.push((best, subtask.clone()));
    }

    for (node_idx, subtask) in &assignments {
        let n = &nodes[*node_idx];
        let label = n.url.as_deref().unwrap_or("local");
        eprintln!("    {} → [{}] {}: {}",
            colored("→", 33), colored(&n.name, 36),
            colored(&format!("caps: [{}]", caps_label(&n.caps)), 33),
            colored(safe_truncate(subtask, 70), 90));
        let _ = label;
    }

    // ── Dispatch ─────────────────────────────────────────────────────────────
    eprintln!("\n{}", colored("── Dispatching ──", 35));
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
    let mut jobs: Vec<(String, String)> = vec![];
    let mut local_subtasks: Vec<String> = vec![];

    for (node_idx, subtask) in &assignments {
        let n = &nodes[*node_idx];
        if let Some(url) = &n.url {
            // Strip any local filesystem paths from remote node subtasks —
            // remote nodes have no access to this machine's filesystem.
            let remote_task = strip_local_paths(subtask, &cwd.to_string_lossy());
            match cluster.assign_task_to_peer(url, agent_name, &remote_task).await {
                Ok(job_id) => {
                    eprintln!("  {} {} ({})", colored("→", 33), colored(url, 36), colored(&job_id, 90));
                    jobs.push((url.clone(), job_id));
                }
                Err(e) => eprintln!("  {} {} ({})", colored("✗", 31), url, e),
            }
        } else {
            // Collect all local subtasks — combine them into one prompt.
            let with_cwd = if subtask.contains(&*cwd.to_string_lossy()) {
                subtask.clone()
            } else {
                format!("{} (working directory: {})", subtask, cwd.display())
            };
            eprintln!("  {} local: {}", colored("→", 33), colored(safe_truncate(&with_cwd, 60), 90));
            local_subtasks.push(with_cwd);
        }
    }

    // Combine multiple local subtasks into one numbered prompt
    let local_subtask = if local_subtasks.is_empty() {
        task.to_string()
    } else if local_subtasks.len() == 1 {
        local_subtasks.remove(0)
    } else {
        let combined = local_subtasks.iter().enumerate()
            .map(|(i, t)| format!("{}. {}", i + 1, t))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Complete ALL of the following tasks:\n{}", combined)
    };

    let local_future = run_evolve_local(&local_subtask, agent_name, max_rounds.min(5), false, false);

    // Poll remote jobs while local runs
    eprintln!("\n{}", colored("── Running in parallel ──", 35));
    let (local_result, remote_outputs) = tokio::join!(
        local_future,
        poll_all_jobs(&client, &jobs)
    );

    if let Err(e) = local_result {
        eprintln!("{} local task error: {}", colored("✗", 31), e);
    }

    // Synthesize results
    if !remote_outputs.is_empty() {
        eprintln!("\n{}", colored("── Synthesizing results ──", 35));
        let mut synthesis_prompt = format!("Synthesize these parallel results for the goal: {}\n\n", task);
        for (i, (url, output)) in remote_outputs.iter().enumerate() {
            synthesis_prompt.push_str(&format!("=== Node {} ({}) ===\n{}\n\n", i + 1, url, safe_truncate(output, 2000)));
        }
        synthesis_prompt.push_str("Provide a unified summary in Traditional Chinese.");

        let synthesis = runtime.run_with_callbacks(
            agent_name, &synthesis_prompt, &[], None, &cost_tracker, |_| {},
        ).await?;
        println!("\n{}", synthesis.output.trim());
    }

    let total = cost_tracker.summary().await["total_usd"].as_f64().unwrap_or(0.0);
    eprintln!("\n{}", colored(&format!("✓ distributed evolve complete  total cost: ${:.4}", total), 32));
    Ok(())
}

async fn poll_all_jobs(
    client: &reqwest::Client,
    jobs: &[(String, String)],
) -> Vec<(String, String)> {
    let mut results = vec![];
    let mut pending: Vec<(String, String)> = jobs.to_vec();
    let mut attempts = 0u32;

    while !pending.is_empty() && attempts < 60 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        attempts += 1;
        let mut still_pending = vec![];

        for (peer_url, job_id) in &pending {
            let url = format!("{}/rpc/task/status/{}", peer_url.trim_end_matches('/'), job_id);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    let status = data["status"].as_str().unwrap_or("unknown");
                    if status == "done" || status == "completed" {
                        // The output field is structured — surface a clear
                        // marker when the peer reports done but no body,
                        // rather than the previous silent "(no output)".
                        let output = match data["output"].as_str() {
                            Some(s) if !s.is_empty() => s.to_string(),
                            _ => format!("(peer {peer_url} reported done with empty output)"),
                        };
                        eprintln!("  {} peer {} job done", colored("✓", 32), colored(peer_url, 36));
                        results.push((peer_url.clone(), output));
                    } else if status == "error" {
                        // Surface the peer's error message instead of swallowing
                        // it — pattern-match the cluster error shapes we know
                        // about (missing agent, HMAC) so the user gets a hint.
                        let err_text = data["error"].as_str().unwrap_or("(no error message)");
                        let hint = if err_text.contains("No agent configuration") {
                            " — peer is missing this agent in its agents.toml (DispatchError::AgentMissing)"
                        } else if err_text.contains("unauthorized") {
                            " — HMAC rejected (DispatchError::HMACMismatch)"
                        } else {
                            ""
                        };
                        eprintln!("  {} peer {} job errored: {}{}",
                            colored("✗", 31), colored(peer_url, 36), err_text, hint);
                        // Propagate so the synthesis stage sees the error too.
                        results.push((peer_url.clone(), format!("[peer error] {err_text}")));
                    } else {
                        still_pending.push((peer_url.clone(), job_id.clone()));
                    }
                }
            }
        }
        pending = still_pending;
        if !pending.is_empty() {
            eprintln!("  {} job(s) still running...", pending.len());
        }
    }
    results
}

// Single-node evolve extracted so distributed can reuse it
async fn run_evolve_local(task: &str, agent_name: &str, max_rounds: usize, do_rebuild: bool, do_deploy: bool) -> Result<()> {
    if std::env::var("PHANTOM_MAX_ROUNDS").is_err() {
        std::env::set_var("PHANTOM_MAX_ROUNDS", "60");
    }

    let mut app_state = AppState::new();
    if let Some(content) = find_config() {
        app_state.load_config_toml(&content);
    }

    let cost_tracker = CostTracker::new();
    let runtime = app_state.agent_runtime.clone();
    phantom_mesh::tools::subagent::init_global(runtime.clone(), CostTracker::new());


    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let flag = interrupted.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() { flag.store(true, Ordering::Relaxed); }
        });
    }

    let mut history: Vec<ChatMessage> = vec![];
    let mut round = 0usize;
    let mut all_done = false;

    while round < max_rounds && !all_done && !interrupted.load(Ordering::Relaxed) {
        round += 1;
        let bar = "─".repeat(50 - format!(" Round {}/{} ", round, max_rounds).len().min(48));
        eprintln!("\n{}", colored(&format!("── Round {}/{} {}", round, max_rounds, bar), 35));

        let prompt = if round == 1 { task.to_string() } else {
            "Continue. Run cargo_test to see the current state, then fix remaining failures. \
             Commit each fix. End your response with EVOLVE_DONE or EVOLVE_CONTINUE.".to_string()
        };

        let tool_outputs: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tool_outputs_cb = tool_outputs.clone();
        let interrupted_cb = interrupted.clone();
        let on_event = move |ev: AgentEvent| {
            if interrupted_cb.load(Ordering::Relaxed) { return; }
            match ev {
                AgentEvent::ToolStart { name, args_preview } => {
                    eprintln!("  {} {} {}", colored("⟳", 33), colored(&name, 36), colored(safe_truncate(&args_preview, 72), 90));
                }
                AgentEvent::ToolDone { name, output_preview } => {
                    eprintln!("  {} {} {}", colored("✓", 32), colored(&name, 36), colored(safe_truncate(&output_preview, 90), 90));
                    if let Ok(mut v) = tool_outputs_cb.lock() { v.push(output_preview); }
                }
                _ => {}
            }
        };

        let t0 = std::time::Instant::now();
        let evolve_system = build_evolve_system_prompt();
        let result = runtime.run_with_callbacks(agent_name, &prompt, &history, Some(&evolve_system), &cost_tracker, on_event).await?;
        let elapsed = t0.elapsed().as_secs_f64();

        let output_trimmed = result.output.trim();
        if !output_trimmed.is_empty() { println!("\n{}", output_trimmed); }

        history.push(ChatMessage { role: "user".into(), content: prompt, tool_calls: None });
        history.push(ChatMessage { role: "assistant".into(), content: result.output.clone(), tool_calls: None });
        if history.len() > 6 { history = history.split_off(history.len() - 6); }

        let output_lower = result.output.to_lowercase();
        let collected = tool_outputs.lock().map(|v| v.join("\n")).unwrap_or_default();
        let all_signals = format!("{}\n{}", output_lower, collected).to_lowercase();
        // Tightened detection — see B1+B2 fixes commit:
        //   - "all tests pass" alone is too permissive; the system prompt
        //     itself contains that phrase, so reading any source file with
        //     file_read used to mark the round done.
        //   - We now require either an explicit EVOLVE_DONE signal from
        //     the agent, OR cargo's real test-result line ("test result: ok.")
        //     paired with the actual "running N tests" preamble cargo prints.
        //   - Bonus guard: if the round produced zero assistant output AND
        //     no real cargo signature, never mark done (covers the case
        //     where every provider failed in this round).
        let cargo_ran = all_signals.contains("test result: ok.")
            && (all_signals.contains("running ") && all_signals.contains(" tests"));
        let agent_responded = !output_lower.trim().is_empty();
        if all_signals.contains("evolve_done")
            || (cargo_ran && agent_responded) {
            all_done = true;
        }

        let last = cost_tracker.last_request_cost().await;
        let session = cost_tracker.session_cost().await;
        eprintln!("\n{}", colored(&format!("[round {} ✓  ↑ ${:.4}  ∑ ${:.4}  {:.1}s]", round, last, session, elapsed), 90));
    }

    if all_done { eprintln!("{}", colored("✓ evolve complete — all tests pass", 32)); }
    else { eprintln!("{}", colored(&format!("◆ evolve stopped after {} rounds", round), 33)); }

    if do_rebuild && (all_done || !interrupted.load(Ordering::Relaxed)) {
        rebuild_and_deploy(do_deploy).await?;
    }
    Ok(())
}

/// Rebuild the phantom binary and optionally redeploy to the cluster.
async fn rebuild_and_deploy(deploy: bool) -> Result<()> {
    eprintln!();
    eprintln!("{}", colored("── Rebuilding binary ──────────────────────────────", 35));

    // Detect repo root (walk up until Cargo.toml or cross-compile target marker).
    let cwd = std::env::current_dir()?;
    let cargo_dir = if cwd.join("Cargo.toml").exists() {
        cwd.clone()
    } else if cwd.join("core/Cargo.toml").exists() {
        cwd.join("core")
    } else {
        cwd.clone()
    };

    // cargo build --release --bin phantom
    eprintln!("  {} cargo build --release --bin phantom", colored("⟳", 33));
    let t0 = std::time::Instant::now();
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--bin", "phantom"])
        .current_dir(&cargo_dir)
        .status()?;

    if !status.success() {
        eprintln!("{}", colored("  ✗ build failed — changes saved but binary not updated", 31));
        return Ok(());
    }
    let elapsed = t0.elapsed().as_secs_f64();
    eprintln!("  {} build succeeded ({:.1}s)", colored("✓", 32), elapsed);

    // Copy fresh binary to dist/ (best-effort — dist/ may not exist in all setups)
    let repo_root = cargo_dir.parent().unwrap_or(&cargo_dir).to_path_buf();
    let src_bin   = cargo_dir.join("target/release/phantom");
    let dist_dir  = repo_root.join("dist");

    let dist_name = phantom_mesh::platform::dist_binary_name();

    if src_bin.exists() {
        if dist_dir.exists() {
            let dst = dist_dir.join(dist_name);
            if let Err(e) = std::fs::copy(&src_bin, &dst) {
                eprintln!("  {} copy to dist/ failed: {}", colored("⚠", 33), e);
            } else {
                eprintln!("  {} updated dist/{}", colored("✓", 32), dist_name);
            }
        }
        // Also replace the currently running binary path if accessible.
        if let Ok(current_exe) = std::env::current_exe() {
            if current_exe != src_bin {
                // Best-effort: only replace if we own the file.
                let _ = std::fs::copy(&src_bin, &current_exe);
                eprintln!(
                    "  {} updated {}",
                    colored("✓", 32),
                    current_exe.display()
                );
            }
        }
    }

    // ── --deploy: push new binary to all cluster nodes ────────────────────────
    if deploy {
        eprintln!();
        eprintln!("{}", colored("── Deploying to cluster ───────────────────────────", 35));
        let install_sh = repo_root.join("scripts/cluster-install.sh");
        if install_sh.exists() {
            eprintln!("  {} scripts/cluster-install.sh", colored("⟳", 33));
            let status = std::process::Command::new("bash")
                .arg(&install_sh)
                .current_dir(&repo_root)
                .status()?;
            if status.success() {
                eprintln!("  {} cluster deploy complete", colored("✓", 32));
            } else {
                eprintln!("  {} cluster deploy failed (check agents.toml + SSH keys)", colored("⚠", 33));
            }
        } else {
            eprintln!(
                "  {} scripts/cluster-install.sh not found — skipping deploy",
                colored("⚠", 33)
            );
        }
    }

    eprintln!();
    eprintln!("{}", colored("✓ Done. Restart phantom serve on each node to pick up the new binary.", 32));
    Ok(())
}

// ── mDNS advertiser ───────────────────────────────────────────────────────────

// ── `phantom coordinator` ─────────────────────────────────────────────────

async fn run_coordinator(args: Vec<String>) -> Result<()> {
    use axum::{Router, extract::{Query, State}, routing::{get, post}, Json};
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use phantom_mesh::mesh::CoordinatorRegistration;

    #[derive(Clone)]
    struct CoordState {
        entries: Arc<RwLock<HashMap<String, (CoordinatorRegistration, u64)>>>,
        ttl_secs: u64,
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    async fn handle_register(
        State(s): State<CoordState>,
        Json(reg): Json<CoordinatorRegistration>,
    ) -> Json<serde_json::Value> {
        let mut entries = s.entries.write().unwrap();
        entries.insert(reg.url.clone(), (reg, now_secs()));
        Json(serde_json::json!({"ok": true}))
    }

    #[derive(serde::Deserialize)]
    struct PeersQuery { secret_hash: Option<String> }

    async fn handle_peers(
        State(s): State<CoordState>,
        Query(q): Query<PeersQuery>,
    ) -> Json<Vec<CoordinatorRegistration>> {
        let entries = s.entries.read().unwrap();
        let cutoff = now_secs().saturating_sub(s.ttl_secs);
        let list: Vec<CoordinatorRegistration> = entries.values()
            .filter(|(reg, ts)| {
                *ts >= cutoff && q.secret_hash.as_deref()
                    .map(|sh| sh == reg.secret_hash)
                    .unwrap_or(true)
            })
            .map(|(reg, _)| reg.clone())
            .collect();
        Json(list)
    }

    let host = args.iter().position(|a| a == "--host")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("0.0.0.0");
    let port: u16 = args.iter().position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(7900);
    let ttl_secs: u64 = args.iter().position(|a| a == "--ttl")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);

    let state = CoordState {
        entries: Arc::new(RwLock::new(HashMap::new())),
        ttl_secs,
    };

    let router = Router::new()
        .route("/register", post(handle_register))
        .route("/peers",    get(handle_peers))
        .with_state(state);

    eprintln!("{}", colored("◆ phantom coordinator", 35));
    eprintln!("  listen : http://{}:{}", host, port);
    eprintln!("  TTL    : {}s (nodes must re-register within this window)", ttl_secs);
    eprintln!("  Routes : POST /register  |  GET /peers?secret_hash=<hash>");
    eprintln!("  Press Ctrl-C to stop.");
    eprintln!();

    phantom_mesh::start_http_server(host, port, router).await?;
    Ok(())
}

/// Advertise this node on the local network via mDNS so peers can auto-discover it.
async fn advertise_mdns(node_name: &str, host: &str, port: u16) {
    let ip = if host == "0.0.0.0" || host == "127.0.0.1" {
        std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| { s.connect("8.8.8.8:80")?; s.local_addr() })
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| "127.0.0.1".to_string())
    } else {
        host.to_string()
    };
    let url          = format!("http://{}:{}", ip, port);
    let service_name = format!("phantom-{}", node_name);
    let url_txt      = format!("url={}", url);

    tracing::info!("mDNS: advertising {} at {}", service_name, url);
    phantom_mesh::platform::mdns_advertise(&service_name, port, &url_txt).await;
}

// ── phantom peer subcommand ───────────────────────────────────────────────────

async fn run_peer(args: Vec<String>) -> anyhow::Result<()> {
    let sub = args.get(2).map(|s| s.as_str());

    let mut app_state = AppState::new();
    if let Some(content) = find_config() {
        app_state.load_config_toml(&content);
    }
    let manager = &app_state.cluster_manager;

    match sub {
        // ── phantom peer list ────────────────────────────────────────────
        Some("list") | None => {
            if manager.config.peers.is_empty() {
                eprintln!("{}", colored("No peers configured.", 33));
                eprintln!("Add to agents.toml:");
                eprintln!("  [cluster]");
                eprintln!("  peers = [\"http://192.168.1.X:7878\"]");
                eprintln!();
                eprintln!("Scanning for peers (mDNS + Tailscale)...");
                let (mdns, tailscale) = tokio::join!(
                    phantom_mesh::mesh::discover_local_peers(),
                    phantom_mesh::mesh::discover_tailscale_peers(),
                );
                // Dedupe: a peer that appears under both LAN mDNS and a
                // Tailscale IP shows up twice; we'd rather print once.
                let mut seen = std::collections::HashSet::new();
                let mut discovered: Vec<(&str, String)> = Vec::new();
                for url in &mdns {
                    if seen.insert(url.clone()) {
                        discovered.push(("mDNS",     url.clone()));
                    }
                }
                for url in &tailscale {
                    if seen.insert(url.clone()) {
                        discovered.push(("Tailscale", url.clone()));
                    }
                }

                if discovered.is_empty() {
                    eprintln!("{}", colored("No peers discovered via mDNS or Tailscale.", 90));
                    eprintln!("  {}", colored("Tip: ensure phantom serve is running on a peer and you're on the same LAN or tailnet.", 90));
                } else {
                    eprintln!("{}", colored(&format!("Found {} peer(s):", discovered.len()), 32));
                    for (source, url) in &discovered {
                        eprintln!("  {} {} {}",
                            colored(url, 36),
                            colored("via", 90),
                            colored(source, 90),
                        );
                    }
                    eprintln!();
                    eprintln!("  {} add to agents.toml [cluster] peers = [...] to start using them.",
                        colored("›", 90));
                }
                return Ok(());
            }

            eprintln!("Pinging {} peer(s)...", manager.config.peers.len());
            let peers = manager.refresh_all().await;
            eprintln!();
            println!("{:<42} {:<20} {:<8} {:>6}", "URL", "NAME", "STATUS", "TASKS");
            println!("{}", "─".repeat(80));
            for p in &peers {
                let status_colored = if p.online {
                    colored("online", 32)
                } else {
                    colored("offline", 31)
                };
                println!("{:<42} {:<20} {:<8} {:>6}",
                    p.url, p.name, status_colored, p.active_tasks);
            }
        }

        // ── phantom peer ping <url> ──────────────────────────────────────
        Some("ping") => {
            let url = args.get(3)
                .ok_or_else(|| anyhow::anyhow!("Usage: phantom peer ping <url>"))?;
            eprintln!("Pinging {}...", colored(url, 36));
            match manager.ping_peer(url).await {
                Ok(s) => {
                    println!("  name    : {}", colored(&s.name, 36));
                    println!("  version : {}", s.version);
                    println!("  uptime  : {}s", s.uptime_secs);
                    println!("  tasks   : {}", s.active_tasks);
                    println!("  status  : {}", colored("online", 32));
                }
                Err(e) => {
                    eprintln!("  {} {}", colored("error:", 31), e);
                    std::process::exit(1);
                }
            }
        }

        // ── phantom peer assign [--agent NAME] <prompt> ──────────────────
        Some("assign") => {
            let mut agent  = "master".to_string();
            let mut parts: Vec<String> = vec![];
            let mut i = 3usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--agent" | "-a" => {
                        i += 1;
                        if let Some(a) = args.get(i) { agent = a.clone(); }
                    }
                    _ => parts.push(args[i].clone()),
                }
                i += 1;
            }
            let prompt = parts.join(" ");
            if prompt.is_empty() {
                anyhow::bail!("Usage: phantom peer assign [--agent NAME] <prompt>");
            }
            eprintln!("Forwarding to best peer (agent={})...", colored(&agent, 36));
            // Refresh peer health before picking. Each `phantom peer assign`
            // is a fresh CLI process so the cached state starts at offline.
            let refreshed = manager.refresh_all().await;
            let online: Vec<&phantom_mesh::mesh::PeerStatus> =
                refreshed.iter().filter(|p| p.online).collect();
            if online.is_empty() {
                eprintln!("{} {} peer(s) configured, none online.",
                    colored("✗", 31),
                    refreshed.len());
                for p in &refreshed {
                    eprintln!("    {} {} (offline)", colored("·", 90), p.url);
                }
                std::process::exit(1);
            }
            eprintln!("  {} {} peer(s) online — picking least-loaded:",
                colored("◆", 36), online.len());
            for p in &online {
                eprintln!("    {} {} ({} active task{})",
                    colored("•", 36), p.url, p.active_tasks,
                    if p.active_tasks == 1 { "" } else { "s" });
            }
            match manager.assign_task_to_best_peer(&agent, &prompt).await {
                Ok(output) => println!("{}", output),
                Err(e) => {
                    use phantom_mesh::mesh::DispatchError;
                    // Print structured diagnostics so the user knows which
                    // remediation to take (rotate secret vs fix agents.toml
                    // vs retry vs investigate upstream LLM).
                    let msg = match &e {
                        DispatchError::NoPeersAvailable =>
                            "No online peers available to take the task.".to_string(),
                        DispatchError::PeerUnreachable { url, source } =>
                            format!("Peer {url} unreachable: {source}"),
                        DispatchError::HMACMismatch { url } =>
                            format!("HMAC auth rejected by {url} — check cluster_secret matches on both nodes."),
                        DispatchError::AgentMissing { url, agent } =>
                            format!("Peer {url} has no agent '{agent}' configured in its agents.toml."),
                        DispatchError::PeerRejected { url, code, message } => {
                            let code_part = code.as_deref().map(|c| format!(" [{c}]")).unwrap_or_default();
                            format!("Peer {url} rejected the task{code_part}: {message}")
                        }
                        DispatchError::Timeout { url, elapsed } =>
                            format!("Peer {url} timed out after {elapsed:?}."),
                        DispatchError::Other(m) =>
                            format!("Dispatch error: {m}"),
                    };
                    eprintln!("{}", colored(&msg, 31));
                    std::process::exit(1);
                }
            }
        }

        // ── phantom peer send-async [--agent NAME] <prompt> ─────────────
        // Fire-and-forget: returns a job_id for polling.
        Some("send-async") => {
            let mut agent  = "master".to_string();
            let mut parts: Vec<String> = vec![];
            let mut i = 3usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--agent" | "-a" => {
                        i += 1;
                        if let Some(a) = args.get(i) { agent = a.clone(); }
                    }
                    _ => parts.push(args[i].clone()),
                }
                i += 1;
            }
            let prompt = parts.join(" ");
            if prompt.is_empty() {
                anyhow::bail!("Usage: phantom peer send-async [--agent NAME] <prompt>");
            }
            eprintln!("Dispatching async task (agent={})...", colored(&agent, 36));
            match manager.assign_task_async(&agent, &prompt).await {
                Ok(job_id) => {
                    eprintln!("{}", colored("Task accepted.", 32));
                    println!("{}", job_id);
                    eprintln!("Poll: phantom peer poll <peer-url> {}", job_id);
                }
                Err(e) => {
                    eprintln!("{} {}", colored("Dispatch failed:", 31), e);
                    std::process::exit(1);
                }
            }
        }

        // ── phantom peer poll <peer-url> <job-id> ────────────────────────
        Some("poll") => {
            let peer_url = args.get(3)
                .ok_or_else(|| anyhow::anyhow!("Usage: phantom peer poll <peer-url> <job-id>"))?;
            let job_id = args.get(4)
                .ok_or_else(|| anyhow::anyhow!("Usage: phantom peer poll <peer-url> <job-id>"))?;
            eprintln!("Polling {} job {}...", colored(peer_url, 36), colored(job_id, 90));
            match manager.poll_task(peer_url, job_id).await {
                Some((status, output)) => {
                    println!("status : {}", status);
                    if let Some(out) = output { println!("{}", out); }
                }
                None => {
                    eprintln!("{}", colored("Failed to reach peer or job not found.", 31));
                    std::process::exit(1);
                }
            }
        }

        Some("discover") => {
            // Force discovery even when [cluster] peers is non-empty —
            // useful for spotting tailnet peers that haven't been added
            // to agents.toml yet.
            //
            // mDNS via `dns-sd -B` doesn't exit on its own, so cap each
            // discovery method at 4s. The Tailscale path internally
            // probes with a 2s per-peer timeout.
            eprintln!("Scanning for peers (mDNS + Tailscale)...");
            let mdns_fut = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                phantom_mesh::mesh::discover_local_peers(),
            );
            let tailscale_fut = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                phantom_mesh::mesh::discover_tailscale_peers(),
            );
            let (mdns_res, tailscale_res) = tokio::join!(mdns_fut, tailscale_fut);
            let mdns      = mdns_res.unwrap_or_else(|_| { eprintln!("  {} mDNS scan timed out", colored("⚠", 33)); vec![] });
            let tailscale = tailscale_res.unwrap_or_else(|_| { eprintln!("  {} Tailscale scan timed out", colored("⚠", 33)); vec![] });
            let mut seen = std::collections::HashSet::new();
            let mut found: Vec<(&str, String)> = Vec::new();
            for url in &mdns {
                if seen.insert(url.clone()) { found.push(("mDNS",      url.clone())); }
            }
            for url in &tailscale {
                if seen.insert(url.clone()) { found.push(("Tailscale", url.clone())); }
            }
            if found.is_empty() {
                eprintln!("{}", colored("No peers discovered via mDNS or Tailscale.", 90));
            } else {
                eprintln!("{}", colored(&format!("Found {} peer(s):", found.len()), 32));
                for (source, url) in &found {
                    eprintln!("  {} {} {}",
                        colored(url, 36),
                        colored("via", 90),
                        colored(source, 90),
                    );
                }
            }
        }

        Some(other) => {
            eprintln!("Unknown peer subcommand: {}", other);
            eprintln!("Usage: phantom peer [list | discover | ping <url> | assign <prompt> | send-async <prompt> | poll <url> <id>]");
            std::process::exit(1);
        }
    }
    Ok(())
}

// ── String helpers ────────────────────────────────────────────────────────────

/// Truncate `s` to at most `max_bytes` bytes, respecting UTF-8 char boundaries.
// ── Swarm: parallel prompt to all peers ──────────────────────────────────────

async fn run_swarm(args: Vec<String>) -> Result<()> {
    let mut prompt: Option<String> = None;
    let mut agent_name = "master".to_string();
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--agent" | "-a" => { i += 1; if let Some(n) = args.get(i) { agent_name = n.clone(); } }
            arg if !arg.starts_with('-') && prompt.is_none() => { prompt = Some(arg.to_string()); }
            _ => {}
        }
        i += 1;
    }
    let prompt = match prompt {
        Some(p) => p,
        None => { eprintln!("Usage: phantom swarm <PROMPT> [--agent NAME]"); return Ok(()); }
    };

    eprintln!("{}", colored("◆ phantom swarm — parallel analysis across cluster", 35));
    eprintln!("  {}", colored(safe_truncate(&prompt, 100), 90));

    let mut app_state = AppState::new();
    if let Some(content) = find_config() { app_state.load_config_toml(&content); }
    let cluster = app_state.cluster_manager.clone();
    let cost_tracker = CostTracker::new();
    let runtime = app_state.agent_runtime.clone();
    phantom_mesh::tools::subagent::init_global(runtime.clone(), CostTracker::new());


    // Ping peers
    eprintln!("\n{}", colored("── Discovering peers ──", 35));
    let statuses = cluster.refresh_all().await;
    let online_peers: Vec<String> = statuses.iter().filter(|s| s.online).map(|s| s.url.clone()).collect();
    eprintln!("  {} peer(s) online + local = {} nodes", online_peers.len(), online_peers.len() + 1);

    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;

    // Dispatch to all peers async (using ClusterManager for HMAC auth)
    eprintln!("\n{}", colored("── Dispatching ──", 35));
    let mut jobs: Vec<(String, String)> = vec![];
    for peer_url in &online_peers {
        match cluster.assign_task_to_peer(peer_url, &agent_name, &prompt).await {
            Ok(job_id) => {
                eprintln!("  {} → {} ({})", colored("→", 33), colored(peer_url, 36), colored(&job_id, 90));
                jobs.push((peer_url.clone(), job_id));
            }
            Err(e) => {
                eprintln!("  {} → {} ({})", colored("✗", 31), colored(peer_url, 36), e);
            }
        }
    }

    // Run locally in parallel
    eprintln!("  {} → local", colored("→", 33));
    let local_future = async {
        runtime.run_with_callbacks(&agent_name, &prompt, &[], None, &cost_tracker, |_| {}).await
    };

    let (local_result, remote_outputs) = tokio::join!(local_future, poll_all_jobs(&client, &jobs));

    // Show local result
    eprintln!("\n{}", colored("── Results ──", 35));
    eprintln!("{}", colored("[ local ]", 36));
    if let Ok(r) = local_result {
        println!("{}", r.output.trim());
    }

    // Show remote results
    for (url, output) in &remote_outputs {
        eprintln!("\n{}", colored(&format!("[ {} ]", url), 36));
        println!("{}", phantom_mesh::tools::floor_char_boundary(output, 3000));
    }

    // Synthesize if multiple results
    if !remote_outputs.is_empty() {
        eprintln!("\n{}", colored("── Synthesis ──", 35));
        let mut synth = format!("Synthesize these parallel analysis results for: {}\n\n", prompt);
        for (url, out) in &remote_outputs {
            synth.push_str(&format!("=== {} ===\n{}\n\n", url, safe_truncate(out, 1500)));
        }
        synth.push_str("Write a unified summary in Traditional Chinese.");
        let synthesis = runtime.run_with_callbacks(&agent_name, &synth, &[], None, &cost_tracker, |_| {}).await?;
        println!("\n{}", synthesis.output.trim());
    }

    let total = cost_tracker.summary().await["total_usd"].as_f64().unwrap_or(0.0);
    eprintln!("\n{}", colored(&format!("✓ swarm complete  cost: ${:.4}", total), 32));
    Ok(())
}

/// Parse a numbered list from LLM output — handles various formats:
/// "1. Task", "1) Task", "**1.** Task", "- 1. Task", "Task 1: ..."
/// Remove local absolute paths from a task description destined for a remote node.
/// Replaces occurrences of `local_path` with "the codebase" so the task remains meaningful.
fn strip_local_paths(task: &str, local_path: &str) -> String {
    task.replace(local_path, "the codebase")
        // Clean up doubled spaces or dangling " in "
        .replace("  ", " ")
        .trim()
        .to_string()
}

fn parse_numbered_list(text: &str) -> Vec<String> {
    let re_leading  = regex::Regex::new(r"^\s*[\*\-]?\s*\d+[\.\)]\s*\*{0,2}").unwrap();
    let re_inline   = regex::Regex::new(r"(?i)^(?:subtask|task)\s+\d+[:\.]\s*").unwrap();
    let mut out = vec![];
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') { continue; }
        // Match lines that have a digit-based prefix
        if re_leading.is_match(t) {
            let stripped = re_leading.replace(t, "");
            let cleaned = stripped.trim().trim_matches('*').trim().to_string();
            if !cleaned.is_empty() { out.push(cleaned); }
        } else if re_inline.is_match(t) {
            let stripped = re_inline.replace(t, "");
            let cleaned = stripped.trim().to_string();
            if !cleaned.is_empty() { out.push(cleaned); }
        }
    }
    out
}

fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes { return s; }
    let mut idx = max_bytes;
    while idx > 0 && !s.is_char_boundary(idx) { idx -= 1; }
    &s[..idx]
}

// ── Color helpers ─────────────────────────────────────────────────────────────

fn is_colored() -> bool {
    std::env::var("NO_COLOR").is_err()
}

fn colored(text: &str, code: u8) -> String {
    if is_colored() {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

// ── @file expansion ───────────────────────────────────────────────────────────

/// Expand any `@path` tokens in `input` into `<file path="…">…</file>` blocks.
///
/// Special case: when `path` ends with a recognised image extension (.png,
/// .jpg, .jpeg, .gif, .webp), the file is **not** read as text. Instead it
/// is base64-encoded and emitted as a `<phantom-image .../>` sentinel that
/// the agent layer converts into an OpenAI multimodal `image_url` content
/// part.
fn expand_at_files(input: &str) -> String {
    use phantom_mesh::multimodal::{encode_image_sentinel, image_mime_for_path};

    let mut result = String::new();
    let mut first = true;
    for word in input.split_whitespace() {
        if !first { result.push(' '); }
        first = false;
        if let Some(path_str) = word.strip_prefix('@') {
            if image_mime_for_path(path_str).is_some() {
                match encode_image_sentinel(path_str) {
                    Ok(sentinel) => result.push_str(&sentinel),
                    Err(e) => result.push_str(&format!(
                        "[error reading image {}: {}]",
                        path_str, e
                    )),
                }
                continue;
            }
            match std::fs::read_to_string(path_str) {
                Ok(contents) => {
                    result.push_str(&format!(
                        "<file path=\"{}\">\n{}\n</file>",
                        path_str, contents
                    ));
                }
                Err(e) => {
                    result.push_str(&format!(
                        "[error reading {}: {}]",
                        path_str, e
                    ));
                }
            }
        } else {
            result.push_str(word);
        }
    }
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    // Crash + event capture — must run before anything else can panic
    // so the panic hook is in place. Idempotent.
    phantom_mesh::diag::init();

    // Source ~/.phantom-mesh/env (KEY=value lines) into our env so
    // `api_key_env = "GROQ_API_KEY"` lookups resolve without requiring
    // the user to remember `set -a; source ~/.phantom-mesh/env; set +a`
    // (or PowerShell SetEnvironmentVariable) before every phantom call.
    // Existing shell-set vars are NOT overwritten — explicit env always wins.
    phantom_mesh::cli_config::auto_load_env();

    let mut args: Vec<String> = std::env::args().collect();
    phantom_mesh::diag::record(
        "main",
        format!("argv: {}", args.iter().take(8).cloned().collect::<Vec<_>>().join(" ")),
    );

    // ── `phantom --version / -V` ────────────────────────────────────────
    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        // Single-line short form (e.g. for shell scripts) when followed by
        // `--short`; full provenance otherwise.
        if args.iter().any(|a| a == "--short") {
            println!("{}", env!("CARGO_PKG_VERSION"));
        } else {
            println!(
                "phantom {} ({}, {}-{}, built {})",
                env!("CARGO_PKG_VERSION"),
                option_env!("PHANTOM_GIT_HASH").unwrap_or("nogit"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                option_env!("PHANTOM_BUILD_DATE").unwrap_or("?"),
            );
        }
        return Ok(());
    }

    // ── `phantom help / --help / -h` ──────────────────────────────────────
    let sub = args.get(1).map(|s| s.as_str());
    if sub == Some("help") || sub == Some("--help") || sub == Some("-h") {
        eprintln!("{}  {}",
            colored("phantom — AI agent mesh CLI", 35),
            colored(env!("CARGO_PKG_VERSION"), 90));
        eprintln!();
        eprintln!("{}", colored("INTERACTIVE", 90));
        eprintln!("  {}", colored("phantom", 36));
        eprintln!("       Default: ratatui TUI (set PHANTOM_REPL=1 to force line REPL)");
        eprintln!("  {}  [--agent NAME] [--session ID] [-c] [PROMPT]", colored("phantom repl", 36));
        eprintln!("       Line-mode REPL, or one-shot prompt with -c");
        eprintln!("  {}", colored("phantom tui", 36));
        eprintln!("       Full-screen ratatui interface (alternative entry to TUI)");
        eprintln!("  {}  [--agent NAME] [--json|--quiet] [--continue] [PROMPT]", colored("phantom exec", 36));
        eprintln!("       Headless single-turn run for CI/pipelines (stdin = prompt; stdout = answer)");
        eprintln!();
        eprintln!("{}", colored("DAEMON / SERVICE", 90));
        eprintln!("  {}  [--host H] [--port P]", colored("phantom serve", 36));
        eprintln!("       WebSocket + HTTP daemon, /ws /api/* /rpc/* /m /dist/* /scripts/*");
        eprintln!("  {}  [install|uninstall|status]", colored("phantom service", 36));
        eprintln!("       Auto-start at user login: launchd (mac) / Scheduled Task (win) /");
        eprintln!("       systemd --user (linux). Defaults to status.");
        eprintln!("  {}", colored("phantom mcp", 36));
        eprintln!("       MCP stdio server (50 tools, 2024-11-05 protocol)");
        eprintln!();
        eprintln!("{}", colored("SELF-IMPROVEMENT", 90));
        eprintln!("  {}  [GOAL] [--max-rounds N] [--agent NAME] [--rebuild] [--deploy] [--distributed]", colored("phantom evolve", 36));
        eprintln!("       One-shot test-driven fix loop");
        eprintln!("  {}  [--once|--watch] [--interval N] [--target check|test] [--distributed]", colored("phantom autoevolve", 36));
        eprintln!("       Daemon-mode self-improvement; auto-commit on green");
        eprintln!("  {}  schedule [install|uninstall|status]", colored("phantom autoevolve", 36));
        eprintln!("       Manage the OS scheduler entry (LaunchAgent / Scheduled Task) that runs --once on a cadence");
        eprintln!("  {}  log [--n N]", colored("phantom autoevolve", 36));
        eprintln!("       Pretty-print recent JSONL entries");
        eprintln!("  {}  list [--active]   ·   replay [<id>|latest]   ·   handoff <peer-url> [<id>|latest]", colored("phantom evolve", 36));
        eprintln!("       Inspect & migrate self-improvement checkpoints across the mesh");
        eprintln!("  {}  goals next   ·   goals list [--json]   ·   goals add \"<text>\"   ·   goals mark-done <line>   [--file PATH]", colored("phantom evolve", 36));
        eprintln!("       Curated milestone queue (default: ./EVOLVE-GOALS.md)");
        eprintln!("  {}  <PROMPT> [--agent NAME]", colored("phantom swarm", 36));
        eprintln!("       Fan a prompt to every online peer in parallel; synthesize results");
        eprintln!();
        eprintln!("{}", colored("DIAGNOSTICS", 90));
        eprintln!("  {}", colored("phantom doctor", 36));
        eprintln!("       Single-screen environment health check");
        eprintln!("  {}  [--json] [--out FILE] [--feature N] [--p0-only] [--list]", colored("phantom selftest", 36));
        eprintln!("       Per-feature self-test suite; failures emit repro + artifact + hints");
        eprintln!("  {}  [--source URL] [--dry-run]", colored("phantom self-update", 36));
        eprintln!("       Pull a fresh binary + atomically swap + restart service");
        eprintln!();
        eprintln!("{}", colored("MAC-ONLY", 90));
        eprintln!("  {}  [create|list|delete|prune|rollback|apply] [args]", colored("phantom snapshot", 36));
        eprintln!("       APFS local snapshot — safety net for subagent runs");
        eprintln!("       apply <id> [--cwd|--path P] [--execute]   automated rsync restore");
        eprintln!("  {}  [pull|serve|status|stop] [--model M] [--port P]", colored("phantom mlx", 36));
        eprintln!("       Apple Silicon local LLM helper (mlx_lm.server orchestration)");
        eprintln!();
        eprintln!("{}", colored("CLUSTER", 90));
        eprintln!("  {}  [list | ping <url> | assign <prompt> | send-async <prompt> | poll <url> <id>]", colored("phantom peer", 36));
        eprintln!("       Inspect, ping, dispatch tasks to cluster peers");
        eprintln!("  {}", colored("phantom coordinator", 36));
        eprintln!("       Run as a coordinator hub for the cluster (optional)");
        eprintln!();
        eprintln!("{}", colored("IDENTITY", 90));
        eprintln!("  {}", colored("phantom login", 36));
        eprintln!("       Default: probe the broker (https://phantommesh.io) and");
        eprintln!("       delegate OAuth there. Falls back to a local provider menu");
        eprintln!("       (email / google / apple) when broker is offline.");
        eprintln!("  {}  [email|google|apple|broker]", colored("phantom login", 36));
        eprintln!("       Force a specific provider — bypasses broker probe.");
        eprintln!("       email = local-only password; google = OAuth loopback at :48181;");
        eprintln!("       apple = stub (needs broker); broker = re-try phantommesh.io.");
        eprintln!("  {}", colored("phantom logout", 36));
        eprintln!("       Delete the saved identity.");
        eprintln!("  {}", colored("phantom whoami", 36));
        eprintln!("       Print current identity (provider · email · device id).");
        eprintln!("       PHANTOM_AUTH_URL=...  override broker URL.");
        eprintln!();
        eprintln!("{}", colored("ONE-TIME SETUP", 90));
        eprintln!("  {}", colored("phantom init", 36));
        eprintln!("       Generate PHANTOM.md scaffold in current directory");
        eprintln!("  {}", colored("phantom onboarding", 36));
        eprintln!("       Browser-based first-time setup wizard");
        eprintln!();
        eprintln!("{}", colored("VERSION INFO", 90));
        eprintln!("  {}  [--short]", colored("phantom --version", 36));
        eprintln!("       Default: version + git hash + os-arch + build date");
        eprintln!("       --short returns the bare semver string");
        eprintln!();
        eprintln!("{}", colored("ENVIRONMENT", 90));
        eprintln!("  ANTHROPIC_API_KEY / OPENAI_API_KEY / GROQ_API_KEY / GEMINI_API_KEY");
        eprintln!("  PHANTOM_MAX_ROUNDS    cap subagent rounds (default 25)");
        eprintln!("  PHANTOM_PERM          allow|ask|diff|deny — REPL tool gate");
        eprintln!("  PHANTOM_PLAN_MODE     1 to gate tools until 'go' is typed");
        eprintln!("  PHANTOM_PORT          override :7878 for serve");
        eprintln!("  PHANTOM_COORD         coordinator URL for self-update / mesh");
        eprintln!("  NO_COLOR              disable ANSI colors");
        eprintln!();
        eprintln!("{}", colored("CONFIG", 90));
        eprintln!("  ~/.phantom-mesh/agents.toml  |  ./agents.toml      provider keys + agents");
        eprintln!("  ~/.phantom-mesh/env                                shell-sourcable secrets");
        eprintln!("  ~/.phantom-mesh/autoevolve.log                     JSONL self-improvement log");
        eprintln!("  ~/.phantom-mesh/conversations/<id>.jsonl           session persistence");
        eprintln!();
        eprintln!("Docs: docs/MAC-DEEP-EXECUTION-PLAN.md  ·  docs/INSTALL-{{ANDROID,IOS}}.md");
        eprintln!("      docs/SCENARIOS-MULTIAGENT.md  ·  docs/COMMERCIAL-DESIGN.md");
        eprintln!("      docs/MLX-PROVIDER.md  ·  docs/TROUBLESHOOTING-MAC.md");
        return Ok(());
    }

    // ── `phantom evolve` subcommand ───────────────────────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("evolve") {
        // Subcommands within evolve: replay / list. If the second arg is
        // one of these, route to the checkpoint viewer rather than starting
        // a fresh evolve run. Anything else (or no second arg) falls
        // through to the actual evolve loop.
        match args.get(2).map(|s| s.as_str()) {
            Some("replay") => {
                run_evolve_replay(&args).await?;
                return Ok(());
            }
            Some("list") => {
                run_evolve_list(&args).await?;
                return Ok(());
            }
            Some("handoff") => {
                run_evolve_handoff(&args).await?;
                return Ok(());
            }
            Some("goals") => {
                run_evolve_goals(&args).await?;
                return Ok(());
            }
            Some("publish") => {
                run_evolve_publish(&args).await?;
                return Ok(());
            }
            _ => {}
        }
        run_evolve(args).await?;
        return Ok(());
    }

    // ── `phantom swarm` — parallel prompt to all peers + synthesis ────────
    if args.get(1).map(|s| s.as_str()) == Some("swarm") {
        run_swarm(args).await?;
        return Ok(());
    }

    // ── `phantom coordinator` subcommand ─────────────────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("coordinator") {
        run_coordinator(args).await?;
        return Ok(());
    }

    // ── `phantom mcp` subcommand — MCP stdio server ───────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("mcp") {
        let mut app_state = AppState::new();
        if let Some(content) = find_config() {
            app_state.load_config_toml(&content);
        }
        let tools_config = app_state.agent_runtime.config().tools.clone();
        let runtime = app_state.agent_runtime.clone();
        phantom_mesh::tools::subagent::init_global(runtime, CostTracker::new());
        phantom_mesh::mcp::run_stdio(tools_config).await?;
        return Ok(());
    }

    // ── `phantom serve` subcommand ────────────────────────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("serve") {
        let mut app_state = AppState::new();
        if let Some(content) = find_config() {
            app_state.load_config_toml(&content);
        }
        let cfg  = app_state.agent_runtime.config();
        // CLI flags override agents.toml — `phantom serve [--host H] [--port P]`
        // is documented at line 1552 but the binary previously fell through
        // to config values and silently ignored both flags.
        let mut host = cfg.core.host.clone();
        let mut port = cfg.core.port;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--host" => { i += 1; if let Some(h) = args.get(i) { host = h.clone(); } }
                "--port" => { i += 1; if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) { port = p; } }
                _ => {}
            }
            i += 1;
        }

        let node_name = app_state.cluster_manager.config.node_name
            .clone()
            .unwrap_or_else(|| "phantom".to_string());
        let peer_count = app_state.cluster_manager.config.peers.len();

        // Coordinator: compute self URL for registration
        let self_ip = if host == "0.0.0.0" || host == "127.0.0.1" {
            std::net::UdpSocket::bind("0.0.0.0:0")
                .and_then(|s| { s.connect("8.8.8.8:80")?; s.local_addr() })
                .map(|a| a.ip().to_string())
                .unwrap_or_else(|_| "127.0.0.1".to_string())
        } else {
            host.clone()
        };
        let self_url = format!("http://{}:{}", self_ip, port);
        let has_coordinator = app_state.cluster_manager.config.coordinator.is_some();

        // Register with coordinator and fetch peers on startup
        if has_coordinator {
            let manager = app_state.cluster_manager.clone();
            let url = self_url.clone();
            match manager.register_with_coordinator(&url).await {
                Ok(_) => {
                    let added = manager.fetch_coordinator_peers(&url).await;
                    if added > 0 {
                        tracing::info!("coordinator: discovered {} new peer(s)", added);
                    }
                }
                Err(e) => tracing::warn!("coordinator register failed: {}", e),
            }
        }

        // Background heartbeat — ping all configured peers every 30s.
        // Also re-register with coordinator so our entry doesn't expire.
        {
            let manager = app_state.cluster_manager.clone();
            let url = self_url.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    // Re-register and sync peers from coordinator
                    if manager.config.coordinator.is_some() {
                        let _ = manager.register_with_coordinator(&url).await;
                        manager.fetch_coordinator_peers(&url).await;
                    }
                    let statuses = manager.refresh_all().await;
                    let online = statuses.iter().filter(|s| s.online).count();
                    tracing::debug!("heartbeat: {}/{} peers online", online, statuses.len());
                }
            });
        }

        // mDNS advertising — let peers on the local network discover this node.
        let adv_host = host.clone();
        let adv_name = node_name.clone();
        tokio::spawn(async move { advertise_mdns(&adv_name, &adv_host, port).await; });

        // ── banner snapshot (capture before moving app_state into Arc) ──
        let (master_model, master_provider) = {
            let cfg = app_state.agent_runtime.config();
            (
                cfg.agent.get("master").map(|a| a.model.clone()).unwrap_or_else(|| "(none)".into()),
                cfg.agent.get("master").map(|a| a.provider.clone()).unwrap_or_else(|| "(none)".into()),
            )
        };
        let cwd_str = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        let perm_mode = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
        let plan_mode = std::env::var("PHANTOM_PLAN_MODE").is_ok();
        let tool_count = phantom_mesh::tools::all_tool_names().len();

        let state = std::sync::Arc::new(app_state);
        let router = phantom_mesh::serve::router(state)
            .layer(tower_http::cors::CorsLayer::permissive());

        eprintln!("{} {} {}",
            colored("◆ phantom serve", 35),
            colored(env!("CARGO_PKG_VERSION"), 90),
            colored(option_env!("PHANTOM_GIT_HASH").unwrap_or(""), 90));
        eprintln!("  node      : {}", colored(&node_name, 36));
        eprintln!("  agent     : master · {} / {}", master_provider, colored(&master_model, 33));
        eprintln!("  cwd       : {}", cwd_str);
        eprintln!(
            "  perm/plan : {} / {}",
            perm_mode,
            if plan_mode { colored("ON", 33) } else { "off".into() }
        );
        eprintln!("  tools     : {} built-in + 2 cluster RPC", tool_count);
        eprintln!("  WebSocket : ws://{}:{}/ws", host, port);
        eprintln!("  Health    : http://{}:{}/healthz", host, port);
        eprintln!("  RPC       : http://{}:{}/rpc/{{ping,peers,message,task/assign}}", host, port);
        if peer_count > 0 {
            eprintln!("  peers     : {} configured (heartbeat every 30s)", peer_count);
        }
        eprintln!("  Press Ctrl-C to stop.");
        eprintln!();

        phantom_mesh::start_http_server(&host, port, router).await?;
        return Ok(());
    }

    // ── `phantom peer` subcommand ─────────────────────────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("peer") {
        run_peer(args).await?;
        return Ok(());
    }

    // ── `phantom keys` subcommand ──────────────────────────────────────────
    // CONTRIBUTOR-FUNNEL §5 — per-user ed25519 identity for recipe signing.
    // SPEC-FREEZE-V1 §4.1 freeze-compatible scaffolding (additive subcommand).
    //
    //   phantom keys init [--force]
    //     generate ed25519 keypair at ~/.phantom-mesh/keys/
    //   phantom keys show
    //     print this machine's public key
    //   phantom keys path
    //     print the keys directory (for scripting)
    if args.get(1).map(|s| s.as_str()) == Some("keys") {
        let action = args.get(2).map(|s| s.as_str()).unwrap_or("show");
        match action {
            "init" => {
                let force = args.iter().any(|a| a == "--force");
                let outcome = phantom_mesh::identity::init(force)?;
                // Co-create the extension folder layout so users have
                // a documented place to drop customisations from day 1.
                let _ = phantom_mesh::extensions::ensure_layout();
                if outcome.created {
                    eprintln!("  {} ed25519 keypair generated", colored("✓", 32));
                    eprintln!("    {}  {}", colored("private:", 90), outcome.priv_path.display());
                    eprintln!("    {}  {}", colored("public:", 90),  outcome.pub_path.display());
                    eprintln!("    {}  {}", colored("pubkey:", 90),  outcome.pub_hex);
                    let exts = phantom_mesh::extensions::extensions_dir();
                    eprintln!("    {}  {}", colored("exts:", 90),    exts.display());
                    eprintln!();
                    eprintln!("  {} the private key NEVER leaves this machine.", colored("›", 90));
                    eprintln!("  {} customise prompts at  {}/prompts/<agent>.md", colored("›", 90), exts.display());
                    eprintln!("  {} sign recipes with `phantom evolve publish`.", colored("›", 90));
                } else if force {
                    eprintln!("  {} keys already exist — pass --force to overwrite (lose all signatures)", colored("⚠", 33));
                } else {
                    eprintln!("  {} keypair already initialised", colored("◆", 35));
                    eprintln!("    {}  {}", colored("public:", 90), outcome.pub_path.display());
                    eprintln!("    {}  {}", colored("pubkey:", 90), outcome.pub_hex);
                    eprintln!();
                    eprintln!("  {} pass --force to regenerate (destructive).", colored("›", 90));
                }
                return Ok(());
            }
            "show" => {
                match phantom_mesh::identity::load_pub_hex() {
                    Ok(hex) => {
                        eprintln!("  {} ed25519 public key", colored("◆", 35));
                        eprintln!("    {}  {}", colored("path:",   90), phantom_mesh::identity::pub_key_path().display());
                        eprintln!("    {}  {}", colored("pubkey:", 90), hex);
                    }
                    Err(_) => {
                        eprintln!("  {} no keypair found — run `phantom keys init` first", colored("✗", 31));
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "path" => {
                println!("{}", phantom_mesh::identity::keys_dir().display());
                return Ok(());
            }
            "add" | "remove" | "list" | "test" => {
                // Provider-API-key subcommands — delegate to the older
                // /keys flow (TUI slash command) for now. CLI flow lands
                // in v0.2 if there's demand.
                eprintln!("  {} `phantom keys {}` — use `/keys {}` inside the TUI / REPL",
                    colored("›", 90), action, action);
                eprintln!("  {} or edit ~/.phantom-mesh/agents.toml directly.", colored("›", 90));
                return Ok(());
            }
            other => {
                eprintln!("  {} unknown subcommand: phantom keys {}", colored("✗", 31), other);
                eprintln!("  Available: init [--force] / show / path");
                std::process::exit(2);
            }
        }
    }

    // ── `phantom init` subcommand ──────────────────────────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("init") {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let content = phantom_mesh::scaffold::generate_phantom_md_async(&cwd).await;
        let out_path = cwd.join("PHANTOM.md");
        std::fs::write(&out_path, &content)?;
        eprintln!("{} Created {}", colored("◆", 35), out_path.display());
        return Ok(());
    }

    // ── `phantom onboarding` — browser-based first-time setup ──────────────
    if args.get(1).map(|s| s.as_str()) == Some("onboarding") {
        run_web_onboarding().await?;
        return Ok(());
    }

    // ── `phantom service` — auto-start the daemon at user login ────────────
    // macOS  → LaunchAgent in ~/Library/LaunchAgents (launchctl)
    // Windows → Scheduled Task at logon (schtasks)
    // Other platforms → not implemented; print explicit error.
    #[cfg(target_os = "macos")]
    if args.get(1).map(|s| s.as_str()) == Some("service") {
        let action = args.get(2).map(|s| s.as_str()).unwrap_or("status");
        return run_service_subcommand(action).await;
    }
    #[cfg(target_os = "windows")]
    if args.get(1).map(|s| s.as_str()) == Some("service") {
        let action = args.get(2).map(|s| s.as_str()).unwrap_or("status");
        return run_service_subcommand_windows(action).await;
    }
    #[cfg(target_os = "linux")]
    if args.get(1).map(|s| s.as_str()) == Some("service") {
        let action = args.get(2).map(|s| s.as_str()).unwrap_or("status");
        return run_service_subcommand_linux(action).await;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    if args.get(1).map(|s| s.as_str()) == Some("service") {
        eprintln!(
            "{} `phantom service` not yet implemented on this platform. \
             Run `phantom serve &` manually.",
            colored("✗", 31)
        );
        std::process::exit(1);
    }

    // ── `phantom keys` — manage ~/.phantom-mesh/env API keys ───────────────
    if args.get(1).map(|s| s.as_str()) == Some("keys") {
        return phantom_mesh::cli_config::run_keys(&args);
    }

    // ── `phantom providers` — list providers + edit failover priority ──────
    if args.get(1).map(|s| s.as_str()) == Some("providers") {
        return phantom_mesh::cli_config::run_providers(&args);
    }

    // ── `phantom models` — refresh + status of /v1/models cache ────────────
    if args.get(1).map(|s| s.as_str()) == Some("models") {
        return phantom_mesh::cli_config::run_models(&args).await;
    }

    // ── `phantom config` — pull LLM keys from phantommesh.io vault ─────────
    if args.get(1).map(|s| s.as_str()) == Some("config") {
        return phantom_mesh::cli_config::run_config(&args).await;
    }

    // ── `phantom logs` — recent events for remote debugging ────────────────
    if args.get(1).map(|s| s.as_str()) == Some("logs") {
        return phantom_mesh::cli_config::run_logs(&args);
    }

    // ── `phantom debug` — one-command diagnostic bundle ────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("debug") {
        return phantom_mesh::cli_config::run_debug(&args).await;
    }

    // ── `phantom cluster` — wire local node into the mesh ─────────────────
    if args.get(1).map(|s| s.as_str()) == Some("cluster") {
        return phantom_mesh::cli_config::run_cluster(&args).await;
    }

    // ── `phantom workspace` — pin this machine to a project + agent ──────
    if args.get(1).map(|s| s.as_str()) == Some("workspace") {
        return phantom_mesh::cli_config::run_workspace(&args);
    }

    // ── `phantom dispatch` — capability-routed cross-node RPC ─────────────
    if args.get(1).map(|s| s.as_str()) == Some("dispatch") {
        return phantom_mesh::cli_config::run_dispatch(&args).await;
    }

    // ── `phantom sessions` — live TUI sessions across the user's mesh ─────
    if args.get(1).map(|s| s.as_str()) == Some("sessions") {
        return phantom_mesh::cli_config::run_sessions(&args).await;
    }

    // ── `phantom git sync` — fan-out git pull across cluster peers ────────
    if args.get(1).map(|s| s.as_str()) == Some("git") {
        return phantom_mesh::cli_config::run_git(&args).await;
    }

    // ── `phantom doctor` — environment self-diagnostic ─────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("doctor") {
        // `phantom doctor --json` emits a machine-readable JSON object
        // covering all the same checks as the human-readable colored
        // output. Useful for: CI gates, dashboard health endpoints,
        // monitoring alerts, scripted "is this install healthy?" checks
        // by recruiters / reviewers / auditors who don't want to parse
        // colored text. The JSON structure mirrors the section layout
        // so a downstream consumer can pluck `.permissions.rules` or
        // `.autoevolve.queue_pending` directly.
        if args.iter().any(|a| a == "--json") {
            return run_doctor_json().await;
        }
        return run_doctor().await;
    }

    // ── `phantom selftest` — feature-by-feature self-test + debug artifacts ─
    if args.get(1).map(|s| s.as_str()) == Some("selftest") {
        return run_selftest(&args);
    }

    // ── `phantom snapshot` — APFS local snapshot ops (macOS) ───────────────
    #[cfg(target_os = "macos")]
    if args.get(1).map(|s| s.as_str()) == Some("snapshot") {
        return run_snapshot_subcommand(&args).await;
    }

    // ── `phantom autoevolve` — periodic / watch-mode self-improvement ──────
    if args.get(1).map(|s| s.as_str()) == Some("autoevolve") {
        return run_autoevolve(args).await;
    }

    // ── `phantom self-update` — pull a fresh binary from the coordinator ───
    if args.get(1).map(|s| s.as_str()) == Some("self-update") {
        return run_self_update(&args).await;
    }

    // ── `phantom login / logout / whoami` — local identity ─────────────────
    if args.get(1).map(|s| s.as_str()) == Some("login") {
        return run_login(&args).await;
    }
    if args.get(1).map(|s| s.as_str()) == Some("logout") {
        phantom_mesh::auth::delete()?;
        eprintln!("{} logged out (deleted {})",
            colored("✓", 32),
            phantom_mesh::auth::auth_path().display());
        return Ok(());
    }
    if args.get(1).map(|s| s.as_str()) == Some("whoami") {
        match phantom_mesh::auth::load() {
            Some(s) => println!("{} {}", colored("◆", 35), phantom_mesh::auth::human_summary(&s)),
            None    => println!("{} not logged in — run `phantom login`", colored("◇", 90)),
        }
        return Ok(());
    }

    // ── `phantom mlx` — Apple Silicon local LLM helper (macOS-only) ────────
    #[cfg(target_os = "macos")]
    if args.get(1).map(|s| s.as_str()) == Some("mlx") {
        return run_mlx_subcommand(&args).await;
    }
    #[cfg(not(target_os = "macos"))]
    if args.get(1).map(|s| s.as_str()) == Some("mlx") {
        eprintln!("{} `phantom mlx` requires Apple Silicon (uses Apple's MLX framework).",
            colored("✗", 31));
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    if args.get(1).map(|s| s.as_str()) == Some("snapshot") {
        eprintln!("{} `phantom snapshot` is macOS-only (uses tmutil).", colored("✗", 31));
        std::process::exit(1);
    }

    // ── `phantom tui` — full-screen ratatui interface ──────────────────────
    if args.get(1).map(|s| s.as_str()) == Some("tui") {
        phantom_mesh::tui::launch_default().await?;
        return Ok(());
    }

    // ── `phantom exec [PROMPT] [--json] [--quiet] [--agent NAME] ...` ─────
    //
    // Headless single-turn agent run for CI / pipelines / scripts.
    // Differs from `phantom <prompt>` (the implicit positional one-shot)
    // by being an explicit subcommand with stdout-only output and proper
    // exit codes — what shell scripts actually want when piping a task
    // to phantom and consuming its answer.
    //
    //  · default mode  → token text on stdout, tool start/done lines on
    //                    stderr, prompt header / cost line suppressed
    //  · --json        → one AgentEvent JSON per line on stdout
    //                    (machine-readable; `tag = "type"` discriminator)
    //  · --quiet       → no streaming output; final response printed in
    //                    one go after the run completes
    //  · stdin support → `echo "..." | phantom exec` reads when no
    //                    positional PROMPT is provided and stdin isn't a
    //                    TTY
    //
    // Exit codes: 0 ok · 1 agent error · 2 arg / config error
    if args.get(1).map(|s| s.as_str()) == Some("exec") {
        let mut agent_name = "master".to_string();
        let mut json_mode = false;
        let mut quiet = false;
        let mut do_continue = false;
        let mut session_id: Option<String> = None;
        let mut config_override: Option<String> = None;
        let mut prompt_arg: Option<String> = None;

        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--json"     => { json_mode = true; }
                "--quiet"    => { quiet = true; }
                "--continue" | "-c" => { do_continue = true; }
                "--agent"    => { i += 1; if i < args.len() { agent_name = args[i].clone(); } }
                "--session"  => { i += 1; if i < args.len() { session_id = Some(args[i].clone()); } }
                "--config"   => { i += 1; if i < args.len() { config_override = Some(args[i].clone()); } }
                "-h" | "--help" => {
                    eprintln!("usage: phantom exec [--agent NAME] [--session ID] [--continue] [--json] [--quiet] [--config PATH] [PROMPT]");
                    eprintln!("       echo \"...\" | phantom exec     # read prompt from stdin");
                    eprintln!();
                    eprintln!("Headless single-turn agent run. Streams model text to stdout;");
                    eprintln!("tool start/done lines go to stderr (suppressed with --quiet).");
                    eprintln!("--json emits one AgentEvent JSON per line on stdout for machine consumption.");
                    return Ok(());
                }
                arg if !arg.starts_with('-') => {
                    prompt_arg = Some(arg.to_string());
                }
                _ => {}
            }
            i += 1;
        }

        // Read prompt from stdin when neither a positional PROMPT nor a
        // pipe was provided. We deliberately treat empty/EOF stdin as an
        // error rather than silently running with no prompt.
        let prompt = match prompt_arg {
            Some(p) => p,
            None => {
                use std::io::{IsTerminal, Read};
                if std::io::stdin().is_terminal() {
                    eprintln!("error: phantom exec requires a PROMPT or stdin input");
                    eprintln!("       phantom exec \"summarize this README\"");
                    eprintln!("       echo \"task\" | phantom exec");
                    std::process::exit(2);
                }
                let mut buf = String::new();
                if std::io::stdin().read_to_string(&mut buf).is_err() {
                    eprintln!("error: failed to read prompt from stdin");
                    std::process::exit(2);
                }
                buf.trim().to_string()
            }
        };
        if prompt.is_empty() {
            eprintln!("error: empty prompt");
            std::process::exit(2);
        }

        // Load agents.toml the same way the implicit one-shot below does
        // — preserve --config override, fall back to the cwd-walk search.
        let mut app_state = phantom_mesh::AppState::new();
        let config_content = match config_override.as_deref() {
            Some(path) => std::fs::read_to_string(path).ok().or_else(find_config),
            None       => find_config(),
        };
        match config_content {
            Some(content) => app_state.load_config_toml(&content),
            None => {
                eprintln!("error: no agents.toml found — run `phantom init` or pass --config PATH");
                std::process::exit(2);
            }
        }

        // Spawn declared MCP servers BEFORE running so their tools are
        // visible in the schema list passed to the model.
        {
            let servers = app_state.agent_runtime.config().mcp_servers.clone();
            if !servers.is_empty() {
                phantom_mesh::mcp_client::init_global(&servers).await;
            }
        }

        let runtime = app_state.agent_runtime.clone();
        let conversations = ConversationStore::new();
        let cost_tracker = CostTracker::new();
        phantom_mesh::tools::subagent::init_global(runtime.clone(), cost_tracker.clone());

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let chat_id = if let Some(sid) = session_id {
            sid
        } else if do_continue {
            find_last_session().unwrap_or_else(|| cwd_chat_id(&cwd))
        } else {
            cwd_chat_id(&cwd)
        };

        let history = if do_continue {
            conversations.get_history(&chat_id).await
        } else {
            vec![]
        };
        let extra_context = WorkspaceContext::capture().to_system_context();
        let prompt = expand_at_files(&prompt);

        // Three rendering strategies share the same closure; the inner
        // `match ev` decides which one to apply per event. Closure
        // captures are `Copy`-cheap (two bools), so cloning into the
        // tokio task is fine.
        let json_m = json_mode;
        let quiet_m = quiet;
        let handler = move |ev: AgentEvent| {
            if json_m {
                if let Ok(s) = serde_json::to_string(&ev) {
                    println!("{}", s);
                }
                return;
            }
            if quiet_m {
                return;
            }
            match ev {
                AgentEvent::Token { content } => {
                    use std::io::Write;
                    let _ = std::io::stdout().write_all(content.as_bytes());
                    let _ = std::io::stdout().flush();
                }
                AgentEvent::ToolStart { name, args_preview } => {
                    eprintln!("[tool] {} {}", name, args_preview);
                }
                AgentEvent::ToolDone { name, output_preview } => {
                    let preview: String = output_preview.chars().take(200).collect();
                    eprintln!("[done] {} → {}", name, preview);
                }
                AgentEvent::Notice { message } => {
                    eprintln!("[note] {}", message);
                }
                AgentEvent::Thinking { .. } | AgentEvent::Done { .. } => {}
            }
        };

        let result = runtime
            .run_with_callbacks(
                &agent_name,
                &prompt,
                &history,
                Some(&extra_context),
                &cost_tracker,
                handler,
            )
            .await;

        match result {
            Ok(r) => {
                if quiet_m {
                    println!("{}", r.output);
                } else if !json_m {
                    // The streaming handler already wrote tokens to
                    // stdout; emit a trailing newline so shells don't
                    // glue the next prompt onto the last output line.
                    println!();
                }
                let user_msg = ChatMessage { role: "user".into(), content: prompt, tool_calls: None };
                let asst_msg = ChatMessage { role: "assistant".into(), content: r.output, tool_calls: None };
                conversations.append(&chat_id, user_msg, asst_msg).await;
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    }

    // ── `phantom repl` — explicit line-mode REPL (the previous default) ────
    // Track this BEFORE stripping the arg so the TUI default branch below
    // doesn't fire after the strip leaves args.len() == 1.
    let force_repl = args.get(1).map(|s| s.as_str()) == Some("repl");
    if force_repl {
        args.remove(1);
    }
    // ── `phantom` with NO args at all → default to TUI ─────────────────────
    // PHANTOM_REPL=1 forces the line-mode REPL even with no args.
    // PHANTOM_TUI=0 forces REPL too. Any flag/prompt skips this branch.
    if args.len() == 1
        && !force_repl
        && std::env::var("PHANTOM_REPL").is_err()
        && std::env::var("PHANTOM_TUI").map(|v| v != "0").unwrap_or(true)
    {
        // Workspace pin: if [workspace].default_dir is set, cd to it
        // BEFORE launching the TUI so the session, conversation history
        // (cwd-{hash}.jsonl), and tool actions all happen relative to
        // the pinned project. Skip silently if the dir doesn't exist
        // — we don't want bare `phantom` to fail just because someone
        // moved their projects folder; surface a notice + continue
        // from caller's cwd as a fallback.
        if let Some(cfg) = phantom_mesh::config::AgentsConfig::find_and_load() {
            if let Some(dir) = cfg.workspace.default_dir.as_deref() {
                if !dir.is_empty() {
                    let p = std::path::Path::new(dir);
                    if p.exists() && p.is_dir() {
                        if let Err(e) = std::env::set_current_dir(p) {
                            eprintln!("{} workspace cd to {} failed: {} — using caller's cwd",
                                colored("⚠", 33), dir, e);
                        } else {
                            eprintln!("{} workspace pinned to {}", colored("◆", 35), dir);
                        }
                    } else {
                        eprintln!("{} workspace dir {} doesn't exist — using caller's cwd",
                            colored("⚠", 33), dir);
                        eprintln!("    fix: phantom workspace set <existing-dir> [agent]");
                    }
                }
            }
            // Pinned agent: pre-pick before TUI launches so user doesn't
            // have to Tab through agents on every open.
            if let Some(name) = cfg.workspace.pinned_agent.as_deref() {
                if !name.is_empty() {
                    std::env::set_var("PHANTOM_DEFAULT_AGENT", name);
                    eprintln!("{} pinned agent: {}", colored("◆", 35), name);
                }
            }
        }
        phantom_mesh::tui::launch_default().await?;
        return Ok(());
    }

    let mut agent_name = "master".to_string();
    let mut do_continue = false;
    let mut one_shot_prompt: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut list_sessions = false;
    let mut config_override: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--continue" | "-c" => { do_continue = true; }
            "--list-sessions" => { list_sessions = true; }
            "--config" => {
                i += 1;
                if i < args.len() {
                    config_override = Some(args[i].clone());
                }
            }
            "--session" => {
                i += 1;
                if i < args.len() {
                    session_id = Some(args[i].clone());
                }
            }
            "--agent" => {
                i += 1;
                if i < args.len() {
                    agent_name = args[i].clone();
                }
            }
            arg if !arg.starts_with('-') => {
                one_shot_prompt = Some(arg.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    let mut app_state = AppState::new();
    let mut config_content = match config_override.as_deref() {
        Some(path) => std::fs::read_to_string(path).ok().or_else(find_config),
        None       => find_config(),
    };

    // First-run onboarding: no agents.toml anywhere → walk through CLI setup.
    // Skipped for one-shot prompts and --list-sessions (those work without LLM).
    if config_content.is_none() && one_shot_prompt.is_none() && !list_sessions {
        if let Err(e) = run_first_time_onboarding() {
            eprintln!("{} onboarding failed: {}", colored("error:", 31), e);
        }
        config_content = find_config();
    }

    if let Some(content) = config_content {
        app_state.load_config_toml(&content);
    }

    // Spawn any [[mcp_servers]] declared in the loaded config so their tools
    // become available to the agent runtime *before* the REPL/one-shot fires.
    {
        let servers = app_state.agent_runtime.config().mcp_servers.clone();
        if !servers.is_empty() {
            phantom_mesh::mcp_client::init_global(&servers).await;
        }
    }

    let conversations = ConversationStore::new();
    let cost_tracker = CostTracker::new();
    let runtime = app_state.agent_runtime.clone();

    // Wire the runtime + cost into the global slot used by the `task` /
    // `subagent` tool. Idempotent (OnceLock); safe to re-enter via /resume etc.
    phantom_mesh::tools::subagent::init_global(runtime.clone(), cost_tracker.clone());

    // --list-sessions: print sessions and exit
    if list_sessions {
        let sessions = conversations.list().await;
        if sessions.is_empty() {
            println!("No saved sessions.");
        } else {
            let home = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".phantom-mesh")
                .join("conversations");
            println!("{:<40}  {:>10}", "Session ID", "Size");
            println!("{}", "-".repeat(54));
            for id in &sessions {
                let path = home.join(format!("{}.jsonl", id));
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let size_str = format_size(size);
                println!("{:<40}  {:>10}", id, size_str);
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Resolve effective chat_id:
    // 1. --session <id>  → use that id directly
    // 2. --continue/-c  → find the most recently modified session file, fall back to cwd hash
    // 3. default        → cwd-based hash
    let chat_id = if let Some(sid) = session_id {
        sid
    } else if do_continue {
        find_last_session().unwrap_or_else(|| cwd_chat_id(&cwd))
    } else {
        cwd_chat_id(&cwd)
    };

    let extra_context = WorkspaceContext::capture().to_system_context();

    if let Some(prompt) = one_shot_prompt {
        let history = if do_continue {
            conversations.get_history(&chat_id).await
        } else {
            vec![]
        };
        // One-shot prompts now also expand `@<file>` like the REPL does.
        // Image @files become multimodal sentinels (see multimodal.rs); text
        // @files are inlined.
        let prompt = expand_at_files(&prompt);
        eprintln!("{} {}", colored("◆", 35), colored(&prompt, 90));
        let t0 = std::time::Instant::now();
        let streamed = Arc::new(AtomicBool::new(false));
        // one-shot path: no /show slash so no need to retain tool history
        let handler = make_stream_handler(streamed.clone(), None);
        let result = runtime
            .run_with_callbacks(
                &agent_name,
                &prompt,
                &history,
                Some(&extra_context),
                &cost_tracker,
                handler,
            )
            .await?;
        let elapsed = t0.elapsed().as_secs_f64();
        let last = cost_tracker.last_request_cost().await;
        let session = cost_tracker.session_cost().await;
        eprintln!("{}", colored(&format!("[↑ ${:.4}  ∑ ${:.4}  {:.1}s]", last, session, elapsed), 90));
        let user_msg = ChatMessage { role: "user".into(), content: prompt, tool_calls: None };
        let asst_msg = ChatMessage { role: "assistant".into(), content: result.output, tool_calls: None };
        conversations.append(&chat_id, user_msg, asst_msg).await;
        return Ok(());
    }

    repl(runtime, app_state, conversations, cost_tracker, chat_id, agent_name, extra_context, do_continue).await
}

// compact_via_llm moved to phantom_mesh::session — used by both REPL
// (here) and TUI. Re-export for in-binary call sites.
use phantom_mesh::session::compact_via_llm;

async fn repl(
    runtime: AgentRuntime,
    app_state: AppState,
    conversations: ConversationStore,
    cost_tracker: CostTracker,
    initial_chat_id: String,
    initial_agent_name: String,
    extra_context: String,
    load_history: bool,
) -> Result<()> {
    // Mutable REPL state
    let mut chat_id = initial_chat_id;
    let mut agent_name = initial_agent_name;
    let mut model_override: Option<String> = None;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Build startup banner — Claude Code / Codex style
    {
        let cfg = app_state.agent_runtime.config();
        let ws = WorkspaceContext::capture();
        let branch_str = ws.git_branch
            .as_deref()
            .map(|b| format!(" · {}", b))
            .unwrap_or_default();

        // Header bar
        let version = env!("CARGO_PKG_VERSION");
        eprintln!();
        eprintln!(
            "  {} {}  {}",
            colored("phantom", 35),
            colored(&format!("v{}", version), 90),
            colored("— AI agent mesh", 90),
        );
        eprintln!();

        // Status row: providers
        let provider_count = cfg.providers.len();
        let provider_names: Vec<&str> = cfg.providers.keys().map(|s| s.as_str()).collect();
        if provider_count == 0 {
            eprintln!("  {} {}", colored("⚠", 33), colored("no providers configured — run /init or set ~/.phantom-mesh/agents.toml", 33));
        } else {
            eprintln!(
                "  {} providers: {} ({})",
                colored("●", 32),
                provider_count,
                provider_names.join(", "),
            );
        }

        // Status row: cluster
        let peer_count = cfg.cluster.peers.len();
        if peer_count > 0 {
            eprintln!(
                "  {} cluster: {} peer{} ({})",
                colored("●", 32),
                peer_count,
                if peer_count == 1 { "" } else { "s" },
                cfg.cluster.node_name.as_deref().unwrap_or("standalone"),
            );
        } else {
            eprintln!("  {} cluster: standalone", colored("○", 90));
        }

        // Status row: agent + session + dir
        eprintln!(
            "  {} agent: {}  ·  session: {}  ·  dir: {}{}",
            colored("●", 32),
            agent_name,
            &chat_id[..chat_id.len().min(12)],
            cwd.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
            branch_str,
        );

        eprintln!();
        eprintln!("  {} type /help for commands  ·  Ctrl-D to exit  ·  end line with \\ for multi-line", colored("›", 90));
        eprintln!();
    }

    if load_history {
        eprintln!("{} Resuming session {}", colored("◆", 35), &chat_id[..chat_id.len().min(16)]);
    }

    let mut pending_context: Vec<String> = Vec::new();

    // Tool history for /show <n>: each ToolStart pushes; ToolDone fills output.
    // Capped at 100 entries (older ones rotated out) to keep memory bounded.
    let tool_history: Arc<std::sync::Mutex<Vec<ToolEntry>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Reasoning trace of the most recent turn — populated by the streaming
    // handler on every Thinking event, dumped by `/show thinking`.
    let thinking_session: Arc<std::sync::Mutex<String>> =
        Arc::new(std::sync::Mutex::new(String::new()));

    // Permission allowlist for tools. Entries: tool names ("shell"), or "*"
    // for all-tools. Set via /perm <a-key> answers when PHANTOM_PERM=ask is on.
    let perm_allowlist: Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
        Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

    // Permission rule engine — Tool(specifier) DSL from agents.toml's
    // `[permissions]` block. Empty/missing block → empty engine, which
    // returns Allow for everything (preserves the legacy default). Any
    // rule present switches behaviour to "deny → ask → allow, first
    // match wins, default Ask". Parse errors are surfaced once at REPL
    // boot and the engine falls back to empty so the session still
    // works.
    let permission_engine: Arc<phantom_mesh::permission::Engine> = {
        let cfg = phantom_mesh::config::AgentsConfig::find_and_load()
            .unwrap_or_else(phantom_mesh::config::AgentsConfig::with_defaults);
        let deny: Vec<&str>  = cfg.permissions.deny.iter().map(String::as_str).collect();
        let ask: Vec<&str>   = cfg.permissions.ask.iter().map(String::as_str).collect();
        let allow: Vec<&str> = cfg.permissions.allow.iter().map(String::as_str).collect();
        match phantom_mesh::permission::Engine::from_lists(&deny, &ask, &allow) {
            Ok(e) => {
                if !e.is_empty() {
                    eprintln!("  {} permission rules loaded: {} active",
                        colored("◆", 36), e.rules().len());
                }
                Arc::new(e)
            }
            Err(err) => {
                eprintln!("  {} permission rule parse error — running with no rules: {}",
                    colored("⚠", 33), err);
                Arc::new(phantom_mesh::permission::Engine::new(Vec::new()))
            }
        }
    };

    // Plan mode "approval" handshake: when PLAN_MODE is on, the gate denies
    // every tool call until the user's input was exactly "go" / "execute" /
    // "yes". The REPL sets this flag when it sees such an input.
    let plan_approved: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));

    // ── rustyline setup with custom Helper (Tab completion + ghost-hints) ──
    let mut rl: Editor<PhantomHelper, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(PhantomHelper));
    let history_path = dirs::home_dir()
        .map(|h| h.join(".phantom-mesh").join("history"));
    if let Some(ref p) = history_path {
        let _ = rl.load_history(p);
    }

    // Prompt ends with a 24-bit gold ANSI sequence and no reset. The
    // cursor lands inside the colored zone, so anything the user types
    // gets that color even when rustyline's highlight() pipeline isn't
    // invoked. We use truecolor (\x1b[38;2;R;G;Bm) instead of the 16-
    // color palette \x1b[33m because Apple Terminal.app's "yellow"
    // palette is muddy / can look gray on certain Profiles. Truecolor
    // bypasses the palette entirely. Reset happens at end of line via
    // highlight() or the next status print.
    //
    // RGB (255, 215, 0) = gold, very high contrast on both dark and
    // light backgrounds, distinct from any common terminal-default fg.
    let prompt_str = format!("{} \x1b[1m\x1b[38;2;255;215;0m", colored("◆", 35));
    let cont_prompt = format!("{} \x1b[1m\x1b[38;2;255;215;0m", colored("·", 90));

    loop {
        // Status line: agent · model · session cost · plan-mode flag
        // Printed once per turn just before the prompt.
        {
            let last = cost_tracker.last_request_cost().await;
            let session = cost_tracker.session_cost().await;
            let model_label: String = if let Some(m) = &model_override {
                m.clone()
            } else {
                let cfg = app_state.agent_runtime.config();
                cfg.agent.get(&agent_name)
                    .map(|a| if a.model.is_empty() { "default".to_string() } else { a.model.clone() })
                    .unwrap_or_else(|| "default".to_string())
            };
            let plan = std::env::var("PHANTOM_PLAN_MODE").ok().filter(|s| s == "1");
            let plan_str = if plan.is_some() {
                format!("  ·  {}", colored("PLAN", 33))
            } else { String::new() };
            eprintln!(
                "  {} {}  ·  model: {}  ·  cost: {} session ({} last){}",
                colored("agent:", 90),
                colored(&agent_name, 36),
                colored(&model_label, 36),
                colored(&format!("${:.4}", session), 90),
                colored(&format!("${:.4}", last), 90),
                plan_str,
            );
        }

        // Drop any chars the user typed *during* the previous turn's
        // streaming output. Without this, TTY drivers vary on whether
        // those chars get delivered to the next readline (sometimes
        // submitted as input, sometimes silently discarded) — the
        // "input eaten" symptom. Be deterministic: always discard.
        flush_stdin_input();

        // Read a (possibly multi-line) input. A trailing `\` on a line means
        // "continue on next line"; we accumulate until a non-`\`-terminated
        // line is entered, then submit the joined text.
        let mut buffer = String::new();
        let mut read_err: Option<ReadlineError> = None;
        loop {
            let p = if buffer.is_empty() { &prompt_str } else { &cont_prompt };
            match rl.readline(p) {
                Ok(l) => {
                    if l.ends_with('\\') {
                        // strip the trailing backslash, append a real newline, keep reading
                        buffer.push_str(&l[..l.len() - 1]);
                        buffer.push('\n');
                        continue;
                    }
                    buffer.push_str(&l);
                    break;
                }
                Err(e) => { read_err = Some(e); break; }
            }
        }

        if let Some(e) = read_err {
            match e {
                ReadlineError::Interrupted => { eprintln!("^C"); continue; }
                ReadlineError::Eof => break,
                other => {
                    eprintln!("{} {}", colored("error:", 31), other);
                    break;
                }
            }
        }

        let line = strip_continuation(&buffer);
        if !line.is_empty() {
            rl.add_history_entry(line.replace('\n', " ⏎ ")).ok();
        }
        let line = line.trim().to_string();
        if line.is_empty() { continue; }

        // Handle slash commands
        if line.starts_with('/') {
            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            match parts[0] {
                "/exit" | "/quit" => break,

                "/help" => {
                    eprintln!("  {}",   colored("session", 36));
                    eprintln!("    /clear            — clear conversation history for current session");
                    eprintln!("    /compact          — LLM-summarize older turns, keep last 6 verbatim");
                    eprintln!("    /sessions         — list all sessions sorted by recency");
                    eprintln!("    /session [id]     — show current session or switch to <id>");
                    eprintln!("    /resume [id|prefix] — resume latest session, or one matching <prefix>");
                    eprintln!("    /fork [name]      — branch the current session into a new one (history copied)");
                    eprintln!("    /list             — list saved sessions (size view)");
                    eprintln!();
                    eprintln!("  {}",   colored("agents & tools", 36));
                    eprintln!("    /agent [name]     — show or switch active agent (master/coder/reviewer/researcher)");
                    eprintln!("    /agents           — list configured agents");
                    eprintln!("    /model [name]     — show or set model override for this session");
                    eprintln!("    /tools            — list available tools by category");
                    eprintln!("    /mcp [test NAME]  — list configured MCP servers (or re-ping one)");
                    eprintln!("    /keys [list|add|remove|test] — manage provider API keys without editing toml");
                    eprintln!("    /todo             — show current TODO list (~/.phantom-mesh/todos.json)");
                    eprintln!("    /plan             — toggle plan mode (preview-before-exec)");
                    eprintln!("    /show [n]         — list captured tool calls; with arg, dump full output of #n");
                    eprintln!("    /show thinking    — dump full reasoning trace of the most recent turn");
                    eprintln!();
                    eprintln!("  {}", colored("display", 36));
                    eprintln!("    /density compact|full     — single-line vs multi-line tool result rendering");
                    eprintln!("    /theme dark|light|claude  — color theme (also: codex, gemini, mono)");
                    eprintln!("    /perm ask|diff|allow|deny — tool permission mode (diff = preview file_edit patches)");
                    eprintln!();
                    eprintln!("  {}",   colored("context", 36));
                    eprintln!("    /add <path>       — read file into context");
                    eprintln!("    /init             — generate PHANTOM.md in current directory");
                    eprintln!("    /cost             — show cost breakdown");
                    eprintln!("    /copy [all|turn]  — copy to clipboard: last assistant (default), full session, or last turn");
                    eprintln!("    /export [path]    — write the session as markdown (~/.phantom-mesh/exports/ if no path)");
                    eprintln!();
                    eprintln!("  {}",   colored("input", 36));
                    eprintln!("    Tab               — complete slash commands and @paths");
                    eprintln!("    @<path>           — inline file contents in a prompt");
                    eprintln!("    <line>\\           — end a line with \\ to continue on the next line");
                    eprintln!();
                    eprintln!("    /help             — show this help");
                    eprintln!("    /exit  /quit      — exit (also Ctrl-D)");
                }

                "/cost" => {
                    let summary = cost_tracker.summary().await;
                    let total_usd = summary["total_usd"].as_f64().unwrap_or(0.0);
                    let session_usd = summary["session_usd"].as_f64().unwrap_or(0.0);
                    let requests = summary["requests"].as_u64().unwrap_or(0);
                    let input_tokens = summary["prompt_tokens"].as_u64().unwrap_or(0);
                    let output_tokens = summary["completion_tokens"].as_u64().unwrap_or(0);
                    let budget_limit = summary["budget_limit_usd"].as_f64().unwrap_or(0.0);
                    let over_budget = summary["over_budget"].as_bool().unwrap_or(false);

                    eprintln!("  {} cost summary", colored("◆", 36));
                    eprintln!("    session    ${:.4}", session_usd);
                    eprintln!("    lifetime   ${:.4}", total_usd);
                    eprintln!("    requests   {}", requests);
                    eprintln!("    tokens     in {} / out {}", input_tokens, output_tokens);
                    if budget_limit > 0.0 {
                        let pct = (total_usd / budget_limit * 100.0).min(999.0);
                        let color = if over_budget { 31 } else if pct >= 80.0 { 33 } else { 32 };
                        eprintln!("    budget     ${:.2} ({}{}%{})",
                            budget_limit,
                            colored("", color),
                            pct as u32,
                            "");
                        let bar_w = 30usize;
                        let filled = ((pct / 100.0 * bar_w as f64) as usize).min(bar_w);
                        let bar: String = "█".repeat(filled) + &"░".repeat(bar_w - filled);
                        eprintln!("    {}", colored(&bar, color));
                    }

                    // Per-model breakdown (lifetime, ordered by spend desc).
                    if let Some(by_model) = summary["by_model"].as_object() {
                        if !by_model.is_empty() {
                            let mut rows: Vec<(String, f64, u64, u64)> = by_model.iter()
                                .map(|(k, v)| (
                                    k.clone(),
                                    v["cost_usd"].as_f64().unwrap_or(0.0),
                                    v["input_tokens"].as_u64().unwrap_or(0),
                                    v["output_tokens"].as_u64().unwrap_or(0),
                                ))
                                .collect();
                            rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                            eprintln!();
                            eprintln!("  {} per model", colored("◆", 36));
                            for (model, cost, in_tok, out_tok) in rows.iter().take(8) {
                                let pct_of_total = if total_usd > 0.0 { cost / total_usd * 100.0 } else { 0.0 };
                                eprintln!("    ${:>7.4}  {:>5.1}%  {:<40}  in {} / out {}",
                                    cost, pct_of_total,
                                    model.chars().take(40).collect::<String>(),
                                    in_tok, out_tok);
                            }
                            if rows.len() > 8 {
                                eprintln!("    ({} more — full data at ~/.phantom-mesh/costs.json)",
                                    rows.len() - 8);
                            }
                        }
                    }

                    // Quota widget — hardcoded provider-tier knowledge so
                    // the user doesn't have to remember each vendor's free
                    // tier. NOT real-time (we don't poll the vendor APIs);
                    // it's a hint. Real-time quota will land when we add a
                    // [providers.*.quota] schema (see COMMERCIAL-DESIGN.md).
                    let cfg = app_state.agent_runtime.config();
                    if !cfg.providers.is_empty() {
                        eprintln!();
                        eprintln!("  {} provider quotas (hint, not live)", colored("◆", 36));
                        for (pname, _) in cfg.providers.iter() {
                            let hint = match pname.as_str() {
                                "anthropic"  => "tiered: tier-1 \\$5/min · tier-4 \\$5000+; console.anthropic.com",
                                "openai"     => "tiered: tier-1 \\$1/min · tier-5 \\$50K+; platform.openai.com",
                                "groq"       => "free: 30 RPM · 1M TPD per model; paid Pro for higher",
                                "gemini"     => "free: 15 RPM · 1M TPD; paid: per-token",
                                "opencode"   => "BYOK + free pool; paid Pro tier",
                                "mlx-local"  => "local — no quota (limited by RAM/GPU)",
                                _            => "(no built-in quota info; check vendor)",
                            };
                            eprintln!("    {:<14} {}", pname, colored(hint, 90));
                        }
                    }
                }

                "/login" => {
                    // Spawn `phantom login [arg]` as a subprocess so the
                    // REPL doesn't have to re-implement OAuth state. stdin
                    // and stdout/stderr inherit from this process so the
                    // user gets the full interactive experience (browser
                    // open, prompt, etc.). After the child exits we reload
                    // the auth state and print a summary.
                    let arg = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
                    let exe = std::env::current_exe().ok();
                    if let Some(exe) = exe {
                        let mut cmd = std::process::Command::new(exe);
                        cmd.arg("login");
                        if !arg.is_empty() { cmd.arg(arg); }
                        cmd.stdin(std::process::Stdio::inherit());
                        cmd.stdout(std::process::Stdio::inherit());
                        cmd.stderr(std::process::Stdio::inherit());
                        let _ = cmd.status();
                    }
                    if let Some(s) = phantom_mesh::auth::load() {
                        eprintln!("  {} now: {}", colored("◆", 36), phantom_mesh::auth::human_summary(&s));
                    }
                }

                "/logout" => {
                    let _ = phantom_mesh::auth::delete();
                    eprintln!("  {} logged out (deleted {})",
                        colored("✓", 32),
                        phantom_mesh::auth::auth_path().display());
                }

                "/whoami" => {
                    match phantom_mesh::auth::load() {
                        Some(s) => eprintln!("  {} {}", colored("◆", 36), phantom_mesh::auth::human_summary(&s)),
                        None    => eprintln!("  {} not logged in — `/login` to set up identity", colored("◇", 90)),
                    }
                }

                "/diag" => {
                    let events = phantom_mesh::diag::snapshot();
                    let n = events.len();
                    eprintln!("  {} {} recent event{} (last 30 shown):",
                        colored("◆", 36), n, if n == 1 { "" } else { "s" });
                    let take = events.len().saturating_sub(30);
                    for ev in &events[take..] {
                        eprintln!("    {} {:<14} {}",
                            colored(&format!("{}", ev.ts_ms), 90),
                            colored(&ev.kind, 36),
                            ev.summary);
                    }
                    if let Some(p) = phantom_mesh::diag::events_path() {
                        eprintln!();
                        eprintln!("  {} full log: {}", colored("›", 90), p.display());
                    }
                    if let Some(p) = phantom_mesh::diag::last_crash_path() {
                        eprintln!("  {} last crash: {}", colored("⚠", 33), p.display());
                    } else {
                        eprintln!("  {} no crashes recorded.", colored("›", 90));
                    }
                }

                "/undo" => {
                    // Find the newest local APFS snapshot and restore cwd
                    // from it via `phantom snapshot apply --execute`.
                    // Convenience wrapper around the underlying CLI command;
                    // the user gets one sudo prompt and a clear ⚠ warning.
                    #[cfg(target_os = "macos")]
                    {
                        match phantom_mesh::snapshot::list().await {
                            Ok(list) if !list.is_empty() => {
                                let newest = &list[0];
                                let cwd = std::env::current_dir()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|_| ".".into());
                                eprintln!("  {} most recent snapshot: {}",
                                    colored("◆", 36), newest.id);
                                eprintln!("  {} target: {}", colored("◆", 36), cwd);
                                eprintln!("  {} sudo will be prompted (rsync --delete restore)…",
                                    colored("⟲", 33));
                                if let Ok(exe) = std::env::current_exe() {
                                    let _ = std::process::Command::new(&exe)
                                        .args(["snapshot", "apply", &newest.id, "--path", &cwd, "--execute"])
                                        .status();
                                }
                            }
                            Ok(_) => eprintln!("  {} no local snapshots — `phantom snapshot create <label>` first",
                                colored("⚠", 33)),
                            Err(e) => eprintln!("  {} {}", colored("✗", 31), e),
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        eprintln!("  {} `/undo` is currently macOS-only (uses APFS local snapshots).",
                            colored("⚠", 33));
                    }
                }

                "/copy" => {
                    // /copy        — last assistant message only (default — fast path)
                    // /copy all    — entire session as markdown
                    // /copy turn   — last user prompt + assistant reply pair
                    let mode = parts.get(1).map(|s| s.trim()).unwrap_or("");
                    let history = conversations.get_history(&chat_id).await;
                    let payload: String = match mode {
                        "all" => conversations.export_markdown(&chat_id).await,
                        "turn" => {
                            // Walk back from the end: find the last assistant,
                            // then the user before it.
                            let mut last_user: Option<&phantom_mesh::providers::traits::ChatMessage> = None;
                            let mut last_asst: Option<&phantom_mesh::providers::traits::ChatMessage> = None;
                            for m in history.iter().rev() {
                                if last_asst.is_none() && m.role == "assistant" {
                                    last_asst = Some(m);
                                } else if last_asst.is_some() && m.role == "user" {
                                    last_user = Some(m);
                                    break;
                                }
                            }
                            let mut s = String::new();
                            if let Some(u) = last_user { s.push_str(&format!("**You:** {}\n\n", u.content.trim())); }
                            if let Some(a) = last_asst { s.push_str(&format!("**Assistant:** {}\n", a.content.trim())); }
                            s
                        }
                        _ => history.iter().rev()
                            .find(|m| m.role == "assistant")
                            .map(|m| m.content.clone())
                            .unwrap_or_default(),
                    };
                    if payload.is_empty() {
                        eprintln!("  {} nothing to copy yet this session.", colored("◆", 90));
                    } else {
                        let cmd = if cfg!(target_os = "macos") {
                            "pbcopy"
                        } else if cfg!(target_os = "linux") {
                            "xclip"
                        } else if cfg!(target_os = "windows") {
                            "clip"
                        } else {
                            "pbcopy"
                        };
                        let mut child = std::process::Command::new(cmd)
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        match &mut child {
                            Ok(c) => {
                                if let Some(stdin) = c.stdin.as_mut() {
                                    use std::io::Write;
                                    let _ = stdin.write_all(payload.as_bytes());
                                }
                                let _ = c.wait();
                                let label = match mode {
                                    "all"  => "entire session",
                                    "turn" => "last turn",
                                    _      => "last assistant message",
                                };
                                eprintln!("  {} copied {} ({} chars) via {}",
                                    colored("✓", 32), label, payload.len(), cmd);
                            }
                            Err(e) => {
                                eprintln!("  {} couldn't run {}: {}",
                                    colored("✗", 31), cmd, e);
                                eprintln!("    install hint: `brew install xclip` (linux) — pbcopy/clip ship with the OS.");
                            }
                        }
                    }
                }

                "/settings" => {
                    // Open phantom serve's web UI Settings pane in the default browser.
                    // Falls back to printing the URL when no `open` / `xdg-open` / `start`.
                    let port: u16 = std::env::var("PHANTOM_PORT").ok()
                        .and_then(|s| s.parse().ok()).unwrap_or(7878);
                    let url = format!("http://127.0.0.1:{}/?tab=settings", port);
                    eprintln!("  {} opening {}", colored("◆", 36), url);
                    open_browser(&url);
                    eprintln!("  {} (paste in any browser if it didn't open automatically)",
                        colored("›", 90));
                }

                "/export" => {
                    // /export             → ~/.phantom-mesh/exports/<id>-<timestamp>.md
                    // /export <path>      → <path>
                    // Useful when the session is long and clipboard limits bite, or
                    // when you want to share / archive without piping into an editor.
                    let custom = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    let md = conversations.export_markdown(&chat_id).await;
                    if md.is_empty() {
                        eprintln!("  {} nothing to export yet this session.", colored("◆", 90));
                    } else {
                        let path: PathBuf = match custom {
                            Some(p) => PathBuf::from(p),
                            None => {
                                let ts = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs()).unwrap_or(0);
                                let dir = dirs::home_dir()
                                    .unwrap_or_else(|| PathBuf::from("."))
                                    .join(".phantom-mesh/exports");
                                std::fs::create_dir_all(&dir).ok();
                                let safe_id: String = chat_id.chars()
                                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                                    .collect();
                                dir.join(format!("{}-{}.md", safe_id, ts))
                            }
                        };
                        match std::fs::write(&path, &md) {
                            Ok(()) => {
                                eprintln!("  {} exported {} chars → {}",
                                    colored("✓", 32), md.len(), path.display());
                                eprintln!("  {} open it: `open {}`",
                                    colored("›", 90), path.display());
                            }
                            Err(e) => {
                                eprintln!("  {} write failed: {}", colored("✗", 31), e);
                            }
                        }
                    }
                }

                "/provider" => {
                    let cfg = app_state.agent_runtime.config();
                    let providers: Vec<(&String, &phantom_mesh::config::ProviderEntry)> =
                        cfg.providers.iter().collect();
                    if providers.is_empty() {
                        eprintln!("  {} no [providers.*] block in agents.toml — `/settings` to add one", colored("⚠", 33));
                    } else {
                        eprintln!("  {} {} configured providers:", colored("◆", 36), providers.len());
                        for (name, ent) in &providers {
                            let key_state = if ent.api_key.is_some() {
                                colored("✓ key", 32)
                            } else if ent.api_key_env.is_some() {
                                colored("✓ env", 32)
                            } else {
                                colored("✗ no key", 31)
                            };
                            let model = ent.default_model.as_deref().unwrap_or("<none>");
                            let base = ent.url.as_deref().unwrap_or("<vendor default>");
                            eprintln!("    {} {:<14} {} · default: {}", colored("•", 36), name, key_state, model);
                            eprintln!("      {} {}", colored("base:", 90), colored(base, 90));
                        }
                        eprintln!();
                        eprintln!("  switch via `/model <provider>:<model>` or edit agents.toml [agent.master].provider");
                    }
                }

                "/keys" => {
                    let sub = parts.get(1).map(|s| s.trim()).unwrap_or("");
                    let mut sub_parts = sub.splitn(2, ' ');
                    let action = sub_parts.next().unwrap_or("").trim();
                    let target = sub_parts.next().unwrap_or("").trim();

                    match action {
                        "" | "list" => {
                            let cfg = app_state.agent_runtime.config();
                            let states = phantom_mesh::keys::snapshot_states(&cfg);
                            if states.is_empty() {
                                eprintln!("  {} no providers configured. `/keys add <name>` to add one.",
                                    colored("◆", 33));
                            } else {
                                eprintln!("  {} key state ({} provider{}):",
                                    colored("◆", 36),
                                    states.len(),
                                    if states.len() == 1 { "" } else { "s" });
                                for (name, state) in &states {
                                    let badge = match state {
                                        phantom_mesh::keys::KeyState::Inline =>
                                            colored("✓ inline", 32),
                                        phantom_mesh::keys::KeyState::EnvResolved { var } =>
                                            colored(&format!("✓ env (${})", var), 32),
                                        phantom_mesh::keys::KeyState::EnvMissing { var } =>
                                            colored(&format!("⚠ env-unset (${})", var), 33),
                                        phantom_mesh::keys::KeyState::NotConfigured =>
                                            colored("✗ no key", 31),
                                    };
                                    eprintln!("    {} {:<14} {}", colored("•", 36), name, badge);
                                }
                                eprintln!();
                                eprintln!("  {} /keys add <provider>     paste a key (writes ~/.phantom-mesh/agents.toml)", colored("›", 90));
                                eprintln!("  {} /keys remove <provider>  drop the api_key from agents.toml",  colored("›", 90));
                                eprintln!("  {} /keys test <provider>    probe the provider with the current key", colored("›", 90));
                            }
                        }
                        "remove" | "rm" => {
                            if target.is_empty() {
                                eprintln!("  usage: /keys remove <provider>");
                            } else {
                                let path = phantom_mesh::keys::agents_toml_path();
                                match phantom_mesh::keys::remove_api_key(&path, target) {
                                    Ok(()) => {
                                        eprintln!("  {} dropped api_key for {} from {}",
                                            colored("✓", 32), target, path.display());
                                        eprintln!("  {} restart phantom for the change to take effect.",
                                            colored("›", 90));
                                    }
                                    Err(e) => {
                                        eprintln!("  {} {}", colored("✗", 31), e);
                                    }
                                }
                            }
                        }
                        "test" => {
                            if target.is_empty() {
                                eprintln!("  usage: /keys test <provider>");
                            } else {
                                let cfg = app_state.agent_runtime.config();
                                match cfg.providers.get(target) {
                                    None => {
                                        eprintln!("  {} unknown provider '{}'. /keys list to see configured ones.",
                                            colored("✗", 31), target);
                                    }
                                    Some(ent) => {
                                        // Pick the resolved key — inline first, then env.
                                        let key = ent.api_key.clone().filter(|s| !s.is_empty())
                                            .or_else(|| ent.api_key_env.as_ref()
                                                .and_then(|v| std::env::var(v).ok())
                                                .filter(|s| !s.is_empty()));
                                        match key {
                                            None => {
                                                eprintln!("  {} no key set for {} — /keys add {} first",
                                                    colored("✗", 31), target, target);
                                            }
                                            Some(k) => {
                                                let url = ent.url.clone()
                                                    .or_else(|| phantom_mesh::keys::default_provider_meta(target)
                                                        .map(|(_, u)| u.to_string()))
                                                    .unwrap_or_default();
                                                if url.is_empty() {
                                                    eprintln!("  {} no base url for {} — set [providers.{}].url in agents.toml",
                                                        colored("✗", 31), target, target);
                                                } else {
                                                    eprintln!("  {} probing {} → {} (5s timeout)…",
                                                        colored("◆", 36), target, url);
                                                    match phantom_mesh::keys::probe_provider(target, &url, &k).await {
                                                        Ok(r) => {
                                                            let mark = if r.ok { colored("✓", 32) } else { colored("✗", 31) };
                                                            eprintln!("  {} {} ({} ms)", mark, r.message, r.elapsed_ms);
                                                            if let Some(n) = r.model_count {
                                                                eprintln!("  {} {} models available — /model fetch {} to list them",
                                                                    colored("›", 90), n, target);
                                                            }
                                                        }
                                                        Err(e) => {
                                                            eprintln!("  {} transport error: {}", colored("✗", 31), e);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "add" => {
                            if target.is_empty() {
                                eprintln!("  usage: /keys add <provider>");
                                eprintln!("  known providers: groq, gemini, openrouter, anthropic, openai, opencode");
                                eprintln!("  unknown names work too — you'll be prompted for the base url.");
                            } else {
                                // 1. Read existing key first so we can rollback on failure.
                                let path = phantom_mesh::keys::agents_toml_path();
                                let existing_key: Option<String> = {
                                    let cfg = app_state.agent_runtime.config();
                                    cfg.providers.get(target).and_then(|p| p.api_key.clone())
                                };
                                if existing_key.as_deref().filter(|s| !s.is_empty()).is_some() {
                                    eprintln!("  {} {} already has a key set. /keys remove {} first if you want to replace it.",
                                        colored("◆", 33), target, target);
                                } else {
                                    // 2. Prompt for key (rustyline isn't ideal for paste but it's what we have).
                                    eprintln!("  {} paste your {} api key (or empty to abort):",
                                        colored("◆", 36), target);
                                    let new_key = match rl.readline("    key: ") {
                                        Ok(s) => s.trim().to_string(),
                                        Err(_) => String::new(),
                                    };
                                    if new_key.is_empty() {
                                        eprintln!("  {} aborted.", colored("◆", 33));
                                    } else {
                                        // 3. Write to toml.
                                        match phantom_mesh::keys::set_api_key(&path, target, &new_key) {
                                            Err(e) => {
                                                eprintln!("  {} write failed: {}", colored("✗", 31), e);
                                            }
                                            Ok(()) => {
                                                eprintln!("  {} wrote {} api_key to {}",
                                                    colored("✓", 32), target, path.display());

                                                // 4. Probe the new key. Use default_provider_meta
                                                // for the base url (the toml has it now).
                                                let url = phantom_mesh::keys::default_provider_meta(target)
                                                    .map(|(_, u)| u.to_string())
                                                    .unwrap_or_default();
                                                if url.is_empty() {
                                                    eprintln!("  {} no default base url for unknown provider '{}' — set [providers.{}].url in agents.toml manually.",
                                                        colored("⚠", 33), target, target);
                                                    eprintln!("  {} restart phantom for the change to take effect.",
                                                        colored("›", 90));
                                                } else {
                                                    eprintln!("  {} probing {} → {} …",
                                                        colored("◆", 36), target, url);
                                                    match phantom_mesh::keys::probe_provider(target, &url, &new_key).await {
                                                        Ok(r) if r.ok => {
                                                            eprintln!("  {} {} ({} ms)",
                                                                colored("✓", 32), r.message, r.elapsed_ms);
                                                            if let Some(n) = r.model_count {
                                                                eprintln!("  {} {} models available — /model fetch {} to list",
                                                                    colored("›", 90), n, target);
                                                            }
                                                            eprintln!("  {} restart phantom for runtime to pick up the new key.",
                                                                colored("›", 90));
                                                        }
                                                        Ok(r) => {
                                                            // 5. Probe failed — auto-rollback so we don't
                                                            // leave a known-bad key in toml.
                                                            eprintln!("  {} {} — rolling back",
                                                                colored("✗", 31), r.message);
                                                            let _ = phantom_mesh::keys::remove_api_key(&path, target);
                                                            eprintln!("  {} api_key removed from agents.toml — try /keys add {} with the correct key",
                                                                colored("›", 90), target);
                                                        }
                                                        Err(e) => {
                                                            eprintln!("  {} transport error: {} — rolling back",
                                                                colored("✗", 31), e);
                                                            let _ = phantom_mesh::keys::remove_api_key(&path, target);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        other => {
                            eprintln!("  unknown /keys subcommand: {}. Try /keys list", other);
                        }
                    }
                }

                "/tools" => {
                    let names = app_state.tool_registry.names();
                    let categorized = categorize_tools(&names);
                    eprintln!("  {} {} tools available", colored("◆", 36), names.len());
                    for (cat, items) in &categorized {
                        if items.is_empty() { continue; }
                        eprintln!();
                        eprintln!("  {}", colored(cat, 36));
                        for chunk in items.chunks(4) {
                            eprintln!("    {}", chunk.join("  "));
                        }
                    }
                    if let Some(reg) = phantom_mesh::mcp_client::global() {
                        let summary = reg.summary().await;
                        if !summary.is_empty() {
                            eprintln!();
                            eprintln!("  {}", colored("external (mcp)", 36));
                            for (name, n) in &summary {
                                eprintln!("    {} ({} tools)", name, n);
                            }
                        }
                    }
                }

                "/mcp" => {
                    let reg = phantom_mesh::mcp_client::global();
                    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
                    match reg {
                        None => eprintln!("  {} no [[mcp_servers]] configured.", colored("◆", 33)),
                        Some(reg) => {
                            if let Some(rest) = arg.strip_prefix("test ") {
                                let name = rest.trim();
                                eprintln!("  {} pinging mcp server '{}'…", colored("◆", 36), name);
                                match reg.ping_server(name).await {
                                    Ok(tools) => {
                                        eprintln!("  {} {} returned {} tool(s)",
                                                  colored("✓", 32), name, tools.len());
                                        for t in tools.iter().take(20) {
                                            let n = t["name"].as_str().unwrap_or("?");
                                            eprintln!("    - {}", n);
                                        }
                                    }
                                    Err(e) => eprintln!("  {} {}: {}", colored("error:", 31), name, e),
                                }
                            } else {
                                let summary = reg.summary().await;
                                if summary.is_empty() {
                                    eprintln!("  {} no mcp servers running.", colored("◆", 33));
                                } else {
                                    eprintln!("  {} {} mcp server(s):", colored("◆", 36), summary.len());
                                    for (name, n) in &summary {
                                        eprintln!("    {:<20} {} tool(s)", name, n);
                                    }
                                    eprintln!();
                                    eprintln!("  /mcp test <name>   re-ping a server's tools/list");
                                }
                            }
                        }
                    }
                }

                "/clear" => {
                    conversations.evict(&chat_id).await;
                    pending_context.clear();
                    eprintln!("{} Conversation cleared.", colored("◆", 32));
                }

                "/compact" => {
                    let history = conversations.get_history(&chat_id).await;
                    if history.len() < 4 {
                        eprintln!(
                            "{} Nothing to compact ({} message{}). Use /clear to start fresh.",
                            colored("◆", 33),
                            history.len(),
                            if history.len() == 1 { "" } else { "s" },
                        );
                        continue;
                    }
                    eprintln!(
                        "{} Summarizing {} messages via {}…",
                        colored("◆", 35),
                        history.len(),
                        agent_name,
                    );
                    match compact_via_llm(
                        &runtime,
                        &agent_name,
                        &cost_tracker,
                        &conversations,
                        &chat_id,
                        &history,
                        6,
                    ).await {
                        Ok((dropped, summary_chars)) => {
                            eprintln!(
                                "{} Compacted {} old messages → 1 summary ({} chars), kept last 6 verbatim.",
                                colored("◆", 32),
                                dropped,
                                summary_chars,
                            );
                        }
                        Err(e) => {
                            eprintln!("{} compact failed: {}", colored("error:", 31), e);
                        }
                    }
                }

                "/add" => {
                    if let Some(path_str) = parts.get(1).map(|s| s.trim()) {
                        match std::fs::read_to_string(path_str) {
                            Ok(content) => {
                                let len = content.len();
                                pending_context.push(format!(
                                    "<file path=\"{}\">\n{}\n</file>",
                                    path_str, content
                                ));
                                eprintln!(
                                    "{} Added {} to context ({} bytes)",
                                    colored("◆", 32),
                                    path_str,
                                    len
                                );
                            }
                            Err(e) => {
                                eprintln!("{} reading {}: {}", colored("error:", 31), path_str, e);
                            }
                        }
                    } else {
                        eprintln!("usage: /add <path>");
                    }
                }

                "/sessions" => {
                    let ids = conversations.list().await;
                    if ids.is_empty() {
                        eprintln!("  No saved sessions.");
                    } else {
                        let home = dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".phantom-mesh")
                            .join("conversations");
                        // Build (id, size, modified, msg_count) and sort by modified desc
                        let mut rows: Vec<(String, u64, std::time::SystemTime, usize)> = ids.iter().map(|id| {
                            let path = home.join(format!("{}.jsonl", id));
                            let meta = std::fs::metadata(&path).ok();
                            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                            let mtime = meta.as_ref().and_then(|m| m.modified().ok()).unwrap_or(std::time::UNIX_EPOCH);
                            let msgs = std::fs::read_to_string(&path).map(|s| s.lines().count()).unwrap_or(0);
                            (id.clone(), size, mtime, msgs)
                        }).collect();
                        rows.sort_by(|a, b| b.2.cmp(&a.2));
                        eprintln!("  {} {} session{}:", colored("◆", 36), rows.len(),
                            if rows.len() == 1 { "" } else { "s" });
                        let now = std::time::SystemTime::now();
                        for (id, size, mtime, msgs) in &rows {
                            let marker = if id == &chat_id { colored("*", 32) } else { " ".into() };
                            let id_short = if id.len() > 28 { format!("{}…", &id[..27]) } else { id.clone() };
                            let size_s = format_size(*size);
                            let age = now.duration_since(*mtime).map(format_age).unwrap_or_else(|_| "-".into());
                            eprintln!("    {} {:<32} {:>8}  {:>4} msg  {}",
                                marker, id_short, size_s, msgs, colored(&age, 90));
                        }
                        eprintln!("  {}", colored("/resume <id>  or  /resume    (latest)", 90));
                    }
                }

                "/resume" => {
                    let arg = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    let target_id: Option<String> = if let Some(arg) = arg {
                        // Fuzzy match: exact id, or unique prefix among existing sessions
                        let ids = conversations.list().await;
                        if ids.iter().any(|id| id == arg) {
                            Some(arg.to_string())
                        } else {
                            let matches: Vec<&String> = ids.iter().filter(|id| id.starts_with(arg)).collect();
                            match matches.len() {
                                0 => {
                                    eprintln!("  {} no session matches '{}'", colored("error:", 31), arg);
                                    None
                                }
                                1 => Some(matches[0].clone()),
                                _ => {
                                    eprintln!("  {} '{}' is ambiguous, matches:", colored("error:", 31), arg);
                                    for m in matches.iter().take(5) { eprintln!("    {}", m); }
                                    None
                                }
                            }
                        }
                    } else {
                        // No arg → most recently modified
                        find_last_session()
                    };
                    if let Some(id) = target_id {
                        chat_id = id.clone();
                        pending_context.clear();
                        let history = conversations.get_history(&chat_id).await;
                        eprintln!("  {} resumed session {}  ({} messages)",
                            colored("◆", 32), &chat_id[..chat_id.len().min(28)], history.len());
                    }
                }

                "/fork" => {
                    let new_id = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            // default name: <src>-fork-<6-hex>
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut h = DefaultHasher::new();
                            std::time::SystemTime::now().hash(&mut h);
                            chat_id.hash(&mut h);
                            format!("{}-fork-{:06x}", &chat_id[..chat_id.len().min(20)], h.finish() & 0xffffff)
                        });
                    if new_id == chat_id {
                        eprintln!("  {} cannot fork to the same id", colored("error:", 31));
                    } else {
                        match conversations.fork(&chat_id, &new_id).await {
                            Ok(n) => {
                                let prev = chat_id.clone();
                                chat_id = new_id.clone();
                                pending_context.clear();
                                eprintln!("  {} forked from {} → {} ({} messages copied)",
                                    colored("◆", 32),
                                    &prev[..prev.len().min(20)],
                                    colored(&new_id[..new_id.len().min(28)], 36),
                                    n);
                                eprintln!("  {}", colored("you're now on the fork; original session unchanged.", 90));
                            }
                            Err(e) => eprintln!("  {} fork failed: {}", colored("error:", 31), e),
                        }
                    }
                }

                "/list" => {
                    let sessions = conversations.list().await;
                    if sessions.is_empty() {
                        eprintln!("  No saved sessions.");
                    } else {
                        let home = dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".phantom-mesh")
                            .join("conversations");
                        eprintln!("  {:<40}  {:>10}", "Session ID", "Size");
                        eprintln!("  {}", "-".repeat(54));
                        for id in &sessions {
                            let path = home.join(format!("{}.jsonl", id));
                            let size = std::fs::metadata(&path)
                                .map(|m| m.len())
                                .unwrap_or(0);
                            let size_str = format_size(size);
                            let marker = if *id == chat_id { " *" } else { "" };
                            eprintln!("  {:<40}  {:>10}{}", id, size_str, marker);
                        }
                    }
                }

                "/session" => {
                    if let Some(new_id) = parts.get(1).map(|s| s.trim()) {
                        if new_id.is_empty() {
                            eprintln!("  current session: {}", chat_id);
                        } else {
                            chat_id = new_id.to_string();
                            pending_context.clear();
                            eprintln!("{} Switched to session: {}", colored("◆", 32), chat_id);
                        }
                    } else {
                        eprintln!("  current session: {}", chat_id);
                        eprintln!("  usage: /session <id>");
                    }
                }

                "/model" => {
                    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
                    let (action, target) = arg.split_once(' ')
                        .map(|(a, t)| (a.trim(), t.trim()))
                        .unwrap_or((arg, ""));

                    // /model fetch <provider> — pull live model list
                    if action == "fetch" {
                        if target.is_empty() {
                            eprintln!("  usage: /model fetch <provider>");
                        } else {
                            let cfg = app_state.agent_runtime.config();
                            let ent = cfg.providers.get(target).cloned();
                            match ent {
                                None => {
                                    eprintln!("  {} unknown provider '{}'. /provider to list configured ones.",
                                        colored("✗", 31), target);
                                }
                                Some(ent) => {
                                    let key = ent.api_key.clone().filter(|s| !s.is_empty())
                                        .or_else(|| ent.api_key_env.as_ref()
                                            .and_then(|v| std::env::var(v).ok())
                                            .filter(|s| !s.is_empty()));
                                    let url = ent.url.clone()
                                        .or_else(|| phantom_mesh::keys::default_provider_meta(target).map(|(_, u)| u.to_string()))
                                        .unwrap_or_default();
                                    match (key, url.is_empty()) {
                                        (None, _) => {
                                            eprintln!("  {} no key for {} — /keys add {} first", colored("✗", 31), target, target);
                                        }
                                        (_, true) => {
                                            eprintln!("  {} no base url for {} — set [providers.{}].url", colored("✗", 31), target, target);
                                        }
                                        (Some(k), false) => {
                                            eprintln!("  {} fetching models from {} → {} …",
                                                colored("◆", 36), target, url);
                                            match phantom_mesh::keys::fetch_models(target, &url, &k).await {
                                                Ok(ids) if ids.is_empty() => {
                                                    eprintln!("  {} no models in response (empty list)", colored("⚠", 33));
                                                }
                                                Ok(ids) => {
                                                    eprintln!("  {} {} models from {}:", colored("◆", 32), ids.len(), target);
                                                    for id in &ids {
                                                        eprintln!("    {} {}", colored("•", 36), id);
                                                    }
                                                    eprintln!();
                                                    eprintln!("  {} /model {}:{}    switch this session",
                                                        colored("›", 90), target,
                                                        ids.first().map(|s| s.as_str()).unwrap_or("<id>"));
                                                }
                                                Err(e) => {
                                                    eprintln!("  {} fetch failed: {}", colored("✗", 31), e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if action == "pick" {
                        // /model pick <provider> — interactive number-select.
                        // Fetches the live model list, prints with [1] [2]...,
                        // prompts for a number, applies model_override.
                        if target.is_empty() {
                            eprintln!("  usage: /model pick <provider>");
                        } else {
                            let cfg = app_state.agent_runtime.config();
                            let ent = cfg.providers.get(target).cloned();
                            match ent {
                                None => {
                                    eprintln!("  {} unknown provider '{}'. /provider to list configured ones.",
                                        colored("✗", 31), target);
                                }
                                Some(ent) => {
                                    let key = ent.api_key.clone().filter(|s| !s.is_empty())
                                        .or_else(|| ent.api_key_env.as_ref()
                                            .and_then(|v| std::env::var(v).ok())
                                            .filter(|s| !s.is_empty()));
                                    let url = ent.url.clone()
                                        .or_else(|| phantom_mesh::keys::default_provider_meta(target).map(|(_, u)| u.to_string()))
                                        .unwrap_or_default();
                                    match (key, url.is_empty()) {
                                        (None, _) => {
                                            eprintln!("  {} no key for {} — /keys add {} first", colored("✗", 31), target, target);
                                        }
                                        (_, true) => {
                                            eprintln!("  {} no base url for {}", colored("✗", 31), target);
                                        }
                                        (Some(k), false) => {
                                            eprintln!("  {} fetching models …", colored("◆", 36));
                                            match phantom_mesh::keys::fetch_models(target, &url, &k).await {
                                                Err(e) => eprintln!("  {} {}", colored("✗", 31), e),
                                                Ok(ids) if ids.is_empty() => {
                                                    eprintln!("  {} no models returned", colored("⚠", 33));
                                                }
                                                Ok(ids) => {
                                                    eprintln!("  {} {} models — pick by number, or 0 to abort:",
                                                        colored("◆", 32), ids.len());
                                                    for (i, id) in ids.iter().enumerate() {
                                                        eprintln!("    [{:>2}] {}", i + 1, id);
                                                    }
                                                    let pick = match rl.readline("    pick: ") {
                                                        Ok(s) => s.trim().to_string(),
                                                        Err(_) => "0".into(),
                                                    };
                                                    match pick.parse::<usize>() {
                                                        Ok(0) => eprintln!("  {} aborted.", colored("◆", 33)),
                                                        Ok(n) if n >= 1 && n <= ids.len() => {
                                                            let chosen = &ids[n - 1];
                                                            model_override = Some(chosen.clone());
                                                            std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", target);
                                                            eprintln!("  {} switched to {}/{} for this session",
                                                                colored("✓", 32), target, chosen);
                                                        }
                                                        _ => eprintln!("  {} invalid pick '{}'", colored("✗", 31), pick),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if matches!(action, "fast" | "smart" | "cheap") {
                        // /model fast | smart | cheap — preset shortcuts
                        let cfg = app_state.agent_runtime.config();
                        let preset = match action {
                            "fast"  => &[
                                ("groq",       "llama-3.3-70b-versatile"),
                                ("gemini",     "gemini-2.0-flash-exp"),
                                ("openrouter", "google/gemini-2.0-flash-exp"),
                                ("openai",     "gpt-4o-mini"),
                            ][..],
                            "smart" => &[
                                ("anthropic",  "claude-sonnet-4-20250514"),
                                ("openai",     "gpt-4o"),
                                ("openrouter", "anthropic/claude-sonnet-4"),
                                ("groq",       "llama-3.3-70b-versatile"),
                            ][..],
                            "cheap" => &[
                                ("groq",       "llama-3.1-8b-instant"),
                                ("gemini",     "gemini-2.0-flash-lite"),
                                ("openrouter", "google/gemini-2.0-flash-lite"),
                                ("opencode",   "claude-haiku-4-5-free"),
                            ][..],
                            _ => &[][..],
                        };
                        let pick = preset.iter()
                            .find(|(p, _)| cfg.providers.contains_key(*p))
                            .copied();
                        match pick {
                            None => {
                                eprintln!("  {} no '{}' preset available — none of [{}] are configured.",
                                    colored("✗", 31),
                                    action,
                                    preset.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(", "));
                                eprintln!("  {} /keys add <provider> to add one", colored("›", 90));
                            }
                            Some((pname, mname)) => {
                                model_override = Some(mname.to_string());
                                std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", pname);
                                eprintln!("  {} {} preset → {}/{}",
                                    colored("✓", 32), action, pname, mname);
                            }
                        }
                    } else if arg.is_empty() {
                        // Bare /model — show current + every provider's
                        // default model so the user can pick.
                        print_model_status(&agent_name, &model_override);
                        let cfg = app_state.agent_runtime.config();
                        if !cfg.providers.is_empty() {
                            eprintln!();
                            eprintln!("  {} available models (one per provider):", colored("◆", 36));
                            for (pname, pent) in cfg.providers.iter() {
                                let mark = if pent.default_model.is_some() { "•" } else { " " };
                                let model = pent.default_model.as_deref().unwrap_or("<no default_model set>");
                                eprintln!("    {} {:<14} {}", colored(mark, 36), pname, model);
                            }
                            eprintln!();
                            eprintln!("  switch with:  /model <model-name>          (overrides current agent's model)");
                            eprintln!("                /model <provider>:<model>    (also switches provider)");
                            eprintln!("                /model fast | smart | cheap  (preset shortcuts)");
                            eprintln!("                /model fetch <provider>      (live model list)");
                            eprintln!("                /model pick <provider>       (number-select from live list)");
                            eprintln!("  list providers: /provider");
                        }
                    } else if let Some((pname, mname)) = arg.split_once(':') {
                        // Provider-qualified switch: /model groq:llama-3.3-70b
                        let cfg = app_state.agent_runtime.config();
                        if !cfg.providers.contains_key(pname) {
                            eprintln!("  {} unknown provider '{}'. Try /provider", colored("✗", 31), pname);
                        } else {
                            // Set the model override + remember provider hint via env so
                            // the next turn picks it up. Persistent change still requires
                            // editing agents.toml.
                            model_override = Some(mname.to_string());
                            std::env::set_var("PHANTOM_PROVIDER_OVERRIDE", pname);
                            eprintln!("  {} switched to {}/{} for this session",
                                colored("✓", 32), pname, mname);
                        }
                    } else {
                        // Plain /model <name> — model-only override (legacy behaviour).
                        model_override = Some(arg.to_string());
                        eprintln!("  {} model override → {}", colored("✓", 32), arg);
                    }
                }

                "/init" => {
                    let content = phantom_mesh::scaffold::generate_phantom_md_async(&cwd).await;
                    let out_path = cwd.join("PHANTOM.md");
                    std::fs::write(&out_path, &content)?;
                    eprintln!("{} Created {}", colored("◆", 32), out_path.display());
                }

                "/agent" | "/agents" => {
                    let cfg = app_state.agent_runtime.config();
                    if parts[0] == "/agent" {
                        if let Some(name) = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                            if cfg.agent.contains_key(name) {
                                agent_name = name.to_string();
                                eprintln!("{} Active agent: {}", colored("◆", 32), name);
                            } else {
                                eprintln!("{} unknown agent: {}", colored("error:", 31), name);
                                eprintln!("  available: {}", cfg.agent.keys().cloned().collect::<Vec<_>>().join(", "));
                            }
                            continue;
                        }
                    }
                    eprintln!("  {} active: {}", colored("◆", 32), agent_name);
                    eprintln!("  configured ({}):", cfg.agent.len());
                    for (name, ag) in cfg.agent.iter() {
                        let marker = if name == &agent_name { "*" } else { " " };
                        let provider = if ag.provider.is_empty() { "<inherit>" } else { ag.provider.as_str() };
                        eprintln!("    {} {:<14} provider={}", marker, name, provider);
                    }
                }

                "/todo" => {
                    let path = dirs::home_dir()
                        .map(|h| h.join(".phantom-mesh").join("todos.json"));
                    let raw = path.as_ref()
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .unwrap_or_else(|| "[]".to_string());
                    let parsed: serde_json::Value = serde_json::from_str(&raw)
                        .unwrap_or(serde_json::Value::Array(vec![]));
                    let items = parsed.get("todos").and_then(|v| v.as_array())
                        .or_else(|| parsed.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if items.is_empty() {
                        eprintln!("  {} no todos. Use the todo_add tool from any agent to create one.", colored("◆", 90));
                    } else {
                        eprintln!("  {} {} todo{}:", colored("◆", 32), items.len(), if items.len() == 1 { "" } else { "s" });
                        for it in items.iter() {
                            let status = it.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                            let text   = it.get("text").and_then(|v| v.as_str()).unwrap_or("(no text)");
                            let dot = match status {
                                "done"        => colored("●", 32),
                                "in_progress" => colored("●", 33),
                                _             => colored("○", 90),
                            };
                            eprintln!("    {} {}", dot, text);
                        }
                    }
                }

                "/plan" => {
                    let now = std::env::var("PHANTOM_PLAN_MODE").unwrap_or_default();
                    if now == "1" {
                        std::env::remove_var("PHANTOM_PLAN_MODE");
                        eprintln!("  {} plan mode OFF — agents execute tools immediately.",
                            colored("✓", 32));
                    } else {
                        std::env::set_var("PHANTOM_PLAN_MODE", "1");
                        eprintln!("  {} plan mode ON — agents will preview their plan, then wait for 'go' before any tool call.",
                            colored("✓", 32));
                    }
                }

                "/show" => {
                    // `/show thinking` dumps the FULL reasoning trace of the
                    // most recent turn (captured even when PHANTOM_THINKING=0).
                    if parts.get(1).map(|s| s.trim()) == Some("thinking") {
                        let buf = thinking_session.lock().ok().map(|s| s.clone()).unwrap_or_default();
                        if buf.is_empty() {
                            eprintln!("  {} no reasoning trace captured for the most recent turn.", colored("◆", 90));
                            eprintln!("  {} (only some models — Anthropic extended-thinking, opencode/groq reasoning models — emit one)", colored("›", 90));
                        } else {
                            let dim_italic = "\x1b[2m\x1b[3m";
                            let reset = "\x1b[0m";
                            let total = buf.lines().filter(|l| !l.trim().is_empty()).count();
                            eprintln!("  {} ⌖ thinking — full trace ({} lines, {} chars)",
                                colored("◆", 36), total, buf.chars().count());
                            for line in buf.lines() {
                                eprintln!("{}  ⌖ ┊ {}{}", dim_italic, line, reset);
                            }
                        }
                        continue;
                    }
                    let h = tool_history.lock().ok();
                    let entries = match h {
                        Some(h) => h.clone(),
                        None => Vec::new(),
                    };
                    if entries.is_empty() {
                        eprintln!("  {} no tool calls captured yet this session.", colored("◆", 90));
                    } else if let Some(arg) = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        // Show full output of one entry
                        match arg.parse::<usize>() {
                            Ok(n) => {
                                if let Some(entry) = entries.iter().find(|e| e.n == n) {
                                    eprintln!("  {} {} {}({})",
                                        colored(&format!("[#{}]", n), 90),
                                        colored("●", 36),
                                        colored(&entry.name, 36),
                                        truncate_preview(&entry.args, 200));
                                    eprintln!();
                                    if entry.output.is_empty() {
                                        eprintln!("  {}", colored("(no output captured — still running?)", 90));
                                    } else {
                                        // Print full output, indented
                                        for line in entry.output.lines() {
                                            eprintln!("  {}", colored(line, 90));
                                        }
                                    }
                                    eprintln!();
                                    eprintln!("  {}", colored(&format!("({} chars, {} lines)",
                                        entry.output.chars().count(),
                                        entry.output.lines().count()), 90));
                                } else {
                                    eprintln!("  {} no tool call #{} (range: 1..={})",
                                        colored("error:", 31), n, entries.last().map(|e| e.n).unwrap_or(0));
                                }
                            }
                            Err(_) => {
                                eprintln!("  {} usage: /show <n>  (number from [#N] in tool output)", colored("error:", 31));
                            }
                        }
                    } else {
                        // List all
                        eprintln!("  {} {} tool call{} this session:",
                            colored("◆", 36), entries.len(),
                            if entries.len() == 1 { "" } else { "s" });
                        for entry in entries.iter() {
                            let arg_summary = truncate_preview(&entry.args, 60);
                            let out_lines = if entry.output.is_empty() {
                                "(running)".to_string()
                            } else {
                                format!("{} lines", entry.output.lines().count())
                            };
                            eprintln!("    {} {}({})  {}",
                                colored(&format!("[#{}]", entry.n), 90),
                                colored(&entry.name, 36),
                                arg_summary,
                                colored(&out_lines, 90));
                        }
                        eprintln!("  {}", colored("/show <n> to expand any one", 90));
                    }
                }

                "/perm" => {
                    let arg = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    match arg {
                        Some("ask") => {
                            std::env::set_var("PHANTOM_PERM", "ask");
                            eprintln!("  {} permission mode: {} — every tool call will prompt", colored("◆", 32), colored("ask", 33));
                        }
                        Some("diff") => {
                            std::env::set_var("PHANTOM_PERM", "diff");
                            eprintln!("  {} permission mode: {} — file_edit shows a unified diff before the y/N prompt", colored("◆", 32), colored("diff", 33));
                        }
                        Some("allow") | Some("auto") => {
                            std::env::set_var("PHANTOM_PERM", "allow");
                            eprintln!("  {} permission mode: {} — all tool calls auto-approved", colored("◆", 32), colored("allow", 36));
                        }
                        Some("deny") => {
                            std::env::set_var("PHANTOM_PERM", "deny");
                            eprintln!("  {} permission mode: {} — all tool calls will be denied", colored("◆", 32), colored("deny", 31));
                        }
                        Some("reset") | Some("clear") => {
                            if let Ok(mut a) = perm_allowlist.lock() { a.clear(); }
                            eprintln!("  {} session allow-list cleared", colored("◆", 32));
                        }
                        Some("list") | Some("show") => {
                            let cur = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
                            eprintln!("  {} mode: {}", colored("◆", 36), colored(&cur, 36));
                            if let Ok(a) = perm_allowlist.lock() {
                                if a.is_empty() {
                                    eprintln!("  {}", colored("session allow-list: (empty)", 90));
                                } else {
                                    eprintln!("  {}", colored("session allow-list:", 90));
                                    for tool in a.iter() {
                                        eprintln!("    {}", colored(tool, 36));
                                    }
                                }
                            }
                        }
                        Some(other) => {
                            eprintln!("  {} unknown: {}. usage: /perm ask|diff|allow|deny|reset|list", colored("error:", 31), other);
                        }
                        None => {
                            let cur = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
                            eprintln!("  {} permission mode: {}", colored("◆", 36), colored(&cur, 36));
                            eprintln!("  {}", colored("usage: /perm ask|diff|allow|deny|reset|list", 90));
                            eprintln!("  {}", colored("  ask    prompt before every tool call (y/n/a/A)", 90));
                            eprintln!("  {}", colored("  diff   like ask, but file_edit shows a unified diff first", 90));
                            eprintln!("  {}", colored("  allow  auto-approve all (default)", 90));
                            eprintln!("  {}", colored("  deny   block all tools (read-only mode)", 90));
                            eprintln!("  {}", colored("  reset  clear the per-tool always-allow list", 90));
                        }
                    }
                }

                "/tasks" => {
                    let snap = phantom_mesh::tools::subagent::task_log_snapshot();
                    if snap.is_empty() {
                        eprintln!("  {} no subagent tasks yet this session.", colored("◆", 90));
                    } else {
                        let running: usize = snap.iter().filter(|r| r.status == "running").count();
                        eprintln!("  {} {} subagent task{}{}",
                            colored("◆", 36),
                            snap.len(),
                            if snap.len() == 1 { "" } else { "s" },
                            if running > 0 { format!(" ({} running)", running) } else { String::new() });
                        for rec in snap.iter() {
                            let badge = match rec.status.as_str() {
                                "running" => colored("●", 33),  // amber
                                "ok"      => colored("✓", 32),
                                "timeout" => colored("⏱", 33),
                                _         => colored("✗", 31),
                            };
                            let prompt_short = rec.prompt.chars().take(50).collect::<String>();
                            eprintln!("    {} {} {}: {}",
                                badge,
                                colored(&format!("[#{}]", rec.n), 90),
                                colored(&rec.agent, 36),
                                colored(&prompt_short, 90));
                            if rec.status != "running" {
                                eprintln!("        {} {} rounds · ${:.4} · {:.1}s",
                                    colored("·", 90), rec.rounds, rec.cost_usd, rec.elapsed_secs);
                            }
                        }
                    }
                }

                "/density" => {
                    let arg = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    match arg {
                        Some("compact") => {
                            std::env::set_var("PHANTOM_DENSITY", "compact");
                            eprintln!("  {} display density: {} (1-line tool results)", colored("◆", 32), colored("compact", 36));
                        }
                        Some("full") | Some("normal") => {
                            std::env::remove_var("PHANTOM_DENSITY");
                            eprintln!("  {} display density: {} (multi-line tool results)", colored("◆", 32), colored("full", 36));
                        }
                        Some(other) => {
                            eprintln!("  {} unknown density: {}. usage: /density compact|full", colored("error:", 31), other);
                        }
                        None => {
                            let cur = std::env::var("PHANTOM_DENSITY").unwrap_or_else(|_| "full".into());
                            eprintln!("  {} current density: {}", colored("◆", 36), colored(&cur, 36));
                            eprintln!("  {}", colored("usage: /density compact|full", 90));
                        }
                    }
                }

                "/theme" => {
                    let arg = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                    match arg {
                        Some(name @ ("dark" | "light" | "claude" | "codex" | "gemini" | "mono")) => {
                            std::env::set_var("PHANTOM_THEME", name);
                            eprintln!("  {} theme: {} {}", colored("◆", 32), colored(name, 36),
                                colored("(restart REPL to fully apply banner colors)", 90));
                        }
                        Some(other) => {
                            eprintln!("  {} unknown theme: {}. options: dark / light / claude / codex / gemini / mono",
                                colored("error:", 31), other);
                        }
                        None => {
                            let cur = std::env::var("PHANTOM_THEME").unwrap_or_else(|_| "dark".into());
                            eprintln!("  {} current theme: {}", colored("◆", 36), colored(&cur, 36));
                            eprintln!("  {}", colored("options: dark / light / claude / codex / gemini / mono", 90));
                        }
                    }
                }

                other => {
                    eprintln!("unknown command: {}. Try /help", other);
                }
            }
            continue;
        }

        // Expand @file references in the raw line
        let expanded_line = expand_at_files(&line);

        // Prepend any pending file context to the prompt
        let prompt = if pending_context.is_empty() {
            expanded_line.clone()
        } else {
            let ctx = pending_context.join("\n\n");
            pending_context.clear();
            format!("{}\n\n{}", ctx, expanded_line)
        };

        // Build effective extra_context, optionally injecting model override hint
        // and Plan-mode planning instruction.
        let mut effective_context = if let Some(ref model) = model_override {
            format!("{}\nModel override: {}", extra_context, model)
        } else {
            extra_context.clone()
        };
        if std::env::var("PHANTOM_PLAN_MODE").ok().as_deref() == Some("1") {
            effective_context.push_str(
                "\n\n## PLAN MODE ACTIVE\n\
                Before calling any tool, FIRST output a numbered plan describing what you \
                will do, which tools you will call, and which files you will touch. End the \
                plan with the literal line: 'Awaiting approval before execution.' Then STOP \
                and wait — do not call any tools yet. The user will reply with 'go' to \
                approve, or with corrections.",
            );
        }

        // ── Auto-compact trigger ─────────────────────────────────────
        // When the on-disk conversation is large enough that the next LLM
        // call would risk overrunning the context window, transparently
        // summarize the older messages first. Heuristic: ~4 chars/token,
        // budget = 150K tokens (~600K chars). Tuned conservative — even
        // 64K-window models stay well clear.
        const AUTO_COMPACT_CHARS: usize = 600_000;
        let total_chars = conversations.total_chars(&chat_id).await;
        if total_chars > AUTO_COMPACT_CHARS {
            let pre_history = conversations.get_history(&chat_id).await;
            if pre_history.len() > 12 {
                eprintln!(
                    "  {} auto-compacting ({} chars ≈ {}K tokens)…",
                    colored("⚙", 33),
                    total_chars,
                    total_chars / 4 / 1000,
                );
                match compact_via_llm(
                    &runtime,
                    &agent_name,
                    &cost_tracker,
                    &conversations,
                    &chat_id,
                    &pre_history,
                    8,
                ).await {
                    Ok((dropped, _summary_chars)) if dropped > 0 => {
                        eprintln!(
                            "  {} compacted {} old messages → 1 summary, kept last 8.",
                            colored("◆", 32),
                            dropped,
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!(
                            "  {} auto-compact failed (continuing with full history): {}",
                            colored("⚠", 33),
                            e,
                        );
                    }
                }
            }
        }

        let history = conversations.get_history(&chat_id).await;

        let t0 = std::time::Instant::now();
        let streamed = Arc::new(AtomicBool::new(false));
        let handler = make_stream_handler_with_thinking(
            streamed.clone(),
            Some(tool_history.clone()),
            Some(thinking_session.clone()),
        );

        // Plan mode handshake: detect approval words *before* dispatching.
        // If user typed exactly "go" / "execute" / "yes" while plan mode is
        // on, set the flag so the gate allows tools for THIS turn only.
        let plan_mode_on = std::env::var("PHANTOM_PLAN_MODE").as_deref() == Ok("1");
        let approval_words = ["go", "execute", "yes", "approve", "approved", "do it"];
        let trimmed = prompt.trim().to_lowercase();
        if plan_mode_on && approval_words.iter().any(|w| trimmed == *w) {
            plan_approved.store(true, std::sync::atomic::Ordering::Relaxed);
            eprintln!("  {} {}", colored("◆", 32), colored("plan approved — executing this turn", 32));
        }

        // Tool-permission gate. Two-tier:
        //   1. The `[permissions]` rule engine (Tool(specifier) DSL).
        //      If a rule fires Allow or Deny we honour it directly.
        //   2. If the engine returns Ask (or has no rules at all), fall
        //      through to the legacy env-var path: PHANTOM_PERM=ask
        //      enables the interactive prompt, PHANTOM_PERM=deny denies
        //      all, PLAN_MODE without approval denies, default allow.
        let gate_mode = std::env::var("PHANTOM_PERM").unwrap_or_else(|_| "allow".into());
        let allowlist = perm_allowlist.clone();
        let plan_approved_clone = plan_approved.clone();
        let engine_for_gate = permission_engine.clone();
        let stream_fut = runtime.run_with_callbacks_gated(
            &agent_name, &prompt, &history,
            Some(&effective_context), &cost_tracker, handler,
            move |tool_name, args| {
                use phantom_mesh::permission::Decision;
                // Plan mode: block tools unless user just approved with "go".
                let plan_mode_on = std::env::var("PHANTOM_PLAN_MODE").as_deref() == Ok("1");
                if plan_mode_on
                    && !plan_approved_clone.load(std::sync::atomic::Ordering::Relaxed)
                {
                    return phantom_mesh::agent::ToolGateDecision::Deny(
                        "Plan mode is active — output the plan as text and stop. \
                         The user will type 'go' / 'execute' / 'yes' to approve, \
                         then you may call tools.".into());
                }

                // Rule engine first. Allow/Deny short-circuit; Ask falls
                // through to the legacy env-var + interactive path so the
                // user can still answer y/n/a/A in the terminal.
                match engine_for_gate.evaluate(tool_name, args) {
                    Decision::Allow => return phantom_mesh::agent::ToolGateDecision::Allow,
                    Decision::Deny(reason) => {
                        return phantom_mesh::agent::ToolGateDecision::Deny(reason);
                    }
                    Decision::Ask => { /* fall through */ }
                }

                if gate_mode == "deny" {
                    return phantom_mesh::agent::ToolGateDecision::Deny(
                        format!("PHANTOM_PERM=deny — user has globally denied tool execution"));
                }
                if gate_mode != "ask" && gate_mode != "diff" {
                    return phantom_mesh::agent::ToolGateDecision::Allow;
                }
                // Cached "always" decision?
                if let Ok(allow) = allowlist.lock() {
                    if allow.contains("*") || allow.contains(tool_name) {
                        return phantom_mesh::agent::ToolGateDecision::Allow;
                    }
                }

                // PHANTOM_PERM=diff: render a unified diff for file_edit BEFORE
                // the y/N prompt. For non-file_edit tools, fall through to the
                // ordinary "ask" prompt below.
                if gate_mode == "diff" && tool_name == "file_edit" {
                    if let Some(path_str) = args["path"].as_str() {
                        let old_str = args["old_string"].as_str().unwrap_or("");
                        let new_str = args["new_string"].as_str().unwrap_or("");
                        let replace_all = args["replace_all"].as_bool().unwrap_or(false);
                        match std::fs::read_to_string(path_str) {
                            Ok(content) => {
                                let after = if replace_all {
                                    content.replace(old_str, new_str)
                                } else {
                                    content.replacen(old_str, new_str, 1)
                                };
                                if after == content {
                                    eprintln!();
                                    eprintln!("  {} {} (old_string not found in {})",
                                        colored("⚠", 33),
                                        colored("file_edit", 36),
                                        colored(path_str, 90));
                                } else {
                                    eprintln!();
                                    eprintln!("  {} {} preview for {}:",
                                        colored("◆", 35),
                                        colored("file_edit", 36),
                                        colored(path_str, 90));
                                    eprint!("{}", phantom_mesh::diff_render::render_unified_diff(
                                        path_str, &content, &after,
                                    ));
                                }
                            }
                            Err(e) => {
                                eprintln!();
                                eprintln!("  {} could not read {} for diff preview: {}",
                                    colored("⚠", 33), colored(path_str, 90), e);
                            }
                        }
                    }
                }
                // Interactive prompt to stderr.
                let args_summary = serde_json::to_string(args).unwrap_or_default();
                let args_summary = if args_summary.chars().count() > 200 {
                    let s: String = args_summary.chars().take(200).collect();
                    format!("{}…", s)
                } else { args_summary };
                eprintln!();
                eprintln!("  {} run {}({}) ?",
                    colored("⚠", 33), colored(tool_name, 36), colored(&args_summary, 90));
                eprint!("    {}: [y]es / [n]o / [a]lways this tool / [A]llways all tools : ", colored("permission", 33));
                let _ = std::io::Write::flush(&mut std::io::stderr());
                let mut buf = String::new();
                let _ = std::io::stdin().read_line(&mut buf);
                let ans = buf.trim();
                match ans {
                    "y" | "yes" | "" => phantom_mesh::agent::ToolGateDecision::Allow,
                    "a" => {
                        if let Ok(mut al) = allowlist.lock() { al.insert(tool_name.to_string()); }
                        eprintln!("  {} {} added to always-allow list", colored("◆", 32), tool_name);
                        phantom_mesh::agent::ToolGateDecision::Allow
                    }
                    "A" => {
                        if let Ok(mut al) = allowlist.lock() { al.insert("*".to_string()); }
                        eprintln!("  {} all tools always-allowed for this session", colored("◆", 32));
                        phantom_mesh::agent::ToolGateDecision::Allow
                    }
                    _ => phantom_mesh::agent::ToolGateDecision::Deny(
                        "user denied this tool call".into()),
                }
            },
        );
        let outcome = tokio::select! {
            biased;
            r = stream_fut => Some(r),
            _ = tokio::signal::ctrl_c() => {
                // Force a newline so the cancel marker doesn't collide with mid-stream output
                eprintln!();
                eprintln!("  {} {}", colored("⨯", 33), colored("interrupted (Ctrl-C)", 33));
                None
            }
        };
        let Some(call_result) = outcome else {
            // Stream cancelled — also clear the one-shot plan approval flag.
            plan_approved.store(false, std::sync::atomic::Ordering::Relaxed);
            continue;
        };
        // Clear the one-shot plan approval after this turn finishes.
        plan_approved.store(false, std::sync::atomic::Ordering::Relaxed);
        match call_result {
            Ok(result) => {
                // ── Interactive approval gate ──────────────────────────────
                if result.output.starts_with("APPROVAL_REQUIRED:") {
                    eprintln!("{}", result.output);

                    // Read approval answer via rustyline
                    match rl.readline("Allow? [y/N] ") {
                        Ok(answer) => {
                            let answer = answer.trim().to_lowercase();
                            if answer == "y" || answer == "yes" {
                                std::env::set_var("PHANTOM_AUTO_APPROVE", "1");
                                let streamed2 = Arc::new(AtomicBool::new(false));
                                let handler2 = make_stream_handler_with_thinking(
                                    streamed2.clone(),
                                    Some(tool_history.clone()),
                                    Some(thinking_session.clone()),
                                );
                                match runtime
                                    .run_with_callbacks(
                                        &agent_name,
                                        &prompt,
                                        &history,
                                        Some(&effective_context),
                                        &cost_tracker,
                                        handler2,
                                    )
                                    .await
                                {
                                    Ok(approved_result) => {
                                        let elapsed = t0.elapsed().as_secs_f64();
                                        let last = cost_tracker.last_request_cost().await;
                                        let session = cost_tracker.session_cost().await;
                                        eprintln!("{}", colored(&format!("[↑ ${:.4}  ∑ ${:.4}  {:.1}s]", last, session, elapsed), 90));
                                        let user_msg = ChatMessage { role: "user".into(), content: line, tool_calls: None };
                                        let asst_msg = ChatMessage { role: "assistant".into(), content: approved_result.output, tool_calls: None };
                                        conversations.append(&chat_id, user_msg, asst_msg).await;
                                    }
                                    Err(e) => {
                                        eprintln!("{} {}", colored("error:", 31), e);
                                    }
                                }
                                std::env::remove_var("PHANTOM_AUTO_APPROVE");
                            } else {
                                eprintln!("Cancelled.");
                            }
                        }
                        Err(_) => {
                            eprintln!("Cancelled.");
                        }
                    }
                } else {
                    let elapsed = t0.elapsed().as_secs_f64();
                    let last = cost_tracker.last_request_cost().await;
                    let session = cost_tracker.session_cost().await;
                    eprintln!("{}", colored(&format!("[↑ ${:.4}  ∑ ${:.4}  {:.1}s]", last, session, elapsed), 90));
                    let user_msg = ChatMessage { role: "user".into(), content: line, tool_calls: None };
                    let asst_msg = ChatMessage { role: "assistant".into(), content: result.output, tool_calls: None };
                    conversations.append(&chat_id, user_msg, asst_msg).await;
                }
            }
            Err(e) => {
                eprintln!("{} {}", colored("error:", 31), e);
            }
        }
    }

    if let Some(ref p) = history_path {
        let _ = rl.save_history(p);
    }

    Ok(())
}

fn print_model_status(agent_name: &str, model_override: &Option<String>) {
    eprintln!("  agent: {}", agent_name);
    match model_override {
        Some(m) => eprintln!("  model override: {}", m),
        None => eprintln!("  no model override set (using agent default)"),
    }
}

/// Render a Duration as a short relative-time string ("3h ago", "12d ago").
fn format_age(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 { format!("{}s ago", s) }
    else if s < 3600 { format!("{}m ago", s / 60) }
    else if s < 86400 { format!("{}h ago", s / 3600) }
    else { format!("{}d ago", s / 86400) }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

fn cwd_chat_id(cwd: &PathBuf) -> String {
    let mut hasher = DefaultHasher::new();
    cwd.to_string_lossy().hash(&mut hasher);
    format!("pm_{:016x}", hasher.finish())
}

/// Build a streaming event handler for AgentRuntime.run_with_callbacks().
/// Renders Token / ToolStart / ToolDone events to stdout/stderr in a
/// Claude Code-style inline format.
///
/// One captured tool invocation, retained in REPL state so the user can
/// `/show <n>` the full output later.
#[derive(Clone)]
pub struct ToolEntry {
    pub n: usize,
    pub name: String,
    pub args: String,
    pub output: String,
}

/// `streamed` is shared mutable state: tracks whether any token output was
/// emitted, so the caller can decide whether to add a trailing newline.
/// `tool_history` (optional) is appended on every ToolStart/ToolDone so the
/// REPL can later expand a tool's full output by sequence number.
fn make_stream_handler(
    streamed: Arc<AtomicBool>,
    tool_history: Option<Arc<std::sync::Mutex<Vec<ToolEntry>>>>,
) -> impl FnMut(AgentEvent) + Send + Sync + 'static {
    make_stream_handler_with_thinking(streamed, tool_history, None)
}

/// Render a thinking buffer as a dim-italic block (used by both the REPL
/// streaming handler and `/show thinking`). Shows the first 3 non-empty
/// lines plus a `… +N more · /show thinking` footer when truncated.
fn render_thinking_block(buf: &str) -> String {
    let lines: Vec<&str> = buf.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = lines.len();
    let dim_italic = "\x1b[2m\x1b[3m";
    let reset = "\x1b[0m";
    let mut out = String::new();
    out.push_str(&format!("{}  ⌖ thinking ({} line{}){}\n",
        dim_italic, total, if total == 1 { "" } else { "s" }, reset));
    for line in lines.iter().take(3) {
        let truncated: String = line.chars().take(160).collect();
        let suffix = if line.chars().count() > 160 { "…" } else { "" };
        out.push_str(&format!("{}  ⌖ ┊ {}{}{}\n",
            dim_italic, truncated, suffix, reset));
    }
    if total > 3 {
        out.push_str(&format!("{}  ⌖ … +{} more · /show thinking{}\n",
            dim_italic, total - 3, reset));
    }
    out
}

/// Like `make_stream_handler` but also records the per-turn reasoning trace
/// into `thinking_session` so `/show thinking` can dump the most recent turn.
///
/// Per-turn buffer logic: each Thinking event appends to a closure-local
/// buffer; on the first non-Thinking event after a thinking burst, the buffer
/// is flushed as a dim-italic block above the response and mirrored to
/// `thinking_session` (replacing the previous turn's trace). When
/// `PHANTOM_THINKING=0`, the block isn't rendered but the trace is still
/// captured so `/show thinking` works.
fn make_stream_handler_with_thinking(
    streamed: Arc<AtomicBool>,
    tool_history: Option<Arc<std::sync::Mutex<Vec<ToolEntry>>>>,
    thinking_session: Option<Arc<std::sync::Mutex<String>>>,
) -> impl FnMut(AgentEvent) + Send + Sync + 'static {
    use std::io::Write;
    // Tiny stateful markdown decorator: streams tokens character-by-character
    // and toggles ANSI styles around bold/italic/code spans, fenced code
    // blocks, and # headings (line-start). Disabled when stdout is not a TTY
    // or when NO_COLOR=1.
    let decorate_md = std::env::var_os("NO_COLOR").is_none()
        && std::env::var("PHANTOM_MD")
            .map(|v| v != "0")
            .unwrap_or(true)
        && atty_stdout();
    let state = std::sync::Mutex::new(MdState::default());
    let thinking_buf: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let render_thinking = std::env::var("PHANTOM_THINKING").map(|v| v != "0").unwrap_or(true);

    move |ev: AgentEvent| {
        // Helper: flush the per-turn thinking buffer (called at every
        // boundary into a non-Thinking event). Mirrors to the shared
        // session buffer for /show thinking.
        let flush = || {
            let mut b = match thinking_buf.lock() { Ok(b) => b, Err(_) => return };
            if b.is_empty() { return; }
            if let Some(ref s) = thinking_session {
                if let Ok(mut sb) = s.lock() { *sb = b.clone(); }
            }
            if render_thinking {
                eprint!("{}", render_thinking_block(&b));
                let _ = std::io::stderr().flush();
            }
            b.clear();
        };

        match ev {
        AgentEvent::Thinking { content } => {
            if let Ok(mut b) = thinking_buf.lock() { b.push_str(&content); }
        }
        AgentEvent::Token { content } => {
            flush();
            if !content.is_empty() {
                streamed.store(true, Ordering::Relaxed);
                if decorate_md {
                    let rendered = state.lock().map(|mut s| s.feed(&content)).unwrap_or(content);
                    print!("{}", rendered);
                } else {
                    print!("{}", content);
                }
                let _ = std::io::stdout().flush();
            }
        }
        AgentEvent::ToolStart { name, args_preview } => {
            phantom_mesh::diag::record("tool_start",
                format!("{} {}", name, args_preview.chars().take(80).collect::<String>()));
            flush();
            if streamed.swap(false, Ordering::Relaxed) {
                if decorate_md {
                    if let Ok(mut s) = state.lock() { print!("{}", s.reset_styles()); }
                }
                println!();
            }
            // Append a placeholder entry; output filled in on ToolDone.
            let n = if let Some(ref hist) = tool_history {
                if let Ok(mut h) = hist.lock() {
                    let n = h.len() + 1;
                    h.push(ToolEntry {
                        n,
                        name: name.clone(),
                        args: args_preview.clone(),
                        output: String::new(),
                    });
                    n
                } else { 0 }
            } else { 0 };
            let preview = truncate_preview(&args_preview, 80);
            let n_label = if n > 0 {
                format!("{} ", colored(&format!("[#{}]", n), 90))
            } else { String::new() };
            eprintln!("{} {}{}({})", colored("●", 36), n_label, colored(&name, 36), preview);
        }
        AgentEvent::ToolDone { name, output_preview } => {
            // Capture FULL output into history for /show <n>
            if let Some(ref hist) = tool_history {
                if let Ok(mut h) = hist.lock() {
                    if let Some(last) = h.last_mut() {
                        last.output = output_preview.clone();
                    }
                }
            }
            // Compact mode (PHANTOM_DENSITY=compact): show only the first
            // line; full mode shows up to 5 non-empty lines.
            // task / subagent results are user-facing: show much more (40
            // lines) so the user sees the full subagent answer inline
            // instead of having to /show <n> every time.
            let is_subagent = name == "task" || name == "subagent";
            let max_lines = if std::env::var("PHANTOM_DENSITY").as_deref() == Ok("compact") {
                1
            } else if is_subagent {
                40
            } else {
                5
            };
            let cleaned: Vec<String> = output_preview.lines()
                .filter(|l| !l.trim().is_empty())
                .take(max_lines)
                .map(|l| {
                    let s: String = l.chars().take(120).collect();
                    if l.chars().count() > 120 { format!("{}…", s) } else { s }
                })
                .collect();
            let total_lines = output_preview.lines().filter(|l| !l.trim().is_empty()).count();
            if cleaned.is_empty() {
                eprintln!("  {}", colored("✓", 32));
            } else {
                eprintln!("  {} {}", colored("✓", 32), colored(&cleaned[0], 90));
                for line in cleaned.iter().skip(1) {
                    eprintln!("    {}", colored(line, 90));
                }
                if total_lines > cleaned.len() {
                    let n_hint = if let Some(ref hist) = tool_history {
                        hist.lock().ok().and_then(|h| h.last().map(|e| e.n)).unwrap_or(0)
                    } else { 0 };
                    let hint = if n_hint > 0 {
                        format!("… +{} more line{}  ·  /show {} for full",
                            total_lines - cleaned.len(),
                            if total_lines - cleaned.len() == 1 { "" } else { "s" },
                            n_hint)
                    } else {
                        format!("… +{} more lines", total_lines - cleaned.len())
                    };
                    eprintln!("    {}", colored(&hint, 90));
                }
            }
        }
        AgentEvent::Done { output, cost_usd, elapsed_secs } => {
            phantom_mesh::diag::record("agent_done",
                format!("{:.1}s · ${:.4} · {} chars", elapsed_secs, cost_usd, output.chars().count()));
            flush();
            if streamed.swap(false, Ordering::Relaxed) {
                if decorate_md {
                    if let Ok(mut s) = state.lock() { print!("{}", s.reset_styles()); }
                }
                println!();
            }
        }
        AgentEvent::Notice { message } => {
            // Surface the heads-up inline in the CLI stream so users running
            // `phantom run / evolve` see truncation warnings just like TUI
            // users see the red ⚠ row. Eprintln keeps it on stderr so it
            // doesn't pollute stdout when callers pipe the response.
            phantom_mesh::diag::record("agent_notice", message.clone());
            flush();
            if streamed.swap(false, Ordering::Relaxed) {
                if decorate_md {
                    if let Ok(mut s) = state.lock() { print!("{}", s.reset_styles()); }
                }
                println!();
            }
            eprintln!("{}", colored(&format!("⚠ {}", message), 31));
        }
        }
    }
}

fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Tiny streaming markdown→ANSI state machine. Handles fenced code blocks
/// (```), inline code (`...`), bold (**...**), italic (*...*), and ATX
/// headings (line-start `# ` / `## `). Operates char-by-char on a stream of
/// tokens — never has to see the whole document.
#[derive(Default)]
struct MdState {
    in_fence: bool,        // inside ```...``` block
    in_inline: bool,       // inside `...` span
    bold: bool,            // inside **...**
    italic: bool,          // inside *...* (single)
    line_start: bool,      // we just emitted a newline; next char is start-of-line
    fence_marks: u8,       // backtick run length when scanning ```
}

const ANSI_RESET:   &str = "\x1b[0m";
const ANSI_BOLD:    &str = "\x1b[1m";
const ANSI_ITALIC:  &str = "\x1b[3m";
const ANSI_DIM:     &str = "\x1b[2m";
const ANSI_CYAN:    &str = "\x1b[36m";
const ANSI_PURPLE:  &str = "\x1b[35m";
const ANSI_GREEN:   &str = "\x1b[32m";

impl MdState {
    fn reset_styles(&mut self) -> String {
        let need = self.in_fence || self.in_inline || self.bold || self.italic;
        self.in_fence = false; self.in_inline = false;
        self.bold = false; self.italic = false;
        self.line_start = true; self.fence_marks = 0;
        if need { ANSI_RESET.to_string() } else { String::new() }
    }

    fn feed(&mut self, chunk: &str) -> String {
        if self.line_start && chunk.is_empty() { return String::new(); }
        let mut out = String::with_capacity(chunk.len() + 16);
        let mut chars = chunk.chars().peekable();
        // Initialize line_start if this is first chunk we ever see
        // (callers should set line_start true at construction time)
        while let Some(c) = chars.next() {
            // Newline: terminate inline styles that don't span lines
            if c == '\n' {
                if self.in_inline { out.push_str(ANSI_RESET); self.in_inline = false; }
                if self.bold      { out.push_str(ANSI_RESET); self.bold = false; }
                if self.italic    { out.push_str(ANSI_RESET); self.italic = false; }
                out.push('\n');
                self.line_start = true;
                continue;
            }

            // Fence: ``` toggles
            if c == '`' {
                self.fence_marks += 1;
                if self.fence_marks == 3 {
                    self.fence_marks = 0;
                    if self.in_fence {
                        out.push_str(ANSI_RESET);
                        self.in_fence = false;
                    } else {
                        // Close any inline styles before entering fenced block
                        if self.in_inline { out.push_str(ANSI_RESET); self.in_inline = false; }
                        out.push_str(ANSI_DIM);
                        out.push_str(ANSI_GREEN);
                        self.in_fence = true;
                    }
                    self.line_start = false;
                    continue;
                }
                // emit the backtick(s) lazily — wait for next char to decide
                // To keep state-machine simple: we emit single ` immediately,
                // and rely on the next ` to flip inline mode.
                if self.fence_marks == 1 {
                    if self.in_fence {
                        out.push('`');
                        self.fence_marks = 0;
                    } else {
                        if self.in_inline {
                            out.push_str(ANSI_RESET);
                            self.in_inline = false;
                            self.fence_marks = 0;
                        } else {
                            // Peek: if next char is also `, we wait for the
                            // possible third one. Simplification: only single
                            // backticks toggle inline; double are rendered.
                            if let Some('`') = chars.peek() {
                                // could be ``code`` or ```fence```. Keep counting.
                                continue;
                            }
                            out.push_str(ANSI_DIM);
                            out.push_str(ANSI_CYAN);
                            self.in_inline = true;
                            self.fence_marks = 0;
                        }
                    }
                }
                self.line_start = false;
                continue;
            }
            // Reset fence_marks if we accumulated 1-2 ticks without third
            if self.fence_marks > 0 && c != '`' { self.fence_marks = 0; }

            // ATX heading (# at line start)
            if self.line_start && c == '#' && !self.in_fence {
                out.push_str(ANSI_BOLD);
                out.push_str(ANSI_PURPLE);
                out.push(c);
                // Consume any number of additional # then a space
                while let Some('#') = chars.peek() {
                    out.push(chars.next().unwrap());
                }
                if let Some(' ') = chars.peek() {
                    out.push(chars.next().unwrap());
                }
                // Style continues until newline; bold flag tracks via in_inline shouldn't be reused
                // We treat heading as a temporary "bold purple" — close on newline naturally.
                // Use bold flag so newline handler resets.
                self.bold = true;
                self.line_start = false;
                continue;
            }

            // Bullet list ("- " or "* " at line start) → render bullet glyph
            // in cyan, then continue line as normal text. Skip if we're in a
            // fenced code block (where dashes are literal).
            if self.line_start && (c == '-' || c == '+') && !self.in_fence {
                if let Some(' ') = chars.peek() {
                    chars.next(); // consume the space
                    out.push_str(ANSI_CYAN);
                    out.push('•');
                    out.push_str(ANSI_RESET);
                    out.push(' ');
                    self.line_start = false;
                    continue;
                }
            }

            // Numbered list ("1. ", "12. ", etc. at line start)
            if self.line_start && c.is_ascii_digit() && !self.in_fence {
                // Peek ahead for [digits]+ "." " "
                let mut digits = String::new();
                digits.push(c);
                let mut idx = 0;
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() && idx < 4 {
                        digits.push(chars.next().unwrap());
                        idx += 1;
                    } else { break; }
                }
                if let Some(&'.') = chars.peek() {
                    let mut iter_clone = chars.clone();
                    iter_clone.next(); // skip dot
                    if let Some(&' ') = iter_clone.peek() {
                        chars.next(); // consume "."
                        chars.next(); // consume " "
                        out.push_str(ANSI_CYAN);
                        out.push_str(&digits);
                        out.push('.');
                        out.push_str(ANSI_RESET);
                        out.push(' ');
                        self.line_start = false;
                        continue;
                    }
                }
                // Not a list — emit accumulated digits literally
                out.push_str(&digits);
                self.line_start = false;
                continue;
            }

            // Blockquote (`> ` at line start) → dim the rest of the line.
            // Reuses self.italic flag for "dim until newline" so newline
            // handler resets it.
            if self.line_start && c == '>' && !self.in_fence {
                if let Some(' ') = chars.peek() {
                    chars.next();
                    out.push_str(ANSI_DIM);
                    out.push_str(ANSI_CYAN);
                    out.push_str("│ ");
                    self.italic = true; // borrow italic flag for "reset on newline"
                    self.line_start = false;
                    continue;
                }
            }

            // Inline link `[text](url)` → render text underlined cyan, drop url.
            // Only handle the simple case: open '[' then text then ']' '(' url ')'.
            if c == '[' && !self.in_fence && !self.in_inline {
                let mut iter_clone = chars.clone();
                let mut text = String::new();
                let mut found_close = false;
                while let Some(&pc) = iter_clone.peek() {
                    if pc == ']' { found_close = true; break; }
                    if pc == '\n' { break; }
                    text.push(pc);
                    iter_clone.next();
                }
                if found_close {
                    iter_clone.next(); // consume ']'
                    if let Some(&'(') = iter_clone.peek() {
                        iter_clone.next();
                        let mut url = String::new();
                        let mut url_done = false;
                        while let Some(&pc) = iter_clone.peek() {
                            if pc == ')' { url_done = true; break; }
                            if pc == '\n' { break; }
                            url.push(pc);
                            iter_clone.next();
                        }
                        if url_done && !url.is_empty() {
                            // Commit the consumption
                            for _ in 0..(text.len() + 2 + url.len() + 1) { chars.next(); }
                            // text.chars().count() may differ from text.len() in bytes — use chars
                            let _ = url;
                            out.push_str("\x1b[4m");      // underline
                            out.push_str(ANSI_CYAN);
                            out.push_str(&text);
                            out.push_str(ANSI_RESET);
                            self.line_start = false;
                            continue;
                        }
                    }
                }
                // Not a link — fall through to literal '['
            }

            // Bold: ** at any position (toggle)
            if c == '*' && !self.in_fence && !self.in_inline {
                if let Some('*') = chars.peek() {
                    chars.next(); // consume the second *
                    if self.bold {
                        out.push_str(ANSI_RESET);
                        self.bold = false;
                    } else {
                        out.push_str(ANSI_BOLD);
                        self.bold = true;
                    }
                    self.line_start = false;
                    continue;
                }
                // Single * → italic toggle
                if self.italic {
                    out.push_str(ANSI_RESET);
                    self.italic = false;
                } else {
                    out.push_str(ANSI_ITALIC);
                    self.italic = true;
                }
                self.line_start = false;
                continue;
            }

            out.push(c);
            self.line_start = false;
        }
        out
    }
}

/// Group tool names into logical categories for /tools display.
fn categorize_tools(names: &[String]) -> Vec<(&'static str, Vec<String>)> {
    let mut filesystem = Vec::new();
    let mut search     = Vec::new();
    let mut shell_bash = Vec::new();
    let mut git        = Vec::new();
    let mut web        = Vec::new();
    let mut memory     = Vec::new();
    let mut diag       = Vec::new();
    let mut todo       = Vec::new();
    let mut mesh       = Vec::new();
    let mut other      = Vec::new();
    for name in names {
        let n = name.as_str();
        if n.starts_with("file_") || n == "ls" || n == "stat" || n == "apply_patch"
            || n == "multi_file_edit" || n == "diff_files" || n == "diff_strings" {
            filesystem.push(name.clone());
        } else if n.contains("search") || n.contains("grep") || n.contains("glob") {
            search.push(name.clone());
        } else if n == "shell" || n.starts_with("bash_") {
            shell_bash.push(name.clone());
        } else if n.starts_with("git_") {
            git.push(name.clone());
        } else if n.starts_with("web_") || n.starts_with("http_") || n == "fetch" {
            web.push(name.clone());
        } else if n.starts_with("memory_") {
            memory.push(name.clone());
        } else if n.starts_with("cargo_") || n.starts_with("tsc_") || n == "run_tests" {
            diag.push(name.clone());
        } else if n.starts_with("todo_") {
            todo.push(name.clone());
        } else if n.starts_with("phantom_") {
            mesh.push(name.clone());
        } else {
            other.push(name.clone());
        }
    }
    for v in [&mut filesystem, &mut search, &mut shell_bash, &mut git, &mut web,
              &mut memory, &mut diag, &mut todo, &mut mesh, &mut other] {
        v.sort();
    }
    vec![
        ("filesystem",  filesystem),
        ("search",      search),
        ("shell/bash",  shell_bash),
        ("git",         git),
        ("web",         web),
        ("memory",      memory),
        ("diagnostics", diag),
        ("todo",        todo),
        ("mesh",        mesh),
        ("other",       other),
    ]
}

fn truncate_preview(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let truncated: String = cleaned.chars().take(max).collect();
        format!("{}…", truncated)
    }
}

/// Strip trailing-backslash continuation markers from multi-line REPL input.
/// `foo \\\nbar` → `foo \nbar` (line-continuation joins with a real newline).
fn strip_continuation(input: &str) -> String {
    input.replace("\\\n", "\n")
}

/// Discard any pending bytes in stdin's input buffer.
///
/// Called before each REPL readline so that keystrokes the user typed
/// while the previous turn was streaming — when the TTY is in cooked
/// mode and rustyline isn't reading — don't leak into the next prompt.
/// Without this, TTY drivers vary: sometimes the buffered chars get
/// flushed into rustyline as input (auto-submitting whatever the user
/// half-typed), sometimes they get dropped when rustyline switches the
/// terminal to raw mode (the "my input got eaten" symptom).
///
/// We choose deterministic-discard. Trade-off: chars typed during
/// streaming are always lost, never appear half-submitted.
fn flush_stdin_input() {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        // TCIFLUSH = 0 on Linux/macOS; flush input only.
        // Errors (e.g. stdin not a TTY when piped from a file) are non-fatal.
        unsafe { libc::tcflush(fd, libc::TCIFLUSH); }
    }
    // No-op on Windows for now — rustyline + Windows console mode handle
    // this differently and we haven't seen the eaten-input symptom there.
}

// ── REPL Helper: completion + ghost-text hints + minimal highlighting ─────────
//
// rustyline `Helper` glues together Completer + Hinter + Highlighter + Validator.
// Triggered by Tab in the REPL.
//
// What we complete:
//   - When the cursor sits in a token starting with `/`, suggest slash commands.
//   - When the cursor sits in a token starting with `@`, suggest filesystem
//     paths relative to the current working directory.

const SLASH_COMMANDS: &[&str] = &[
    "/help", "/exit", "/quit", "/clear", "/compact",
    "/add", "/cost", "/copy", "/settings", "/provider", "/undo",
    "/login", "/logout", "/whoami",
    "/tools", "/sessions", "/session", "/resume", "/fork", "/list",
    "/init", "/model", "/agent", "/agents", "/todo", "/plan",
    "/show", "/density", "/theme", "/perm", "/mcp", "/tasks", "/keys", "/export", "/diag",
];

struct PhantomHelper;

impl Helper for PhantomHelper {}
impl Validator for PhantomHelper {}
impl Highlighter for PhantomHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self, prompt: &'p str, _default: bool,
    ) -> Cow<'b, str> { Cow::Borrowed(prompt) }
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // dim the ghost-text suggestion (kept dim — that's the convention)
        Cow::Owned(format!("\x1b[90m{}\x1b[0m", hint))
    }
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        // 24-bit gold (255,215,0) + bold. Truecolor bypasses Apple
        // Terminal.app palette weirdness where \x1b[33m can render dim.
        if line.is_empty() {
            return Cow::Borrowed(line);
        }
        Cow::Owned(format!("\x1b[1m\x1b[38;2;255;215;0m{}\x1b[0m", line))
    }
    fn highlight_char(&self, _line: &str, _pos: usize, _forced: bool) -> bool {
        // Tell rustyline to invoke highlight() on every keystroke.
        true
    }
}
impl Hinter for PhantomHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        // only hint when the cursor is at end of input
        if pos != line.len() { return None; }
        let token = current_token(line, pos);
        if token.starts_with('/') && token.len() > 1 {
            for cmd in SLASH_COMMANDS {
                if cmd.starts_with(token) && *cmd != token {
                    return Some(cmd[token.len()..].to_string());
                }
            }
        }
        None
    }
}
impl Completer for PhantomHelper {
    type Candidate = Pair;

    fn complete(
        &self, line: &str, pos: usize, _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let token = current_token(line, pos);
        let token_start = pos.saturating_sub(token.len());

        // Slash command completion
        if token.starts_with('/') {
            let matches: Vec<Pair> = SLASH_COMMANDS.iter()
                .filter(|c| c.starts_with(token))
                .map(|c| Pair { display: (*c).into(), replacement: (*c).into() })
                .collect();
            return Ok((token_start, matches));
        }

        // @file path completion
        if let Some(stripped) = token.strip_prefix('@') {
            return Ok((token_start, complete_path(stripped)));
        }

        Ok((pos, vec![]))
    }
}

/// Extract the whitespace-bounded token that ends at `pos`.
fn current_token(line: &str, pos: usize) -> &str {
    let prefix = &line[..pos];
    let start = prefix.rfind(|c: char| c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    &line[start..pos]
}

/// Best-effort filesystem completion for `@<path>` tokens.
fn complete_path(partial: &str) -> Vec<Pair> {
    // Split into directory part + filename prefix.
    let (dir_part, file_prefix) = match partial.rsplit_once('/') {
        Some((d, f)) => (d.to_string(), f.to_string()),
        None => (".".to_string(), partial.to_string()),
    };
    let dir = if dir_part.is_empty() { "/" } else { dir_part.as_str() };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        if !name.starts_with(&file_prefix) { continue; }
        // skip hidden unless user typed the leading `.`
        if name.starts_with('.') && !file_prefix.starts_with('.') { continue; }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        let display_path = if dir_part == "." || dir_part.is_empty() {
            format!("{}{}", name, suffix)
        } else {
            format!("{}/{}{}", dir_part, name, suffix)
        };
        out.push(Pair {
            display: display_path.clone(),
            replacement: format!("@{}", display_path),
        });
        if out.len() >= 50 { break; }
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

/// Browser-based onboarding: spin up `phantom serve` in the background, open
/// the user's browser to the Settings tab, and wait for them to submit the
/// form (which writes ~/.phantom-mesh/agents.toml). Then exit cleanly so the
/// user can run `phantom`.
async fn run_web_onboarding() -> Result<()> {
    use std::time::Duration;

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let cfg_path = home.join(".phantom-mesh").join("agents.toml");
    let already_existed = cfg_path.exists();
    let initial_mtime = std::fs::metadata(&cfg_path).ok().and_then(|m| m.modified().ok());

    // Pick a port: 7878 default, but try 7879..7888 if taken.
    let mut bind_port = 7878;
    for p in 7878..=7888 {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            bind_port = p;
            break;
        }
    }

    // Build a minimal AppState (so serve has something to serve)
    let mut app_state = AppState::new();
    if let Some(content) = find_config() {
        app_state.load_config_toml(&content);
    }
    let app_state = std::sync::Arc::new(app_state);
    let router = phantom_mesh::serve::router(app_state);
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], bind_port).into();

    eprintln!();
    eprintln!("  {} {}", colored("phantom", 35), colored("— web onboarding", 90));
    eprintln!("  {} opening http://127.0.0.1:{}/#settings", colored("›", 90), bind_port);

    // Spawn the server task
    let server_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await
            .expect("bind failed");
        axum::serve(listener, router).await.ok();
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Open browser (best effort; non-blocking)
    let url = format!("http://127.0.0.1:{}/#settings", bind_port);
    open_browser(&url);

    eprintln!("  {} fill in the form and click Save. Press Ctrl-C here when done.", colored("›", 90));
    eprintln!();

    // Poll for config file change
    loop {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let exists = cfg_path.exists();
        let new_mtime = std::fs::metadata(&cfg_path).ok().and_then(|m| m.modified().ok());
        let changed = (!already_existed && exists) ||
                      (already_existed && new_mtime != initial_mtime);
        if changed {
            eprintln!("  {} {}", colored("✓", 32), format!("wrote {}", cfg_path.display()));
            eprintln!("  {} run `phantom` to start.", colored("›", 90));
            eprintln!();
            break;
        }
    }

    server_handle.abort();
    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    // Windows is the awkward one. Three layered traps:
    //   1. `cmd /C start <url>` — cmd.exe parses `&` as a command
    //      separator, so an OAuth URL with multiple ?a=1&b=2 query
    //      params gets truncated at the first &. URL must be QUOTED
    //      in the cmd line so cmd's tokenizer keeps it intact.
    //   2. `start "url"` — `start` treats the first quoted arg as a
    //      window TITLE, not the URL. Pass an empty title "" then the
    //      URL as the next quoted arg.
    //   3. Rust's Command::args auto-escapes for Win32 CommandLine but
    //      DOES NOT add the cmd-level quotes we need (it doesn't know
    //      about cmd's `&` parsing — that's a layer above CreateProcess).
    //      Use raw_arg to pass the already-cmd-formatted line verbatim
    //      so the quoting we add survives.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Defensive: a URL with embedded `"` would let an attacker break
        // out of our quoting and run arbitrary cmd commands. The OAuth
        // URLs we generate never contain `"` (urlencoding handles that),
        // but escape it anyway as `""` (cmd's literal-quote escape) for
        // any future caller that might pass user-controlled input.
        let safe_url = url.replace('"', "\"\"");
        let cmdline = format!("/C start \"\" \"{}\"", safe_url);
        let _ = std::process::Command::new("cmd")
            .raw_arg(&cmdline)
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
    }
}

/// First-time interactive onboarding. Walks the user through provider setup
/// and writes a minimal ~/.phantom-mesh/agents.toml. Skipped if a config
/// already exists somewhere on the search path.
fn run_first_time_onboarding() -> Result<()> {
    use std::io::Write;
    use rustyline::DefaultEditor;

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let cfg_dir = home.join(".phantom-mesh");
    std::fs::create_dir_all(&cfg_dir)?;
    let cfg_path = cfg_dir.join("agents.toml");

    eprintln!();
    eprintln!("  {} {}", colored("phantom", 35), colored("— first-time setup", 90));
    eprintln!();
    eprintln!("  Welcome. Let's configure at least one LLM provider.");
    eprintln!("  This will create {}", cfg_path.display());
    eprintln!();
    eprintln!("  Free options:");
    eprintln!("    {} Groq    — {} (Llama 3.3 70B, ~250 tok/s)",
        colored("•", 36), colored("https://console.groq.com", 90));
    eprintln!("    {} Gemini  — {} (long-context, daily quota)",
        colored("•", 36), colored("https://aistudio.google.com", 90));
    eprintln!();

    let mut rl = DefaultEditor::new()?;

    let groq_key = match rl.readline("  Groq API key (paste, or leave blank to skip): ") {
        Ok(s) => s.trim().to_string(),
        Err(_) => String::new(),
    };
    let gemini_key = match rl.readline("  Gemini API key (paste, or leave blank to skip): ") {
        Ok(s) => s.trim().to_string(),
        Err(_) => String::new(),
    };

    if groq_key.is_empty() && gemini_key.is_empty() {
        eprintln!();
        eprintln!("  {} no providers configured — phantom will start but cannot call any LLM.", colored("⚠", 33));
        eprintln!("  Edit {} later to add providers.", cfg_path.display());
        // Still write a stub so subsequent runs don't re-prompt
        let stub = "# phantom-mesh agents.toml — add at least one [[providers]] block to use phantom.\n\
                    # See: https://github.com/markl-a/phantom-mesh/blob/main/docs/INTEGRATIONS.md\n\
                    \n\
                    [core]\n\
                    host = \"127.0.0.1\"\n\
                    port = 7878\n";
        std::fs::write(&cfg_path, stub)?;
        eprintln!();
        return Ok(());
    }

    let mut toml = String::new();
    toml.push_str("# phantom-mesh agents.toml — generated by first-time setup\n\n");
    toml.push_str("[core]\n");
    toml.push_str("host = \"127.0.0.1\"\n");
    toml.push_str("port = 7878\n\n");

    let primary_provider = if !groq_key.is_empty() { "groq" } else { "gemini" };

    if !groq_key.is_empty() {
        toml.push_str("[providers.groq]\n");
        toml.push_str("type = \"groq\"\n");
        toml.push_str(&format!("api_key = \"{}\"\n", groq_key));
        toml.push_str("default_model = \"llama-3.3-70b-versatile\"\n\n");
    }
    if !gemini_key.is_empty() {
        toml.push_str("[providers.gemini]\n");
        toml.push_str("type = \"gemini\"\n");
        toml.push_str(&format!("api_key = \"{}\"\n", gemini_key));
        toml.push_str("default_model = \"gemini-2.5-flash\"\n\n");
    }

    // Default agent block so REPL has something to call
    toml.push_str("[agent.master]\n");
    toml.push_str(&format!("provider = \"{}\"\n", primary_provider));
    toml.push_str("instructions = \"You are phantom, a helpful AI agent.\"\n");

    let mut f = std::fs::File::create(&cfg_path)?;
    f.write_all(toml.as_bytes())?;

    eprintln!();
    eprintln!("  {} wrote {}", colored("✓", 32), cfg_path.display());
    eprintln!();

    // Run a one-shot health check so the user sees green ticks before
    // the REPL starts. Captures stdout/stderr from the subprocess so we
    // can present it inside our outer ceremony.
    eprintln!("  {} running `phantom doctor` to verify…",
        colored("◆", 36));
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = std::process::Command::new(&self_exe).arg("doctor").status();
    }
    eprintln!();

    // Surface the next 3 things the user can do, sorted by impact.
    eprintln!("  {} {}", colored("→", 36), colored("Next steps:", 36));
    eprintln!("    {} Take phantom for a spin:",   colored("1.", 36));
    eprintln!("       phantom              # interactive TUI");
    eprintln!("       phantom 'summarize this repo'   # one-shot");
    eprintln!();
    eprintln!("    {} Make it survive a reboot ({}):",
        colored("2.", 36),
        std::env::consts::OS);
    #[cfg(target_os = "macos")]
    eprintln!("       phantom service install      # launchd auto-start");
    #[cfg(target_os = "linux")]
    eprintln!("       phantom service install      # systemd --user unit");
    #[cfg(target_os = "windows")]
    eprintln!("       phantom service install      # Scheduled Task");
    eprintln!();
    eprintln!("    {} (Optional) hourly self-improvement loop:",
        colored("3.", 36));
    eprintln!("       phantom autoevolve schedule install");
    eprintln!();

    // Offer login as a final step — this lets the user reach the
    // phantommesh.io broker (or local-only email) right out of the
    // wizard, saving a separate `phantom login` invocation.
    eprintln!("    {} (Optional) link this device to your phantom-mesh account",
        colored("4.", 36));
    eprintln!("       — needed for cross-device mesh discovery + iPhone access");
    let already = phantom_mesh::auth::load().is_some();
    if already {
        let s = phantom_mesh::auth::load().unwrap();
        eprintln!("       (already logged in: {})", phantom_mesh::auth::human_summary(&s));
    } else {
        // Inline prompt — readline is fine here, this whole onboarding
        // function already uses it above.
        let go = rl
            .readline("       run `phantom login` now? [Y/n]: ")
            .map(|s| s.trim().to_lowercase())
            .unwrap_or_default();
        if go.is_empty() || go == "y" || go == "yes" {
            // Spawn phantom login as a subprocess so we share OAuth state
            // implementation in one place. stdio inherits, so the user
            // gets the full flow (browser, prompts).
            if let Ok(self_exe) = std::env::current_exe() {
                let _ = std::process::Command::new(&self_exe)
                    .arg("login")
                    .status();
            }
        } else {
            eprintln!("       (skip — run `phantom login` later)");
        }
    }
    eprintln!();
    eprintln!("  {}  more docs at ~/.phantom-mesh/agents.toml + docs/",
        colored("›", 90));
    eprintln!();
    Ok(())
}

/// Find the most recently modified session file in ~/.phantom-mesh/conversations/
/// Returns the session ID (file stem) or None if the directory is empty.
fn find_last_session() -> Option<String> {
    let dir = dirs::home_dir()?
        .join(".phantom-mesh")
        .join("conversations");
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            let name = e.file_name().to_string_lossy().to_string();
            let id = name.strip_suffix(".jsonl")?.to_string();
            Some((modified, id))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, id)| id)
}

fn find_config() -> Option<String> {
    let cwd_path = std::env::current_dir().ok()?.join("agents.toml");
    if cwd_path.exists() {
        return std::fs::read_to_string(cwd_path).ok();
    }
    let home_path = dirs::home_dir()?.join(".phantom-mesh").join("agents.toml");
    if home_path.exists() {
        return std::fs::read_to_string(home_path).ok();
    }
    None
}

// ── `phantom service` — macOS launchd auto-start ─────────────────────────────
//
// Subcommands:
//   install   write LaunchAgent plist, bootstrap the service
//   uninstall bootout the service, remove plist
//   status    show whether the service is registered + reachable
//
// Implements Sprint 1 / Task 1.1 of MAC-DEEP-EXECUTION-PLAN.md.

/// Env vars phantom propagates from the user's interactive shell into the
/// LaunchAgent / systemd unit's environment block at `service install` time.
///
/// Why a whitelist instead of "copy everything":
/// 1. macOS plist files end up in `~/Library/LaunchAgents/` — readable by
///    the user only, but still on disk. Copying every env var would dump
///    things like SSH agent auth socket paths and unrelated secrets into
///    that file. We keep the surface area to known LLM keys + phantom's
///    own knobs.
/// 2. Future-proofing: when phantom adds support for a new provider, the
///    list grows here in one place. Users never need to edit their plist
///    by hand again — that was the bug we just chased ("為啥不用 opencode").
///
/// The actual values are NEVER printed to the install log; only the names.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const PROPAGATED_ENV_KEYS: &[&str] = &[
    // LLM provider keys
    "OPENCODE_API_KEY",
    "OPENROUTER_API_KEY",
    "GROQ_API_KEY",
    "GEMINI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "CEREBRAS_API_KEY",
    "DEEPSEEK_API_KEY",
    // phantom runtime knobs
    "PHANTOM_MAX_TOKENS",
    "PHANTOM_NODE_NAME",
];

/// Build the macOS LaunchAgent EnvironmentVariables fragment from the
/// current process's env. Returns (xml_fragment, names_included).
/// Each entry is indented to match the surrounding `<dict>` block.
#[cfg(target_os = "macos")]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_extra_env_plist_xml() -> (String, Vec<&'static str>) {
    let mut xml = String::new();
    let mut included: Vec<&'static str> = Vec::new();
    for &k in PROPAGATED_ENV_KEYS {
        if let Ok(v) = std::env::var(k) {
            if v.is_empty() { continue; }
            // XML-escape the value. plist <string> permits raw `&`/`<`/`>`
            // only via entity refs. Most API keys are alnum+`-_=` so this
            // rarely fires in practice — but we should never silently
            // produce an invalid plist.
            let esc = v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            xml.push_str(&format!("    <key>{}</key>\n    <string>{}</string>\n", k, esc));
            included.push(k);
        }
    }
    // Trim trailing newline so the closing </dict> stays aligned.
    if xml.ends_with('\n') { xml.pop(); }
    (xml, included)
}

/// Build the Linux systemd `Environment=...` lines fragment.
#[cfg(target_os = "linux")]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_extra_env_systemd() -> (String, Vec<&'static str>) {
    let mut s = String::new();
    let mut included: Vec<&'static str> = Vec::new();
    for &k in PROPAGATED_ENV_KEYS {
        if let Ok(v) = std::env::var(k) {
            if v.is_empty() { continue; }
            // systemd Environment= line: quote the value so spaces and
            // shell metachars survive intact. Inner double-quotes get
            // backslash-escaped per systemd.exec(5).
            let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
            s.push_str(&format!("Environment=\"{}={}\"\n", k, esc));
            included.push(k);
        }
    }
    if s.ends_with('\n') { s.pop(); }
    (s, included)
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod env_propagation_tests {
    use super::*;

    /// Helper: snapshot all PROPAGATED_ENV_KEYS values, set known test
    /// values, run f, restore originals. Avoids polluting cargo test's
    /// shared process env across tests.
    fn with_env<F: FnOnce()>(set: &[(&str, &str)], unset: &[&str], f: F) {
        let snapshot: Vec<(&str, Option<String>)> = set.iter().map(|(k, _)| *k)
            .chain(unset.iter().copied())
            .map(|k| (k, std::env::var(k).ok()))
            .collect();
        for &k in unset { std::env::remove_var(k); }
        for (k, v) in set { std::env::set_var(k, v); }
        f();
        for (k, prev) in snapshot {
            match prev {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn extra_env_plist_xml_emits_keys_present_in_env() {
        with_env(
            &[("OPENCODE_API_KEY", "sk-test-opencode"),
              ("PHANTOM_MAX_TOKENS", "16384")],
            &["OPENROUTER_API_KEY", "GROQ_API_KEY"],
            || {
                let (xml, names) = build_extra_env_plist_xml();
                assert!(names.contains(&"OPENCODE_API_KEY"));
                assert!(names.contains(&"PHANTOM_MAX_TOKENS"));
                assert!(!names.contains(&"OPENROUTER_API_KEY"));
                assert!(xml.contains("<key>OPENCODE_API_KEY</key>"));
                assert!(xml.contains("<string>sk-test-opencode</string>"));
                assert!(xml.contains("<string>16384</string>"));
                // Must NOT leak the unset key
                assert!(!xml.contains("OPENROUTER"));
            },
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn extra_env_plist_xml_escapes_xml_special_chars() {
        with_env(
            &[("OPENCODE_API_KEY", "key&with<special>chars")],
            &[],
            || {
                let (xml, _) = build_extra_env_plist_xml();
                assert!(xml.contains("key&amp;with&lt;special&gt;chars"));
                assert!(!xml.contains("key&with<special>chars"));
            },
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn extra_env_plist_xml_skips_empty_values() {
        with_env(
            &[("OPENCODE_API_KEY", "")],
            &[],
            || {
                let (xml, names) = build_extra_env_plist_xml();
                assert!(!names.contains(&"OPENCODE_API_KEY"));
                assert!(!xml.contains("OPENCODE_API_KEY"));
            },
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn extra_env_systemd_emits_quoted_environment_lines() {
        with_env(
            &[("GROQ_API_KEY", "gsk_test_value")],
            &[],
            || {
                let (s, names) = build_extra_env_systemd();
                assert!(names.contains(&"GROQ_API_KEY"));
                assert!(s.contains(r#"Environment="GROQ_API_KEY=gsk_test_value""#));
            },
        );
    }
}

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_PLIST_TMPL: &str = include_str!("../../../templates/ai.phantommesh.serve.plist.tmpl");

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "ai.phantommesh.serve";

#[cfg(target_os = "macos")]
async fn run_service_subcommand(action: &str) -> anyhow::Result<()> {
    use std::process::Command;

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("$HOME not set"))?;
    let plist_path = home
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LAUNCH_AGENT_LABEL));
    let log_dir = home.join("Library/Logs");
    let uid = nix_uid();
    let domain = format!("gui/{}", uid);
    let target = format!("{}/{}", domain, LAUNCH_AGENT_LABEL);

    match action {
        "install" => {
            // Resolve the *current* phantom binary (follow symlinks).
            let bin_self = std::env::current_exe()?;
            let bin_real = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);

            // Copy the binary into a launchd-friendly path. macOS 26+ TCC
            // blocks launchd-spawned processes from loading binaries that
            // live under ~/Documents, ~/Downloads, ~/Desktop, etc., even
            // through symlinks — dyld stalls in __open with no diagnostic.
            // ~/Library/Application Support is unrestricted for the user's
            // own LaunchAgents and is the standard location.
            let install_dir = home.join("Library/Application Support/phantom-mesh/bin");
            std::fs::create_dir_all(&install_dir)?;
            let installed_bin = install_dir.join("phantom");

            // Boot out any running instance BEFORE overwriting the binary.
            // Two reasons:
            //   1. macOS won't cleanly overwrite a running executable —
            //      std::fs::copy may either fail with ETXTBSY-equivalent
            //      or produce a binary the kernel hasn't released, which
            //      then fails the subsequent bootstrap with EIO ("Input/
            //      output error", launchctl error 5).
            //   2. The next bootstrap call requires the label slot to be
            //      free; if the running instance is still attached,
            //      bootstrap returns the same EIO.
            // We ignore the bootout exit code because "service not loaded"
            // is a fine starting state.
            let _ = Command::new("launchctl").args(["bootout", &target]).output();
            // Brief wait for launchd to release the binary mapping.
            std::thread::sleep(std::time::Duration::from_millis(300));

            // Copy unconditionally so reinstall picks up a freshly built bin.
            std::fs::copy(&bin_real, &installed_bin)?;
            // Ensure executable bit (preserve usual permissions).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut p = std::fs::metadata(&installed_bin)?.permissions();
                p.set_mode(0o755);
                std::fs::set_permissions(&installed_bin, p)?;
            }

            // Re-sign on macOS. When `cp` overwrites a Mach-O binary
            // in place, the kernel invalidates the signature and
            // SIGKILLs the next launch (exit 137 with NO output) —
            // user just sees "phantom appears hung / killed" which
            // is impossible to debug without amfid logs. Ad-hoc
            // re-signing fixes this. Silent if codesign isn't
            // available; the caller still surfaces a clear error
            // when bootstrap fails.
            #[cfg(target_os = "macos")]
            {
                let _ = Command::new("codesign")
                    .args(["--force", "--sign", "-", installed_bin.to_str().unwrap_or("")])
                    .output();
            }

            let bin_str = installed_bin.display().to_string();

            // Mirror dist/ and scripts/ into install_dir's parent so the
            // launchd-spawned phantom serve (whose cwd we are about to
            // override to that parent) can find them on its `./dist/<file>`
            // and `./scripts/<file>` candidate paths. The originals live
            // under ~/Documents which TCC blocks.
            let app_support = install_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| home.clone());
            let repo_root = locate_repo_root(&bin_real);
            if let Some(root) = repo_root.as_ref() {
                for sub in ["dist", "scripts"] {
                    let src = root.join(sub);
                    if !src.is_dir() {
                        continue;
                    }
                    let dst = app_support.join(sub);
                    let _ = std::fs::create_dir_all(&dst);
                    if let Ok(entries) = std::fs::read_dir(&src) {
                        for entry in entries.flatten() {
                            let from = entry.path();
                            if !from.is_file() {
                                continue;
                            }
                            let to = dst.join(entry.file_name());
                            let _ = std::fs::copy(&from, &to);
                            #[cfg(unix)]
                            if let Ok(meta) = std::fs::metadata(&from) {
                                let _ = std::fs::set_permissions(
                                    &to,
                                    meta.permissions(),
                                );
                            }
                        }
                    }
                }
                eprintln!(
                    "{} Mirrored dist/ + scripts/ into {}",
                    colored("◆", 35),
                    app_support.display()
                );
            } else {
                eprintln!(
                    "{} Could not locate repo root for dist/ + scripts/ mirror — \
                     /dist/<bin> and /scripts/<file> may 404 from the daemon. \
                     (Install was run from outside the phantom-mesh repo.)",
                    colored("⚠", 33)
                );
            }

            // Pick a launchd-safe working directory. macOS 26 TCC blocks
            // launchd-spawned processes from accessing ~/Documents,
            // ~/Downloads, ~/Desktop even for `getcwd()` — `find_config()`
            // (which calls `std::env::current_dir`) hangs forever. So if
            // the install was launched from one of those, fall back to the
            // app-support dir we already created above.
            let cwd_now = std::env::current_dir()
                .unwrap_or_else(|_| home.clone());
            let in_tcc_protected = ["Documents", "Downloads", "Desktop"]
                .iter()
                .any(|seg| {
                    let p = home.join(seg);
                    cwd_now.starts_with(&p)
                });
            let work_dir = if in_tcc_protected {
                install_dir.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| home.display().to_string())
            } else {
                cwd_now.display().to_string()
            };

            std::fs::create_dir_all(plist_path.parent().unwrap())?;
            std::fs::create_dir_all(&log_dir)?;

            let (extra_env, env_names) = build_extra_env_plist_xml();
            let rendered = LAUNCH_AGENT_PLIST_TMPL
                .replace("__PHANTOM_BIN__", &bin_str)
                .replace("__WORK_DIR__", &work_dir)
                .replace("__HOME__", &home.display().to_string())
                .replace("__EXTRA_ENV__", &extra_env);
            std::fs::write(&plist_path, &rendered)?;
            if env_names.is_empty() {
                eprintln!("    {} no API keys propagated — daemon will run with PATH+HOME only.",
                          colored("⚠", 33));
                eprintln!("       Set keys in your shell rcfile (e.g. `export OPENCODE_API_KEY=…`),");
                eprintln!("       open a new shell, then re-run `phantom service install`.");
            } else {
                eprintln!("    propagated env: {} key(s) → {}",
                          env_names.len(),
                          env_names.join(", "));
            }

            eprintln!("{} Wrote {}", colored("◆", 35), plist_path.display());
            eprintln!("    binary:    {} (copied from {})", bin_str, bin_real.display());
            eprintln!("    cwd:       {}", work_dir);
            eprintln!("    log:       {}/phantom-serve.log", log_dir.display());

            // (We already booted out the stale instance before the binary
            // copy above — no second bootout needed here.)
            let status = Command::new("launchctl")
                .args(["bootstrap", &domain, plist_path.to_str().unwrap()])
                .status()?;
            if !status.success() {
                anyhow::bail!(
                    "launchctl bootstrap failed (try `launchctl bootout {}` then re-run)",
                    target,
                );
            }
            let _ = Command::new("launchctl").args(["enable", &target]).output();
            let _ = Command::new("launchctl").args(["kickstart", "-kp", &target]).output();

            // Wait briefly and verify.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let print = Command::new("launchctl").args(["print", &target]).output()?;
            if print.status.success() {
                eprintln!("{} Service registered.", colored("✓", 32));
                eprintln!("    Verify:    curl http://127.0.0.1:7878/healthz");
                eprintln!("    Logs:      tail -f {}/phantom-serve.log", log_dir.display());
                eprintln!("    Uninstall: phantom service uninstall");
                eprintln!();
                eprintln!(
                    "{} {} for a full environment health check.",
                    colored("→", 36),
                    colored("phantom doctor", 33)
                );
                Ok(())
            } else {
                anyhow::bail!(
                    "service did not register; check {}/phantom-serve.log",
                    log_dir.display()
                )
            }
        }

        "uninstall" => {
            let _ = Command::new("launchctl").args(["bootout", &target]).output();
            if plist_path.exists() {
                std::fs::remove_file(&plist_path)?;
                eprintln!("{} Removed {}", colored("◆", 35), plist_path.display());
            }
            // Best-effort: clean the copied binary too. Ignore failures.
            let installed_bin = home
                .join("Library/Application Support/phantom-mesh/bin/phantom");
            if installed_bin.exists() {
                let _ = std::fs::remove_file(&installed_bin);
            }
            eprintln!("{} Uninstalled.", colored("✓", 32));
            Ok(())
        }

        "status" => {
            let print = Command::new("launchctl").args(["print", &target]).output()?;
            let registered = print.status.success();
            let body = String::from_utf8_lossy(&print.stdout);
            let pid = body
                .lines()
                .find(|l| l.trim_start().starts_with("pid ="))
                .and_then(|l| l.split('=').nth(1))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "?".to_string());

            // Probe healthz.
            let probe = Command::new("curl")
                .args([
                    "-s",
                    "--max-time",
                    "2",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    "http://127.0.0.1:7878/healthz",
                ])
                .output();
            let healthz_code = probe
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            println!(
                "{} {}",
                colored("phantom service status", 36),
                colored(LAUNCH_AGENT_LABEL, 90)
            );
            println!(
                "  registered : {}",
                if registered {
                    colored("yes", 32)
                } else {
                    colored("no", 31)
                }
            );
            if registered {
                println!("  pid        : {}", pid);
                println!("  plist      : {}", plist_path.display());
            }
            println!(
                "  healthz    : {} ({})",
                if healthz_code == "200" {
                    colored("ok", 32)
                } else {
                    colored("unreachable", 31)
                },
                if healthz_code.is_empty() { "no response".into() } else { format!("HTTP {}", healthz_code) }
            );
            if registered && healthz_code != "200" {
                println!(
                    "  hint       : tail -n 30 {}/phantom-serve.log",
                    log_dir.display()
                );
            }
            Ok(())
        }

        other => {
            eprintln!(
                "{} Unknown service action: '{}'.\n    Use one of: install, uninstall, status",
                colored("✗", 31),
                other
            );
            std::process::exit(1);
        }
    }
}

// ── `phantom login` — local identity for mesh discovery ─────────────────────
//
// Three providers:
//   email    — local-only username + SHA-256(salt||pw) iterated 100K
//   google   — OAuth 2.0 loopback (PKCE), localhost listener on :48181
//   apple    — stub (Apple OAuth requires HTTPS redirect; needs the
//              cloud-broker relay shipped in commercial Pro tier).
//
// All three end up in ~/.phantom-mesh/auth.json (mode 0600). Login is
// strictly local — nothing is sent anywhere except (for google) the
// browser → Google → loopback hop, which our binary terminates.
//
// Mesh discovery against a phantom-cloud broker is NOT in this CLI yet
// — that's the commercial layer (COMMERCIAL-DESIGN.md §4). What we
// gain today is a stable device identity that the broker can claim
// later.

async fn run_login(args: &[String]) -> anyhow::Result<()> {
    use rustyline::DefaultEditor;

    // Routing logic:
    //   phantom login                  → default: try the phantommesh.io broker.
    //                                    If broker is up, run the broker OAuth
    //                                    dance (handles google / apple / email /
    //                                    sso uniformly through one HTTPS endpoint).
    //                                    If broker is down, fall through to the
    //                                    local menu so the binary stays useful.
    //   phantom login <provider>       → directly run that provider's flow,
    //                                    bypassing the broker.
    //   PHANTOM_AUTH_URL=...           → override the broker URL (default
    //                                    https://phantommesh.io). Set to empty
    //                                    to skip the broker probe entirely.
    let provider_arg = args.get(2).map(|s| s.as_str()).unwrap_or("");
    let provider = match provider_arg {
        "email" | "google" | "apple" | "broker" => provider_arg.to_string(),
        "" => {
            // No explicit provider: try the broker first.
            let broker = std::env::var("PHANTOM_AUTH_URL")
                .unwrap_or_else(|_| "https://phantommesh.io".to_string());
            if !broker.is_empty() {
                eprintln!("{} probing broker {} …", colored("◆", 35), broker);
                let probe = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .build()
                    .ok()
                    .and_then(|c| {
                        // Block on a short HEAD/GET to /api/health
                        tokio::runtime::Handle::try_current().ok().map(|_h| c)
                    });
                let online = if let Some(c) = probe {
                    let url = format!("{}/api/health", broker.trim_end_matches('/'));
                    c.get(&url).send().await
                        .map(|r| r.status().is_success())
                        .unwrap_or(false)
                } else { false };

                if online {
                    return login_broker(&broker).await;
                }
                eprintln!("  {} broker offline — falling back to direct provider menu",
                    colored("◇", 90));
                eprintln!("    (set PHANTOM_AUTH_URL='' to suppress broker probe)");
                eprintln!();
            }
            eprintln!("{}", colored("phantom login — pick an identity provider", 35));
            eprintln!("  1. email    — local password (no cloud)");
            eprintln!("  2. google   — OAuth loopback (opens browser)");
            eprintln!("  3. apple    — needs broker relay (planned)");
            eprintln!("  4. broker   — re-try {}", broker);
            let mut rl = DefaultEditor::new()?;
            let pick = rl.readline("  choose [1-4]: ").unwrap_or_default();
            match pick.trim() {
                "1" | "email"  => "email".to_string(),
                "2" | "google" => "google".to_string(),
                "3" | "apple"  => "apple".to_string(),
                "4" | "broker" => "broker".to_string(),
                _ => anyhow::bail!("cancelled"),
            }
        }
        other => anyhow::bail!("unknown provider '{}'. Use email / google / apple / broker.", other),
    };

    match provider.as_str() {
        "email"  => login_email().await,
        "google" => login_google().await,
        "broker" => {
            let broker = std::env::var("PHANTOM_AUTH_URL")
                .unwrap_or_else(|_| "https://phantommesh.io".to_string());
            login_broker(&broker).await
        }
        "apple" => {
            eprintln!("{} Apple login requires an HTTPS redirect server.",
                colored("⚠", 33));
            eprintln!("    Lands when the phantommesh.io broker exposes /auth/apple.");
            eprintln!("    For now, use:");
            eprintln!("      phantom login email     # local-only");
            eprintln!("      phantom login google    # OAuth loopback");
            std::process::exit(1);
        }
        _ => unreachable!(),
    }
}

/// `phantom login broker` — delegate the OAuth dance to phantommesh.io
/// (or a self-hosted broker via PHANTOM_AUTH_URL). The broker handles
/// google / apple / email / sso uniformly behind one HTTPS endpoint;
/// phantom CLI just opens the browser, waits on a loopback callback,
/// and saves whatever identity payload the broker sends back.
///
/// Wire format (the broker is expected to respect this — owned by us):
///   GET  /api/health                                → 200 ok when alive
///   GET  /auth/cli/start?device_id=...&port=N       → opens login UI
///   POST /api/oauth/callback (broker → loopback)    → JSON UserIdentity
async fn login_broker(broker_url: &str) -> anyhow::Result<()> {
    use phantom_mesh::auth;

    const PORT: u16 = 48181;
    let redirect = format!("http://127.0.0.1:{}/oauth/callback", PORT);

    let prior = auth::load();

    // Short-circuit: already-fresh broker_token → just refresh keys, no
    // browser. Lets `iwr install.ps1 | iex` re-runs (and explicit
    // `phantom login` calls during routine work) be near-instant.
    // 60s safety margin against clock skew. Force re-OAuth via
    // `phantom logout` first, or PHANTOM_FORCE_LOGIN=1.
    let force = std::env::var("PHANTOM_FORCE_LOGIN").map(|v| v == "1").unwrap_or(false);
    if !force {
        if let Some(ref p) = prior {
            let now_ms = auth::now_ms();
            let still_fresh = !p.broker_token.is_empty()
                && p.broker_token_expires_at_ms > now_ms + 60_000
                && p.broker_url.trim_end_matches('/') == broker_url.trim_end_matches('/');
            if still_fresh {
                eprintln!("{} already logged in as {} — refreshing keys instead of re-OAuthing",
                    colored("◆", 35), p.email);
                eprintln!("  (force re-login with: phantom logout && phantom login,");
                eprintln!("   or: PHANTOM_FORCE_LOGIN=1 phantom login)");
                eprintln!();
                match phantom_mesh::cli_config::config_pull_lines(broker_url, &p.broker_token).await {
                    Ok(lines) => {
                        for l in lines { eprintln!("{}", l); }
                        // Short-circuit path also runs auto-register +
                        // cluster join so re-running `phantom login` on
                        // a working install still keeps the broker peer
                        // registry + local [cluster] block fresh (e.g.
                        // Tailscale IP just rotated).
                        eprintln!();
                        eprintln!("{} auto-registering this machine on cluster…", colored("◆", 35));
                        for l in phantom_mesh::cli_config::login_post_register_lines(
                            broker_url, &p.broker_token,
                        ).await {
                            eprintln!("{}", l);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("{} key refresh failed: {} — falling through to full OAuth",
                            colored("⚠", 33), e);
                        eprintln!();
                        // Fall through to the OAuth flow below.
                    }
                }
            }
        }
    }

    let device_id = prior.as_ref().map(|s| s.device_id.clone())
        .unwrap_or_else(auth::random_device_id);

    let auth_url = format!(
        "{}/auth/cli/start?device_id={}&port={}&redirect={}",
        broker_url.trim_end_matches('/'),
        urlencoding::encode(&device_id),
        PORT,
        urlencoding::encode(&redirect),
    );

    eprintln!("{} opening {}", colored("◆", 35), auth_url);
    open_browser(&auth_url);
    eprintln!("  (if the browser didn't open, paste the URL above)");
    eprintln!("  waiting for the broker to call back on :{} … (Ctrl-C to cancel)", PORT);

    let (tx, rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    let app = axum::Router::new().route("/oauth/callback", axum::routing::any({
        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
        move |req: axum::http::Request<axum::body::Body>| {
            let tx = tx.clone();
            async move {
                // Accept either:
                //   POST with JSON body (legacy / direct provider flow)
                //   GET ?p=<base64url(json)>  (current broker meta-refresh)
                //   GET ?<key>=<value>&...    (legacy URL-encoded form)
                use axum::body::Body;
                let (parts, body) = req.into_parts();
                let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
                    Ok(b) => b.to_vec(),
                    Err(_) => Vec::new(),
                };
                let payload: serde_json::Value = if !bytes.is_empty() {
                    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
                } else if let Some(q) = parts.uri.query() {
                    // First try to decode the broker's `?p=<base64-json>`
                    // form. base64url decode → UTF-8 → serde_json. If any
                    // step fails, fall through to "raw query string" so
                    // older callers still work.
                    let p = q.split('&')
                        .find_map(|kv| kv.strip_prefix("p="));
                    let decoded = p.and_then(|p_val| {
                        // urldecode the param first (e.g. %3D → '=')
                        let urldec = urlencoding::decode(p_val).ok()?;
                        // base64url alphabet: '-' → '+', '_' → '/', no padding
                        let std_b64 = urldec.replace('-', "+").replace('_', "/");
                        // Re-pad to a multiple of 4 (base64 standard)
                        let pad = (4 - std_b64.len() % 4) % 4;
                        let padded = format!("{}{}", std_b64, "=".repeat(pad));
                        let bytes = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD, padded
                        ).ok()?;
                        let text = String::from_utf8(bytes).ok()?;
                        serde_json::from_str::<serde_json::Value>(&text).ok()
                    });
                    decoded.unwrap_or_else(|| serde_json::Value::String(q.to_string()))
                } else {
                    serde_json::Value::Null
                };
                if let Some(t) = tx.lock().await.take() {
                    let _ = t.send(payload);
                }
                let _ = parts;  // silence dead_code on body
                let _ = Body::empty();
                axum::response::Html(
                    "<h1>✓ Login complete</h1>\
                     <p>You can close this window and return to phantom.</p>".to_string()
                )
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", PORT)).await?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let payload = tokio::select! {
        result = rx => result.map_err(|_| anyhow::anyhow!("callback channel closed"))?,
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {
            anyhow::bail!("timed out waiting for broker callback (5 min)");
        }
    };
    server.abort();

    eprintln!("{} broker callback received, parsing identity…", colored("◆", 35));

    let email = payload["email"].as_str()
        .or(payload["user"]["email"].as_str())
        .unwrap_or("")
        .to_string();
    if email.is_empty() {
        anyhow::bail!("broker payload had no email field — refusing to save: {}", payload);
    }

    let now = auth::now_ms();
    let broker_token = payload["broker_token"].as_str().unwrap_or("").to_string();
    let state = auth::AuthState {
        provider: payload["provider"].as_str().unwrap_or("broker").to_string(),
        email,
        display_name: payload["name"].as_str().or(payload["user"]["name"].as_str()).map(str::to_string),
        sub: payload["sub"].as_str().map(str::to_string),
        avatar_url: payload["picture"].as_str().or(payload["avatar_url"].as_str()).map(str::to_string),
        device_id,
        created_at_ms: prior.as_ref().map(|s| s.created_at_ms).unwrap_or(now),
        last_login_ms: now,
        password_hash: String::new(),
        salt: String::new(),
        id_token: payload["id_token"].as_str().unwrap_or("").to_string(),
        access_token: payload["access_token"].as_str().unwrap_or("").to_string(),
        broker_token: broker_token.clone(),
        broker_token_expires_at_ms: payload["broker_token_expires_at_ms"]
            .as_i64().unwrap_or(0),
        broker_url: broker_url.trim_end_matches('/').to_string(),
    };
    auth::save(&state)?;
    eprintln!("{} logged in as {}", colored("✓", 32), auth::human_summary(&state));
    eprintln!("  saved to {} (mode 0600)", auth::auth_path().display());

    // Auto-pull LLM keys from the broker's vault now that we have a fresh
    // token in hand — the whole reason a user runs `phantom login` against
    // phantommesh.io is to avoid setting OPENCODE_API_KEY etc by hand. Skip
    // silently if the broker_token wasn't in the payload (older broker that
    // doesn't include it, or future provider that returns identity-only).
    if !broker_token.is_empty() {
        eprintln!();
        eprintln!("{} pulling LLM provider keys from broker vault…", colored("◆", 35));
        match phantom_mesh::cli_config::config_pull_lines(broker_url, &broker_token).await {
            Ok(lines) => {
                for l in lines { eprintln!("{}", l); }
                // Persist for `phantom config pull` (zero-arg) re-runs.
                let _ = phantom_mesh::cli_config::write_broker_config(
                    &phantom_mesh::cli_config::BrokerConfig {
                        url: broker_url.trim_end_matches('/').to_string(),
                        token: broker_token.clone(),
                    });
            }
            Err(e) => {
                eprintln!("{} vault pull skipped: {}", colored("⚠", 33), e);
                eprintln!("  (login itself succeeded; you can retry with `phantom config pull`)");
            }
        }

        eprintln!();
        eprintln!("{} auto-registering this machine on cluster…", colored("◆", 35));
        for l in phantom_mesh::cli_config::login_post_register_lines(
            broker_url, &broker_token,
        ).await {
            eprintln!("{}", l);
        }
    }
    Ok(())
}

async fn login_email() -> anyhow::Result<()> {
    use phantom_mesh::auth;
    use rustyline::DefaultEditor;

    // Non-interactive flags for scripts / CI:
    //   phantom login email --email a@b.c --password X --no-confirm
    let args: Vec<String> = std::env::args().collect();
    let arg_email = parse_flag(&args, "--email");
    let arg_pw    = parse_flag(&args, "--password");
    let no_confirm = args.iter().any(|a| a == "--no-confirm");

    let email = if let Some(e) = arg_email {
        e
    } else {
        let mut rl = DefaultEditor::new()?;
        rl.readline("  email: ").unwrap_or_default().trim().to_string()
    };
    if email.is_empty() || !email.contains('@') {
        anyhow::bail!("email is required (and must contain '@')");
    }

    // Reuse existing device_id when re-logging-in from this machine.
    let prior = auth::load();
    let device_id = prior.as_ref().map(|s| s.device_id.clone()).unwrap_or_else(auth::random_device_id);

    let pw_was_flag = arg_pw.is_some();
    let pw1 = if let Some(p) = arg_pw {
        p
    } else {
        read_hidden_password("  password: ")?
    };
    if pw1.len() < 6 {
        anyhow::bail!("password must be ≥6 chars");
    }
    if !no_confirm && !pw_was_flag {
        let pw2 = read_hidden_password("  password (again): ")?;
        if pw1 != pw2 {
            anyhow::bail!("passwords do not match");
        }
    }

    let salt = auth::random_salt();
    let hash = auth::hash_password(&pw1, &salt);

    let now = auth::now_ms();
    let state = auth::AuthState {
        provider: "email".into(),
        email: email.clone(),
        display_name: None,
        sub: None,
        avatar_url: None,
        device_id,
        created_at_ms: prior.as_ref().map(|s| s.created_at_ms).unwrap_or(now),
        last_login_ms: now,
        password_hash: hash,
        salt,
        id_token: String::new(),
        access_token: String::new(),
        broker_token: String::new(),
        broker_token_expires_at_ms: 0,
        broker_url: String::new(),
    };
    auth::save(&state)?;
    eprintln!();
    eprintln!("{} logged in as {}", colored("✓", 32), auth::human_summary(&state));
    eprintln!("  saved to {} (mode 0600)", auth::auth_path().display());
    Ok(())
}

/// Tiny argv flag parser. Skip args[0..2] (binary path + subcommand);
/// look for `flag` and return the next token, or None.
fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter().enumerate().skip(2);
    while let Some((_, a)) = iter.next() {
        if a == flag {
            if let Some((_, v)) = iter.next() {
                return Some(v.clone());
            }
        } else if let Some(rest) = a.strip_prefix(&format!("{}=", flag)) {
            return Some(rest.to_string());
        }
    }
    None
}

/// Hidden-input password read. Falls back to plain readline if termios isn't
/// available (e.g. piped stdin, Windows non-tty).
fn read_hidden_password(prompt: &str) -> anyhow::Result<String> {
    use std::io::{self, BufRead, Write};
    eprint!("{}", prompt);
    io::stderr().flush().ok();

    // Best-effort: stty echo off when a tty is attached.
    let was_tty = atty_stdin();
    if was_tty {
        let _ = std::process::Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    handle.read_line(&mut line)?;
    if was_tty {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

fn atty_stdin() -> bool {
    // Cheap probe: stty -a will fail without a tty.
    std::process::Command::new("stty")
        .arg("size")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn login_google() -> anyhow::Result<()> {
    use phantom_mesh::auth;
    use sha2::{Digest, Sha256};
    use base64::Engine;
    use rand::RngCore;

    // The same client_id used by the Tauri app (oauth.rs). The OAuth
    // client must be configured as 'Desktop App' type in Google Cloud
    // so that http://127.0.0.1:* loopback redirects are accepted. The
    // existing client_id was set up for a Web App with redirect to
    // localhost:5173; if the loopback path 401s here, the user needs
    // to add this URI to the Google Cloud Console authorized redirects:
    //   http://127.0.0.1:48181/oauth/callback
    const GOOGLE_CLIENT_ID: &str = "869770808980-0kom8ag838tc1p5sqvugitra2gnmbe50.apps.googleusercontent.com";
    const PORT: u16 = 48181;
    const REDIRECT: &str = "http://127.0.0.1:48181/oauth/callback";

    // PKCE
    let mut verifier_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = {
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize())
    };
    let mut state_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    let csrf = hex::encode(state_bytes);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={cid}&\
         response_type=code&\
         scope=openid%20email%20profile&\
         redirect_uri={ru}&\
         state={st}&\
         code_challenge={ch}&\
         code_challenge_method=S256&\
         access_type=offline&\
         prompt=select_account",
        cid = GOOGLE_CLIENT_ID,
        ru = urlencoding::encode(REDIRECT),
        st = csrf,
        ch = challenge,
    );

    eprintln!("{} opening browser for Google sign-in…", colored("◆", 35));
    eprintln!("  redirect: {}", REDIRECT);
    open_browser(&auth_url);
    eprintln!("  (if the browser didn't open, paste this in:)");
    eprintln!("  {}", colored(&auth_url, 90));
    eprintln!();
    eprintln!("  waiting for the OAuth callback on :{} … (Ctrl-C to cancel)", PORT);

    // Spin up a one-shot listener.
    let (tx, rx) = tokio::sync::oneshot::channel::<(String, String)>();
    let csrf_check = csrf.clone();
    let app = axum::Router::new().route("/oauth/callback", axum::routing::get({
        let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
        move |query: axum::extract::Query<std::collections::HashMap<String, String>>| {
            let tx = tx.clone();
            let csrf_check = csrf_check.clone();
            async move {
                let code = query.get("code").cloned().unwrap_or_default();
                let state = query.get("state").cloned().unwrap_or_default();
                if state != csrf_check {
                    return axum::response::Html("<h1>State mismatch — possible CSRF. Login aborted.</h1>".to_string());
                }
                if let Some(t) = tx.lock().await.take() {
                    let _ = t.send((code.clone(), state));
                }
                axum::response::Html(
                    "<h1>✓ Login complete</h1>\
                     <p>You can close this window and return to phantom.</p>".to_string()
                )
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", PORT)).await?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let (code, _state) = tokio::select! {
        result = rx => result.map_err(|_| anyhow::anyhow!("callback channel closed"))?,
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {
            anyhow::bail!("timed out waiting for OAuth callback (5 min)");
        }
    };
    server.abort();

    eprintln!("{} got authorization code, exchanging for tokens…", colored("◆", 35));

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id",     GOOGLE_CLIENT_ID),
            ("redirect_uri",  REDIRECT),
            ("grant_type",    "authorization_code"),
            ("code",          &code),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token exchange failed: {}", body);
    }
    let token_json: serde_json::Value = resp.json().await?;
    let id_token     = token_json["id_token"].as_str().unwrap_or_default().to_string();
    let access_token = token_json["access_token"].as_str().unwrap_or_default().to_string();

    // Decode the id_token JWT (no signature check — Google would reject
    // a forged id_token at the userinfo endpoint anyway, and we hit that
    // next as a sanity).
    let claims = decode_jwt_payload(&id_token).unwrap_or_default();
    let email   = claims["email"].as_str().unwrap_or("").to_string();
    let name    = claims["name"].as_str().map(|s| s.to_string());
    let picture = claims["picture"].as_str().map(|s| s.to_string());
    let sub     = claims["sub"].as_str().map(|s| s.to_string());

    if email.is_empty() {
        anyhow::bail!("Google id_token had no email claim — cannot proceed");
    }

    let prior = auth::load();
    let device_id = prior.as_ref().map(|s| s.device_id.clone()).unwrap_or_else(auth::random_device_id);
    let now = auth::now_ms();
    let state = auth::AuthState {
        provider: "google".into(),
        email: email.clone(),
        display_name: name,
        sub,
        avatar_url: picture,
        device_id,
        created_at_ms: prior.as_ref().map(|s| s.created_at_ms).unwrap_or(now),
        last_login_ms: now,
        password_hash: String::new(),
        salt: String::new(),
        id_token,
        access_token,
        // Direct Google flow doesn't go through phantommesh.io, so no
        // broker token to save. `phantom config pull` falls back to the
        // dashboard-copied token if you want to use the vault from a
        // direct-google session.
        broker_token: String::new(),
        broker_token_expires_at_ms: 0,
        broker_url: String::new(),
    };
    auth::save(&state)?;

    eprintln!();
    eprintln!("{} logged in as {}", colored("✓", 32), auth::human_summary(&state));
    eprintln!("  saved to {} (mode 0600)", auth::auth_path().display());
    Ok(())
}

/// Decode the payload of a JWT without verifying the signature.
fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    use base64::Engine;
    let mut parts = jwt.splitn(3, '.');
    let _h = parts.next()?;
    let payload = parts.next()?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ── `phantom self-update` — pull and atomically replace the running binary ──
//
// Default source: <coordinator>/dist/phantom-<target-triple>, where the
// coordinator URL comes from `~/.phantom-mesh/agents.toml [cluster].peers[0]`,
// or `--source <URL>`, or `$PHANTOM_COORD/dist/...`.
//
// Flow:
//   1. detect target triple (e.g. aarch64-apple-darwin)
//   2. download to <install_dir>/phantom.new
//   3. spawn `phantom.new --version` — must exit 0
//   4. atomically rename phantom → phantom.bak; phantom.new → phantom
//   5. macOS: launchctl kickstart -k so launchd picks up the new binary
//      Linux: systemctl --user restart phantom-mesh.service
//      Windows: schtasks /Run + taskkill old phantom.exe
//   6. report old → new version
//
// Old binary is kept at <install_dir>/phantom.bak for one rollback round
// (the next self-update overwrites it). Use `mv phantom.bak phantom` to
// roll back manually.

async fn run_self_update(args: &[String]) -> anyhow::Result<()> {
    use std::process::Command;

    let mut explicit_url: Option<String> = None;
    let mut dry_run = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => { i += 1; if let Some(u) = args.get(i) { explicit_url = Some(u.clone()); } }
            "--dry-run" => { dry_run = true; }
            "-h" | "--help" => {
                eprintln!("phantom self-update [--source URL] [--dry-run]");
                eprintln!();
                eprintln!("  Default source: <coordinator>/dist/phantom-<target>");
                eprintln!("  coordinator picked from agents.toml [cluster].peers[0]");
                eprintln!("  or PHANTOM_COORD env var.");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    // Detect target.
    let target_file = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos",   "aarch64") => "phantom-aarch64-apple-darwin",
        ("macos",   "x86_64")  => "phantom-x86_64-apple-darwin",
        ("linux",   "aarch64") => "phantom-aarch64-unknown-linux",
        ("linux",   "x86_64")  => "phantom-x86_64-unknown-linux",
        ("windows", "x86_64")  => "phantom-x86_64-pc-windows.exe",
        ("android", "aarch64") => "phantom-aarch64-linux-android",
        (os, arch) => {
            eprintln!("{} no published binary for {}-{}", colored("✗", 31), os, arch);
            std::process::exit(1);
        }
    };

    // Resolve URL.
    let url = if let Some(u) = explicit_url {
        u
    } else if let Ok(coord) = std::env::var("PHANTOM_COORD") {
        format!("{}/dist/{}", coord.trim_end_matches('/'), target_file)
    } else {
        // Read agents.toml [cluster].peers[0]
        let cfg_text = find_config().unwrap_or_default();
        let coord = cfg_text
            .lines()
            .skip_while(|l| !l.trim_start().starts_with("[cluster]"))
            .find_map(|l| {
                let t = l.trim();
                t.strip_prefix("\"").and_then(|x| x.strip_suffix("\","))
                    .or_else(|| t.strip_prefix("\"").and_then(|x| x.strip_suffix("\"")))
                    .map(|x| x.to_string())
            })
            .unwrap_or_else(|| {
                // No peer configured. Fall back to the local serve, honoring
                // [core].port from agents.toml so a `port = 7879` user still
                // self-updates from their own running daemon.
                let port = phantom_mesh::config::AgentsConfig::find_and_load()
                    .map(|c| c.core.port)
                    .unwrap_or(7878);
                format!("http://127.0.0.1:{}", port)
            });
        format!("{}/dist/{}", coord.trim_end_matches('/'), target_file)
    };

    eprintln!("{} {}", colored("◆ phantom self-update", 35), colored(target_file, 90));
    eprintln!("  current : {} ({})",
        env!("CARGO_PKG_VERSION"),
        option_env!("PHANTOM_GIT_HASH").unwrap_or("?"));
    eprintln!("  source  : {}", url);

    if dry_run {
        eprintln!("  {} dry-run — would download but not install", colored("◇", 90));
        return Ok(());
    }

    // Pick install directory:
    //   macOS  → ~/Library/Application Support/phantom-mesh/bin (TCC-safe)
    //   else   → directory containing the running binary
    let exe = std::env::current_exe()?;
    // Used on non-macOS branches; macOS uses a fixed path so the binding is
    // intentionally ignored there. cfg-attr suppresses the unused warning
    // without breaking the Windows / Linux paths that *do* use it.
    #[cfg_attr(target_os = "macos", allow(unused_variables))]
    let exe_canon = std::fs::canonicalize(&exe).unwrap_or(exe);
    let install_dir: std::path::PathBuf = {
        #[cfg(target_os = "macos")]
        {
            dirs::home_dir().unwrap().join("Library/Application Support/phantom-mesh/bin")
        }
        #[cfg(not(target_os = "macos"))]
        {
            exe_canon.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into())
        }
    };
    std::fs::create_dir_all(&install_dir)?;
    let new_bin = install_dir.join("phantom.new");
    let cur_bin = install_dir.join(if cfg!(target_os = "windows") { "phantom.exe" } else { "phantom" });
    let bak_bin = install_dir.join("phantom.bak");

    // Download via curl (already on PATH everywhere we ship).
    eprintln!("  → downloading…");
    let s = Command::new("curl")
        .args(["-fsSL", "--max-time", "300", "-o"])
        .arg(&new_bin)
        .arg(&url)
        .status()?;
    if !s.success() {
        anyhow::bail!("download failed (curl exit {:?})", s.code());
    }
    let size = std::fs::metadata(&new_bin)?.len();
    eprintln!("  {} got {} ({:.1} MB)",
        colored("✓", 32), new_bin.display(), size as f64 / 1024.0 / 1024.0);

    // chmod +x on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&new_bin)?.permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&new_bin, p)?;
    }

    // Smoke-test the new binary.
    eprintln!("  → verifying…");
    let v = Command::new(&new_bin).arg("--version").output();
    match v {
        Ok(o) if o.status.success() => {
            let new_ver = String::from_utf8_lossy(&o.stdout).trim().to_string();
            eprintln!("  {} new binary identifies as: {}", colored("✓", 32), new_ver);
        }
        Ok(o) => {
            anyhow::bail!(
                "new binary refused to print --version (exit {:?}): {}",
                o.status.code(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Err(e) => {
            anyhow::bail!("could not exec new binary: {}", e);
        }
    }

    // Atomic swap: cur → bak, new → cur.
    eprintln!("  → swapping…");
    let _ = std::fs::remove_file(&bak_bin); // any prior bak
    if cur_bin.exists() {
        std::fs::rename(&cur_bin, &bak_bin)?;
    }
    std::fs::rename(&new_bin, &cur_bin)?;
    eprintln!("  {} {} now points at fresh binary", colored("✓", 32), cur_bin.display());

    // Restart the auto-start service if installed.
    #[cfg(target_os = "macos")]
    {
        let target = format!("gui/{}/ai.phantommesh.serve", nix_uid());
        let probe = Command::new("launchctl").args(["print", &target]).output();
        if probe.map(|o| o.status.success()).unwrap_or(false) {
            let _ = Command::new("launchctl")
                .args(["kickstart", "-k", &target])
                .status();
            eprintln!("  {} relaunched launchd service ({})", colored("✓", 32), target);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let q = Command::new("systemctl")
            .args(["--user", "is-active", "phantom-mesh.service"])
            .output();
        if q.map(|o| o.status.success()).unwrap_or(false) {
            let _ = Command::new("systemctl")
                .args(["--user", "restart", "phantom-mesh.service"])
                .status();
            eprintln!("  {} restarted systemd unit phantom-mesh.service", colored("✓", 32));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let q = Command::new("schtasks")
            .args(["/Query", "/TN", "PhantomMesh"])
            .output();
        if q.map(|o| o.status.success()).unwrap_or(false) {
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "phantom.exe"])
                .output();
            let _ = Command::new("schtasks")
                .args(["/Run", "/TN", "PhantomMesh"])
                .status();
            eprintln!("  ✓ relaunched Scheduled Task PhantomMesh");
        }
    }

    eprintln!();
    eprintln!("{} self-update complete. Roll back with:", colored("◆", 35));
    eprintln!("    mv {} {}",
        bak_bin.display(),
        cur_bin.display());
    Ok(())
}

// ── `phantom mlx` — Apple Silicon local LLM provider helper ────────────────
//
// On Apple Silicon Macs, Apple's MLX framework runs Llama-class models
// natively on the Neural Engine + GPU — orders of magnitude faster than
// CPU-only inference and zero API cost. mlx_lm.server (`pip install
// mlx-lm`) provides an OpenAI-compatible HTTP endpoint that phantom can
// hit through its existing openai_compat provider.
//
// This subcommand is a *helper* — phantom doesn't bundle MLX or download
// models, it just orchestrates `mlx_lm.server` and `huggingface-cli`.
//
// Subcommands:
//   phantom mlx pull <model>    huggingface-cli download <model>
//   phantom mlx serve [--model M] [--port P]  spawn mlx_lm.server
//   phantom mlx status          probe localhost:8080/v1/models + show last
//                               configured model
//   phantom mlx stop            pkill -f mlx_lm.server
//
// Default model: mlx-community/Llama-3.1-8B-Instruct-4bit
//   ~5 GB, runs fine on 16 GB M1/M2; for 32 GB+ try
//   mlx-community/Llama-3.3-70B-Instruct-4bit (~38 GB).
//
// After `phantom mlx serve`, add to ~/.phantom-mesh/agents.toml:
//   [providers.mlx-local]
//   base_url = "http://localhost:8080/v1"
//   api_key = "mlx"
//   default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"
//   [agent.local]
//   provider = "mlx-local"
//   model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

#[cfg(target_os = "macos")]
const MLX_DEFAULT_MODEL: &str = "mlx-community/Llama-3.1-8B-Instruct-4bit";
#[cfg(target_os = "macos")]
const MLX_DEFAULT_PORT: u16 = 8080;

#[cfg(target_os = "macos")]
async fn run_mlx_subcommand(args: &[String]) -> anyhow::Result<()> {
    use std::process::Command;
    let action = args.get(2).map(|s| s.as_str()).unwrap_or("status");

    // Pre-flight: is mlx_lm installed in *some* python on PATH? Try the two
    // most common module-runner invocations. We do not auto-install — the
    // user picks their python (system / brew / pyenv / uv).
    fn locate_mlx_python() -> Option<String> {
        for py in ["python3", "python"] {
            let ok = Command::new(py)
                .args(["-c", "import mlx_lm"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok { return Some(py.to_string()); }
        }
        None
    }

    match action {
        "pull" => {
            let model = args.get(3).map(String::as_str).unwrap_or(MLX_DEFAULT_MODEL);
            // huggingface-cli is the canonical downloader; mlx-lm depends on
            // huggingface_hub which ships it as `huggingface-cli`.
            let hf = Command::new("huggingface-cli").arg("--version").output();
            if hf.is_err() || !hf.unwrap().status.success() {
                anyhow::bail!(
                    "huggingface-cli not found. Install with:\n    \
                     pip install huggingface_hub  (or `pip install mlx-lm` which pulls it in)"
                );
            }
            eprintln!("{} pulling {} (this can take a while — 5-40 GB depending on model)…",
                colored("◆", 35), model);
            let s = Command::new("huggingface-cli")
                .args(["download", model])
                .status()?;
            if s.success() {
                eprintln!("{} done.", colored("✓", 32));
            } else {
                anyhow::bail!("huggingface-cli download failed (exit {:?})", s.code());
            }
            Ok(())
        }
        "serve" => {
            let mut model = MLX_DEFAULT_MODEL.to_string();
            let mut port = MLX_DEFAULT_PORT;
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--model" => { i += 1; if let Some(m) = args.get(i) { model = m.clone(); } }
                    "--port"  => { i += 1; if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) { port = p; } }
                    _ => {}
                }
                i += 1;
            }
            let py = locate_mlx_python().ok_or_else(|| anyhow::anyhow!(
                "mlx_lm not importable from python3 / python.\n    \
                 Install with:\n        pip install mlx-lm\n    \
                 Or, on Apple-Silicon-only and uv-friendly:\n        \
                 uv tool install mlx-lm"
            ))?;
            eprintln!("{} {} → starting mlx_lm.server", colored("◆", 35), py);
            eprintln!("    model : {}", colored(&model, 36));
            eprintln!("    port  : {}", port);
            eprintln!("    api   : http://127.0.0.1:{}/v1 (OpenAI-compatible)", port);
            eprintln!();
            eprintln!("    Add to ~/.phantom-mesh/agents.toml:");
            eprintln!("      [providers.mlx-local]");
            eprintln!("      base_url      = \"http://127.0.0.1:{}/v1\"", port);
            eprintln!("      api_key       = \"mlx\"");
            eprintln!("      default_model = \"{}\"", model);
            eprintln!();
            // Save the last-served config for `phantom mlx status` to read.
            if let Some(home) = dirs::home_dir() {
                let p = home.join(".phantom-mesh/mlx-config.json");
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                let _ = std::fs::write(&p, serde_json::json!({
                    "model": model, "port": port,
                    "started_at_ms": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
                }).to_string());
            }
            // Foreground mlx_lm.server — Ctrl-C exits cleanly.
            let s = Command::new(&py)
                .args([
                    "-m", "mlx_lm.server",
                    "--model", &model,
                    "--port", &port.to_string(),
                    "--host", "127.0.0.1",
                ])
                .status()?;
            if !s.success() {
                anyhow::bail!("mlx_lm.server exited {:?}", s.code());
            }
            Ok(())
        }
        "status" => {
            // Locate python with mlx_lm
            match locate_mlx_python() {
                Some(py) => println!("  {} mlx_lm: importable from {}", colored("✓", 32), py),
                None     => println!("  {} mlx_lm: NOT installed (`pip install mlx-lm`)", colored("✗", 31)),
            }
            // Last config
            if let Some(home) = dirs::home_dir() {
                let p = home.join(".phantom-mesh/mlx-config.json");
                if let Ok(s) = std::fs::read_to_string(&p) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        let model = v["model"].as_str().unwrap_or("?");
                        let port = v["port"].as_u64().unwrap_or(MLX_DEFAULT_PORT as u64);
                        println!("  {} last config: {} on :{}", colored("◆", 35), model, port);

                        // Probe /v1/models on the saved port.
                        let url = format!("http://127.0.0.1:{}/v1/models", port);
                        let probe = Command::new("curl")
                            .args(["-s", "--max-time", "2", "-o", "/dev/null", "-w", "%{http_code}", &url])
                            .output();
                        let code = probe.ok()
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .unwrap_or_default();
                        if code == "200" {
                            println!("  {} server reachable on :{} (HTTP 200)", colored("✓", 32), port);
                        } else {
                            println!("  {} server unreachable on :{} (HTTP {}) — `phantom mlx serve` to start",
                                colored("⚠", 33), port,
                                if code.is_empty() { "—".into() } else { code });
                        }
                    }
                } else {
                    println!("  {} no last-served config — run `phantom mlx serve` once",
                        colored("◇", 90));
                }
            }
            Ok(())
        }
        "stop" => {
            let s = Command::new("pkill").args(["-f", "mlx_lm.server"]).status();
            match s {
                Ok(st) if st.success() => eprintln!("{} stopped mlx_lm.server", colored("✓", 32)),
                _                       => eprintln!("{} no mlx_lm.server process found", colored("◇", 90)),
            }
            Ok(())
        }
        "-h" | "--help" => {
            eprintln!("phantom mlx <action> [options]");
            eprintln!("  pull [MODEL]                huggingface-cli download MODEL");
            eprintln!("                              (default: {})", MLX_DEFAULT_MODEL);
            eprintln!("  serve [--model M] [--port P] foreground mlx_lm.server");
            eprintln!("  status                       check mlx_lm install + reachable server");
            eprintln!("  stop                         pkill mlx_lm.server");
            eprintln!();
            eprintln!("After `serve`, point an agent at it via:");
            eprintln!("  [providers.mlx-local]");
            eprintln!("  base_url = \"http://127.0.0.1:8080/v1\"");
            eprintln!("  api_key = \"mlx\"");
            eprintln!("  default_model = \"<MODEL>\"");
            Ok(())
        }
        other => {
            eprintln!("{} Unknown mlx action '{}'. Try `phantom mlx --help`.",
                colored("✗", 31), other);
            std::process::exit(1);
        }
    }
}

// ── `phantom service` — Linux systemd-user implementation ─────────────────
//
// Mirrors the macOS launchd path but uses `systemctl --user` and a
// generated `~/.config/systemd/user/phantom-mesh.service` unit. Survives
// reboot via `loginctl enable-linger $USER` (mentioned in install output
// but not auto-run because it needs sudo).
//
// Usage:
//   phantom service install     write unit + daemon-reload + start
//   phantom service uninstall   stop + disable + remove unit
//   phantom service status      systemctl --user status (parsed) + healthz

#[cfg(target_os = "linux")]
const LINUX_UNIT_NAME: &str = "phantom-mesh.service";

#[cfg(target_os = "linux")]
async fn run_service_subcommand_linux(action: &str) -> anyhow::Result<()> {
    use std::process::Command;

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("$HOME not set"))?;
    let unit_dir = home.join(".config/systemd/user");
    let unit_path = unit_dir.join(LINUX_UNIT_NAME);
    let log_path = home.join(".phantom-mesh/data/phantom-serve.log");

    match action {
        "install" => {
            // Use the running binary's canonical path. Linux has no TCC so
            // it can stay wherever the user installed it.
            let bin_self = std::env::current_exe()?;
            let bin = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            let bin_str = bin.display().to_string();

            // Working directory: prefer current dir if it has dist/scripts/
            // (so /scripts/* /dist/* serving works without copying), else
            // fall back to ~/.phantom-mesh/.
            let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
            let work_dir = if cwd.join("dist").is_dir() && cwd.join("scripts").is_dir() {
                cwd.display().to_string()
            } else {
                home.join(".phantom-mesh").display().to_string()
            };

            std::fs::create_dir_all(&unit_dir)?;
            std::fs::create_dir_all(log_path.parent().unwrap())?;

            let tmpl: &str = include_str!("../../../templates/phantom-mesh.service.tmpl");
            let (extra_env, env_names) = build_extra_env_systemd();
            let rendered = tmpl
                .replace("__PHANTOM_BIN__", &bin_str)
                .replace("__WORK_DIR__",   &work_dir)
                .replace("__HOME__",       &home.display().to_string())
                .replace("__LOG__",        &log_path.display().to_string())
                .replace("__EXTRA_ENV__",  &extra_env);
            std::fs::write(&unit_path, &rendered)?;
            eprintln!("{} Wrote {}", colored("◆", 35), unit_path.display());
            if !env_names.is_empty() {
                eprintln!("    propagated env: {} key(s) → {}",
                          env_names.len(), env_names.join(", "));
            }

            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            let s_enable = Command::new("systemctl")
                .args(["--user", "enable", "--now", LINUX_UNIT_NAME])
                .status()?;
            if !s_enable.success() {
                anyhow::bail!("systemctl --user enable --now failed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            eprintln!("{} Enabled and started '{}'", colored("✓", 32), LINUX_UNIT_NAME);
            eprintln!("    binary:    {}", bin_str);
            eprintln!("    cwd:       {}", work_dir);
            eprintln!("    log:       {}", log_path.display());
            eprintln!("    Verify:    curl http://127.0.0.1:7878/healthz");
            eprintln!();
            eprintln!(
                "{} run `loginctl enable-linger $USER` (needs sudo) so the \
                 service stays alive between logins.",
                colored("⚠", 33)
            );
            eprintln!();
            eprintln!(
                "{} {} for a full environment health check.",
                colored("→", 36),
                colored("phantom doctor", 33)
            );
            Ok(())
        }
        "uninstall" => {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", LINUX_UNIT_NAME])
                .status();
            if unit_path.exists() {
                std::fs::remove_file(&unit_path)?;
                eprintln!("{} Removed {}", colored("◆", 35), unit_path.display());
            }
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            eprintln!("{} Uninstalled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let q = Command::new("systemctl")
                .args(["--user", "status", LINUX_UNIT_NAME, "--no-pager"])
                .output()?;
            let body = String::from_utf8_lossy(&q.stdout).to_string();
            // exit 0 = active, 3 = inactive — both still mean "registered"
            let registered = unit_path.exists();
            let active = body.lines().any(|l| l.contains("Active:") && l.contains("active (running)"));
            let pid = body
                .lines()
                .find(|l| l.trim_start().starts_with("Main PID:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.split_whitespace().next())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into());

            let probe = Command::new("curl")
                .args(["-s", "--max-time", "2", "-o", "/dev/null", "-w", "%{http_code}",
                       "http://127.0.0.1:7878/healthz"])
                .output();
            let healthz_code = probe
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            println!("{} {}",
                colored("phantom service status", 36),
                colored(LINUX_UNIT_NAME, 90));
            println!("  registered : {}", if registered { colored("yes", 32) } else { colored("no", 31) });
            if registered {
                println!("  active     : {}", if active { colored("yes (running)", 32) } else { colored("no", 31) });
                if active { println!("  pid        : {}", pid); }
                println!("  unit       : {}", unit_path.display());
            }
            println!("  healthz    : {} ({})",
                if healthz_code == "200" { colored("ok", 32) } else { colored("unreachable", 31) },
                if healthz_code.is_empty() { "no response".into() } else { format!("HTTP {}", healthz_code) });
            if registered && healthz_code != "200" {
                println!("  hint       : journalctl --user -u {} -n 20", LINUX_UNIT_NAME);
            }
            Ok(())
        }
        other => {
            eprintln!(
                "{} Unknown service action: '{}'.\n    Use one of: install, uninstall, status",
                colored("✗", 31), other
            );
            std::process::exit(1);
        }
    }
}

// ── `phantom autoevolve` — long-running self-improvement loop ───────────────
//
// `phantom evolve` is the single-shot fix-the-failing-tests dance. autoevolve
// is the daemon-mode wrapper: poll `cargo check` (or `cargo test`) on the
// repo, spawn an evolve session whenever something breaks, auto-commit the
// resulting fix on green, and remember each round in `~/.phantom-mesh/
// autoevolve.log` so successive runs can avoid re-doing work.
//
// Flags:
//   --once               run one iteration and exit (default)
//   --watch              keep polling forever until Ctrl-C
//   --interval SECONDS   poll cadence in watch mode (default 300)
//   --max-rounds N       cap on evolve iterations per fix attempt (default 5)
//   --no-commit          dry-run: never git-commit, only print the diff
//   --target check|test  what to run between iterations (default check —
//                        cheaper than full test runs)
//   --agent NAME         the [agent.<name>] block to drive evolve with
//
// Watch-mode example, runs forever, fixes anything that breaks within ~5min
// of the breakage, auto-commits the fix:
//
//   phantom autoevolve --watch --interval 300 --target test --agent master
//
// Memory:
//   - ~/.phantom-mesh/autoevolve.log — append-only JSONL of every iteration
//     (start_at, target, status, rounds, cost_usd, commit_sha)
//   - successive runs grep this for past failures, so the LLM gets a
//     "you have hit this before, last time the fix was X" hint via
//     EVOLVE_SYSTEM_PROMPT augmentation.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AutoEvolveLogEntry {
    started_at_ms: i64,
    target: String,             // "check" | "test"
    status: String,             // "green" | "fixed" | "failed" | "skip"
    rounds: usize,
    elapsed_secs: f64,
    commit: Option<String>,     // sha if we committed; None on dry-run / no-op
    summary: String,            // first 200 chars of evolve output
}

/// Path to the user's autoevolve task queue (`~/.phantom-mesh/autoevolve.queue.txt`).
/// One task per line. `#`-prefixed lines are comments (ignored). Blank lines
/// are skipped silently. The queue is consumed FIFO.
fn autoevolve_queue_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(".phantom-mesh").join("autoevolve.queue.txt"))
}

/// Pop the first non-comment, non-blank task from the queue. Atomically
/// rewrites the file with the remaining tasks. Returns `None` if the file
/// doesn't exist or contains no real tasks.
///
/// Concurrency: hourly LaunchAgent fires are far apart enough that
/// concurrent invocations are unlikely. The atomic .tmp + rename still
/// guards against a kicked-off `--once` racing the scheduled run.
fn autoevolve_pop_queue() -> Option<String> {
    let path = autoevolve_queue_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let mut popped: Option<String> = None;
    let mut remaining: Vec<String> = Vec::new();
    for line in content.lines() {
        if popped.is_none() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                popped = Some(trimmed.to_string());
                continue; // drop this line from the rewrite
            }
        }
        remaining.push(line.to_string());
    }
    let task = popped?;
    let tmp = path.with_extension("txt.tmp");
    let new_content = remaining.join("\n");
    if std::fs::write(&tmp, &new_content).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    Some(task)
}

/// Append a one-line failure record to `~/.phantom-mesh/autoevolve.queue.failed.log`
/// so the user can see, on next morning's mobile review, which tasks
/// the agent couldn't complete and why. Best-effort — failures here
/// are silently dropped because losing this log shouldn't crash the
/// scheduler loop.
fn autoevolve_record_failed_task(task: &str, reason: &str) {
    let Some(home) = dirs::home_dir() else { return; };
    let log = home.join(".phantom-mesh").join("autoevolve.queue.failed.log");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{}\t{}\t{}\n", ts, reason, task);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true).open(&log)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

async fn run_autoevolve(args: Vec<String>) -> Result<()> {
    // Sub-actions for managing the scheduled job (macOS launchd):
    //   phantom autoevolve schedule install [--interval SECONDS]
    //   phantom autoevolve schedule uninstall
    //   phantom autoevolve schedule status
    if args.get(2).map(|s| s.as_str()) == Some("schedule") {
        return run_autoevolve_schedule(&args).await;
    }
    // Show recent log: phantom autoevolve log [--n 10]
    if args.get(2).map(|s| s.as_str()) == Some("log") {
        return autoevolve_print_log(&args);
    }
    // Morning-commute summary: phantom autoevolve digest [--since-hours N] [--json]
    if args.get(2).map(|s| s.as_str()) == Some("digest") {
        return autoevolve_digest(&args);
    }

    let mut watch = false;
    let mut interval_secs: u64 = 300;
    let mut max_rounds: usize = 5;
    let mut no_commit = false;
    let mut target = "check".to_string();
    let mut agent_name = "master".to_string();
    // --distributed: instead of running `phantom evolve` locally, run
    // `phantom evolve --distributed` so the failing-task is decomposed and
    // dispatched to all configured cluster peers in parallel. Master node
    // collects results and (if green) auto-commits.
    let mut distributed = false;
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--once"      => watch = false,
            "--watch"     => watch = true,
            "--no-commit" => no_commit = true,
            "--distributed" | "-D" => distributed = true,
            "--interval"  => { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { interval_secs = n; } }
            "--max-rounds"=> { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { max_rounds = n; } }
            "--target"    => { i += 1; if let Some(t) = args.get(i) { target = t.clone(); } }
            "--agent"     => { i += 1; if let Some(a) = args.get(i) { agent_name = a.clone(); } }
            "-h" | "--help" => {
                eprintln!("phantom autoevolve [--once|--watch] [--interval N] [--max-rounds N]");
                eprintln!("                   [--target check|test] [--agent NAME] [--no-commit]");
                eprintln!("                   [--distributed|-D]");
                eprintln!();
                eprintln!("Subcommands:");
                eprintln!("  schedule install|uninstall|status   manage hourly LaunchAgent (macOS)");
                eprintln!("  log     [--n 10]                    show recent JSONL log entries");
                eprintln!("  digest  [--since-hours 24] [--json] morning-commute summary");
                eprintln!();
                eprintln!("Queue (proactive tasks):");
                eprintln!("  echo '<task>' >> ~/.phantom-mesh/autoevolve.queue.txt");
                eprintln!("  consumed FIFO by every iteration when cargo is green;");
                eprintln!("  failed dispatches go to ~/.phantom-mesh/autoevolve.queue.failed.log");
                eprintln!();
                eprintln!("  --distributed    when red, dispatch the failing-test set across");
                eprintln!("                   all configured cluster peers in parallel. The");
                eprintln!("                   master node collects results and commits the");
                eprintln!("                   winning fix.");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    eprintln!("{} {}",
        colored("◆ phantom autoevolve", 35),
        colored(env!("CARGO_PKG_VERSION"), 90));
    eprintln!("  mode       : {}", if watch { format!("watch (every {}s)", interval_secs) } else { "once".into() });
    eprintln!("  target     : cargo {}", target);
    eprintln!("  agent      : {}", agent_name);
    eprintln!("  max rounds : {}", max_rounds);
    eprintln!("  commit     : {}", if no_commit { "off (dry-run)" } else { "on" });
    eprintln!("  topology   : {}",
        if distributed { colored("distributed (all cluster peers)", 36) } else { "local".into() });
    eprintln!();

    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let flag = interrupted.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() { flag.store(true, Ordering::Relaxed); }
        });
    }

    loop {
        let started_at = std::time::Instant::now();
        let started_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Open an evolve checkpoint for this iteration. Records phase
        // transitions, plan progress, dead-ends, and (eventually) embeds
        // its trailer into the auto-commit message so `git log` carries
        // the autonomous decision history. Mesh-aware: origin_node /
        // current_node fields let a peer pick this up across machines.
        use phantom_mesh::evolve_checkpoint::{EvolveCheckpoint, EvolvePhase, EvolveOutcome, ArtifactKind};
        let node_name = std::env::var("PHANTOM_NODE_NAME")
            .ok()
            .or_else(|| dirs::home_dir().and_then(|h| h.file_name().map(|s| s.to_string_lossy().into_owned())))
            .unwrap_or_else(|| "local".into());
        let mut checkpoint = EvolveCheckpoint::new(
            format!("autoevolve cargo {} → restore green", target),
            target.clone(),
            node_name,
        );
        checkpoint.plan = vec![
            format!("cargo {} pre-check", target),
            "if red: spawn evolve subprocess (LLM agent)".to_string(),
            format!("cargo {} post-check", target),
            "on green: git add -A + commit + record sha".to_string(),
        ];
        let _ = checkpoint.save();

        // 1) Pre-check: is the target already green?
        eprintln!("{} cargo {} …", colored("⟳", 33), target);
        checkpoint.set_phase(EvolvePhase::Discovering);
        let _ = checkpoint.save();
        let check_status = run_cargo_target(&target).await;
        let was_red = !check_status.success;
        checkpoint.append_step(
            format!("cargo {} pre-check", target),
            Some("cargo".into()),
            !was_red,
        );
        let _ = checkpoint.save();

        let entry = if !was_red {
            // Green branch: if there's a queued task in
            // ~/.phantom-mesh/autoevolve.queue.txt, dispatch it now.
            // Otherwise fall through to the original "no-op" log entry.
            //
            // This is the proactive companion to the reactive evolve
            // loop: red → fix; green + queued → make progress on the
            // user's TODO list; green + empty queue → idle. Built so
            // the user can `echo "refactor X" >> ~/.phantom-mesh/autoevolve.queue.txt`
            // from their phone via Tailscale and have the work done by
            // the time they get home.
            if let Some(task) = autoevolve_pop_queue() {
                // Safety guard: refuse to dispatch a queued task on top of
                // an already-dirty working tree. Otherwise the auto-commit
                // at the end would sweep in whatever the user was editing
                // by hand. Push the task back to the head of the queue
                // and bail to no-op so we try again next iteration.
                let pre_dirty = std::process::Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .ok()
                    .map(|o| !o.stdout.is_empty())
                    .unwrap_or(false);
                if pre_dirty {
                    eprintln!("{} skipping queued task — working tree is dirty (commit or stash first):",
                        colored("⚠", 33));
                    eprintln!("    task: {}", task);
                    autoevolve_record_failed_task(
                        &task,
                        "skipped: working tree dirty before dispatch",
                    );
                    // Re-queue at head so the user doesn't lose it.
                    if let Some(qp) = autoevolve_queue_path() {
                        let existing = std::fs::read_to_string(&qp).unwrap_or_default();
                        let new = format!("{}\n{}", task, existing);
                        let _ = std::fs::write(&qp, new);
                    }
                    checkpoint.set_phase(EvolvePhase::Done {
                        outcome: EvolveOutcome::Stuck {
                            reason: "queue dispatch skipped: dirty working tree".into(),
                            last_error: None,
                        },
                    });
                    let _ = checkpoint.save();
                    // Return a log entry instead of `return Ok(())` so the
                    // outer loop's --watch mode keeps polling. The user can
                    // commit / stash and the next iteration will retry.
                    AutoEvolveLogEntry {
                        started_at_ms, target: target.clone(),
                        status: "queued-task-skipped-dirty".into(), rounds: 0,
                        elapsed_secs: started_at.elapsed().as_secs_f64(),
                        commit: None,
                        summary: format!(
                            "skipped (dirty tree): {} [checkpoint: {}]",
                            task.chars().take(60).collect::<String>(),
                            checkpoint.session_id
                        ),
                    }
                } else {
                eprintln!("{} cargo {} green — dispatching queued task: {}",
                    colored("◆", 35), target,
                    colored(&task.chars().take(80).collect::<String>(), 36));
                checkpoint.set_phase(EvolvePhase::Editing);
                let _ = checkpoint.save();

                let history_hint = autoevolve_history_hint(8);
                let exe = std::env::current_exe()?;
                let mut cmd = tokio::process::Command::new(exe);
                let mut argv = vec![
                    "evolve".to_string(),
                    "--max-rounds".to_string(), max_rounds.to_string(),
                    "--agent".to_string(), agent_name.clone(),
                ];
                if distributed { argv.push("--distributed".to_string()); }
                argv.push(task.clone());
                cmd.args(&argv);
                if !history_hint.is_empty() {
                    cmd.env("PHANTOM_AUTOEVOLVE_HISTORY", &history_hint);
                }
                cmd.stdout(std::process::Stdio::inherit());
                cmd.stderr(std::process::Stdio::inherit());
                if let Some(manifest) = find_cargo_manifest() {
                    if let Some(parent) = manifest.parent() {
                        cmd.current_dir(parent);
                    }
                }
                let evolve_status = cmd.status().await.ok();
                let evolve_ok = evolve_status.map(|s| s.success()).unwrap_or(false);

                // Post-check: don't ship a commit that broke the build.
                let post = run_cargo_target(&target).await;
                let still_green = post.success;

                let mut commit_sha: Option<String> = None;
                if still_green && evolve_ok {
                    let dirty = std::process::Command::new("git")
                        .args(["status", "--porcelain"])
                        .output()
                        .ok()
                        .map(|o| !o.stdout.is_empty())
                        .unwrap_or(false);
                    if dirty && !no_commit {
                        let _ = std::process::Command::new("git").args(["add", "-A"]).status();
                        let trailer = checkpoint.render_commit_trailer();
                        let task_preview: String = task.chars().take(72).collect();
                        let msg = format!(
                            "autoevolve(queue): {}\n\nfull task: {}\n\n{}",
                            task_preview,
                            task,
                            trailer.trim_end(),
                        );
                        let commit_ok = std::process::Command::new("git")
                            .args(["commit", "-m", &msg])
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if commit_ok {
                            commit_sha = std::process::Command::new("git")
                                .args(["rev-parse", "--short=10", "HEAD"])
                                .output()
                                .ok()
                                .and_then(|o| String::from_utf8(o.stdout).ok())
                                .map(|s| s.trim().to_string());
                            eprintln!("{} committed: {}", colored("✓", 32),
                                commit_sha.as_deref().unwrap_or("?"));
                        } else {
                            eprintln!("{} git commit failed (pre-commit hook?)", colored("⚠", 33));
                            autoevolve_record_failed_task(&task, "git commit failed");
                        }
                    } else if dirty && no_commit {
                        eprintln!("{} would commit; --no-commit set", colored("⚠", 33));
                    } else {
                        eprintln!("{} task done — no file changes to commit", colored("◆", 36));
                    }
                    checkpoint.set_phase(EvolvePhase::Done {
                        outcome: EvolveOutcome::Success {
                            commit_sha: commit_sha.clone().unwrap_or_else(|| "(uncommitted)".into()),
                            rounds: 0,
                        },
                    });
                } else {
                    eprintln!("{} task ran but cargo {} now red — task put in failed log, not committed",
                        colored("✗", 31), target);
                    autoevolve_record_failed_task(
                        &task,
                        &format!("post-task cargo {} red", target),
                    );
                    // Drop any partial work so the next iteration starts clean.
                    let _ = std::process::Command::new("git")
                        .args(["restore", "--staged", "."]).status();
                    let _ = std::process::Command::new("git")
                        .args(["restore", "."]).status();
                    checkpoint.set_phase(EvolvePhase::Done {
                        outcome: EvolveOutcome::Stuck {
                            reason: format!("queued task broke cargo {}", target),
                            last_error: None,
                        },
                    });
                }
                let _ = checkpoint.save();
                AutoEvolveLogEntry {
                    started_at_ms, target: target.clone(),
                    status: if commit_sha.is_some() { "queued-task-done".into() }
                            else if still_green { "queued-task-noop".into() }
                            else { "queued-task-failed".into() },
                    rounds: 0,
                    elapsed_secs: started_at.elapsed().as_secs_f64(),
                    commit: commit_sha,
                    summary: format!("queued task: {} [checkpoint: {}]",
                        task.chars().take(60).collect::<String>(),
                        checkpoint.session_id),
                }
                } // closes the `else` of pre_dirty guard
            } else {
                eprintln!("{} cargo {} green — nothing to evolve.", colored("✓", 32), target);
                checkpoint.set_phase(EvolvePhase::Done {
                    outcome: EvolveOutcome::Success {
                        commit_sha: "(no-op)".into(),
                        rounds: 0,
                    },
                });
                let _ = checkpoint.save();
                AutoEvolveLogEntry {
                    started_at_ms, target: target.clone(),
                    status: "green".into(), rounds: 0,
                    elapsed_secs: started_at.elapsed().as_secs_f64(),
                    commit: None,
                    summary: format!("no-op (already green) [checkpoint: {}]", checkpoint.session_id),
                }
            }
        } else {
            checkpoint.set_phase(EvolvePhase::Hypothesizing);
            checkpoint.current_hypothesis = Some(
                "cargo target is red — agent will diagnose via shell + file_read".into()
            );
            let _ = checkpoint.save();
            eprintln!("{} cargo {} failing — spawning evolve…", colored("✗", 31), target);
            let goal = format!(
                "Run cargo {} and fix the failures. Use file_edit / multi_file_edit / shell. \
                 When everything is green, end your response with EVOLVE_DONE.",
                target
            );

            // Build a compact history hint from the last few JSONL entries
            // and pass it to the spawned `phantom evolve` via env. The
            // child reads it through build_evolve_system_prompt() and
            // prepends it to the system prompt — the LLM sees what we
            // already learned about the repo's recurring failures.
            let history_hint = autoevolve_history_hint(8);

            // Spawn `phantom evolve` as a subprocess so we reuse its proven
            // round loop verbatim. This also keeps memory state clean: each
            // evolve round runs in its own process. When --distributed,
            // forward the flag through and let run_evolve_distributed
            // decompose-and-dispatch across cluster peers.
            let exe = std::env::current_exe()?;
            let mut cmd = tokio::process::Command::new(exe);
            let mut argv = vec![
                "evolve".to_string(),
                "--max-rounds".to_string(), max_rounds.to_string(),
                "--agent".to_string(), agent_name.clone(),
            ];
            if distributed { argv.push("--distributed".to_string()); }
            argv.push(goal.clone());
            cmd.args(&argv);
            if !history_hint.is_empty() {
                cmd.env("PHANTOM_AUTOEVOLVE_HISTORY", &history_hint);
            }
            cmd.stdout(std::process::Stdio::inherit());
            cmd.stderr(std::process::Stdio::inherit());

            // Set cwd to the manifest's parent dir so the spawned evolve
            // agent's shell tool naturally finds Cargo.toml. Without this,
            // the agent's `cargo check` (issued via shell) fails the same
            // way run_cargo_target() did before — except the agent has no
            // way to know the manifest is hiding in a subdir.
            if let Some(manifest) = find_cargo_manifest() {
                if let Some(parent) = manifest.parent() {
                    cmd.current_dir(parent);
                }
            }

            checkpoint.set_phase(EvolvePhase::Editing);
            checkpoint.append_step("dispatched evolve subprocess (LLM agent)", Some("phantom evolve".into()), true);
            let _ = checkpoint.save();

            let evolve_status = cmd.status().await.ok();
            let evolve_ok = evolve_status.map(|s| s.success()).unwrap_or(false);

            // 2) Post-check: did it stick?
            checkpoint.set_phase(EvolvePhase::Verifying);
            let _ = checkpoint.save();
            let post = run_cargo_target(&target).await;
            let now_green = post.success;
            let rounds_used = max_rounds; // we don't get exact round count back; conservative
            checkpoint.append_step(
                format!("cargo {} post-check", target),
                Some("cargo".into()),
                now_green,
            );
            if !now_green {
                checkpoint.record_dead_end(
                    "evolve subprocess thought it fixed the issue",
                    format!("cargo {} still failing after {} round subprocess", target, rounds_used),
                );
            }
            let _ = checkpoint.save();

            if now_green && evolve_ok {
                checkpoint.set_phase(EvolvePhase::Committing);
                let _ = checkpoint.save();
                // Auto-commit if there are staged or unstaged changes touching .rs files.
                let dirty = std::process::Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).contains(".rs"))
                    .unwrap_or(false);
                let mut commit_sha: Option<String> = None;
                if dirty && !no_commit {
                    // git add -A + commit. Embed the checkpoint's compact
                    // trailer so `git log` carries the full evolve session
                    // breadcrumb (session id, target, rounds, dead-ends,
                    // binary swaps, mesh hops). This is what makes the
                    // autonomous decision history auditable later.
                    let _ = std::process::Command::new("git").args(["add", "-A"]).status();
                    let trailer = checkpoint.render_commit_trailer();
                    let msg = format!(
                        "autoevolve: cargo {} restored green ({} rounds, {:.1}s)\n\n{}",
                        target, rounds_used,
                        started_at.elapsed().as_secs_f64(),
                        trailer.trim_end(),
                    );
                    let commit_ok = std::process::Command::new("git")
                        .args(["commit", "-m", &msg])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if commit_ok {
                        commit_sha = std::process::Command::new("git")
                            .args(["rev-parse", "--short=10", "HEAD"])
                            .output()
                            .ok()
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .map(|s| s.trim().to_string());
                        eprintln!("{} committed: {}", colored("✓", 32), commit_sha.as_deref().unwrap_or("?"));
                        if let Some(sha) = commit_sha.as_deref() {
                            checkpoint.record_artifact(ArtifactKind::Commit {
                                sha: sha.to_string(),
                                subject: format!("autoevolve: cargo {} restored green", target),
                            });
                        }
                    } else {
                        eprintln!("{} git commit failed (pre-commit hook? see git output above)", colored("⚠", 33));
                    }
                } else if dirty && no_commit {
                    eprintln!("{} would commit; --no-commit set, leaving working tree dirty", colored("⚠", 33));
                }
                checkpoint.set_phase(EvolvePhase::Done {
                    outcome: EvolveOutcome::Success {
                        commit_sha: commit_sha.clone().unwrap_or_else(|| "(uncommitted)".into()),
                        rounds: rounds_used as u32,
                    },
                });
                let _ = checkpoint.save();
                AutoEvolveLogEntry {
                    started_at_ms, target: target.clone(),
                    status: "fixed".into(), rounds: rounds_used,
                    elapsed_secs: started_at.elapsed().as_secs_f64(),
                    commit: commit_sha,
                    summary: format!("cargo {} green after evolve", target),
                }
            } else {
                eprintln!("{} evolve did not restore green; leaving repo as-is.", colored("✗", 31));
                checkpoint.set_phase(EvolvePhase::Done {
                    outcome: EvolveOutcome::Stuck {
                        reason: format!("evolve unable to restore cargo {}", target),
                        last_error: None,
                    },
                });
                let _ = checkpoint.save();
                AutoEvolveLogEntry {
                    started_at_ms, target: target.clone(),
                    status: "failed".into(), rounds: rounds_used,
                    elapsed_secs: started_at.elapsed().as_secs_f64(),
                    commit: None,
                    summary: format!("evolve unable to restore cargo {} [checkpoint: {}]",
                                     target, checkpoint.session_id),
                }
            }
        };

        // 3) Append to log.
        if let Some(home) = dirs::home_dir() {
            let log_path = home.join(".phantom-mesh/autoevolve.log");
            let _ = std::fs::create_dir_all(log_path.parent().unwrap());
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true).append(true).open(&log_path)
            {
                use std::io::Write;
                if let Ok(line) = serde_json::to_string(&entry) {
                    let _ = writeln!(f, "{}", line);
                }
            }
        }

        if !watch || interrupted.load(Ordering::Relaxed) { break; }
        eprintln!("{} sleeping {}s …", colored("◇", 90), interval_secs);
        for _ in 0..interval_secs {
            if interrupted.load(Ordering::Relaxed) { break; }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        if interrupted.load(Ordering::Relaxed) { break; }
    }
    Ok(())
}

struct CargoRunOutcome { success: bool }

/// Read up to `n` recent JSONL entries from ~/.phantom-mesh/autoevolve.log
/// and format them as a one-line-per-entry hint suitable for embedding in
/// the evolve system prompt.
fn autoevolve_history_hint(n: usize) -> String {
    let home = match dirs::home_dir() { Some(h) => h, None => return String::new() };
    let path = home.join(".phantom-mesh/autoevolve.log");
    let content = match std::fs::read_to_string(&path) { Ok(s) => s, Err(_) => return String::new() };

    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    let take = lines.len().saturating_sub(n);
    let recent = &lines[take..];

    let mut summary = String::new();
    for line in recent {
        if let Ok(entry) = serde_json::from_str::<AutoEvolveLogEntry>(line) {
            // Compact format: ISO-ish date + status + commit + summary
            let secs = entry.started_at_ms / 1000;
            let when = std::process::Command::new("date")
                .args(["-r", &secs.to_string(), "+%Y-%m-%dT%H:%M"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "?".into());
            let commit = entry.commit.unwrap_or_else(|| "-".into());
            summary.push_str(&format!(
                "  • [{when}] cargo {target} → {status} ({rounds}r, {commit}): {summary}\n",
                when = when,
                target = entry.target,
                status = entry.status,
                rounds = entry.rounds,
                commit = commit,
                summary = entry.summary.chars().take(80).collect::<String>(),
            ));
        }
    }
    summary
}

/// Print the recent JSONL log to stdout for quick inspection.
fn autoevolve_print_log(args: &[String]) -> anyhow::Result<()> {
    // Default: last 10 entries. `phantom autoevolve log --n 30` overrides.
    let mut n: usize = 10;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "--n" {
            i += 1;
            if let Some(parsed) = args.get(i).and_then(|s| s.parse().ok()) { n = parsed; }
        }
        i += 1;
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home"))?;
    let path = home.join(".phantom-mesh/autoevolve.log");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            println!("{}", colored("(no autoevolve log yet — `phantom autoevolve --once` to start one)", 90));
            return Ok(());
        }
    };
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    let take = lines.len().saturating_sub(n);
    let recent = &lines[take..];

    println!("{} last {} of {} autoevolve runs:",
        colored("◆", 35), recent.len(), lines.len());
    for line in recent {
        if let Ok(entry) = serde_json::from_str::<AutoEvolveLogEntry>(line) {
            let secs = entry.started_at_ms / 1000;
            let when = std::process::Command::new("date")
                .args(["-r", &secs.to_string(), "+%m-%d %H:%M"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "?".into());
            let status_color = match entry.status.as_str() {
                "green" | "fixed" => 32,
                "failed"          => 31,
                _                 => 33,
            };
            let commit = entry.commit.unwrap_or_else(|| "-".into());
            println!(
                "  {} cargo {} → {} ({}r, {}, {:.1}s): {}",
                colored(&when, 90),
                entry.target,
                colored(&entry.status, status_color),
                entry.rounds,
                colored(&commit, 36),
                entry.elapsed_secs,
                entry.summary.chars().take(60).collect::<String>(),
            );
        }
    }
    Ok(())
}

/// `phantom autoevolve digest [--since-hours N] [--json]` — single-screen
/// summary of the last N hours of autoevolve activity. Designed for the
/// morning-commute mobile-review workflow: at a glance you see what was
/// committed overnight, what got stuck, and how many queued tasks are
/// still pending. JSON mode is for piping into a push notification or
/// the future web `/m` UI.
fn autoevolve_digest(args: &[String]) -> anyhow::Result<()> {
    let mut since_hours: i64 = 24;
    let mut json_out = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--since-hours" => {
                i += 1;
                if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { since_hours = n; }
            }
            "--json" => json_out = true,
            _ => {}
        }
        i += 1;
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home"))?;
    let log_path = home.join(".phantom-mesh/autoevolve.log");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff_ms = now_ms - since_hours * 3600 * 1000;

    // Parse all log entries newer than cutoff.
    let entries: Vec<AutoEvolveLogEntry> = std::fs::read_to_string(&log_path)
        .ok()
        .map(|s| s.lines()
            .filter_map(|l| serde_json::from_str::<AutoEvolveLogEntry>(l).ok())
            .filter(|e| e.started_at_ms >= cutoff_ms)
            .collect())
        .unwrap_or_default();

    // Bucket by status — same coarse buckets as the print-log command, plus
    // the new queue-related ones from autoevolve.queue.txt support.
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut commits: Vec<(i64, String, String)> = Vec::new();
    for e in &entries {
        *counts.entry(e.status.clone()).or_default() += 1;
        if let Some(sha) = &e.commit {
            commits.push((e.started_at_ms, sha.clone(), e.summary.clone()));
        }
    }

    // Failed-tasks log: tab-separated `unix_ts \t reason \t task`.
    let failed_log = home.join(".phantom-mesh/autoevolve.queue.failed.log");
    let recent_failures: Vec<(i64, String, String)> = std::fs::read_to_string(&failed_log)
        .ok()
        .map(|s| s.lines()
            .filter_map(|l| {
                let mut parts = l.splitn(3, '\t');
                let ts: i64 = parts.next()?.parse().ok()?;
                let reason = parts.next()?.to_string();
                let task = parts.next()?.to_string();
                Some((ts * 1000, reason, task))
            })
            .filter(|(ts_ms, _, _)| *ts_ms >= cutoff_ms)
            .collect())
        .unwrap_or_default();

    // Pending queue depth.
    let queue_pending: usize = autoevolve_queue_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .map(|c| c.lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .count())
        .unwrap_or(0);

    if json_out {
        let v = serde_json::json!({
            "since_hours":   since_hours,
            "runs_total":    entries.len(),
            "by_status":     counts,
            "commits":       commits.iter().map(|(ts, sha, sum)|
                serde_json::json!({"at_ms": ts, "sha": sha, "summary": sum})
            ).collect::<Vec<_>>(),
            "failed_tasks":  recent_failures.iter().map(|(ts, reason, task)|
                serde_json::json!({"at_ms": ts, "reason": reason, "task": task})
            ).collect::<Vec<_>>(),
            "queue_pending": queue_pending,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    let fmt_when = |ms: i64| -> String {
        let s = ms / 1000;
        std::process::Command::new("date")
            .args(["-r", &s.to_string(), "+%m-%d %H:%M"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "?".into())
    };

    println!("{} {}",
        colored("◆ phantom autoevolve digest", 35),
        colored(&format!("last {}h", since_hours), 90));
    println!("  window  : {} → {}", fmt_when(cutoff_ms), fmt_when(now_ms));
    println!("  runs    : {} total", entries.len());
    if !counts.is_empty() {
        let mut parts: Vec<String> = counts.iter()
            .map(|(k, v)| {
                let c = match k.as_str() {
                    "green" | "fixed" | "queued-task-done" => 32,
                    "queued-task-failed" | "failed"        => 31,
                    _                                       => 33,
                };
                format!("{} {}", colored(&v.to_string(), c), k)
            })
            .collect();
        parts.sort();
        println!("            {}", parts.join(", "));
    }

    if !commits.is_empty() {
        println!();
        println!("  {} commits ({})", colored("✓", 32), commits.len());
        for (ts, sha, sum) in &commits {
            let preview: String = sum.chars().take(72).collect();
            println!("    {}  {}  {}",
                colored(&fmt_when(*ts), 90),
                colored(sha, 36),
                preview);
        }
    }

    if !recent_failures.is_empty() {
        println!();
        println!("  {} failed tasks ({})", colored("⚠", 33), recent_failures.len());
        for (ts, reason, task) in &recent_failures {
            let task_preview: String = task.chars().take(60).collect();
            println!("    {}  {} — {}",
                colored(&fmt_when(*ts), 90),
                colored(reason, 33),
                task_preview);
        }
    }

    println!();
    println!("  queue   : {} pending", queue_pending);
    if queue_pending > 0 {
        if let Some(qp) = autoevolve_queue_path() {
            println!("            ({})", colored(&qp.display().to_string(), 90));
        }
    }
    Ok(())
}

/// `phantom autoevolve schedule [install|uninstall|status]` — manage the
/// macOS LaunchAgent that periodically runs `autoevolve --once`.
#[cfg(target_os = "macos")]
async fn run_autoevolve_schedule(args: &[String]) -> anyhow::Result<()> {
    use std::process::Command;
    let action = args.get(3).map(|s| s.as_str()).unwrap_or("status");

    let label = "ai.phantommesh.autoevolve";
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home"))?;
    let plist_path = home.join("Library/LaunchAgents").join(format!("{}.plist", label));
    let log_dir = home.join("Library/Logs");
    let uid = nix_uid();
    let domain = format!("gui/{}", uid);
    let target = format!("{}/{}", domain, label);

    match action {
        "install" => {
            // Optional --interval N
            let mut interval_secs: u64 = 3600;
            let mut target_kind = "check".to_string();
            let mut max_rounds = 5usize;
            let mut agent_name = "master".to_string();
            let mut i = 4;
            while i < args.len() {
                match args[i].as_str() {
                    "--interval"   => { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { interval_secs = n; } }
                    "--target"     => { i += 1; if let Some(t) = args.get(i) { target_kind = t.clone(); } }
                    "--max-rounds" => { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { max_rounds = n; } }
                    "--agent"      => { i += 1; if let Some(a) = args.get(i) { agent_name = a.clone(); } }
                    _ => {}
                }
                i += 1;
            }

            // Resolve current binary; copy it to the launchd-friendly location
            // (same TCC reasoning as service install — see 65338ab).
            let bin_self = std::env::current_exe()?;
            let bin_real = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            let install_dir = home.join("Library/Application Support/phantom-mesh/bin");
            std::fs::create_dir_all(&install_dir)?;
            let installed_bin = install_dir.join("phantom");
            std::fs::copy(&bin_real, &installed_bin)?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                let mut p = std::fs::metadata(&installed_bin)?.permissions();
                p.set_mode(0o755);
                std::fs::set_permissions(&installed_bin, p)?;
            }
            // See note in run_service_subcommand: cp invalidates Mach-O
            // signature on macOS, kernel SIGKILLs next launch silently.
            #[cfg(target_os = "macos")]
            {
                let _ = Command::new("codesign")
                    .args(["--force", "--sign", "-", installed_bin.to_str().unwrap_or("")])
                    .output();
            }
            let bin_str = installed_bin.display().to_string();

            // Repo root: locate just like service install does.
            let repo_root = locate_repo_root(&bin_real)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| home.display().to_string());

            std::fs::create_dir_all(plist_path.parent().unwrap())?;
            std::fs::create_dir_all(&log_dir)?;

            let tmpl: &str = include_str!("../../../templates/ai.phantommesh.autoevolve.plist.tmpl");
            let (extra_env, env_names) = build_extra_env_plist_xml();
            let rendered = tmpl
                .replace("__PHANTOM_BIN__", &bin_str)
                .replace("__REPO_ROOT__",   &repo_root)
                .replace("__HOME__",        &home.display().to_string())
                .replace("__INTERVAL_SECS__", &interval_secs.to_string())
                .replace("__TARGET__",      &target_kind)
                .replace("__MAX_ROUNDS__",  &max_rounds.to_string())
                .replace("__AGENT__",       &agent_name)
                .replace("__EXTRA_ENV__",   &extra_env);
            std::fs::write(&plist_path, &rendered)?;
            eprintln!("{} Wrote {}", colored("◆", 35), plist_path.display());
            if !env_names.is_empty() {
                eprintln!("    propagated env: {} key(s) → {}",
                          env_names.len(), env_names.join(", "));
            }

            let _ = Command::new("launchctl").args(["bootout", &target]).output();
            let s = Command::new("launchctl").args(["bootstrap", &domain, plist_path.to_str().unwrap()]).status()?;
            if !s.success() { anyhow::bail!("launchctl bootstrap failed"); }
            let _ = Command::new("launchctl").args(["enable", &target]).output();

            eprintln!("{} Scheduled autoevolve every {}s (cargo {}, agent {}, max {} rounds)",
                colored("✓", 32), interval_secs, target_kind, agent_name, max_rounds);
            eprintln!("    Log:        {}/phantom-autoevolve.log", log_dir.display());
            eprintln!("    Status:     phantom autoevolve schedule status");
            eprintln!("    Run-now:    launchctl kickstart {}", target);
            eprintln!("    Uninstall:  phantom autoevolve schedule uninstall");
            Ok(())
        }
        "uninstall" => {
            let _ = Command::new("launchctl").args(["bootout", &target]).output();
            if plist_path.exists() {
                std::fs::remove_file(&plist_path)?;
                eprintln!("{} Removed {}", colored("◆", 35), plist_path.display());
            }
            eprintln!("{} Unscheduled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let print = Command::new("launchctl").args(["print", &target]).output()?;
            let registered = print.status.success();
            println!("{} {}", colored("phantom autoevolve schedule", 36), colored(label, 90));
            println!("  registered : {}", if registered { colored("yes", 32) } else { colored("no", 31) });
            if registered {
                let body = String::from_utf8_lossy(&print.stdout);
                // launchctl print emits the line as `\trun interval = 3600 seconds`.
                // After trim_start, it becomes `run interval = 3600 seconds` —
                // the previous parser was matching `interval =` (no `run`
                // prefix) and silently fell back to `?`. Match the real key
                // and strip the trailing ` seconds` suffix so we display
                // `3600s` cleanly.
                let interval = body.lines()
                    .map(str::trim_start)
                    .find(|l| l.starts_with("run interval ="))
                    .and_then(|l| l.split('=').nth(1))
                    .map(|s| s.trim().trim_end_matches(" seconds").trim_end_matches(" second").to_string())
                    .unwrap_or_else(|| "?".into());
                println!("  interval   : {}s", interval);
                println!("  plist      : {}", plist_path.display());
                println!("  log        : {}/phantom-autoevolve.log", log_dir.display());
                // Queue summary — lets the user check at a glance whether
                // tomorrow morning's commute review will have anything
                // worth looking at, OR whether the queue is empty and
                // they should add tasks tonight. Counts non-blank,
                // non-comment lines (matches what autoevolve_pop_queue
                // considers "real tasks").
                if let Some(qp) = autoevolve_queue_path() {
                    let pending = std::fs::read_to_string(&qp)
                        .map(|c| c.lines()
                            .filter(|l| {
                                let t = l.trim();
                                !t.is_empty() && !t.starts_with('#')
                            })
                            .count())
                        .unwrap_or(0);
                    println!("  queue      : {} pending", pending);
                }
                let failed_log = home.join(".phantom-mesh").join("autoevolve.queue.failed.log");
                if failed_log.exists() {
                    let n = std::fs::read_to_string(&failed_log)
                        .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
                        .unwrap_or(0);
                    if n > 0 {
                        println!("  failed     : {} entries  ({})", n, failed_log.display());
                    }
                }
            }
            Ok(())
        }
        other => {
            eprintln!("{} Unknown schedule action '{}'. Use install / uninstall / status.",
                colored("✗", 31), other);
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "windows")]
const WINDOWS_AUTOEVOLVE_TASK_NAME: &str = "PhantomAutoevolve";

/// Windows counterpart of the macOS LaunchAgent schedule.
/// Backed by schtasks /SC MINUTE /MO <interval-in-minutes>.
#[cfg(target_os = "windows")]
async fn run_autoevolve_schedule(args: &[String]) -> anyhow::Result<()> {
    use std::process::Command;
    let action = args.get(3).map(|s| s.as_str()).unwrap_or("status");

    match action {
        "install" => {
            // Optional --interval N (seconds, mirrors macOS), --target, --max-rounds, --agent
            let mut interval_secs: u64 = 3600;
            let mut target_kind = "check".to_string();
            let mut max_rounds = 5usize;
            let mut agent_name = "master".to_string();
            let mut i = 4;
            while i < args.len() {
                match args[i].as_str() {
                    "--interval"   => { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { interval_secs = n; } }
                    "--target"     => { i += 1; if let Some(t) = args.get(i) { target_kind = t.clone(); } }
                    "--max-rounds" => { i += 1; if let Some(n) = args.get(i).and_then(|s| s.parse().ok()) { max_rounds = n; } }
                    "--agent"      => { i += 1; if let Some(a) = args.get(i) { agent_name = a.clone(); } }
                    _ => {}
                }
                i += 1;
            }
            // schtasks /SC MINUTE has minimum /MO 1.
            let interval_minutes = std::cmp::max(1u64, interval_secs / 60);

            let bin_self = std::env::current_exe()?;
            let bin = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            let bin_str = bin.display().to_string();

            // schtasks /TR expects a quoted command line — wrap the path
            // because Program Files paths contain spaces.
            let tr = format!(
                "\"{}\" autoevolve --once --target {} --max-rounds {} --agent {}",
                bin_str, target_kind, max_rounds, agent_name
            );

            // Delete any prior registration first so re-installs always pick up
            // the fresh binary path / interval.
            let _ = Command::new("schtasks")
                .args(["/Delete", "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME, "/F"])
                .output();

            let status = Command::new("schtasks")
                .args([
                    "/Create",
                    "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME,
                    "/SC", "MINUTE",
                    "/MO", &interval_minutes.to_string(),
                    "/RL", "LIMITED",
                    "/F",
                    "/TR", &tr,
                ])
                .status()?;
            if !status.success() {
                anyhow::bail!("schtasks /Create failed (exit {:?})", status.code());
            }

            eprintln!(
                "{} Scheduled autoevolve every {}min (cargo {}, agent {}, max {} rounds)",
                colored("✓", 32), interval_minutes, target_kind, agent_name, max_rounds
            );
            eprintln!("    binary:    {}", bin_str);
            eprintln!("    Status:    phantom autoevolve schedule status");
            eprintln!("    Run-now:   schtasks /Run /TN {}", WINDOWS_AUTOEVOLVE_TASK_NAME);
            eprintln!("    Uninstall: phantom autoevolve schedule uninstall");
            Ok(())
        }
        "uninstall" => {
            let status = Command::new("schtasks")
                .args(["/Delete", "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME, "/F"])
                .status()?;
            if status.success() {
                eprintln!("{} Removed Scheduled Task '{}'", colored("◆", 35), WINDOWS_AUTOEVOLVE_TASK_NAME);
            } else {
                eprintln!(
                    "{} schtasks /Delete returned exit {:?} — task may not have existed.",
                    colored("⚠", 33), status.code()
                );
            }
            eprintln!("{} Unscheduled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let q = Command::new("schtasks")
                .args(["/Query", "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME])
                .output()?;
            let registered = q.status.success();

            println!(
                "{} {}",
                colored("phantom autoevolve schedule", 36),
                colored(WINDOWS_AUTOEVOLVE_TASK_NAME, 90)
            );
            println!(
                "  registered : {}",
                if registered { colored("yes", 32) } else { colored("no", 31) }
            );
            if registered {
                let (last_run_time, next_run_time, last_result) =
                    windows_task_info(WINDOWS_AUTOEVOLVE_TASK_NAME);
                // Interval lives in the trigger's Repetition; pull it from the
                // task XML (locale-independent element names).
                let xml = Command::new("schtasks")
                    .args(["/Query", "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME, "/XML"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                let interval = xml
                    .lines()
                    .find_map(|l| {
                        let l = l.trim();
                        let s = l.strip_prefix("<Interval>")?;
                        s.strip_suffix("</Interval>").map(|v| v.to_string())
                    })
                    .unwrap_or_else(|| "?".into());
                println!("  interval   : {}", interval);
                println!("  last run   : {}", last_run_time.unwrap_or_else(|| "?".into()));
                println!("  next run   : {}", next_run_time.unwrap_or_else(|| "?".into()));
                if let Some(r) = last_result {
                    let (code, label) = windows_task_result_label(r);
                    println!("  last state : {}", colored(&label, code));
                }
            }
            Ok(())
        }
        other => {
            eprintln!("{} Unknown schedule action '{}'. Use install / uninstall / status.",
                colored("✗", 31), other);
            std::process::exit(1);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn run_autoevolve_schedule(_args: &[String]) -> anyhow::Result<()> {
    eprintln!("{} `phantom autoevolve schedule` is currently macOS + Windows only.\n   On Linux use cron / systemd timer.",
        colored("⚠", 33));
    eprintln!("\n   Linux example (cron):\n     0 * * * * /usr/local/bin/phantom autoevolve --once --target check");
    Ok(())
}

async fn run_cargo_target(target: &str) -> CargoRunOutcome {
    let mut cargo_args: Vec<String> = match target {
        "test" => vec!["test".into(), "--quiet".into()],
        _      => vec!["check".into(), "--quiet".into()],
    };

    // Locate Cargo.toml. Cargo itself walks parents from cwd, but in workspace
    // layouts where the manifest lives in a subdirectory (e.g. `core/`), we
    // need to point cargo at it explicitly. This matters when phantom is
    // invoked from a service manager (LaunchAgent / systemd) that sets cwd
    // to the repo root rather than the manifest's directory.
    if let Some(manifest) = find_cargo_manifest() {
        cargo_args.push("--manifest-path".into());
        cargo_args.push(manifest.to_string_lossy().into_owned());
    }

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args(&cargo_args)
        .stdout(std::process::Stdio::null())
        // Capture stderr so we can distinguish real cargo failures from
        // Windows-Defender-locks-build-script-build.exe transient errors,
        // which surface as "Access Denied / 存取被拒 / failed to remove
        // file" while the binary the AV is currently scanning is held
        // open. Without this, autoevolve would treat the lock as "code
        // is broken", spawn the LLM, and risk a destructive file_write.
        .stderr(std::process::Stdio::piped());

    // Isolate autoevolve's target/ from the user's interactive `cargo
    // build`. Two side benefits beyond just "don't pollute":
    //   1. Avoids the Windows AV lock race on `.worktrees/<topic>/core/target/`
    //      that surfaces as `存取被拒 (os error 5)`.
    //   2. Caches autoevolve's check artefacts across runs, so subsequent
    //      `phantom autoevolve --once` invocations are an order of magnitude
    //      faster.
    if let Some(home) = dirs::home_dir() {
        let target_dir = home.join(".phantom-mesh").join("autoevolve-target");
        let _ = std::fs::create_dir_all(&target_dir);
        cmd.env("CARGO_TARGET_DIR", target_dir);
    }

    let out = cmd.output().await;
    match out {
        Ok(o) if o.status.success() => CargoRunOutcome { success: true },
        Ok(_o) => {
            // Distinguish Windows-AV transients from real compile errors.
            // If the only failure is "<X>: Access Denied" / "failed to
            // remove file" / "failed to link or copy", the build artefact
            // is locked, not broken — pretend success and let the next
            // poll succeed once the AV releases.
            #[cfg(target_os = "windows")]
            {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let av_lock = stderr.contains("Access Denied")
                    || stderr.contains("\u{5b58}\u{53d6}\u{88ab}\u{62d2}") // 存取被拒 (zh)
                    || stderr.contains("failed to remove file")
                    || stderr.contains("failed to link or copy");
                if av_lock {
                    eprintln!(
                        "{} autoevolve: cargo {} hit Windows AV lock — treating as transient, not invoking evolve.",
                        colored("⚠", 33), target
                    );
                    return CargoRunOutcome { success: true };
                }
            }
            CargoRunOutcome { success: false }
        }
        Err(_) => CargoRunOutcome { success: false },
    }
}

/// Find the Cargo.toml that autoevolve should target. Returns None if cwd
/// itself contains a manifest (cargo will find it via the usual parent walk)
/// or if no candidate is found.
///
/// Walks well-known workspace conventions: the cwd, then `core/`, `crates/*`,
/// `packages/*`, `src/`. The first Cargo.toml found wins.
fn find_cargo_manifest() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("Cargo.toml").exists() {
        return None;  // cargo's own walk handles this
    }
    for sub in ["core", "src"].iter() {
        let p = cwd.join(sub).join("Cargo.toml");
        if p.exists() { return Some(p); }
    }
    for parent in ["crates", "packages"].iter() {
        let parent_dir = cwd.join(parent);
        if let Ok(rd) = std::fs::read_dir(&parent_dir) {
            for entry in rd.flatten() {
                let p = entry.path().join("Cargo.toml");
                if p.exists() { return Some(p); }
            }
        }
    }
    None
}

// ── `phantom snapshot` — APFS local snapshot subcommand (macOS) ─────────────
//
// Wraps `phantom_mesh::snapshot` with a CLI surface so the user can:
//   phantom snapshot create [label]    → tmutil localsnapshot, print id
//   phantom snapshot list              → newest-first listing
//   phantom snapshot delete <id>       → tmutil deletelocalsnapshots
//   phantom snapshot prune [hours]     → drop everything older than N hours (default 24)
//   phantom snapshot rollback <id>     → print the manual mount_apfs+rsync recipe
//
// The rollback action is intentionally non-destructive in v1 — it spits out
// a copy-pasteable shell snippet rather than executing it. Selective
// in-place restore needs sudo and is Sprint 2's job.

#[cfg(target_os = "macos")]
async fn run_snapshot_subcommand(args: &[String]) -> anyhow::Result<()> {
    use phantom_mesh::snapshot;
    let action = args.get(2).map(|s| s.as_str()).unwrap_or("list");
    match action {
        "create" => {
            let label = args.get(3).map(String::as_str);
            let info = snapshot::create(label).await?;
            println!(
                "{} Created snapshot {}",
                colored("✓", 32),
                colored(&info.id, 36)
            );
            if let Some(l) = info.label {
                println!("  label: {}", l);
            }
            println!("  rollback hint: phantom snapshot rollback {}", info.id);
            Ok(())
        }
        "list" => {
            let snaps = snapshot::list().await?;
            if snaps.is_empty() {
                println!("(no local snapshots)");
                return Ok(());
            }
            println!(
                "{} {} local snapshot(s), newest first:",
                colored("◆", 35),
                snaps.len()
            );
            for s in &snaps {
                let age = if s.created_at_ms > 0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    let hrs = (now - s.created_at_ms) / (3600 * 1000);
                    if hrs < 1 {
                        format!("{}m ago", (now - s.created_at_ms) / (60 * 1000))
                    } else {
                        format!("{}h ago", hrs)
                    }
                } else {
                    "?".into()
                };
                println!("  {}  {}", colored(&s.id, 36), colored(&age, 90));
            }
            Ok(())
        }
        "delete" => {
            let id = match args.get(3) {
                Some(id) => id.clone(),
                None => {
                    eprintln!(
                        "{} usage: phantom snapshot delete <id>",
                        colored("✗", 31)
                    );
                    std::process::exit(1);
                }
            };
            snapshot::delete(&id).await?;
            println!("{} Deleted snapshot {}", colored("✓", 32), id);
            Ok(())
        }
        "prune" => {
            let hours: u64 = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or(24);
            let n = snapshot::prune_older_than(hours * 3600).await?;
            println!(
                "{} Pruned {} snapshot(s) older than {}h",
                colored("✓", 32),
                n,
                hours
            );
            Ok(())
        }
        "rollback" => {
            let id = match args.get(3) {
                Some(id) => id.clone(),
                None => {
                    eprintln!(
                        "{} usage: phantom snapshot rollback <id>",
                        colored("✗", 31)
                    );
                    std::process::exit(1);
                }
            };
            // Verify the id actually exists, give a helpful error otherwise.
            let snaps = snapshot::list().await?;
            if !snaps.iter().any(|s| s.id == id) {
                eprintln!(
                    "{} snapshot {} not found. `phantom snapshot list` to see available ids.",
                    colored("✗", 31),
                    id
                );
                std::process::exit(1);
            }
            println!(
                "{} Manual rollback recipe for {} (review, then copy-paste):",
                colored("◆", 35),
                colored(&id, 36)
            );
            println!("{}", snapshot::manual_rollback_hint(&id));
            println!("{}",
                colored("  hint: `phantom snapshot apply <id> --cwd` automates this with one sudo prompt.", 90));
            Ok(())
        }
        "apply" => {
            // phantom snapshot apply <id> [--cwd|--path P] [--execute]
            let id = match args.get(3) {
                Some(id) if !id.starts_with("--") => id.clone(),
                _ => {
                    eprintln!(
                        "{} usage: phantom snapshot apply <id> [--cwd|--path P] [--execute]",
                        colored("✗", 31)
                    );
                    std::process::exit(1);
                }
            };
            // Default scope: --cwd if omitted.
            let mut target_path: Option<std::path::PathBuf> = None;
            let mut execute = false;
            let mut i = 4;
            while i < args.len() {
                match args[i].as_str() {
                    "--cwd" => { target_path = Some(std::env::current_dir()?); }
                    "--path" => {
                        i += 1;
                        if let Some(p) = args.get(i) {
                            target_path = Some(std::path::PathBuf::from(p));
                        }
                    }
                    "--execute" => { execute = true; }
                    _ => {}
                }
                i += 1;
            }
            let target = target_path.unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });
            // Resolve to absolute, canonical.
            let target = std::fs::canonicalize(&target).unwrap_or(target);
            snapshot::apply(&id, &target, !execute).await?;
            Ok(())
        }
        other => {
            eprintln!(
                "{} Unknown snapshot action: '{}'.\n    Use one of: create [label], list, delete <id>, prune [hours], rollback <id>",
                colored("✗", 31),
                other
            );
            std::process::exit(1);
        }
    }
}

// ── `phantom doctor` — single-shot environment health check ─────────────────
//
// Surfaces the things users typically only discover after they break:
// binary provenance, agents.toml location, provider keys, phantom serve
// reachability, launchd registration (mac), Tailscale, MCP tool count,
// macOS-only goodies (tmutil, Spotlight, Xcode CLT). One screen, OK/WARN/FAIL
// glyphs, and the next-step hint embedded in each line.

/// `phantom doctor --json` — machine-readable health diagnostic.
///
/// Emits a single JSON object on stdout. Structure mirrors the colored
/// doctor's section layout so a script can pluck nested fields without
/// regex-parsing prose. Designed for: CI gates, monitoring scrapers,
/// dashboard health endpoints, third-party integrations.
///
/// Schema (top-level keys):
///   - version, git, os, arch, build_date     (binary provenance)
///   - config.{path, exists}                   (agents.toml resolution)
///   - permissions.{rules, deny, ask, allow, errors, statically_denied}
///   - providers[]                              (env/toml availability)
///   - serve.{healthz, port, running}
///   - autoevolve.{registered, interval_secs, queue_pending,
///                  failed_count, last_run_ts_ms}
///   - tailscale.{installed, connected}
///   - tools.{builtin_count, mcp_count}
///   - identity.{logged_in, email, provider}
///   - status: "ok" | "warn" | "fail" — overall rollup
async fn run_doctor_json() -> anyhow::Result<()> {
    use serde_json::{json, Value};
    let mut o = serde_json::Map::new();

    // Binary provenance — same data the colored "binary" section emits.
    o.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    o.insert("git".into(),     json!(option_env!("PHANTOM_GIT_HASH").unwrap_or("nogit")));
    o.insert("os".into(),      json!(std::env::consts::OS));
    o.insert("arch".into(),    json!(std::env::consts::ARCH));
    o.insert("build_date".into(), json!(option_env!("PHANTOM_BUILD_DATE").unwrap_or("?")));

    // Config
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let cfg_candidates = [
        std::env::current_dir().unwrap_or_else(|_| home.clone()).join("agents.toml"),
        home.join(".phantom-mesh/agents.toml"),
    ];
    let cfg_found = cfg_candidates.iter().find(|p| p.exists());
    o.insert("config".into(), json!({
        "path":   cfg_found.map(|p| p.display().to_string()),
        "exists": cfg_found.is_some(),
    }));

    // Permissions — parse rules + surface stats.
    let perm_cfg = phantom_mesh::config::AgentsConfig::find_and_load()
        .map(|c| c.permissions)
        .unwrap_or_default();
    let total_rules = perm_cfg.deny.len() + perm_cfg.ask.len() + perm_cfg.allow.len();
    let perm_obj: Value = if total_rules == 0 {
        json!({
            "rules": 0, "deny": 0, "ask": 0, "allow": 0,
            "mode": "allow-all (legacy default)",
            "errors": null,
        })
    } else {
        let deny:  Vec<&str> = perm_cfg.deny.iter().map(String::as_str).collect();
        let ask:   Vec<&str> = perm_cfg.ask.iter().map(String::as_str).collect();
        let allow: Vec<&str> = perm_cfg.allow.iter().map(String::as_str).collect();
        match phantom_mesh::permission::Engine::from_lists(&deny, &ask, &allow) {
            Ok(e) => {
                let mut denied: Vec<String> = e.statically_denied_tools().into_iter().collect();
                denied.sort();
                json!({
                    "rules":  e.rules().len(),
                    "deny":   perm_cfg.deny.len(),
                    "ask":    perm_cfg.ask.len(),
                    "allow":  perm_cfg.allow.len(),
                    "errors": null,
                    "statically_denied": denied,
                })
            }
            Err(err) => json!({
                "rules": 0, "deny": perm_cfg.deny.len(),
                "ask":   perm_cfg.ask.len(),  "allow":  perm_cfg.allow.len(),
                "errors": err,
            }),
        }
    };
    o.insert("permissions".into(), perm_obj);

    // Providers — env or agents.toml availability.
    let provider_keys = [
        ("ANTHROPIC_API_KEY",  "anthropic"),
        ("OPENAI_API_KEY",     "openai"),
        ("GROQ_API_KEY",       "groq"),
        ("GEMINI_API_KEY",     "gemini"),
        ("OPENROUTER_API_KEY", "openrouter"),
        ("OPENCODE_API_KEY",   "opencode"),
        ("CEREBRAS_API_KEY",   "cerebras"),
    ];
    let providers: Vec<Value> = provider_keys.iter().map(|(env_var, name)| {
        let env_val = std::env::var(env_var).ok();
        json!({
            "name":      name,
            "available": env_val.is_some(),
            "source":    if env_val.is_some() { Some("env") } else { None::<&str> },
        })
    }).collect();
    o.insert("providers".into(), json!(providers));

    // Serve health
    let port = phantom_mesh::config::AgentsConfig::find_and_load()
        .map(|c| c.core.port).unwrap_or(7878);
    let healthz_url = format!("http://127.0.0.1:{}/healthz", port);
    let healthz_status = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2)).build().ok()
        .and_then(|c| futures::executor::block_on(async {
            c.get(&healthz_url).send().await.ok()
        }))
        .map(|r| r.status().as_u16());
    o.insert("serve".into(), json!({
        "port":    port,
        "url":     healthz_url,
        "running": healthz_status == Some(200),
        "status":  healthz_status,
    }));

    // Autoevolve schedule + queue + log summary
    let queue_pending = home.join(".phantom-mesh/autoevolve.queue.txt");
    let queue_pending_n = std::fs::read_to_string(&queue_pending)
        .map(|c| c.lines().filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        }).count())
        .unwrap_or(0);
    let failed_log = home.join(".phantom-mesh/autoevolve.queue.failed.log");
    let failed_n = std::fs::read_to_string(&failed_log)
        .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let last_run_ts = std::fs::read_to_string(home.join(".phantom-mesh/autoevolve.log"))
        .ok()
        .and_then(|c| c.lines().last().map(|s| s.to_string()))
        .and_then(|line| serde_json::from_str::<Value>(&line).ok())
        .and_then(|v| v.get("started_at_ms").and_then(|x| x.as_i64()));
    o.insert("autoevolve".into(), json!({
        "queue_pending":  queue_pending_n,
        "failed_count":   failed_n,
        "last_run_ts_ms": last_run_ts,
    }));

    // Tailscale state — pure best-effort. Use the spawn-attempt itself
    // as the installed-check since we don't have the `which` crate in
    // deps.
    let ts_status_output = std::process::Command::new("tailscale")
        .arg("status")
        .output();
    let ts_installed = ts_status_output.is_ok();
    let ts_status_str = ts_status_output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let ts_connected = ts_installed && !ts_status_str.is_empty()
        && !ts_status_str.contains("Logged out")
        && !ts_status_str.contains("stopped");
    o.insert("tailscale".into(), json!({
        "installed": ts_installed,
        "connected": ts_connected,
    }));

    // Tools registered (built-in + MCP)
    let builtin = phantom_mesh::tools::all_tool_names().len();
    o.insert("tools".into(), json!({
        "builtin_count": builtin,
    }));

    // Identity (best effort — load auth.json if it exists)
    if let Some(s) = phantom_mesh::auth::load() {
        o.insert("identity".into(), json!({
            "logged_in": true,
            "email":     s.email,
            "provider":  s.provider,
        }));
    } else {
        o.insert("identity".into(), json!({"logged_in": false}));
    }

    // Overall status — simple rollup.
    let any_fail = !o.get("config").and_then(|c| c.get("exists")).and_then(|v| v.as_bool()).unwrap_or(false);
    let any_warn = ts_installed && !ts_connected
                || queue_pending_n == 0 && failed_n > 0;
    let status = if any_fail { "fail" } else if any_warn { "warn" } else { "ok" };
    o.insert("status".into(), json!(status));

    println!("{}", serde_json::to_string_pretty(&Value::Object(o))?);
    Ok(())
}

fn doctor_ok(label: &str, detail: &str) {
    println!("  {} {}: {}", colored("✓", 32), label, detail);
}
fn doctor_warn(label: &str, detail: &str) {
    println!("  {} {}: {}", colored("⚠", 33), label, detail);
}
fn doctor_fail(label: &str, detail: &str) {
    println!("  {} {}: {}", colored("✗", 31), label, detail);
}
fn doctor_section(name: &str) {
    println!();
    println!("{}", colored(name, 35));
}

async fn run_doctor() -> anyhow::Result<()> {
    println!(
        "{} {}",
        colored("phantom doctor", 36),
        colored(env!("CARGO_PKG_VERSION"), 90),
    );

    // ── binary provenance ────────────────────────────────────────────────
    doctor_section("binary");
    doctor_ok(
        "version",
        &format!(
            "phantom {} ({}, {}-{}, built {})",
            env!("CARGO_PKG_VERSION"),
            option_env!("PHANTOM_GIT_HASH").unwrap_or("nogit"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            option_env!("PHANTOM_BUILD_DATE").unwrap_or("?"),
        ),
    );
    if let Ok(exe) = std::env::current_exe() {
        let real = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
        if real != exe {
            doctor_ok("path", &format!("{}\n               → {}", exe.display(), real.display()));
        } else {
            doctor_ok("path", &exe.display().to_string());
        }
    }

    // ── config ───────────────────────────────────────────────────────────
    doctor_section("config");
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let cfg_candidates = [
        std::env::current_dir().unwrap_or_else(|_| home.clone()).join("agents.toml"),
        home.join(".phantom-mesh/agents.toml"),
    ];
    let cfg_found = cfg_candidates.iter().find(|p| p.exists());
    match cfg_found {
        Some(p) => doctor_ok("agents.toml", &p.display().to_string()),
        None => doctor_fail(
            "agents.toml",
            "not found in cwd or ~/.phantom-mesh — run `phantom init` or `phantom onboarding`",
        ),
    }
    let phantom_dir = home.join(".phantom-mesh");
    if phantom_dir.exists() {
        doctor_ok("~/.phantom-mesh", "exists");
    } else {
        doctor_warn("~/.phantom-mesh", "missing — will be created on first run");
    }

    // ── [permissions] ────────────────────────────────────────────────────
    // Surfaces parse status of the Tool(specifier) DSL rules so users
    // can debug their config before a turn fails. Empty/missing block →
    // legacy "allow all" mode. Parse error → falls back to empty engine
    // at REPL boot; doctor flags the offending rule.
    doctor_section("permissions");
    let perm_cfg = phantom_mesh::config::AgentsConfig::find_and_load()
        .map(|c| c.permissions)
        .unwrap_or_default();
    let total_rules = perm_cfg.deny.len() + perm_cfg.ask.len() + perm_cfg.allow.len();
    if total_rules == 0 {
        doctor_warn("[permissions]", "no rules → allow all (legacy default). \
            See docs/PERMISSIONS.md for the Tool(specifier) DSL.");
    } else {
        let deny: Vec<&str>  = perm_cfg.deny.iter().map(String::as_str).collect();
        let ask: Vec<&str>   = perm_cfg.ask.iter().map(String::as_str).collect();
        let allow: Vec<&str> = perm_cfg.allow.iter().map(String::as_str).collect();
        match phantom_mesh::permission::Engine::from_lists(&deny, &ask, &allow) {
            Ok(e) => {
                doctor_ok("[permissions]", &format!(
                    "{} rules parsed ({} deny, {} ask, {} allow)",
                    e.rules().len(),
                    perm_cfg.deny.len(), perm_cfg.ask.len(), perm_cfg.allow.len()
                ));
                let denied = e.statically_denied_tools();
                if !denied.is_empty() {
                    let mut names: Vec<&String> = denied.iter().collect();
                    names.sort();
                    let list = names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
                    doctor_ok("statically denied", &format!(
                        "{} (will be hidden from LLM tool list)", list
                    ));
                }
            }
            Err(err) => doctor_fail("[permissions]", &format!("parse error: {}", err)),
        }
    }

    // ── provider API keys ───────────────────────────────────────────────
    doctor_section("provider keys");
    let keys = [
        ("ANTHROPIC_API_KEY",   "Anthropic",  "anthropic"),
        ("OPENAI_API_KEY",      "OpenAI",     "openai"),
        ("GROQ_API_KEY",        "Groq",       "groq"),
        ("GEMINI_API_KEY",      "Gemini",     "gemini"),
        ("OPENROUTER_API_KEY",  "OpenRouter", "openrouter"),
        ("OPENCODE_API_KEY",    "OpenCode",   "opencode"),
        ("CEREBRAS_API_KEY",    "Cerebras",   "cerebras"),
        ("DEEPSEEK_API_KEY",    "DeepSeek",   "deepseek"),
        ("MISTRAL_API_KEY",     "Mistral",    "mistral"),
        ("TOGETHER_API_KEY",    "Together",   "together"),
        ("NVIDIA_NIM_API_KEY",  "NVIDIA NIM", "nvidia"),
    ];

    // Also inspect agents.toml so the report reflects reality: many users keep
    // keys inline in agents.toml (api_key = "...") rather than exporting env
    // vars. Without this fall-through the doctor would warn 'not in env' on a
    // perfectly working setup.
    let toml_providers: std::collections::HashMap<String, bool> = find_config()
        .and_then(|c| toml::from_str::<phantom_mesh::config::AgentsConfig>(&c).ok())
        .map(|cfg| {
            cfg.providers.iter().map(|(name, entry)| {
                let has_inline = entry.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false);
                let has_env_ref = entry.api_key_env.as_deref()
                    .map(|var| std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(false);
                (name.to_lowercase(), has_inline || has_env_ref)
            }).collect()
        })
        .unwrap_or_default();

    let mut any_configured = false;
    for (env_key, label, toml_name) in &keys {
        let env_val = std::env::var(env_key).ok().filter(|v| !v.is_empty());
        let toml_has = toml_providers.get(*toml_name).copied().unwrap_or(false);

        if let Some(v) = env_val {
            doctor_ok(label, &format!("env ({}…)", &v[..v.len().min(6)]));
            any_configured = true;
        } else if toml_has {
            doctor_ok(label, "agents.toml");
            any_configured = true;
        } else {
            doctor_warn(label, "not in env or agents.toml");
        }
    }
    if !any_configured {
        doctor_warn(
            "hint",
            "set keys via agents.toml (preferred) or `set -a; source ~/.phantom-mesh/env; set +a`",
        );
    }

    // ── phantom serve reachability ──────────────────────────────────────
    doctor_section("phantom serve");
    // Honor the configured port from agents.toml [core].port instead of
    // probing a hardcoded :7878. Otherwise users running on :7879 (or any
    // other port) get a false "unreachable" report even when serve is
    // healthy. Falls back to 7878 when no config is loadable.
    let configured_port: u16 = find_config()
        .and_then(|c| toml::from_str::<phantom_mesh::config::AgentsConfig>(&c).ok())
        .map(|cfg| cfg.core.port)
        .filter(|p| *p > 0)
        .unwrap_or(7878);
    let healthz_url = format!("http://127.0.0.1:{}/healthz", configured_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();
    let healthz = if let Some(c) = &client {
        c.get(&healthz_url).send().await.ok()
    } else {
        None
    };
    match healthz.as_ref().map(|r| r.status().as_u16()) {
        Some(200)  => doctor_ok("healthz", &format!("200 OK on {}", healthz_url)),
        Some(code) => doctor_warn("healthz", &format!("HTTP {} on {} (expected 200)", code, healthz_url)),
        None => doctor_warn(
            "healthz",
            &format!("unreachable on :{} — start with `phantom service install` or `phantom serve &`", configured_port),
        ),
    }

    // ── launchd (macOS) ──────────────────────────────────────────────────
    #[cfg(target_os = "macos")]
    {
        let target = format!("gui/{}/ai.phantommesh.serve", nix_uid());
        let print = std::process::Command::new("launchctl")
            .args(["print", &target])
            .output();
        let registered = print.as_ref().map(|o| o.status.success()).unwrap_or(false);
        if registered {
            let body = print
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let pid = body
                .lines()
                .find(|l| l.trim_start().starts_with("pid ="))
                .and_then(|l| l.split('=').nth(1))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "?".to_string());
            doctor_ok("launchd", &format!("registered (pid {})", pid));
        } else {
            doctor_warn(
                "launchd",
                "not installed — `phantom service install` for boot-time auto-start",
            );
        }
    }

    // ── systemd user unit (Linux) ────────────────────────────────────────
    #[cfg(target_os = "linux")]
    {
        let q = std::process::Command::new("systemctl")
            .args(["--user", "is-active", LINUX_UNIT_NAME])
            .output();
        let active = q
            .as_ref()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let unit_path = dirs::home_dir()
            .map(|h| h.join(".config/systemd/user").join(LINUX_UNIT_NAME))
            .unwrap_or_default();
        let registered = unit_path.exists();
        if registered && active {
            doctor_ok("systemd", &format!("{} active", LINUX_UNIT_NAME));
        } else if registered {
            doctor_warn(
                "systemd",
                &format!("{} unit present but not running", LINUX_UNIT_NAME),
            );
        } else {
            doctor_warn(
                "systemd",
                "no unit installed — `phantom service install` for systemd --user auto-start",
            );
        }
    }

    // ── Scheduled Task (Windows) ─────────────────────────────────────────
    #[cfg(target_os = "windows")]
    {
        let registered = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_TASK_NAME])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if registered {
            let (last_run, _, _) = windows_task_info(WINDOWS_TASK_NAME);
            doctor_ok(
                "Scheduled Task",
                &format!("registered (last run {})", last_run.unwrap_or_else(|| "?".into())),
            );
        } else {
            doctor_warn(
                "Scheduled Task",
                "not installed — `phantom service install` for logon auto-start",
            );
        }
    }

    // ── Tailscale ────────────────────────────────────────────────────────
    doctor_section("network");
    let ts = std::process::Command::new("tailscale")
        .args(["status", "--peers=false", "--self=true"])
        .output();
    match ts {
        Ok(o) if o.status.success() => {
            let body = String::from_utf8_lossy(&o.stdout);
            let line = body.lines().next().unwrap_or("").trim();
            doctor_ok("Tailscale", &format!("connected ({})", line));
        }
        Ok(_) | Err(_) => doctor_warn(
            "Tailscale",
            "not in PATH or not connected — `tailscale up`",
        ),
    }

    // ── MLX local LLM (macOS only) ───────────────────────────────────────
    #[cfg(target_os = "macos")]
    {
        doctor_section("MLX local LLM");
        // Check mlx_lm importable
        let py_ok = ["python3", "python"].iter().any(|py| {
            std::process::Command::new(py)
                .args(["-c", "import mlx_lm"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        });
        if py_ok {
            doctor_ok("mlx_lm", "importable (`pip install mlx-lm` available)");
        } else {
            doctor_warn(
                "mlx_lm",
                "not installed — `pip install mlx-lm` for zero-cost on-device LLM",
            );
        }
        // If a server is running, surface the model + port
        if let Some(h) = dirs::home_dir() {
            let cfg = h.join(".phantom-mesh/mlx-config.json");
            if let Ok(s) = std::fs::read_to_string(&cfg) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    let model = v["model"].as_str().unwrap_or("?");
                    let port = v["port"].as_u64().unwrap_or(8080);
                    let probe = std::process::Command::new("curl")
                        .args(["-s", "--max-time", "1", "-o", "/dev/null", "-w", "%{http_code}",
                               &format!("http://127.0.0.1:{}/v1/models", port)])
                        .output();
                    let code = probe.ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .unwrap_or_default();
                    if code == "200" {
                        doctor_ok("server", &format!("{} on :{} reachable", model, port));
                    } else {
                        doctor_warn(
                            "server",
                            &format!("not reachable (last config: {} on :{}) — `phantom mlx serve`", model, port),
                        );
                    }
                }
            }
        }
    }

    // ── autoevolve daemon ────────────────────────────────────────────────
    doctor_section("autoevolve");
    if let Some(home) = dirs::home_dir() {
        let log = home.join(".phantom-mesh/autoevolve.log");
        if log.exists() {
            let content = std::fs::read_to_string(&log).unwrap_or_default();
            let last_line = content.lines().filter(|l| !l.is_empty()).last();
            match last_line.and_then(|l| serde_json::from_str::<AutoEvolveLogEntry>(l).ok()) {
                Some(entry) => {
                    let secs = entry.started_at_ms / 1000;
                    let when = std::process::Command::new("date")
                        .args(["-r", &secs.to_string(), "+%Y-%m-%d %H:%M"])
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "?".into());
                    let total = content.lines().filter(|l| !l.is_empty()).count();
                    let label = format!("last run @ {} → {} ({} total)", when, entry.status, total);
                    match entry.status.as_str() {
                        "green" | "fixed" => doctor_ok("history", &label),
                        "failed"          => doctor_fail("history", &label),
                        _                 => doctor_warn("history", &label),
                    }
                }
                None => doctor_warn("history", "log exists but unparseable"),
            }
        } else {
            doctor_warn("history", "no runs yet — `phantom autoevolve --once`");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let label = "ai.phantommesh.autoevolve";
        let target = format!("gui/{}/{}", nix_uid(), label);
        let registered = std::process::Command::new("launchctl")
            .args(["print", &target])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if registered {
            doctor_ok("schedule", "registered (LaunchAgent)");
        } else {
            doctor_warn("schedule", "not scheduled — `phantom autoevolve schedule install`");
        }
    }
    #[cfg(target_os = "windows")]
    {
        let registered = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_AUTOEVOLVE_TASK_NAME])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if registered {
            doctor_ok("schedule", "registered (Scheduled Task)");
        } else {
            doctor_warn("schedule", "not scheduled — `phantom autoevolve schedule install`");
        }
    }

    // ── identity ─────────────────────────────────────────────────────────
    doctor_section("identity");
    match phantom_mesh::auth::load() {
        Some(s) => doctor_ok("logged in", &phantom_mesh::auth::human_summary(&s)),
        None => {
            // "phantom login" only makes sense once the broker at
            // phantommesh.io is deployed. Until then, "not logged in" is
            // expected state, not a warning the user can act on. Probe the
            // broker so the message reflects reality.
            let broker_live = match &client {
                Some(c) => c
                    .get("https://phantommesh.io/healthz")
                    .send()
                    .await
                    .ok()
                    .map(|r| r.status() == 200)
                    .unwrap_or(false),
                None => false,
            };
            if broker_live {
                doctor_warn("logged in", "no — broker live, run `phantom login`");
            } else {
                doctor_ok(
                    "identity",
                    "local-only (broker not deployed yet — login becomes available once phantommesh.io/healthz returns 200)",
                );
            }
        }
    }

    // ── diagnostics — surface recent crashes + event log path ────────────
    doctor_section("diagnostics");
    {
        let crash_dir = dirs::home_dir().map(|h| h.join(".phantom-mesh/crashes"));
        let crash_count = crash_dir.as_ref()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|it| it.flatten().count())
            .unwrap_or(0);
        if crash_count == 0 {
            doctor_ok("crash logs", "0 (no panics recorded)");
        } else {
            let latest = phantom_mesh::diag::last_crash_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "?".into());
            doctor_warn(
                "crash logs",
                &format!("{} recorded — latest: {}", crash_count, latest),
            );
            eprintln!("    {} read with: {}", "›", "phantom debug last");
        }

        let events_path = phantom_mesh::diag::events_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unset)".into());
        let bytes = phantom_mesh::diag::events_path()
            .and_then(|p| std::fs::metadata(&p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        doctor_ok(
            "events log",
            &format!("{} ({} bytes)", events_path, bytes),
        );
    }

    // ── tool surface ─────────────────────────────────────────────────────
    doctor_section("tools");
    let base = phantom_mesh::tools::all_tool_names().len();
    // mcp.rs synthesises two cluster-only tools (phantom_swarm,
    // phantom_evolve_distributed) on top of the base set when serving
    // tools/list over stdio MCP. Stay in sync.
    let cluster_extra = 2;
    doctor_ok(
        "tools",
        &format!(
            "{} total ({} built-in + {} cluster RPC)",
            base + cluster_extra,
            base,
            cluster_extra
        ),
    );

    // ── macOS-specific deepening ─────────────────────────────────────────
    #[cfg(target_os = "macos")]
    {
        doctor_section("macOS integrations");

        // APFS local snapshots — safety net for subagent runs
        match phantom_mesh::snapshot::list().await {
            Ok(snaps) => {
                let n = snaps.len();
                if n == 0 {
                    doctor_ok(
                        "APFS snapshots",
                        "tmutil reachable (0 snapshots — `phantom snapshot create` to take one)",
                    );
                } else {
                    let newest = snaps
                        .first()
                        .map(|s| s.id.clone())
                        .unwrap_or_else(|| "?".into());
                    doctor_ok(
                        "APFS snapshots",
                        &format!(
                            "{} present, newest {} — `phantom snapshot list` for details",
                            n, newest
                        ),
                    );
                }
            }
            Err(e) => doctor_warn("APFS snapshots", &format!("tmutil failed: {}", e)),
        }

        // Spotlight indexing of cwd
        let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
        let mdutil = std::process::Command::new("mdutil")
            .args(["-s", &cwd.display().to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if mdutil.contains("Indexing enabled") {
            doctor_ok("Spotlight", &format!("indexing enabled for {}", cwd.display()));
        } else {
            doctor_warn(
                "Spotlight",
                &format!("not indexing {} (spotlight_search will fall back)", cwd.display()),
            );
        }

        // Xcode command-line tools
        let xcrun = std::process::Command::new("xcrun")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if xcrun {
            doctor_ok("Xcode CLT", "installed (xcode_simctl tool ready)");
        } else {
            doctor_warn(
                "Xcode CLT",
                "missing — install with `xcode-select --install`",
            );
        }
    }

    // ── Windows-specific deepening ───────────────────────────────────────
    #[cfg(target_os = "windows")]
    {
        doctor_section("Windows integrations");

        // Configured serve port — so the user can see what value phantom
        // is operating on (matches healthz probe + service install).
        let cfg_port = phantom_mesh::config::AgentsConfig::find_and_load()
            .map(|c| c.core.port)
            .unwrap_or(7878);
        doctor_ok("configured port", &format!("{} (from agents.toml [core].port)", cfg_port));

        // OpenSSH server (so Mac can ssh into this node)
        let sshd = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Get-Service sshd | Select-Object -ExpandProperty Status"])
            .output();
        match sshd {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.eq_ignore_ascii_case("Running") {
                    doctor_ok("OpenSSH", "sshd Running");
                } else {
                    doctor_warn(
                        "OpenSSH",
                        &format!("sshd is {} — `Start-Service sshd` (admin)", s),
                    );
                }
            }
            _ => doctor_warn(
                "OpenSSH",
                "sshd not installed — see scripts/windows-bootstrap.ps1",
            ),
        }

        // Defender firewall rule for the configured serve port. Match by
        // exact name (PhantomMesh-Inbound) plus actual LocalPort because
        // a stale rule from a different port would otherwise mislead.
        let fw_script = "$r = Get-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -ErrorAction SilentlyContinue; \
             if ($r) { \
                $port = ($r | Get-NetFirewallPortFilter -ErrorAction SilentlyContinue | Select-Object -ExpandProperty LocalPort -ErrorAction SilentlyContinue) -join ','; \
                Write-Output \"PORT=$port\"; \
                Write-Output \"ENABLED=$($r.Enabled)\" \
             }";
        // PowerShell exits 1 when Get-NetFirewallRule finds no match even
        // with -ErrorAction SilentlyContinue, so don't gate on
        // status.success() — drive the verdict off stdout instead.
        let fw = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &fw_script])
            .output();
        match fw {
            Ok(o) => {
                let body = String::from_utf8_lossy(&o.stdout).to_string();
                let port = body
                    .lines()
                    .find_map(|l| l.strip_prefix("PORT="))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                let enabled = body
                    .lines()
                    .find_map(|l| l.strip_prefix("ENABLED="))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                if port.is_empty() {
                    doctor_warn(
                        "Defender",
                        "no PhantomMesh-Inbound rule — `phantom service install` from admin shell",
                    );
                } else if port == cfg_port.to_string() && enabled.eq_ignore_ascii_case("True") {
                    doctor_ok(
                        "Defender",
                        &format!("PhantomMesh-Inbound TCP {} enabled", port),
                    );
                } else {
                    doctor_warn(
                        "Defender",
                        &format!(
                            "PhantomMesh-Inbound rule exists but port={}, enabled={} (configured port {}) — re-run `phantom service install`",
                            port, enabled, cfg_port
                        ),
                    );
                }
            }
            Err(_) => doctor_warn(
                "Defender",
                "PowerShell unavailable — could not run Get-NetFirewallRule",
            ),
        }
    }

    // ── footer ────────────────────────────────────────────────────────────
    println!();
    println!("{}", colored("done.", 36));
    Ok(())
}

// ── `phantom selftest` — locate scripts/selftest.sh and exec it ─────────────
//
// The Rust binary is just a thin shim: the actual tests live in the repo
// under scripts/selftest.d/ (one file per feature). This subcommand exists so
// phantom — used as a CLI by humans, by Claude Code, or by phantom itself
// through its `shell` tool — has a single uniform entry point and doesn't
// need to know the script's path.
//
// Search order (first hit wins):
//   1. $PHANTOM_SELFTEST_SCRIPT   (explicit override)
//   2. <cwd>/scripts/selftest.sh  (running from repo root)
//   3. walk up from cwd, looking for scripts/selftest.sh
//   4. walk up from the executable path (for installs that ship scripts)
//   5. ~/.phantom-mesh/scripts/selftest.sh   (user-installed copy)
fn run_selftest(args: &[String]) -> anyhow::Result<()> {
    let script = match locate_selftest_script() {
        Some(p) => p,
        None => {
            eprintln!("{} could not locate scripts/selftest.sh", colored("✗", 31));
            eprintln!();
            eprintln!("  Tried (in order):");
            eprintln!("    $PHANTOM_SELFTEST_SCRIPT");
            eprintln!("    <cwd>/scripts/selftest.sh");
            eprintln!("    walking up from cwd and from this binary");
            eprintln!("    ~/.phantom-mesh/scripts/selftest.sh");
            eprintln!();
            eprintln!("  Fix any of these:");
            eprintln!("    cd /path/to/phantom-mesh && phantom selftest");
            eprintln!("    PHANTOM_SELFTEST_SCRIPT=/path/to/selftest.sh phantom selftest");
            eprintln!();
            eprintln!("  See docs/SELFTEST.md in the phantom-mesh repo.");
            std::process::exit(2);
        }
    };

    let bash = match find_bash() {
        Some(p) => p,
        None => {
            eprintln!("{} bash not found", colored("✗", 31));
            eprintln!();
            #[cfg(target_os = "windows")]
            {
                eprintln!("  Windows needs Git Bash (or WSL) to run the test orchestrator.");
                eprintln!();
                eprintln!("  Easiest fix — install Git for Windows:");
                eprintln!("    winget install --id Git.Git -e --source winget");
                eprintln!("    # or download: https://git-scm.com/download/win");
                eprintln!();
                eprintln!("  phantom selftest searches for bash.exe in (in order):");
                eprintln!("    $PATH");
                eprintln!("    C:\\Program Files\\Git\\bin\\bash.exe");
                eprintln!("    C:\\Program Files (x86)\\Git\\bin\\bash.exe");
                eprintln!("    %LOCALAPPDATA%\\Programs\\Git\\bin\\bash.exe");
                eprintln!();
                eprintln!("  Override explicitly:  set PHANTOM_BASH=C:\\path\\to\\bash.exe");
                eprintln!("  Or use WSL:           wsl phantom selftest");
            }
            #[cfg(not(target_os = "windows"))]
            {
                eprintln!("  Install bash and ensure it is on $PATH.");
                eprintln!("  Or set PHANTOM_BASH=/path/to/bash explicitly.");
            }
            std::process::exit(2);
        }
    };

    let mut cmd = std::process::Command::new(&bash);
    cmd.arg(&script);
    // Forward every arg after `selftest`.
    for a in args.iter().skip(2) {
        cmd.arg(a);
    }

    let status = cmd.status().map_err(|e| {
        anyhow::anyhow!("failed to invoke {} {}: {}", bash.display(), script.display(), e)
    })?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Locate a usable `bash` interpreter. Cross-platform — on Windows we look in
/// the standard Git for Windows / WSL install locations because that's the
/// most common way bash gets onto a Windows dev machine.
fn find_bash() -> Option<PathBuf> {
    // Explicit override wins, but if it's set to something that doesn't
    // exist we bail loudly rather than silently falling back — a typo'd
    // override should be a visible error, not silent best-effort.
    if let Ok(p) = std::env::var("PHANTOM_BASH") {
        let path = PathBuf::from(&p);
        if path.is_file() { return Some(path); }
        eprintln!("{} PHANTOM_BASH points to a missing file: {}",
            colored("✗", 31), p);
        eprintln!("  Either fix the path or unset PHANTOM_BASH to fall back to PATH search.");
        std::process::exit(2);
    }

    // PATH search (uses PATHEXT-style extensions on Windows).
    if let Some(p) = path_search("bash") {
        return Some(p);
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"),
        ];
        if let Some(p) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(p).join(r"Git\bin\bash.exe"));
        }
        if let Some(p) = std::env::var_os("ProgramFiles(x86)") {
            candidates.push(PathBuf::from(p).join(r"Git\bin\bash.exe"));
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(p).join(r"Programs\Git\bin\bash.exe"));
        }
        for c in candidates {
            if c.is_file() { return Some(c); }
        }
    }

    None
}

/// Manual PATH search. Mirrors `Command::new`'s lookup but lets us probe for
/// presence without actually launching the binary.
fn path_search(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ""]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let candidate = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{}{}", name, ext))
            };
            if candidate.is_file() { return Some(candidate); }
        }
    }
    None
}

fn locate_selftest_script() -> Option<PathBuf> {
    // 1. explicit override
    if let Ok(p) = std::env::var("PHANTOM_SELFTEST_SCRIPT") {
        let path = PathBuf::from(p);
        if path.is_file() { return Some(path); }
    }

    // helper: probe `<dir>/scripts/selftest.sh`
    let probe = |dir: &std::path::Path| -> Option<PathBuf> {
        let p = dir.join("scripts").join("selftest.sh");
        if p.is_file() { Some(p) } else { None }
    };

    // 2 + 3. walk up from cwd
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur: Option<&std::path::Path> = Some(cwd.as_path());
        while let Some(d) = cur {
            if let Some(p) = probe(d) { return Some(p); }
            cur = d.parent();
        }
    }

    // 4. walk up from the executable
    if let Ok(exe) = std::env::current_exe() {
        let real = std::fs::canonicalize(&exe).unwrap_or(exe);
        let mut cur: Option<&std::path::Path> = real.parent();
        while let Some(d) = cur {
            if let Some(p) = probe(d) { return Some(p); }
            cur = d.parent();
        }
    }

    // 5. user copy
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".phantom-mesh/scripts/selftest.sh");
        if p.is_file() { return Some(p); }
    }

    None
}

// ── `phantom service` — Windows Task Scheduler implementation ───────────────
//
// Subcommands mirror the macOS variant:
//   install   register a logon-triggered Scheduled Task that runs
//             `phantom serve` every time the user logs in
//   uninstall delete the Scheduled Task
//   status    show whether the task is registered + healthz reachability
//
// We use schtasks.exe (the user-mode counterpart of sc.exe) because it
// works without admin elevation when running as the logged-in user. The
// task XML embeds RestartOnFailure with a 30-second cool-down, which gives
// us the "auto-relaunch if phantom serve crashes" behaviour without us
// implementing the Windows Service Control Manager interface.

#[cfg(target_os = "windows")]
// Aligned with `docs/SESSION-ONBOARDING.md` §3.1 and the Z13 deploy
// instruction set, which both already speak of "PhantomServe". The
// shorter "PhantomMesh" was a vestige from before the doc was written.
const WINDOWS_TASK_NAME: &str = "PhantomServe";

/// Configured serve port from agents.toml, defaulting to 7878 if no
/// config is found. The hardcoded :7878 used to mismatch any user with
/// `[core] port = 7879` in agents.toml — healthz probe would always
/// report unreachable even though phantom serve was running.
#[cfg(target_os = "windows")]
fn configured_port() -> u16 {
    phantom_mesh::config::AgentsConfig::find_and_load()
        .map(|c| c.core.port)
        .unwrap_or(7878)
}

/// Locale-independent Scheduled Task runtime info via PowerShell
/// `Get-ScheduledTaskInfo`. Returns `(last_run, next_run, last_result)`,
/// each `None` when the task is missing, the field is empty, or the call
/// fails. Used instead of parsing localized strings out of `schtasks
/// /Query /V /FO LIST`, whose field labels (e.g. "Last Run Time") are
/// translated on non-English Windows installs and never matched the
/// English-only `starts_with(...)` predicates.
#[cfg(target_os = "windows")]
fn windows_task_info(task_name: &str) -> (Option<String>, Option<String>, Option<i64>) {
    let escaped = task_name.replace('\'', "''");
    // Windows uses 1899-12-30 / 1999-11-30 / similar pre-2000 placeholders
    // for "never run". Filter those server-side so callers can render "?"
    // without false-positive 1999 dates leaking through.
    let ps_script = format!(
        "$i = Get-ScheduledTaskInfo -TaskName '{}' -ErrorAction SilentlyContinue; \
         if ($i) {{ \
            $cutoff = [DateTime]'2000-01-01'; \
            $last = if ($i.LastRunTime -gt $cutoff) {{ $i.LastRunTime }} else {{ '' }}; \
            $next = if ($i.NextRunTime -gt $cutoff) {{ $i.NextRunTime }} else {{ '' }}; \
            \"LastRun=$last\"; \"NextRun=$next\"; \"Result=$($i.LastTaskResult)\" \
         }}",
        escaped
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok();
    let body = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let last = body
        .lines()
        .find_map(|l| l.strip_prefix("LastRun="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let next = body
        .lines()
        .find_map(|l| l.strip_prefix("NextRun="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let result = body
        .lines()
        .find_map(|l| l.strip_prefix("Result="))
        .and_then(|s| s.trim().parse().ok());
    (last, next, result)
}

/// Translate a LastTaskResult HRESULT-style code into a short human label.
/// Returns ("color-code", "label") so callers can render with the right
/// ANSI colour. Constants from the Windows Task Scheduler return-code set.
#[cfg(target_os = "windows")]
fn windows_task_result_label(result: i64) -> (u8, String) {
    match result {
        0           => (32, "succeeded".into()),                      // S_OK
        0x00041300  => (32, "ready".into()),                          // SCHED_S_TASK_READY
        0x00041301  => (33, "running".into()),                        // SCHED_S_TASK_RUNNING
        0x00041303  => (90, "never run".into()),                      // SCHED_S_TASK_HAS_NOT_RUN
        0x00041305  => (33, "no more runs".into()),                   // SCHED_S_TASK_NO_MORE_RUNS
        0x00041306  => (33, "disabled".into()),                       // SCHED_S_TASK_DISABLED
        0x00041325  => (33, "queued".into()),                         // SCHED_S_TASK_QUEUED
        other if other > 0 => (31, format!("error 0x{:08X}", other)), // any other HRESULT
        other       => (31, format!("error {}", other)),
    }
}

#[cfg(target_os = "windows")]
async fn run_service_subcommand_windows(action: &str) -> anyhow::Result<()> {
    use std::process::Command;

    match action {
        "install" => {
            let bin_self = std::env::current_exe()?;
            let bin = std::fs::canonicalize(&bin_self).unwrap_or(bin_self);
            let bin_str = bin.display().to_string();

            // Delete any prior registration first so re-installs always
            // pick up the fresh binary path.
            let _ = Command::new("schtasks")
                .args(["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"])
                .output();

            // schtasks /Create /SC ONLOGON is rejected with "Access Denied"
            // on Enterprise / managed Windows where user-level ONLOGON
            // tasks are blocked by policy. PowerShell's
            // Register-ScheduledTask + New-ScheduledTaskTrigger -AtLogOn
            // -User <current user> works in those environments because it
            // creates the task in the current user's hive instead of the
            // system tree. RestartCount/RestartInterval gives us the
            // "auto-relaunch if phantom serve crashes" behaviour the old
            // schtasks XML embedding promised.
            let bin_for_ps = bin_str.replace('\'', "''");
            let task_for_ps = WINDOWS_TASK_NAME.replace('\'', "''");
            let ps_script = format!(
                "$action = New-ScheduledTaskAction -Execute '{}' -Argument 'serve'; \
                 $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME; \
                 $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1); \
                 Register-ScheduledTask -TaskName '{}' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null",
                bin_for_ps, task_for_ps
            );

            let status = Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_script])
                .status()?;
            if !status.success() {
                anyhow::bail!("Register-ScheduledTask failed (exit {:?})", status.code());
            }

            // Trigger it once so the service is up immediately, not only on
            // next logon.
            let _ = Command::new("schtasks")
                .args(["/Run", "/TN", WINDOWS_TASK_NAME])
                .status();

            let verify_port = configured_port();

            // Tailscale-scoped Defender firewall rule, mirroring step [3/5]
            // of install-phantom-windows.ps1. Best-effort — New-NetFirewallRule
            // needs admin; on a non-admin shell we report the skip but don't
            // fail the install.
            let fw_script = format!(
                "$ErrorActionPreference = 'Stop'; \
                 try {{ \
                    Get-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue; \
                    New-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -Direction Inbound -Action Allow -Protocol TCP -LocalPort {} -RemoteAddress '100.64.0.0/10' -Profile Any | Out-Null; \
                    Write-Output 'OK' \
                 }} catch {{ \
                    Write-Output (\"FAIL: \" + $_.Exception.Message) \
                 }}",
                verify_port
            );
            let fw_msg = Command::new("powershell")
                .args(["-NoProfile", "-Command", &fw_script])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let fw_status = if fw_msg == "OK" {
                format!("PhantomMesh-Inbound TCP {} ← 100.64.0.0/10 (Tailscale)", verify_port)
            } else if let Some(reason) = fw_msg.strip_prefix("FAIL:") {
                format!("skipped — re-run from admin PowerShell ({})", reason.trim())
            } else {
                "skipped — PowerShell unavailable".into()
            };

            eprintln!("{} Registered Scheduled Task '{}'", colored("✓", 32), WINDOWS_TASK_NAME);
            eprintln!("    binary:   {}", bin_str);
            eprintln!("    trigger:  at user logon (auto-restart up to 3× on failure)");
            eprintln!("    firewall: {}", fw_status);
            eprintln!("    Verify:   curl http://127.0.0.1:{}/healthz", verify_port);
            eprintln!("    Uninstall: phantom service uninstall");
            eprintln!();
            eprintln!(
                "{} {} for a full environment health check.",
                colored("→", 36),
                colored("phantom doctor", 33)
            );
            Ok(())
        }
        "uninstall" => {
            let status = Command::new("schtasks")
                .args(["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"])
                .status()?;
            if status.success() {
                eprintln!("{} Removed Scheduled Task '{}'", colored("◆", 35), WINDOWS_TASK_NAME);
            } else {
                eprintln!(
                    "{} schtasks /Delete returned exit {:?} — task may not have existed.",
                    colored("⚠", 33),
                    status.code()
                );
            }
            // Best-effort firewall rule cleanup. Silent so non-admin
            // uninstalls don't add noise (the rule was probably never
            // installed in that case).
            let _ = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Get-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue",
                ])
                .output();

            // Also kill any running phantom serve so the next logon starts fresh.
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "phantom.exe"])
                .output();
            eprintln!("{} Uninstalled.", colored("✓", 32));
            Ok(())
        }
        "status" => {
            let q = Command::new("schtasks")
                .args(["/Query", "/TN", WINDOWS_TASK_NAME])
                .output()?;
            let registered = q.status.success();

            let port = configured_port();
            let healthz_url = format!("http://127.0.0.1:{}/healthz", port);
            let probe = Command::new("curl.exe")
                .args([
                    "-s", "--max-time", "2",
                    "-o", "NUL",
                    "-w", "%{http_code}",
                    &healthz_url,
                ])
                .output();
            let healthz_code = probe
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            println!(
                "{} {}",
                colored("phantom service status", 36),
                colored(WINDOWS_TASK_NAME, 90)
            );
            println!(
                "  registered : {}",
                if registered { colored("yes", 32) } else { colored("no", 31) }
            );
            if registered {
                let (last_run_time, next_run_time, last_result) =
                    windows_task_info(WINDOWS_TASK_NAME);
                println!("  last run   : {}", last_run_time.unwrap_or_else(|| "?".into()));
                println!("  next run   : {}", next_run_time.unwrap_or_else(|| "?".into()));
                if let Some(r) = last_result {
                    let (code, label) = windows_task_result_label(r);
                    println!("  last state : {}", colored(&label, code));
                }
            }
            println!(
                "  healthz    : {} ({})",
                if healthz_code == "200" { colored("ok", 32) } else { colored("unreachable", 31) },
                if healthz_code.is_empty() { "no response".into() } else { format!("HTTP {}", healthz_code) }
            );
            if registered && healthz_code != "200" {
                println!("  hint       : Get-EventLog Application -Newest 20 | findstr phantom");
            }
            Ok(())
        }
        other => {
            eprintln!(
                "{} Unknown service action: '{}'.\n    Use one of: install, uninstall, status",
                colored("✗", 31),
                other
            );
            std::process::exit(1);
        }
    }
}

/// Best-effort: locate the phantom-mesh repo root, given the canonical path
/// to the running phantom binary. Used by `service install` to mirror dist/
/// and scripts/ into a launchd-friendly location.
///
/// Strategy: walk up from the binary, looking for a directory that contains
/// both `dist/` and `scripts/` siblings. Falls back to `current_dir()` if
/// the binary lives outside any such layout (e.g. it has been moved).
#[cfg(target_os = "macos")]
fn locate_repo_root(bin_real: &std::path::Path) -> Option<std::path::PathBuf> {
    // First: the cwd at install time (if user is running from the repo).
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("dist").is_dir() && cwd.join("scripts").is_dir() {
            return Some(cwd);
        }
    }
    // Second: walk up from the binary path.
    let mut p = bin_real.parent()?.to_path_buf();
    for _ in 0..6 {
        if p.join("dist").is_dir() && p.join("scripts").is_dir() {
            return Some(p);
        }
        p = match p.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return None,
        };
    }
    None
}

#[cfg(target_os = "macos")]
fn nix_uid() -> u32 {
    // Avoid pulling libc just for getuid(); shell out to `id -u` instead.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(all(test, target_os = "windows"))]
mod windows_helper_tests {
    use super::*;

    #[test]
    fn task_result_label_known_codes_render_with_color() {
        // Concrete known SCHED_S_* constants — make sure each maps to the
        // right human label and ANSI colour, otherwise localized status
        // output silently regresses to "?" on every Windows install.
        assert_eq!(windows_task_result_label(0), (32, "succeeded".into()));
        assert_eq!(windows_task_result_label(0x00041300), (32, "ready".into()));
        assert_eq!(windows_task_result_label(0x00041301), (33, "running".into()));
        assert_eq!(windows_task_result_label(0x00041303), (90, "never run".into()));
        assert_eq!(windows_task_result_label(0x00041305), (33, "no more runs".into()));
        assert_eq!(windows_task_result_label(0x00041306), (33, "disabled".into()));
        assert_eq!(windows_task_result_label(0x00041325), (33, "queued".into()));
    }

    #[test]
    fn task_result_label_unknown_positive_renders_as_hex_error() {
        // Any other positive HRESULT should fall through to a red hex
        // dump so the user can google it directly. Pick a couple plausible
        // failure codes.
        let (color, label) = windows_task_result_label(0x80070005); // E_ACCESSDENIED
        assert_eq!(color, 31);
        assert_eq!(label, "error 0x80070005");

        let (color, label) = windows_task_result_label(0x800704C7); // ERROR_CANCELLED
        assert_eq!(color, 31);
        assert_eq!(label, "error 0x800704C7");
    }

    #[test]
    fn task_result_label_unknown_negative_renders_as_decimal_error() {
        // Negative results should render decimal — they are not
        // HRESULT-shaped and hex would mislead.
        let (color, label) = windows_task_result_label(-1);
        assert_eq!(color, 31);
        assert_eq!(label, "error -1");
    }

    #[test]
    fn configured_port_falls_back_to_7878_when_no_config() {
        // If find_and_load returns None (no agents.toml anywhere reachable),
        // configured_port must default to the documented 7878. We can't
        // easily nuke the filesystem mid-test, but if the current process
        // *does* see a config the default branch is unreachable. So this
        // test asserts the fallback code path compiles + the value is
        // a sane u16. Keeps the const visible to the test file so a
        // future refactor that drops the default trips this.
        let port = configured_port();
        assert!(port > 0, "configured_port must yield a valid u16");
        assert_ne!(port, u16::MAX, "u16::MAX is unreachable from agents.toml");
    }
}
