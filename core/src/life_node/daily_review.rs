//! Daily review aggregator — first slice of E003 Coach Node.
//!
//! Reads (EventMeta, AnalysisResult) pairs for a given date and produces
//! a Markdown brief. Pure formatting; no LLM call. The LLM-driven
//! "tomorrow-action planner" pass lives in a separate function (later slice).
//!
//! Spec: `docs/superpowers/specs/_current/E003-coach-node-daily-review.md`

use crate::event_storage_wire::EventMeta;
use crate::life_node::goals::Goal;
use crate::life_node::key_derivation::EventKey;
use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderError, ResponseFormat,
};
use crate::life_node::storage::EventStore;
use std::collections::BTreeMap;
use std::path::Path;

// N3 desktop-nudge: the per-tag cooldown ledger lives in its own file
// (`nudge_ledger.rs`) but is registered here via a `#[path]` attribute so the
// new module lands without touching `life_node/mod.rs` (kept out of this
// slice's scope). The file is the canonical `core/src/life_node/nudge_ledger.rs`.
#[path = "nudge_ledger.rs"]
pub mod nudge_ledger;

/// Errors the tomorrow-action LLM pass can surface.
#[derive(thiserror::Error, Debug)]
pub enum TomorrowActionError {
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
    /// The LLM output tripped the shame-free lint — output rejected.
    /// Per BIG-GOAL operational principle, we'd rather ship NO coach
    /// suggestion than a shaming one.
    #[error("LLM output failed shame-free lint: {0}")]
    ShameLeakage(String),
    #[error("LLM returned empty action")]
    Empty,
}

/// Second pass: given today's brief, ask the LLM for ONE smallest action
/// for tomorrow. Output is lint-gated — a shame-pattern in the response
/// becomes [`TomorrowActionError::ShameLeakage`] rather than reaching
/// the user.
///
/// Uses the templates locked in `coach_prompts::templates` so prompt
/// drift is caught by the template lint test at build time.
pub async fn propose_tomorrow_action(
    brief: &str,
    provider: &dyn MultimodalProvider,
) -> Result<String, TomorrowActionError> {
    let prompt = crate::life_node::coach_prompts::templates::TOMORROW_ACTION_PROMPT
        .replace("{BRIEF}", brief);
    let input = AnalysisInput {
        modalities: vec![Modality::Text(prompt.clone())],
        system_prompt: Some(
            crate::life_node::coach_prompts::templates::COACH_SYSTEM_PROMPT.to_string(),
        ),
        user_prompt: prompt,
        max_output_tokens: Some(128),
        response_format: ResponseFormat::PlainText,
        response_schema: None,
    };
    let result = provider.analyze(input).await?;
    let action = result.summary.trim().to_string();
    if action.is_empty() {
        return Err(TomorrowActionError::Empty);
    }
    if let Err(e) = crate::life_node::coach_prompts::lint::check(&action) {
        return Err(TomorrowActionError::ShameLeakage(e));
    }
    Ok(action)
}

/// Propose the tomorrow-action across a provider FALLBACK CHAIN (SPEC-23: a
/// frontier-class chain, not a single hardcoded provider). Tries each provider
/// in order and returns the first clean action.
///
/// Fallback policy — critical: a `Provider` error (rate-limit / network /
/// auth / unavailable) or an `Empty` output falls THROUGH to the next provider.
/// A `ShameLeakage` is a HARD STOP — it is NOT retried on another model,
/// because retrying a shaming output until one slips past the lint would game
/// the fail-closed safety guarantee (BIG-GOAL operational principle #1). Returns
/// the terminal error if every provider failed (or `Empty` if the chain is
/// empty).
pub async fn propose_tomorrow_action_chain(
    brief: &str,
    providers: &[Box<dyn MultimodalProvider>],
) -> Result<String, TomorrowActionError> {
    let mut last_err: Option<TomorrowActionError> = None;
    for provider in providers {
        match propose_tomorrow_action(brief, provider.as_ref()).await {
            Ok(action) => return Ok(action),
            // Shaming output never ships and is never re-rolled on another model.
            Err(TomorrowActionError::ShameLeakage(e)) => {
                return Err(TomorrowActionError::ShameLeakage(e));
            }
            // Provider failure / empty → try the next provider in the chain.
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or(TomorrowActionError::Empty))
}

/// Scan an events directory (typically `~/.spectyn-mesh/events/`) and
/// return all (meta, analysis) pairs whose `meta.timestamp` ISO-8601
/// prefix matches `date_iso` (e.g. `"2026-05-22"`).
///
/// When `key` is `Some`, encrypted events are decrypted transparently.
/// Events without a sibling `analysis.json` are skipped silently.
/// Sorted by timestamp ascending.
pub fn load_events_for_date(
    events_dir: &Path,
    date_iso: &str,
    key: Option<EventKey>,
) -> std::io::Result<Vec<(EventMeta, AnalysisResult)>> {
    if !events_dir.exists() {
        return Ok(Vec::new());
    }
    let store = match key {
        Some(k) => EventStore::with_key(events_dir, k),
        None => EventStore::new(events_dir),
    };
    let mut pairs = Vec::new();
    let mut total = 0usize;
    let mut meta_failed = 0usize;
    let mut analysis_failed = 0usize;
    let mut wrong_date = 0usize;
    for entry in std::fs::read_dir(events_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        total += 1;
        let event_id = entry.file_name().to_string_lossy().to_string();
        let meta = match store.read_meta(&event_id) {
            Ok(m) => m,
            Err(_) => {
                meta_failed += 1;
                continue;
            }
        };
        // SPEC-16 T-STOR-01: timestamps are UTC; convert to the user's local
        // date before comparing (a raw prefix match would drop events captured
        // near midnight once storage switched off local time).
        if !crate::event_storage_wire::ts_on_local_date(&meta.timestamp, date_iso) {
            wrong_date += 1;
            continue;
        }
        let analysis = match store.read_analysis(&event_id) {
            Ok(a) => a,
            Err(_) => {
                analysis_failed += 1;
                continue;
            }
        };
        pairs.push((meta, analysis));
    }
    // Ascending by absolute instant — string compare isn't chronological across
    // mixed offsets (legacy local +08:00 vs new UTC +00:00). T-STOR-01.
    pairs.sort_by_key(|p| crate::event_storage_wire::ts_epoch_ms(&p.0.timestamp));
    // Diagnostic for silent-skip failure modes (e.g. identity.key rotated → meta
    // decrypt fails for all events → review shows 0). Use `tracing` (not raw
    // `eprintln!`) so it lands in the log — a bare stderr write here corrupted
    // the `/review` TUI pane, which shares this loader.
    if pairs.is_empty() && total > 0 {
        tracing::warn!(
            target: "life_node",
            date = %date_iso,
            scanned = total,
            meta_decrypt_failed = meta_failed,
            wrong_date,
            analysis_missing = analysis_failed,
            "load_events_for_date scanned events but 0 matched"
        );
    }
    Ok(pairs)
}

/// Aggregate event+analysis pairs into a deterministic Markdown brief.
///
/// Structure:
///   `# Daily review — <date>`
///   `**Events captured:** N`
///   `## <goal_tag> (count)`  — one per distinct tag, alphabetical
///     `- **<kind>** (<timestamp>): <analysis.summary>`
///
/// Events with multiple `goal_tags` appear under each section. Events
/// with no tags fall under `untagged`. Empty input returns a stub brief.
/// Some providers (notably food's strict-JSON prompt) return a JSON object as
/// the analysis summary, so `analysis.summary` can be a literal
/// `{"summary": "...", ...}` blob. Extract the inner `summary` text so the
/// review bullet stays readable instead of dumping raw JSON. Prose summaries
/// (text / habit / focus) pass through unchanged. `pub(crate)` so the export
/// path (data_cli::run_export) can reuse it for readable exports.
pub fn clean_summary(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with('{') {
        // (a) Complete JSON object → take the `summary` field.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
            if let Some(s) = v.get("summary").and_then(|s| s.as_str()) {
                return s.trim().to_string();
            }
        }
        // (b) Truncated JSON (some older events were stored with the LLM
        //     response cut mid-object) — recover the `summary` value as long as
        //     that string itself is intact, via a targeted regex.
        if let Ok(re) = regex::Regex::new(r#""summary"\s*:\s*"((?:[^"\\]|\\.)*)""#) {
            if let Some(m) = re.captures(t).and_then(|c| c.get(1)) {
                let s = m.as_str().replace("\\\"", "\"").replace("\\n", " ");
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    t.to_string()
}

pub fn aggregate(date_iso: &str, events: &[(EventMeta, AnalysisResult)]) -> String {
    let mut buf = String::new();
    buf.push_str(&format!("# Daily review — {}\n\n", date_iso));
    buf.push_str(&format!("**Events captured:** {}\n\n", events.len()));

    if events.is_empty() {
        buf.push_str("(no events for this date)\n");
        return buf;
    }

    let mut by_tag: BTreeMap<&str, Vec<&(EventMeta, AnalysisResult)>> = BTreeMap::new();
    for evt in events {
        let (meta, _) = evt;
        if meta.tags.is_empty() {
            by_tag.entry("untagged").or_default().push(evt);
        } else {
            for tag in &meta.tags {
                by_tag.entry(tag.as_str()).or_default().push(evt);
            }
        }
    }

    for (tag, evts) in &by_tag {
        buf.push_str(&format!("## {} ({})\n", tag, evts.len()));
        for (meta, analysis) in evts {
            let cleaned = clean_summary(&analysis.summary);
            let summary = if cleaned.is_empty() {
                "(no summary)".to_string()
            } else {
                cleaned
            };
            // `EventKind` is `serde(rename_all = "snake_case")` but has no
            // `Display` impl — render via JSON serialize → strip quotes, the
            // same trick `coach_wire::format_section` uses so the on-screen
            // bullet stays `food` / `focus` / `habit` / `text`.
            let kind_json = serde_json::to_string(&meta.kind)
                .unwrap_or_else(|_| "\"unknown\"".to_string());
            let kind_str = kind_json.trim_matches('"');
            buf.push_str(&format!(
                "- **{}** ({}): {}\n",
                kind_str, meta.timestamp, summary
            ));
        }
        buf.push('\n');
    }

    buf
}

// ─── Goal deviation (N2: goal-schema target vs today's actual) ────────────────
//
// The goal-schema half (life_node::goals) stores a *quantified* target
// ("focus 180 minutes daily"). This half computes the matching ACTUAL from the
// day's real events and reports the signed deviation so the daily review says
// "you wanted 180 min of focus, you logged 90 → -90" instead of vague vibes.
//
// Where the actual number comes from (the REAL event schema): a focus event
// has no dedicated numeric column — the duration is encoded in its
// `AnalysisResult.summary` text by the real capture path
// (`focus_session::write_focus_event`, which formats `"{mins}m {secs}s focus
// session on …"`). So the actual is parsed back out of that real summary and
// summed across every event whose `meta.tags` carries the goal's `tag`. This is
// the same summary text the `aggregate()` bullets already render — we read the
// durable on-disk signal, we do not invent one.

/// Pull a numeric quantity out of a real event summary for deviation summing.
///
/// Primary form (focus capture path): a leading `"<m>m <s>s …"` duration, e.g.
/// `"45m 0s focus session on \"spec\", 0 interruption(s)."` → `45.0` minutes
/// (seconds fold in as a fraction). This is the exact shape
/// `focus_session::write_focus_event` writes, so the deviation sums REAL
/// captured minutes.
///
/// Fallback form (non-time units like `"times"`/`"km"`): the first bare number
/// in the summary, so a habit/count goal can still aggregate. Returns `None`
/// when the summary carries no parseable quantity (that event contributes 0).
pub fn extract_quantity(summary: &str) -> Option<f64> {
    let s = summary.trim();
    // Primary: "<int>m <int>s" duration → total minutes (focus capture format).
    // Anchored at the start so "30 interruption(s)" style trailing numbers can't
    // be mistaken for the duration.
    if let Ok(re) = regex::Regex::new(r"^\s*(\d+)\s*m\s+(\d+)\s*s\b") {
        if let Some(c) = re.captures(s) {
            let mins: f64 = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
            let secs: f64 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
            return Some(mins + secs / 60.0);
        }
    }
    // Fallback: first bare number (count / generic unit goals).
    if let Ok(re) = regex::Regex::new(r"-?\d+(?:\.\d+)?") {
        if let Some(m) = re.find(s) {
            if let Ok(n) = m.as_str().parse::<f64>() {
                return Some(n);
            }
        }
    }
    None
}

/// Computed deviation for a single goal on a given day.
pub struct GoalDeviation {
    pub goal: Goal,
    /// Sum of the parsed quantity across today's events bearing `goal.tag`.
    pub actual: f64,
    /// `actual - target`. Positive = ahead of target, negative = behind.
    pub deviation: f64,
}

/// For each goal, sum the quantity from today's events whose `meta.tags`
/// contains the goal's `tag`, and return the signed deviation (actual − target).
///
/// An event with no parseable quantity contributes 0 to its tag's sum (it still
/// counts as "happened" via `aggregate`, but adds nothing measurable here).
/// Order mirrors the input `goals` order (which `list_goals_in` already sorts by
/// tag) so the rendered section is deterministic.
pub fn goal_deviations(
    goals: &[Goal],
    events: &[(EventMeta, AnalysisResult)],
) -> Vec<GoalDeviation> {
    goals
        .iter()
        .map(|g| {
            let actual: f64 = events
                .iter()
                .filter(|(meta, _)| meta.tags.iter().any(|t| t == &g.tag))
                .filter_map(|(_, analysis)| extract_quantity(&analysis.summary))
                .sum();
            GoalDeviation {
                goal: g.clone(),
                actual,
                deviation: actual - g.target,
            }
        })
        .collect()
}

/// Render the per-goal deviation lines as a Markdown section. Each goal emits
/// one line carrying the actual, the target, and the SIGNED deviation:
///
///   `## Goal deviations`
///   `- **focus**: 90 / 180 minutes (daily) — deviation -90`
///
/// The sign is explicit (`+90` ahead of target, `-90` behind). Empty `goals`
/// yields an empty string so a user with no goals defined sees no section.
pub fn deviation_section(goals: &[Goal], events: &[(EventMeta, AnalysisResult)]) -> String {
    if goals.is_empty() {
        return String::new();
    }
    let mut buf = String::new();
    buf.push_str("## Goal deviations\n");
    for d in goal_deviations(goals, events) {
        // `{:+}` forces the leading sign so "-90" / "+30" reads as ahead/behind
        // at a glance. `fmt_num` trims the trailing `.0` so whole numbers stay
        // clean (90, not 90.0) while a fractional minute (from the seconds fold)
        // still shows.
        buf.push_str(&format!(
            "- **{}**: {} / {} {} ({}) — deviation {}\n",
            d.goal.tag,
            fmt_num(d.actual),
            fmt_num(d.goal.target),
            d.goal.unit,
            d.goal.window,
            fmt_signed(d.deviation),
        ));
    }
    buf.push('\n');
    buf
}

/// Format a non-negative-leaning quantity: drop a trailing `.0` for whole
/// numbers, keep up to one decimal otherwise.
fn fmt_num(n: f64) -> String {
    if (n - n.round()).abs() < 1e-9 {
        format!("{}", n.round() as i64)
    } else {
        format!("{:.1}", n)
    }
}

/// Like [`fmt_num`] but always prefixes an explicit sign (`+`/`-`).
fn fmt_signed(n: f64) -> String {
    if (n - n.round()).abs() < 1e-9 {
        format!("{:+}", n.round() as i64)
    } else {
        format!("{:+.1}", n)
    }
}

/// Path-injectable deviation core: load the goals ledger from `home`
/// (`<home>/.spectyn-mesh/goals.jsonl`) and render the deviation section against
/// `events`. Fully hermetic — production passes the real home, the acceptance
/// test passes a tempdir holding a real `goals.jsonl` + real event dirs. A
/// missing ledger yields an empty section (no goals defined → nothing to report).
pub fn deviation_section_for_home(
    home: &Path,
    events: &[(EventMeta, AnalysisResult)],
) -> std::io::Result<String> {
    let goals = crate::life_node::goals::list_goals_for_home(home)?;
    Ok(deviation_section(&goals, events))
}

/// Deterministic, byte-stable daily review over IN-MEMORY event pairs — the
/// golden-fixture surface (P0-6). Composes the existing pure [`aggregate`] with
/// the pure [`deviation_section`], so identical `(date_iso, pairs, goals)` ⇒
/// identical bytes on every platform/timezone (SPEC-23 §G5 parity). No LLM, no
/// disk, no clock: callers that want the live "Tomorrow's one action" use
/// [`run_coach_review`].
///
/// SSOT for the asserted bytes: `core/tests/fixtures/daily_review/events.json` +
/// `core/tests/fixtures/daily_review/2026-05-22.golden.md`, pinned by the
/// `golden_review_is_byte_stable` integration test. Regenerate the golden
/// artifact intentionally with `SPECTYN_UPDATE_GOLDEN=1` and review the diff.
pub fn golden_review(date_iso: &str, pairs: &[(EventMeta, AnalysisResult)], goals: &[Goal]) -> String {
    let mut md = aggregate(date_iso, pairs);
    let section = deviation_section(goals, pairs);
    if !section.is_empty() {
        md.push_str(&format!("\n{}", section));
    }
    md
}

// ─── N3 desktop-nudge (closes sense→learn→nudge, capability ③) ────────────────
//
// When `spectyn coach review --notify` runs, ANY goal that is behind target
// (negative deviation) should reach the user as a REAL desktop banner — not just
// a line in a saved (often age-encrypted) markdown file no one opens. This is the
// "nudge" leg of the loop the N2 deviation calc already powers.
//
// To avoid re-nagging (the review may re-run, and the SPEC-23 scheduler fires
// daily), each behind-goal's banner is gated on a PER-TAG cooldown via
// `nudge_ledger`: skip + don't record if a nudge for that tag fired within the
// 30-min window; record after a successful fire. The cooldown core takes
// `now_secs`, so this function takes it too and the caller injects the real
// `SystemTime::now()` at the call site (keeping the ledger core hermetic).

/// Outcome of the notify pass: which goal tags actually fired a banner this run
/// (after the cooldown gate). Empty when nothing was behind, or everything was
/// still inside its cooldown window.
#[derive(Debug, Default, PartialEq)]
pub struct NudgeOutcome {
    /// Goal tags that fired a real desktop banner on this run.
    pub fired: Vec<String>,
    /// Behind-goal tags suppressed because they were still inside the cooldown.
    pub suppressed: Vec<String>,
}

/// Build the desktop-banner body for a single behind-goal. Kept shame-free
/// (capability ③ / BIG-GOAL principle): states the gap plainly, no blame.
fn nudge_body(d: &GoalDeviation) -> (String, String) {
    let title = format!("{} 落後目標 / behind on {}", d.goal.tag, d.goal.tag);
    // `deviation` is negative here; show the gap as a positive "short by N".
    let short_by = fmt_num((-d.deviation).max(0.0));
    let body = format!(
        "{}：目前 {} / {} {}（{}），還差 {} {}。",
        d.goal.tag,
        fmt_num(d.actual),
        fmt_num(d.goal.target),
        d.goal.unit,
        d.goal.window,
        short_by,
        d.goal.unit,
    );
    (title, body)
}

/// N3 entry point. For each goal in `<home>/.spectyn-mesh/goals.jsonl` that is
/// BEHIND target on `events` (negative deviation), fire a REAL macOS/Linux/Windows
/// desktop banner through the existing [`crate::notifications::channels::OsChannel`]
/// (which wraps `notify-rust`) — UNLESS a nudge for that goal's tag is still inside
/// the per-tag cooldown, in which case it is skipped and NOT recorded. A fired
/// nudge is recorded to the cooldown ledger so the next run honours the window.
///
/// `now_secs` is the unix-seconds "now" used for the cooldown comparison and the
/// recorded timestamp; production passes the real `SystemTime::now()` at the call
/// site (this keeps the ledger core itself clock-free and hermetically testable).
///
/// Best-effort: a goal whose banner `send` errors (e.g. headless host with no
/// notification service) is treated as not-fired and is NOT recorded, so a real
/// banner gets another chance on the next run rather than being silently
/// cooled-down after a failed delivery. Returns the [`NudgeOutcome`] so the CLI
/// can print what fired.
///
/// Not compiled on Android/iOS (no `OsChannel` there — mirrors the cfg-gate on
/// the channel module itself).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn fire_behind_goal_nudges(
    home: &Path,
    events: &[(EventMeta, AnalysisResult)],
    now_secs: u64,
) -> std::io::Result<NudgeOutcome> {
    use crate::notifications::channels::{NotificationChannel, OsChannel};
    use pm_types::{Notification, NotificationPriority};

    let spectyn_dir = crate::cli_config::spectyn_dir_under(home);
    let goals = crate::life_node::goals::list_goals_for_home(home)?;
    let deviations = goal_deviations(&goals, events);

    let channel = OsChannel;
    let mut outcome = NudgeOutcome::default();

    for d in deviations {
        // Only behind-target goals nudge. `< 0` so an exactly-met goal (0
        // deviation) does NOT pop a banner.
        if d.deviation >= 0.0 {
            continue;
        }
        let tag = d.goal.tag.clone();
        // Per-tag cooldown gate: skip + don't record if still inside the window.
        if !nudge_ledger::should_nudge_in(&spectyn_dir, &tag, now_secs) {
            outcome.suppressed.push(tag);
            continue;
        }
        let (title, body) = nudge_body(&d);
        let notification = Notification {
            id: uuid::Uuid::new_v4(),
            dedup_key: format!("nudge:goal-behind:{}", tag),
            task_id: None,
            workspace_id: String::new(),
            priority: NotificationPriority::P0,
            title,
            body,
            actions: vec![],
            timestamp: (now_secs as i64).saturating_mul(1000),
        };
        // Fire the REAL banner. Only record (start the cooldown) on a successful
        // send so a failed delivery doesn't silently cool the goal down.
        match channel.send(&notification).await {
            Ok(()) => {
                nudge_ledger::record_nudge_in(&spectyn_dir, &tag, now_secs)?;
                outcome.fired.push(tag);
            }
            Err(e) => {
                tracing::warn!(
                    target: "life_node",
                    tag = %tag,
                    error = %e,
                    "desktop nudge send failed — not recording cooldown so it can retry"
                );
            }
        }
    }
    Ok(outcome)
}

/// Dependencies for the proactive partner-reflection section of a coach review.
///
/// `run_coach_review` reads the encrypted Life Node events store; the partner's
/// PROACTIVE half lives in a DISJOINT place — `partner-signals.jsonl` (coarse
/// location/behaviour + interactions) compared against the user's real Todoist
/// goals. Passing this in folds that second loop into the SAME daily trigger
/// (`spectyn coach review`, which the SPEC-23 scheduler already fires), so the
/// review is the single source of truth for the daily reflection instead of the
/// two loops never meeting (`partner::daily_reflection` was dead code).
///
/// `None` (the test path / any caller without an LLM runtime) skips the section,
/// keeping `run_coach_review` runnable with no provider — the deterministic
/// events aggregate + tomorrow-action still produce a review.
pub struct PartnerReflectionDeps<'a> {
    /// Agent runtime used to generate the warm ally-tone reflection.
    pub runtime: &'a crate::agent::AgentRuntime,
    /// Agent name to drive (e.g. `"master"`).
    pub agent: &'a str,
    /// End of the 24h signal window (unix seconds). Passed through to
    /// `gather_reflection_context` so the window is deterministic, not wall-clock.
    pub now_unix: u64,
    /// Tools config carrying the Todoist token for the goal model (or `None` to
    /// fall back to `TODOIST_API_TOKEN` / the "goals not connected" line).
    pub tools: Option<&'a crate::config::ToolsConfig>,
}

/// Outcome of a full coach-review run (aggregate + tomorrow-action + save).
pub struct CoachReviewResult {
    /// The full review markdown, including the "Tomorrow's one action" section.
    pub markdown: String,
    /// Path the review was written to (only when `save` was requested).
    pub saved_to: Option<std::path::PathBuf>,
    /// Whether the saved file is age-encrypted (true when identity.key exists).
    pub saved_encrypted: bool,
    /// Number of Life Node events the review covered.
    pub event_count: usize,
    /// The lint-clean "Tomorrow's one action" the LLM chain produced. Empty
    /// when `status == Degraded` (the chain was empty — no keys / Ollama off —
    /// or every provider failed): the live path NEVER fabricates an action, it
    /// degrades to stats-only (mirrors `DailyReviewOutcome.next_action`).
    pub next_action: String,
    /// SPEC-23 §8 terminal status, unified with `coach_wire::run_daily_review`
    /// (the DRIFT this field closes): `Completed` = a clean tomorrow-action was
    /// produced; `Degraded` = no provider answered (chain empty or all-fail) or
    /// the shame-free lint rejected the action — the stats-only brief is still
    /// produced and a retryable `coach_review` row is still persisted.
    pub status: crate::coach_wire::ReviewStatus,
}

/// Run a full coach review for `date`, shared by the CLI (`spectyn coach
/// review`) and the app's `daily_review_generate` command so the aggregate +
/// LLM "Tomorrow's one action" + save/encrypt logic stays in one place.
///
/// Aggregates the day's events, runs the shame-free lint (warns, never blocks),
/// then appends the Gemini-driven "Tomorrow's one action" — degrading
/// gracefully to a `(skipped: …)` footer when there's no `GEMINI_API_KEY` or
/// the call fails, so the review is always produced. When `save` is set, writes
/// to `~/.spectyn-mesh/reviews/{date}.md`, age-encrypted with the identity key
/// when present (else plaintext, matching the events store).
///
/// Proactive partner reflection (the life-partner MVP §④): when `partner` deps
/// are supplied, the review ALSO folds in the partner's disjoint proactive loop
/// — the last 24h of `partner-signals.jsonl` (coarse location/behaviour +
/// interactions) compared against the user's real Todoist goals — as a warm
/// "Daily alignment" section via [`crate::partner::daily_reflection`]. This
/// makes `spectyn coach review` the SINGLE daily trigger for both halves instead
/// of `partner::daily_reflection` being dead code that nothing fires. Best-effort:
/// a failed reflection (no provider / network) degrades to a `(skipped: …)`
/// footer and never sinks the events review.
pub async fn run_coach_review(
    home: &Path,
    date: &str,
    save: bool,
    partner: Option<PartnerReflectionDeps<'_>>,
) -> anyhow::Result<CoachReviewResult> {
    let events_dir = home.join(".spectyn-mesh/events");
    let identity_path = home.join(".spectyn-mesh/identity.key");
    let event_key = crate::life_node::key_derivation::load_event_key(&identity_path).ok();
    let pairs = load_events_for_date(&events_dir, date, event_key)?;
    let event_count = pairs.len();
    let mut md = aggregate(date, &pairs);

    // N2 goal deviation: for each quantified goal in `<home>/.spectyn-mesh/
    // goals.jsonl`, append the actual-vs-target signed deviation computed from
    // today's real events (see `deviation_section`). Best-effort: an unreadable
    // ledger degrades to no section rather than sinking the review.
    match deviation_section_for_home(home, &pairs) {
        Ok(section) if !section.is_empty() => md.push_str(&format!("\n{}", section)),
        Ok(_) => {}
        Err(e) => tracing::warn!(target: "life_node", error = %e, "goal deviation section skipped"),
    }

    if let Err(e) = crate::life_node::coach_prompts::lint::check(&md) {
        eprintln!("warning: shame-free lint on review output: {}", e);
    }

    // Tomorrow-action second pass over a provider FALLBACK CHAIN (SPEC-23):
    // Gemini → Groq → local Ollama. The z13 verifier showed Groq's free tier
    // 429ing on this pass with no further fallback, leaving the proactive
    // reflection half-empty. Adding a local Ollama as the FINAL link means a
    // rate-limited hosted chain still produces an action — a local model costs
    // nothing and isn't subject to a free-tier quota. Ollama is enabled by
    // default (no key needed for a bare localhost) and is the last resort, so
    // it never overrides a clean hosted answer. If localhost is unreachable the
    // per-call error simply falls through and the chain ends in the graceful
    // `(skipped: …)` footer — identical to today's behaviour. Set OLLAMA_DISABLE=1
    // to opt out.
    let mut chain: Vec<Box<dyn MultimodalProvider>> = Vec::new();
    if let Ok(p) = crate::life_node::providers::gemini::GeminiMultimodalProvider::from_env() {
        chain.push(Box::new(p));
    }
    if let Ok(p) = crate::life_node::providers::groq::GroqTextProvider::from_env() {
        chain.push(Box::new(p));
    }
    if let Ok(p) = crate::life_node::providers::ollama::OllamaTextProvider::from_env_or_default() {
        chain.push(Box::new(p));
    }
    // SPEC-23 §8 state machine, now unified onto the live path (DRIFT fix):
    // `next_action`/`status` carry the same Completed/Degraded contract
    // `coach_wire::run_daily_review` returns. The chain is the live providers
    // (Gemini → Groq → local Ollama); an EMPTY chain (no keys + Ollama off) and
    // an all-providers-fail are BOTH Degraded — the review degrades to
    // stats-only and NEVER fabricates an action, mirroring the wire path.
    let mut next_action = String::new();
    let mut status = crate::coach_wire::ReviewStatus::Completed;
    if chain.is_empty() {
        md.push_str(
            "\n## Tomorrow's one action\n\n(skipped: no GEMINI_API_KEY / GROQ_API_KEY in env and Ollama disabled)\n",
        );
        status = crate::coach_wire::ReviewStatus::Degraded;
    } else {
        match propose_tomorrow_action_chain(&md, &chain).await {
            Ok(action) => {
                md.push_str(&format!("\n## Tomorrow's one action\n\n{}\n", action));
                next_action = action;
            }
            Err(e) => {
                md.push_str(&format!("\n## Tomorrow's one action\n\n(skipped: {})\n", e));
                // Provider-fail / empty / shame-leak all collapse to Degraded —
                // the same §11.1 catalog collapse the wire path makes.
                status = crate::coach_wire::ReviewStatus::Degraded;
            }
        }
    }

    // Proactive partner reflection — the life-partner MVP's "did today align
    // with what I said I wanted?" half. Reads partner-signals.jsonl (last 24h)
    // + the user's real Todoist goals and produces the warm, ally-tone
    // reflection. Best-effort: a failed reflection becomes a `(skipped: …)`
    // footer so the events review is always produced.
    if let Some(p) = partner {
        match crate::partner::daily_reflection(p.runtime, p.agent, p.now_unix, p.tools).await {
            Ok(reflection) => {
                md.push_str(&format!("\n## Daily alignment\n\n{}\n", reflection.trim()))
            }
            Err(e) => md.push_str(&format!("\n## Daily alignment\n\n(skipped: {})\n", e)),
        }
    }

    // Unified degrade/persist branch (DRIFT fix): a Degraded live review still
    // persists a retryable `coach_review` row to the SPEC-16 EventStore — the
    // SAME row `coach_wire::run_daily_review` writes on its degraded path — so a
    // no-keys / Ollama-off run is recoverable from history (UI shows the「⏳
    // 建議 still cooking」card + retry) instead of silently vanishing as a
    // stats-only footer. Best-effort: a persist failure (locked keystore / full
    // disk) is logged, not fatal — the stats-only review is still returned so a
    // dead LLM never blocks the daily review. The happy path's full review is
    // already saved via the `save`/reviews-dir branch below + the app's own
    // event row, so we only persist the degraded retryable marker here.
    if status == crate::coach_wire::ReviewStatus::Degraded {
        match crate::coach_wire::persist_review(date, &md) {
            Ok(_event_id) => {}
            Err(e) => {
                tracing::warn!(
                    target: "life_node",
                    date = %date,
                    error = %e,
                    "degraded coach review persist skipped (retryable row not written)"
                );
            }
        }
    }

    let mut saved_to = None;
    let mut saved_encrypted = false;
    if save {
        let reviews_dir = home.join(".spectyn-mesh/reviews");
        std::fs::create_dir_all(&reviews_dir)?;
        let p = reviews_dir.join(format!("{}.md", date));
        if identity_path.exists() {
            let key = crate::life_node::key_derivation::load_event_key(&identity_path)
                .map_err(|e| anyhow::anyhow!("load identity key: {}", e))?;
            let ciphertext = crate::life_node::crypto::encrypt(md.as_bytes(), &key)
                .map_err(|e| anyhow::anyhow!("encrypt review: {}", e))?;
            std::fs::write(&p, &ciphertext)?;
            saved_encrypted = true;
        } else {
            std::fs::write(&p, &md)?;
        }
        saved_to = Some(p);
    }

    Ok(CoachReviewResult {
        markdown: md,
        saved_to,
        saved_encrypted,
        event_count,
        next_action,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_storage_wire::EventKind;
    use serde_json::json;

    #[test]
    fn clean_summary_extracts_json_summary_field() {
        // food path: summary is a JSON object → show only the inner summary.
        let raw = r#"{"summary": "Grilled chicken salad is great for fat loss.", "confidence": 0.95, "goal_impact": "High"}"#;
        assert_eq!(
            clean_summary(raw),
            "Grilled chicken salad is great for fat loss."
        );
        // prose path: passes through (trimmed), no JSON.
        assert_eq!(clean_summary("  a 30-minute walk  "), "a 30-minute walk");
        // malformed JSON-ish without a summary field → returned as-is.
        assert_eq!(clean_summary("{not json"), "{not json");
        // truncated JSON (old events) but summary string intact → recovered.
        let trunc = r#"{"summary": "This salad is solid for fat loss.", "confidence": 0.9, "goal_imp"#;
        assert_eq!(clean_summary(trunc), "This salad is solid for fat loss.");
        // truncated mid-summary (string itself cut) → unrecoverable, as-is.
        let cut = r#"{"summary": "A Caesar salad, particularly"#;
        assert_eq!(clean_summary(cut), cut);
    }

    /// Map the legacy free-form kind strings the original test fixtures
    /// used to the SPEC-16 `EventKind` enum, matching the projection in
    /// `storage::project_to_wire`. Unknown / non-canonical kinds fall back
    /// to `EventKind::Text` (the catch-all variant).
    fn kind_from_legacy_str(k: &str) -> EventKind {
        match k {
            "food_log" | "food" => EventKind::Food,
            "focus_session" | "focus" => EventKind::Focus,
            "habit_log" | "habit" | "habit_check" => EventKind::Habit,
            _ => EventKind::Text,
        }
    }

    fn ev(
        kind: &str,
        ts: &str,
        goal_tags: Vec<&str>,
        summary: &str,
    ) -> (EventMeta, AnalysisResult) {
        let meta = EventMeta {
            event_id: format!("evt-{}-{}", kind, ts),
            kind: kind_from_legacy_str(kind),
            timestamp: ts.to_string(),
            tags: goal_tags.into_iter().map(String::from).collect(),
        };
        let analysis = AnalysisResult {
            summary: summary.to_string(),
            goal_impact: None,
            suggestion: None,
            confidence: Some(0.8),
            raw_response: json!({}),
            model_id: "test-model".to_string(),
            latency_ms: 100,
            cost_usd: None,
        };
        (meta, analysis)
    }

    #[test]
    fn aggregate_5_events_returns_stable_markdown() {
        let events = vec![
            ev(
                "food_log",
                "2026-05-22T08:30:00Z",
                vec!["fat_loss"],
                "Two slices of toast with butter — moderate carb load.",
            ),
            ev(
                "focus_session",
                "2026-05-22T10:00:00Z",
                vec!["focus"],
                "50 minutes deep work, no distractions.",
            ),
            ev(
                "food_log",
                "2026-05-22T12:30:00Z",
                vec!["fat_loss", "habit"],
                "Caesar salad and grilled chicken — within targets.",
            ),
            ev(
                "habit_check",
                "2026-05-22T18:00:00Z",
                vec!["habit"],
                "Evening walk 30 min, weather mild.",
            ),
            ev(
                "food_log",
                "2026-05-22T20:00:00Z",
                vec!["fat_loss"],
                "Small portion of pasta, late-ish dinner.",
            ),
        ];
        let md = aggregate("2026-05-22", &events);

        // Required structure
        assert!(
            md.starts_with("# Daily review — 2026-05-22"),
            "heading must lead"
        );
        assert!(
            md.contains("**Events captured:** 5"),
            "must report event count"
        );
        assert!(
            md.contains("## fat_loss"),
            "must have fat_loss goal-tag section"
        );
        assert!(md.contains("## focus"), "must have focus goal-tag section");
        assert!(md.contains("## habit"), "must have habit goal-tag section");

        // Stable length within token budget. ~5 events × 100 chars analysis +
        // headers ≈ 800 chars; ceiling at 4000 keeps Gemini Flash cost predictable.
        let len = md.len();
        assert!(len >= 300, "too short ({} chars) — missing structure?", len);
        assert!(
            len <= 4000,
            "too long ({} chars) — would blow token budget",
            len
        );

        // Determinism
        let md2 = aggregate("2026-05-22", &events);
        assert_eq!(md, md2, "aggregate must be deterministic");

        // Multi-tag fan-out — Caesar salad has [fat_loss, habit], expect 2 mentions
        assert_eq!(
            md.matches("Caesar salad").count(),
            2,
            "event with 2 goal_tags should appear in both sections"
        );
    }

    #[test]
    fn aggregate_empty_input_returns_stub() {
        let md = aggregate("2026-05-22", &[]);
        assert!(md.contains("# Daily review — 2026-05-22"));
        assert!(md.contains("**Events captured:** 0"));
        assert!(md.contains("(no events for this date)"));
    }

    #[test]
    fn aggregate_untagged_event_appears_in_untagged_section() {
        let events = vec![ev("misc", "2026-05-22T09:00:00Z", vec![], "just a thing")];
        let md = aggregate("2026-05-22", &events);
        assert!(md.contains("## untagged"));
    }

    // ── N2 goal deviation (target vs today's actual) ───────────────────────────

    #[test]
    fn extract_quantity_parses_real_focus_summary_minutes() {
        // The exact shape `focus_session::write_focus_event` writes.
        assert_eq!(
            extract_quantity("45m 0s focus session on \"spec\", 0 interruption(s)."),
            Some(45.0)
        );
        // Seconds fold in as a fraction of a minute.
        assert_eq!(
            extract_quantity("45m 30s focus session on \"x\", 2 interruption(s)."),
            Some(45.5)
        );
        // Trailing interruption count must NOT be mistaken for the duration.
        assert_eq!(extract_quantity("90m 0s focus session, 7 interruption(s)."), Some(90.0));
        // Fallback: a bare count for non-time goals.
        assert_eq!(extract_quantity("3 sets done"), Some(3.0));
        // Nothing parseable → contributes 0.
        assert_eq!(extract_quantity("a mindful afternoon"), None);
    }

    /// ACCEPTANCE TEST (real-path, no mock): seed a `focus` goal (target 180,
    /// unit minutes, window daily) into a real `goals.jsonl`, write TWO REAL
    /// focus events totalling 90 minutes (45m + 45m) through the real
    /// `EventStore` (real files on disk, real summary format from the capture
    /// path), then read them back via `load_events_for_date` and run the
    /// path-injectable deviation core. The brief must carry a `-90` deviation
    /// line (90 actual − 180 target = −90, behind). Fully hermetic in a tempdir.
    #[test]
    fn deviation_section_reports_minus_90_for_focus_90_of_180() {
        use crate::life_node::goals::{define_goal_in, Goal};
        use crate::life_node::multimodal::Modality;
        use crate::life_node::storage::EventStore;

        let home = tempfile::tempdir().unwrap();
        let spectyn = home.path().join(".spectyn-mesh");

        // 1. Seed the quantified goal into a REAL goals.jsonl ledger.
        define_goal_in(
            &spectyn,
            &Goal {
                tag: "focus".to_string(),
                target: 180.0,
                unit: "minutes".to_string(),
                window: "daily".to_string(),
            },
        )
        .expect("seed focus goal");

        // 2. Write TWO real focus events (45m each = 90m total) via EventStore,
        //    using the exact summary shape the focus capture path produces.
        let events_dir = spectyn.join("events");
        let store = EventStore::new(&events_dir);
        for i in 0..2 {
            let summary = format!(
                "45m 0s focus session on \"deep work {i}\", 0 interruption(s)."
            );
            let meta = store
                .write_event(
                    "focus",
                    &[Modality::Text(summary.clone())],
                    &["focus".to_string()],
                    "test-node",
                )
                .expect("write focus event");
            store
                .write_analysis(
                    &meta.event_id,
                    &AnalysisResult {
                        summary,
                        goal_impact: None,
                        suggestion: None,
                        confidence: None,
                        raw_response: json!({}),
                        model_id: "local-focus-timer".to_string(),
                        latency_ms: 0,
                        cost_usd: None,
                    },
                )
                .expect("write focus analysis");
        }

        // 3. Read the events back off disk for "today" (the real loader), then
        //    compute the deviation section against the real goals.jsonl.
        let today = crate::event_storage_wire::ts_local_date(
            &chrono::Utc::now().to_rfc3339(),
        );
        let pairs = load_events_for_date(&events_dir, &today, None)
            .expect("load today's focus events");
        assert_eq!(pairs.len(), 2, "two real focus events on disk for today");

        let section =
            deviation_section_for_home(home.path(), &pairs).expect("deviation section");

        // 90 actual − 180 target = −90, behind. The brief carries the -90 line.
        assert!(
            section.contains("-90"),
            "deviation section must contain a -90 line (90 of 180 minutes); got:\n{section}"
        );
        assert!(
            section.contains("**focus**"),
            "deviation line names the focus goal; got:\n{section}"
        );
        assert!(
            section.contains("90 / 180 minutes (daily)"),
            "deviation line shows actual/target/unit/window; got:\n{section}"
        );
    }

    /// A goal that is MET / exceeded shows a positive (`+`) deviation, and a
    /// goal with no matching events shows the full negative target. Confirms the
    /// sign convention (+ = ahead, − = behind) both ways, on real events.
    #[test]
    fn deviation_section_signs_ahead_and_behind() {
        use crate::life_node::goals::Goal;

        let goals = vec![
            Goal { tag: "focus".into(), target: 60.0, unit: "minutes".into(), window: "daily".into() },
            Goal { tag: "reading".into(), target: 30.0, unit: "minutes".into(), window: "daily".into() },
        ];
        // One 90-minute focus event (ahead of the 60 target by +30); nothing for
        // reading (behind by the full -30).
        let events = vec![ev(
            "focus",
            "2026-06-10T10:00:00Z",
            vec!["focus"],
            "90m 0s focus session on \"x\", 0 interruption(s).",
        )];
        let section = deviation_section(&goals, &events);
        assert!(section.contains("deviation +30"), "focus ahead: {section}");
        assert!(section.contains("deviation -30"), "reading behind (no events): {section}");
    }

    // ── tomorrow-action LLM second-pass ────────────────────────────────────
    // Use a tiny in-test provider so we don't need a real Gemini key in CI.

    use crate::life_node::multimodal::{
        AnalysisInput, MultimodalProvider, ProviderCapabilities, ProviderError,
    };

    /// Provider that returns a fixed string. Lets us probe the lint gate.
    struct CannedProvider(String);
    #[async_trait::async_trait]
    impl MultimodalProvider for CannedProvider {
        async fn analyze(&self, _input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
            Ok(AnalysisResult {
                summary: self.0.clone(),
                goal_impact: None,
                suggestion: None,
                confidence: None,
                raw_response: json!({}),
                model_id: "canned".into(),
                latency_ms: 0,
                cost_usd: None,
            })
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_image: false,
                supports_audio: false,
                supports_video: false,
                max_image_count: 0,
                max_audio_secs: 0,
                max_total_bytes: 0,
            }
        }
        fn model_id(&self) -> &str {
            "canned"
        }
    }

    #[tokio::test]
    async fn propose_tomorrow_action_returns_clean_llm_output() {
        let p = CannedProvider("明天早上 10 分鐘的散步走完家附近一圈".into());
        let action = propose_tomorrow_action("brief", &p).await.unwrap();
        assert!(action.contains("散步"));
        assert!(!action.is_empty());
    }

    #[tokio::test]
    async fn propose_tomorrow_action_rejects_shame_pattern_from_llm() {
        // LLM hallucinates blame phrasing → must NOT reach the user.
        let p = CannedProvider("你又熬夜了 — 早點睡".into());
        let err = propose_tomorrow_action("brief", &p).await.unwrap_err();
        match err {
            TomorrowActionError::ShameLeakage(_) => {} // expected
            other => panic!("expected ShameLeakage, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn propose_tomorrow_action_rejects_empty_llm_output() {
        let p = CannedProvider("   \n  ".into());
        let err = propose_tomorrow_action("brief", &p).await.unwrap_err();
        assert!(matches!(err, TomorrowActionError::Empty));
    }

    // ── provider fallback chain (SPEC-23) ──────────────────────────────────
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    enum ChainOutcome {
        Text(String),
        RateLimit,
    }
    /// Mock that returns a fixed text or a provider error, counting calls so a
    /// test can prove a later provider was (or was NOT) reached.
    struct ChainMock {
        outcome: ChainOutcome,
        calls: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl MultimodalProvider for ChainMock {
        async fn analyze(&self, _input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.outcome {
                ChainOutcome::Text(t) => Ok(AnalysisResult {
                    summary: t.clone(),
                    goal_impact: None,
                    suggestion: None,
                    confidence: None,
                    raw_response: json!({}),
                    model_id: "mock".into(),
                    latency_ms: 0,
                    cost_usd: None,
                }),
                ChainOutcome::RateLimit => {
                    Err(ProviderError::RateLimit { retry_after_ms: Some(10) })
                }
            }
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_image: false,
                supports_audio: false,
                supports_video: false,
                max_image_count: 0,
                max_audio_secs: 0,
                max_total_bytes: 0,
            }
        }
        fn model_id(&self) -> &str {
            "mock"
        }
    }
    fn boxed(outcome: ChainOutcome, calls: Arc<AtomicUsize>) -> Box<dyn MultimodalProvider> {
        Box::new(ChainMock { outcome, calls })
    }

    #[tokio::test]
    async fn chain_falls_back_on_provider_error() {
        let c = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::RateLimit, c.clone()),
            boxed(ChainOutcome::Text("明天散步 10 分鐘".into()), c.clone()),
        ];
        let action = propose_tomorrow_action_chain("brief", &chain).await.unwrap();
        assert!(action.contains("散步"));
        assert_eq!(c.load(Ordering::SeqCst), 2, "rate-limited primary → fell back to second");
    }

    #[tokio::test]
    async fn chain_hard_stops_on_shame_without_trying_next() {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::Text("你又熬夜了".into()), first.clone()),
            boxed(ChainOutcome::Text("乾淨的建議".into()), second.clone()),
        ];
        let err = propose_tomorrow_action_chain("brief", &chain).await.unwrap_err();
        assert!(matches!(err, TomorrowActionError::ShameLeakage(_)), "got {err:?}");
        assert_eq!(
            second.load(Ordering::SeqCst),
            0,
            "shame leakage is a HARD STOP — must NOT retry on another model (no lint-gaming)"
        );
    }

    #[tokio::test]
    async fn chain_first_clean_action_wins_rest_untouched() {
        let second = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::Text("walk 10 min tomorrow".into()), Arc::new(AtomicUsize::new(0))),
            boxed(ChainOutcome::Text("unused".into()), second.clone()),
        ];
        let action = propose_tomorrow_action_chain("b", &chain).await.unwrap();
        assert_eq!(action, "walk 10 min tomorrow");
        assert_eq!(second.load(Ordering::SeqCst), 0, "first clean action wins; rest untouched");
    }

    #[tokio::test]
    async fn chain_all_providers_fail_returns_provider_error() {
        let c = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::RateLimit, c.clone()),
            boxed(ChainOutcome::RateLimit, c.clone()),
        ];
        let err = propose_tomorrow_action_chain("b", &chain).await.unwrap_err();
        assert!(matches!(err, TomorrowActionError::Provider(_)), "got {err:?}");
        assert_eq!(c.load(Ordering::SeqCst), 2, "all providers attempted");
    }

    // ── 3-provider chain: Gemini → Groq → local Ollama (the new fallback) ──────
    // These model the production ordering: the hosted pair 429s on the free tier
    // and the local Ollama is the FINAL link that still produces an action.

    /// z13-observed failure mode: both hosted providers 429 on the free tier.
    /// With Ollama as the third link, the action still lands. Proves all three
    /// are tried IN ORDER and the local fallback wins.
    #[tokio::test]
    async fn chain_fallover_to_ollama_on_groq_ratelimit() {
        let gemini = Arc::new(AtomicUsize::new(0)); // stands for Gemini 429
        let groq = Arc::new(AtomicUsize::new(0)); // stands for Groq 429
        let ollama = Arc::new(AtomicUsize::new(0)); // stands for local Ollama
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::RateLimit, gemini.clone()),
            boxed(ChainOutcome::RateLimit, groq.clone()),
            boxed(ChainOutcome::Text("明天午休散步 10 分鐘".into()), ollama.clone()),
        ];
        let action = propose_tomorrow_action_chain("brief", &chain).await.unwrap();
        assert!(action.contains("散步"), "ollama action returned: {action}");
        assert_eq!(gemini.load(Ordering::SeqCst), 1, "gemini tried first");
        assert_eq!(groq.load(Ordering::SeqCst), 1, "groq tried after gemini 429");
        assert_eq!(ollama.load(Ordering::SeqCst), 1, "ollama reached after groq 429");
    }

    /// Every link (including Ollama) fails → terminal error returned, which
    /// `run_coach_review` renders as the graceful `(skipped: …)` footer. Proves
    /// the new third link does NOT change the all-fail degradation contract.
    #[tokio::test]
    async fn chain_all_fail_including_ollama_graceful_skip() {
        let c = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::RateLimit, c.clone()),
            boxed(ChainOutcome::RateLimit, c.clone()),
            boxed(ChainOutcome::RateLimit, c.clone()),
        ];
        let err = propose_tomorrow_action_chain("brief", &chain).await.unwrap_err();
        // Terminal error is a provider failure (graceful skip footer), NOT a panic.
        assert!(matches!(err, TomorrowActionError::Provider(_)), "got {err:?}");
        assert_eq!(c.load(Ordering::SeqCst), 3, "all three links attempted");
    }

    /// Shame leakage from the FIRST provider is a hard stop: neither Groq NOR
    /// Ollama is consulted. Re-rolling a shaming output across models until one
    /// slips past the lint would game the fail-closed guarantee (BIG-GOAL #1),
    /// so the new local link must NOT weaken it.
    #[tokio::test]
    async fn shame_leak_hardstop_before_ollama() {
        let gemini = Arc::new(AtomicUsize::new(0));
        let groq = Arc::new(AtomicUsize::new(0));
        let ollama = Arc::new(AtomicUsize::new(0));
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            boxed(ChainOutcome::Text("你又熬夜了".into()), gemini.clone()),
            boxed(ChainOutcome::Text("乾淨的建議".into()), groq.clone()),
            boxed(ChainOutcome::Text("也乾淨的建議".into()), ollama.clone()),
        ];
        let err = propose_tomorrow_action_chain("brief", &chain).await.unwrap_err();
        assert!(matches!(err, TomorrowActionError::ShameLeakage(_)), "got {err:?}");
        assert_eq!(gemini.load(Ordering::SeqCst), 1, "shame came from first provider");
        assert_eq!(groq.load(Ordering::SeqCst), 0, "hard stop: groq NOT consulted");
        assert_eq!(
            ollama.load(Ordering::SeqCst),
            0,
            "hard stop: ollama NOT consulted — no lint-gaming via the local link"
        );
    }

    // ── partner reflection folded into the review (the single proactive trigger) ──

    /// With `partner = None` the review behaves exactly as before: the
    /// deterministic events aggregate is produced, and NO "Daily alignment"
    /// section is appended. This is the no-LLM / test path that keeps
    /// `run_coach_review` runnable without a provider. (The `Some(..)` path
    /// drives a live LLM + Todoist and is exercised end-to-end via the CLI
    /// `spectyn coach review`, not in unit tests, to avoid network in CI.)
    #[tokio::test]
    async fn coach_review_without_partner_deps_has_no_alignment_section() {
        // Serialize on the env mutex: OLLAMA_DISABLE is process-global, and the
        // Ollama fallback is enabled by default — without disabling it this test
        // would attempt a real localhost:11434 connection (hermetic-CI failure /
        // latency). Disable it so the chain is empty and stays offline.
        let _g = crate::env_lock::acquire();
        let saved_ollama = std::env::var("OLLAMA_DISABLE").ok();
        std::env::set_var("OLLAMA_DISABLE", "1");
        let tmp = tempfile::tempdir().unwrap();
        // No GEMINI/GROQ keys in this process and Ollama disabled → tomorrow-action
        // degrades to the skipped footer, and no events dir → an empty events
        // review. Both are fine; we only assert the alignment section is absent
        // without deps.
        let r = run_coach_review(tmp.path(), "2026-06-05", false, None)
            .await
            .expect("review runs with no partner deps");
        match saved_ollama {
            Some(v) => std::env::set_var("OLLAMA_DISABLE", v),
            None => std::env::remove_var("OLLAMA_DISABLE"),
        }
        assert!(
            r.markdown.contains("# Daily review — 2026-06-05"),
            "events review heading present: {}",
            r.markdown
        );
        assert!(
            !r.markdown.contains("## Daily alignment"),
            "no partner reflection section without deps: {}",
            r.markdown
        );
    }

    /// E003 cut evidence (v0.6.0): the generated daily review must contain
    /// the "## Tomorrow's one action" section EXACTLY once — the spec
    /// criterion (`E003-coach-node-daily-review.md`). Exercised through the
    /// full `run_coach_review` path, hermetically: provider keys cleared and
    /// Ollama disabled, so the section is the graceful `(skipped: …)`
    /// variant — the heading itself must still be emitted, and emitted only
    /// once (a double append, or the heading leaking from the aggregate
    /// body, would both violate the spec).
    #[tokio::test]
    async fn coach_review_emits_tomorrows_one_action_section_exactly_once() {
        // Serialize on the env mutex: GEMINI/GROQ keys and OLLAMA_DISABLE are
        // process-global; clearing them keeps the provider chain empty so the
        // test stays offline and deterministic.
        let _g = crate::env_lock::acquire();
        let saved: Vec<(&str, Option<String>)> =
            ["GEMINI_API_KEY", "GROQ_API_KEY", "OLLAMA_DISABLE"]
                .iter()
                .map(|k| (*k, std::env::var(k).ok()))
                .collect();
        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("GROQ_API_KEY");
        std::env::set_var("OLLAMA_DISABLE", "1");

        let tmp = tempfile::tempdir().unwrap();
        let r = run_coach_review(tmp.path(), "2026-06-11", false, None).await;

        // Restore env BEFORE asserting so a failed assert can't leak state
        // into other tests sharing this process.
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }

        let r = r.expect("review must be produced even with no providers");
        assert_eq!(
            r.markdown.matches("## Tomorrow's one action").count(),
            1,
            "E003 spec criterion: review must contain the Tomorrow's-one-action \
             section exactly once; got:\n{}",
            r.markdown
        );
    }
}
