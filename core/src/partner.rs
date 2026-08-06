//! Partner ingress — the "brain" side of the life-partner MVP.
//!
//! Client-agnostic on purpose: the HTTP handlers in `serve.rs` (`/partner/*`),
//! the CLI, or the iOS app all funnel through here. Keeping the partner logic
//! decoupled from any one transport means the same brain serves every client —
//! we can validate the loop with a curl test client now and wire the iOS app
//! (messages + sensor signals) as the real client later, without changing this.
//!
//! Two halves of the partner (see docs/NORTH-STAR.md):
//!   - reactive: an inbound text → an agent turn (tools: search / run / record)
//!     → a reply. [`handle_message`]
//!   - proactive substrate: sensor/behaviour signals appended to a ledger that
//!     the daily alignment reflection reads. [`record_signal`],
//!     [`gather_reflection_context`], [`daily_reflection`]. The reflection is
//!     fired once a day from `spectyn coach review`
//!     (`life_node::daily_review::run_coach_review`) — the single trigger the
//!     SPEC-23 scheduler runs — so it is no longer a disconnected loop.
//!
//! Re-sent requests (an iOS retry, a swarm re-post, a double "send") must NOT
//! execute twice now that the partner is gaining write tools (Todoist add-task,
//! note capture). [`handle_message_idempotent`] is the at-most-once front door:
//! it consults [`crate::idempotency`] before delegating to the pure
//! [`handle_message`] brain, so the dedup rail lives at the ingress while the
//! brain stays transport- and dedup-agnostic (and unit-testable without IO).
//!
//! The ledger is a feature-flag-free JSONL file so the ingress works in the
//! default build. Rich cross-day memory (SkillMemory, behind
//! `experimental-memory`) is wired separately — subtask #9.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::agent::AgentRuntime;
use crate::life_node::note_capture::capture_note;

/// Result of handling one inbound partner message (the reactive half).
pub struct PartnerReply {
    pub reply: String,
    pub turns: u32,
    pub elapsed_secs: f64,
}

/// Who originated an inbound message — the dogfood-moat guard (2026-06-06).
///
/// The whole point of the MVP is "我真天天在用" (genuine daily human usage). That
/// signal is only meaningful if it is *uncheatable*: the autonomous loops, the
/// app's intent-classifier round-trips (`clusterDispatch.classifyIntent` posts
/// its prompt through `/partner/message`), and the smoke-tests all hit the same
/// brain — and every one of them used to land an `interaction` record in the
/// human-usage ledger, inflating the "I used it" count with machine traffic
/// (~93 single-day records, all bot-origin — see the 2026-06-05 pollution loop).
///
///   - [`MessageOrigin::Human`]: a real person typed this (the iOS chat box, a
///     human curl). It IS logged to the human-usage ledger ([`signals_path`]),
///     which is what [`gather_reflection_context`] / [`recent_memory_context`]
///     read.
///   - [`MessageOrigin::Machine`]: a bot/loop/classifier/smoke-test sent this.
///     It is run through the *same* brain (so behaviour is identical) but its
///     interaction record is segregated to [`machine_signals_path`] — it never
///     counts as real usage and is never fed back as "memory".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageOrigin {
    Human,
    Machine,
}

impl MessageOrigin {
    /// Parse the wire marker (case-insensitive). Anything other than an explicit
    /// machine marker is `None` so the caller can apply the content heuristic +
    /// human default — we never *upgrade* an unknown value to Human silently here.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            // `spectyn_self`/`dev-loop`/`dev_loop` are the dev/autonomous-loop
            // self-traffic markers (ACCEL-FRAMEWORK §④): they MUST resolve to
            // Machine so loop-generated signals never reach the human-usage moat.
            "machine" | "bot" | "system" | "classifier" | "loop" | "smoke" | "test"
            | "spectyn_self" | "spectyn-self" | "dev-loop" | "dev_loop" => {
                Some(MessageOrigin::Machine)
            }
            "human" | "user" | "person" => Some(MessageOrigin::Human),
            _ => None,
        }
    }
}

/// Defense-in-depth: detect a machine-origin message by its *content*, so even an
/// untagged classifier/loop call (legacy clients that don't yet send the origin
/// marker) is kept out of the human-usage ledger. Pure + tested.
///
/// The dominant polluter is the app's intent-classifier prompt
/// (`clusterDispatch.classifyIntent`): it instructs the model to reply with a
/// one-line `{"intent":...,"machine":...,"task":...}` JSON. Those prompts carry
/// stable, human-implausible markers. This errs toward *not* misclassifying a
/// real human message: it only fires on these specific classifier/loop markers,
/// never on ordinary length or punctuation.
pub fn looks_like_machine_prompt(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "意圖分類器",            // "intent classifier" — the classifier system prompt
        "只回一行 JSON",         // "reply with one line of JSON"
        "分類這句",              // "classify this sentence"
        "你是 spectyn 的意圖",   // classifier prompt opener
        "\"intent\":\"chat",     // the literal JSON shape the prompt dictates
        "只回:{\"intent\"",      // short classifier prompt form
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

/// Resolve the effective origin of an inbound message: an explicit machine
/// marker always wins; absent any explicit marker, the content heuristic catches
/// untagged bot traffic; otherwise default to [`MessageOrigin::Human`].
///
/// We deliberately do NOT let the content heuristic *downgrade* an explicit
/// `Human` marker — a person is allowed to literally ask about the classifier —
/// but an explicit Human marker is rare; the common safe case is "no marker →
/// heuristic → human", which keeps the ledger honest by default.
pub fn resolve_origin(explicit: Option<MessageOrigin>, text: &str) -> MessageOrigin {
    match explicit {
        Some(o) => o,
        None => {
            if looks_like_machine_prompt(text) {
                MessageOrigin::Machine
            } else {
                MessageOrigin::Human
            }
        }
    }
}

/// Intent of an inbound partner message (NORTH-STAR §5 decision ③: the two
/// first-decided message types). Detected by a small pure fn so the routing is
/// unit-testable without an LLM or any IO.
///
///   - [`Intent::Record`] (記東西): a texted note to persist + make recallable.
///     `body` is the note text with the trigger prefix stripped.
///   - [`Intent::Ask`] (查資料 / general): everything else — routed to an agent
///     turn, which may itself search/recall to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Record { body: String },
    Ask,
}

/// Record-intent triggers. Chinese: `記:` / `記下:` / `記 ` (note: the trailing
/// space form lets `記 買牛奶` work). English: `note:` (case-insensitive).
///
/// Detection is a pure fn — no LLM, no IO — so the fast-path routing is
/// deterministically testable. Matching is longest-prefix-first so `記下:`
/// wins over `記` and the full note body survives.
pub fn detect_intent(text: &str) -> Intent {
    let trimmed = text.trim_start();
    let lower = trimmed.to_lowercase();
    // Chinese triggers (case-insensitive is a no-op for these) + the English
    // `note:`. Ordered longest-first so a longer prefix is preferred.
    const PREFIXES: &[&str] = &["記下:", "記:", "note:", "記 "];
    for p in PREFIXES {
        // English trigger is matched case-insensitively; Chinese ones match as-is
        // (already covered since their lowercase equals themselves).
        let hit = if p.is_ascii() {
            lower.starts_with(p)
        } else {
            trimmed.starts_with(p)
        };
        if hit {
            let body = trimmed[p.len()..].trim().to_string();
            if !body.is_empty() {
                return Intent::Record { body };
            }
        }
    }
    Intent::Ask
}

/// The `.spectyn-mesh` base dir for note capture / recall. Resolved from
/// `$HOME` (via `dirs::home_dir`) so a temp `$HOME` isolates it in tests, and so
/// the note lands in the SAME `~/.spectyn-mesh/events` store the TUI `/recall`
/// and `/review` read. Mirrors the convention in [`signals_path`] and
/// `note_capture`.
fn spectyn_base_dir() -> PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
}

/// Handle a record-intent message synchronously: persist `body` as a recallable
/// note under `<base>/events` (the file event store `recall::search_events`
/// reads) and return a confirming reply. No LLM call. `base` is the
/// `.spectyn-mesh` dir (overridable for tests via `$HOME`).
fn handle_record(base: &Path, body: &str) -> anyhow::Result<PartnerReply> {
    capture_note(base, body, &["partner".into()])?;
    // Short echo so the user sees what was recorded (truncate very long notes so
    // the confirmation stays a confirmation, not a wall of text).
    let short: String = body.chars().take(40).collect();
    let reply = if short.len() < body.len() {
        format!("記下了:{short}…")
    } else {
        format!("記下了:{short}")
    };
    Ok(PartnerReply {
        reply,
        turns: 0,
        elapsed_secs: 0.0,
    })
}

/// Path of the JSONL signal/interaction ledger — the **human-usage** ledger that
/// proves "我真天天在用". Override with `SPECTYN_PARTNER_SIGNALS` (used in tests +
/// for relocating the brain's home). Only [`MessageOrigin::Human`] interactions
/// land here; [`gather_reflection_context`] / [`recent_memory_context`] read it.
pub fn signals_path() -> PathBuf {
    if let Ok(p) = std::env::var("SPECTYN_PARTNER_SIGNALS") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join(".spectyn-mesh"))
        .join("partner-signals.jsonl")
}

/// Path of the **segregated machine-traffic** log. Bot/loop/classifier/smoke-test
/// interactions are appended here instead of the human-usage ledger, so they are
/// still observable (debuggable) but can NEVER inflate the dogfood-usage count or
/// be fed back as "memory". Derived from [`signals_path`] (so the test override
/// `SPECTYN_PARTNER_SIGNALS` relocates both together) by inserting a `.machine`
/// stem: `partner-signals.jsonl` → `partner-signals.machine.jsonl`.
pub fn machine_signals_path() -> PathBuf {
    let human = signals_path();
    let ext = human
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jsonl");
    let stem = human
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("partner-signals");
    human.with_file_name(format!("{stem}.machine.{ext}"))
}

/// Path of the **dev/autonomous-loop self-traffic** log. Any signal whose origin
/// is a dev-loop / `spectyn_self` / machine source is appended here instead of the
/// human-usage ledger, so loop output is still observable (debuggable) but can
/// NEVER contaminate the "我真天天在用" moat (ACCEL-FRAMEWORK §④, owner option B).
/// Derived from [`signals_path`] (so the `SPECTYN_PARTNER_SIGNALS` test override
/// relocates all ledgers together) by replacing the file name with
/// `dev-loop-log.jsonl`: `…/partner-signals.jsonl` → `…/dev-loop-log.jsonl`.
pub fn dev_loop_log_path() -> PathBuf {
    signals_path().with_file_name("dev-loop-log.jsonl")
}

/// Pick the ledger an `interaction` of this `origin` should be written to:
/// [`MessageOrigin::Human`] → the human-usage ledger; [`MessageOrigin::Machine`]
/// → the segregated machine log. This is the single chokepoint that keeps bot
/// traffic out of the dogfood moat.
fn interaction_ledger(origin: MessageOrigin) -> PathBuf {
    match origin {
        MessageOrigin::Human => signals_path(),
        MessageOrigin::Machine => machine_signals_path(),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append one `{ts, kind, payload}` record to `path` (creating parents).
fn append_signal(path: &Path, kind: &str, payload: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = json!({ "ts": now_unix(), "kind": kind, "payload": payload });
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Record a signal to the human-usage ledger. `kind` groups records ("sensor",
/// "interaction", …); `payload` is stored verbatim. Returns the ledger path.
///
/// NOTE: this always writes the human-usage ledger and so must only be used for
/// genuinely human-origin signals (e.g. `/partner/signal` location check-ins).
/// `interaction` records go through [`record_interaction`], which routes by
/// [`MessageOrigin`] so bot traffic is segregated out of the dogfood moat.
pub fn record_signal(kind: &str, payload: &Value) -> std::io::Result<PathBuf> {
    let path = signals_path();
    append_signal(&path, kind, payload)?;
    Ok(path)
}

/// Record a dev/autonomous-loop signal to the segregated [`dev_loop_log_path`].
/// This is the explicit, named sink for self-traffic so it is observable but can
/// NEVER touch the human-usage moat. Returns the dev-loop ledger path.
pub fn record_dev_loop(kind: &str, payload: &Value) -> std::io::Result<PathBuf> {
    let path = dev_loop_log_path();
    append_signal(&path, kind, payload)?;
    Ok(path)
}

/// Pollution hard wall for the `/partner/signal` ingress: route a signal by its
/// resolved [`MessageOrigin`]. A genuine [`MessageOrigin::Human`] signal lands in
/// the human-usage ledger ([`record_signal`]); a [`MessageOrigin::Machine`]
/// signal (dev-loop / `spectyn_self` / classifier / smoke / etc.) is diverted to
/// the dev-loop log ([`record_dev_loop`]) and therefore **never** reaches
/// `partner-signals.jsonl`. Returns the path actually written so the caller can
/// report where the signal landed. This is the write-guard that keeps autonomous
/// loops out of the "我真天天在用" count (ACCEL-FRAMEWORK §④, owner option B).
pub fn record_signal_with_origin(
    origin: MessageOrigin,
    kind: &str,
    payload: &Value,
) -> std::io::Result<PathBuf> {
    match origin {
        MessageOrigin::Human => record_signal(kind, payload),
        MessageOrigin::Machine => record_dev_loop(kind, payload),
    }
}

/// Record an `interaction` record to the ledger chosen by `origin`: human
/// interactions land in the human-usage ledger ([`signals_path`]); machine
/// interactions are segregated to [`machine_signals_path`]. Returns the path
/// actually written. This is the write-guard that keeps autonomous loops,
/// classifier round-trips, and smoke-tests out of the "我真天天在用" count.
pub fn record_interaction(origin: MessageOrigin, payload: &Value) -> std::io::Result<PathBuf> {
    let path = interaction_ledger(origin);
    append_signal(&path, "interaction", payload)?;
    Ok(path)
}

/// Proactive half (NORTH-STAR §5 ④): record a coarse location/behaviour signal
/// as a first-class, typed entry point. This is a thin typed wrapper over
/// [`record_signal`] — it writes a `"sensor"` record with a structured
/// `{place, activity, ts}` payload — so coarse location/behaviour has a named
/// recorder while the HTTP `/partner/signal` endpoint (serve.rs) stays the
/// transport. The recorded content is picked up by
/// [`gather_reflection_context`] (counted under `sensor_signals`, and the most
/// recent in-window place/activity is surfaced into the reflection summary).
///
/// `place`/`activity` are both optional (coarse signals may carry only one);
/// `now_unix` is accepted for an explicit, caller-supplied timestamp in the
/// payload (the ledger's own `ts` is still stamped by [`record_signal`]).
pub fn record_location_behavior(
    now_unix: u64,
    place: Option<&str>,
    activity: Option<&str>,
) -> std::io::Result<PathBuf> {
    let mut payload = serde_json::Map::new();
    if let Some(p) = place {
        payload.insert("place".to_string(), Value::String(p.to_string()));
    }
    if let Some(a) = activity {
        payload.insert("activity".to_string(), Value::String(a.to_string()));
    }
    payload.insert("ts".to_string(), Value::from(now_unix));
    record_signal("sensor", &Value::Object(payload))
}

/// Reactive half: route an inbound text by [`Intent`] (NORTH-STAR §5 ③) and
/// return a reply.
///
///   - 記東西 ([`Intent::Record`]): a fast-path — the note is persisted
///     synchronously via [`capture_note`] into the file event store so a texted
///     note is reliably recallable (via `recall::search_events`), with NO LLM
///     round-trip. The confirmation is returned directly.
///   - 查資料 / general ([`Intent::Ask`]): the existing agent path — the model
///     may itself use tools (search, recall, run, record to memory/Todoist).
///
/// Either way the interaction is recorded best-effort to the signal ledger — a
/// ledger write failure never sinks the reply the user is waiting on. `origin`
/// (the dogfood-moat guard) decides WHICH ledger: a [`MessageOrigin::Machine`]
/// call (classifier round-trip, autonomous loop, smoke-test) runs the identical
/// brain but its interaction is segregated to [`machine_signals_path`] so it can
/// never inflate the human-usage count or be fed back as "memory".
pub async fn handle_message(
    runtime: &AgentRuntime,
    agent: &str,
    text: &str,
    origin: MessageOrigin,
) -> anyhow::Result<PartnerReply> {
    match detect_intent(text) {
        Intent::Record { body } => {
            let reply = handle_record(&spectyn_base_dir(), &body)?;
            let _ = record_interaction(
                origin,
                &json!({ "user": text, "reply": reply.reply, "intent": "record", "turns": 0 }),
            );
            Ok(reply)
        }
        Intent::Ask => {
            // Cross-day continuity (journey ① 懂我): inject a short summary of recent
            // ledger interactions so the partner "remembers" prior context
            // instead of treating each message cold. Clock is read HERE (kept out
            // of the testable builders) and passed in as `now_unix`.
            //
            // Two complementary windows, both sourced ONLY from the human-usage
            // ledger ([`signals_path`] — never `.machine.jsonl`), so bot traffic
            // can never reach the partner's "memory":
            //   - recent_memory_context: the newest individual interactions (72h).
            //   - cross_day_summary_context: the once-a-day reflection digests
            //     from earlier days (so a question on day 4 about day 1 still has
            //     a thread to pull). Concatenated newest-context-first.
            let now = now_unix();
            let recent = recent_memory_context(now, 10);
            let earlier = cross_day_summary_context(now, 3);
            let ctx = match (recent, earlier) {
                (Some(r), Some(e)) => Some(format!("{r}\n\n{e}")),
                (Some(r), None) => Some(r),
                (None, Some(e)) => Some(e),
                (None, None) => None,
            };
            let result = runtime
                .run(agent, text, &[], ctx.as_deref())
                .await?;
            let _ = record_interaction(
                origin,
                &json!({ "user": text, "reply": result.output, "intent": "ask", "turns": result.turns }),
            );
            Ok(PartnerReply {
                reply: result.output,
                turns: result.turns,
                elapsed_secs: result.elapsed_secs,
            })
        }
    }
}

/// At-most-once front door for an inbound message (the reactive ingress called by
/// `serve.rs`). Dedups *before* any side effect runs so a re-sent iOS message, a
/// swarm re-post, or a double-tapped send doesn't execute the agent turn — and,
/// crucially, doesn't fire the write tools the turn may invoke (Todoist add-task,
/// note capture) — twice.
///
/// `client_key` is the client's explicit request id when it sends one (preferred:
/// a UUID minted once per logical message survives even body-identical *distinct*
/// messages). When absent we fall back to a content hash of the text so a body
/// resent without a key still dedups within the TTL window. On a duplicate we
/// return the same confirming reply shape with `turns: 0` and a deduped marker in
/// the ledger interaction — no agent call, no tool call.
///
/// The dedup is recorded only once the key is confirmed new; a true first request
/// then delegates to the pure [`handle_message`] brain.
pub async fn handle_message_idempotent(
    runtime: &AgentRuntime,
    agent: &str,
    text: &str,
    client_key: Option<&str>,
    origin: MessageOrigin,
) -> anyhow::Result<(PartnerReply, bool)> {
    let key = match client_key {
        Some(k) if !k.trim().is_empty() => format!("partner_message:{}", k.trim()),
        _ => crate::idempotency::content_key("partner_message", text),
    };
    match crate::idempotency::check_and_record_default(&key, "partner_message") {
        crate::idempotency::Decision::Duplicate { first_seen } => {
            // Record the suppressed retry to the ledger (best-effort) so the
            // dedup is observable, but do NOT run the agent or any tool. Routed by
            // `origin` so a deduped bot retry stays out of the human-usage ledger.
            let _ = record_interaction(
                origin,
                &json!({
                    "user": text,
                    "reply": "(deduped — duplicate request suppressed)",
                    "intent": "deduped",
                    "turns": 0,
                    "first_seen": first_seen,
                }),
            );
            Ok((
                PartnerReply {
                    reply: "(已忽略重複請求)".to_string(),
                    turns: 0,
                    elapsed_secs: 0.0,
                },
                true,
            ))
        }
        crate::idempotency::Decision::First => {
            let reply = handle_message(runtime, agent, text, origin).await?;
            Ok((reply, false))
        }
    }
}

/// One day's worth of partner activity, distilled from the signal ledger.
///
/// This is the deterministic input to [`daily_reflection`]: build it from a
/// caller-supplied `now_unix` (never the wall clock) so the reflection is
/// reproducible and testable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReflectionContext {
    /// `now_unix` the window was computed against (end of the 24h window).
    pub now_unix: u64,
    /// Total ledger records within the last 24h.
    pub total: u64,
    /// `interaction` records (the user talked to the partner) in the window.
    pub interactions: u64,
    /// `sensor` records (location/motion/activity signals) in the window.
    pub sensor_signals: u64,
    /// Most recent in-window coarse `place` carried by a `sensor` signal's
    /// payload (e.g. "home"), if any — surfaced so the reflection mentions
    /// *where*, not just a count. Determined by newest `ts`.
    pub recent_place: Option<String>,
    /// Most recent in-window coarse `activity` carried by a `sensor` signal's
    /// payload (e.g. "working"), if any. Determined by newest `ts`.
    pub recent_activity: Option<String>,
    /// Count of every other `kind` seen in the window, e.g. earlier
    /// reflections, keyed by kind name. Sorted by kind for stable rendering.
    pub other_kinds: Vec<(String, u64)>,
}

impl ReflectionContext {
    /// A short human-readable line summarizing the day, for the ally prompt.
    pub fn summary(&self) -> String {
        if self.total == 0 {
            return "a quiet day — no interactions or signals logged".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.interactions > 0 {
            parts.push(format!(
                "{} interaction{}",
                self.interactions,
                if self.interactions == 1 { "" } else { "s" }
            ));
        }
        if self.sensor_signals > 0 {
            parts.push(format!(
                "{} sensor signal{}",
                self.sensor_signals,
                if self.sensor_signals == 1 { "" } else { "s" }
            ));
        }
        // Surface the latest coarse place/activity (not just a count) so the
        // ally-tone reflection can say *where* / *what*, e.g. "at home, working".
        match (&self.recent_place, &self.recent_activity) {
            (Some(p), Some(a)) => parts.push(format!("most recently at {p}, {a}")),
            (Some(p), None) => parts.push(format!("most recently at {p}")),
            (None, Some(a)) => parts.push(format!("most recently {a}")),
            (None, None) => {}
        }
        for (kind, n) in &self.other_kinds {
            parts.push(format!("{n} {kind}"));
        }
        parts.join(", ")
    }
}

/// Read the last 24h of the signal ledger (relative to `now_unix`) and tally it.
///
/// `now_unix` is a PARAMETER on purpose — the window is `(now_unix - 86400,
/// now_unix]` so the result is deterministic from the ledger + that timestamp,
/// never the wall clock. Honors the `SPECTYN_PARTNER_SIGNALS` override via
/// [`signals_path`]. A missing/unreadable ledger yields an empty (zeroed)
/// context rather than an error — a partner with no history still reflects.
pub fn gather_reflection_context(now_unix: u64) -> ReflectionContext {
    let mut ctx = ReflectionContext {
        now_unix,
        ..Default::default()
    };
    let window_start = now_unix.saturating_sub(24 * 60 * 60);

    let content = match std::fs::read_to_string(signals_path()) {
        Ok(c) => c,
        Err(_) => return ctx, // no ledger yet → quiet day
    };

    let mut others: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    // Track the newest-ts in-window sensor record that carries a place/activity,
    // so the reflection surfaces the *most recent* one deterministically.
    let mut place_ts: u64 = 0;
    let mut activity_ts: u64 = 0;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines, don't abort the day
        };
        let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
        // Window is the last 24h up to and including `now_unix`; older records
        // (and any from the future relative to `now_unix`) are excluded.
        if ts <= window_start || ts > now_unix {
            continue;
        }
        match rec.get("kind").and_then(Value::as_str).unwrap_or("") {
            // Machine-generated daily reflections are *outputs* of this very
            // reflection loop, not user *activity* — counting them would let each
            // day's reflection inflate the next day's "what I did" tally, a moat
            // self-watering loop (the partner-signals pollution; see also the
            // direction-audit warning). Skip them BEFORE incrementing `total` so
            // they never reach `ctx.other_kinds` or the usage count. They remain
            // in the ledger and are still read by `cross_day_summary_context`
            // (facet ①) for cross-day recall — this only excludes them from the
            // daily *tally*, not from the file.
            "reflection" => continue,
            "interaction" => {
                ctx.total += 1;
                ctx.interactions += 1;
            }
            "sensor" => {
                ctx.total += 1;
                ctx.sensor_signals += 1;
                // Surface the most recent coarse place/activity content. Ties
                // (equal ts) resolve to the later line in the ledger via `>=`.
                let payload = rec.get("payload");
                if let Some(p) = payload
                    .and_then(|v| v.get("place"))
                    .and_then(Value::as_str)
                {
                    if ts >= place_ts {
                        ctx.recent_place = Some(p.to_string());
                        place_ts = ts;
                    }
                }
                if let Some(a) = payload
                    .and_then(|v| v.get("activity"))
                    .and_then(Value::as_str)
                {
                    if ts >= activity_ts {
                        ctx.recent_activity = Some(a.to_string());
                        activity_ts = ts;
                    }
                }
            }
            other => {
                ctx.total += 1;
                *others.entry(other.to_string()).or_insert(0) += 1;
            }
        }
    }
    ctx.other_kinds = others.into_iter().collect();
    ctx
}

/// Build a short "recent memory" block from the signal ledger so the partner
/// carries cross-day continuity into an [`Intent::Ask`] turn (journey ②).
///
/// Source = the `partner-signals.jsonl` ledger ONLY (recent `interaction`
/// records — prior user messages + the partner's replies — and any `record`
/// notes that flowed through there). The ledger is key-free and always present
/// in the default build, so this is deterministic and cheap; we deliberately do
/// NOT read the encrypted `recall` event store here, since decrypting captured
/// notes needs a per-process `EventKey` that the live ingress doesn't install
/// (the plaintext wire/FTS5 branch is often empty — see finding_event_store).
///
/// `now_unix` is a PARAMETER (the window is `(now_unix - 72h, now_unix]`) so the
/// builder is testable without the wall clock — the async [`handle_message`]
/// reads the clock and passes it in. Returns `None` when there is nothing recent
/// worth injecting (no ledger, or no in-window interactions). Output is capped:
/// at most `max_items` of the most-recent interactions, each snippet truncated.
pub fn recent_memory_context(now_unix: u64, max_items: usize) -> Option<String> {
    if max_items == 0 {
        return None;
    }
    // Look back up to 72h so a question the morning after still "remembers".
    let window_start = now_unix.saturating_sub(72 * 60 * 60);
    let content = std::fs::read_to_string(signals_path()).ok()?;

    // Collect in-window interactions as (ts, line) so we can take the newest N.
    let mut items: Vec<(u64, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
        if ts <= window_start || ts > now_unix {
            continue;
        }
        if rec.get("kind").and_then(Value::as_str) != Some("interaction") {
            continue;
        }
        let payload = rec.get("payload");
        let user = payload
            .and_then(|p| p.get("user"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if user.is_empty() {
            continue;
        }
        let reply = payload
            .and_then(|p| p.get("reply"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        // Skip the app's intent-classifier round-trips: those replies are internal
        // {"intent":...,"machine":...,"task":...} JSON, not real conversation. Feeding
        // them back as "memory" makes the model mimic the format and answer in JSON to
        // everything — the partner-signals pollution loop (fixed 2026-06-05).
        if reply.contains("\"intent\"") && reply.contains("\"task\"") {
            continue;
        }
        let u = truncate_snippet(user);
        let summary = if reply.is_empty() {
            format!("you said: \"{u}\"")
        } else {
            let r = truncate_snippet(reply);
            format!("you said: \"{u}\" — I replied: \"{r}\"")
        };
        items.push((ts, summary));
    }

    if items.is_empty() {
        return None;
    }
    // Newest first, keep at most `max_items`.
    items.sort_by_key(|item| std::cmp::Reverse(item.0));
    items.truncate(max_items);

    let mut block = String::from("Recent context (prior messages, newest first):");
    for (_, line) in &items {
        block.push_str("\n- ");
        block.push_str(line);
    }
    Some(block)
}

/// Build a "earlier days" cross-day memory block from the prior daily reflections
/// so the partner carries continuity beyond the 72h [`recent_memory_context`]
/// window into an [`Intent::Ask`] turn (journey ① — cross-day recall).
///
/// Source = the `partner-signals.jsonl` ledger ONLY ([`signals_path`]), reading
/// the `"reflection"` records that [`daily_reflection`] already appends there
/// (its `{ now_unix, summary, text }` payload — see the `record_signal` call in
/// `daily_reflection`). Two safety properties fall out of reusing that ledger:
///
///   - **Machine-origin isolation (machine traffic excluded):** reflections are
///     written via [`record_signal`], which always targets the *human-usage*
///     ledger; the segregated `.machine.jsonl` bot log is never read here. There
///     is no path by which a [`MessageOrigin::Machine`] interaction becomes a
///     reflection, so no bot traffic can leak into this "memory" block. This is
///     why we read the existing reflection records instead of inventing a new
///     summary file: the human-only provenance is inherited, not re-implemented.
///   - **No experimental-memory:** this is the key-free plaintext JSONL
///     ledger of the default build — no FTS5 memory backend, no encrypted recall
///     event store (which the live ingress can't decrypt; see finding_event_store
///     and docs/SKILL-LOOP-SAFETY-EVAL.md R2: that path has no source gating).
///
/// `now_unix` is a PARAMETER (the window is `(now_unix - days*24h, now_unix]`) so
/// the builder is testable without the wall clock. Returns `None` when there are
/// no in-window reflections to inject. Output is capped at `days` of the most
/// recent reflections (a reflection is once-a-day, so `days` ≈ that many days),
/// each digest truncated to keep the prompt compact.
pub fn cross_day_summary_context(now_unix: u64, days: usize) -> Option<String> {
    if days == 0 {
        return None;
    }
    let window_start = now_unix.saturating_sub((days as u64) * 24 * 60 * 60);
    let content = std::fs::read_to_string(signals_path()).ok()?;

    // Collect in-window reflections as (ts, digest) so we can take the newest N.
    let mut items: Vec<(u64, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
        if ts <= window_start || ts > now_unix {
            continue;
        }
        // Only the once-a-day reflection records (written by `daily_reflection`)
        // — never raw interactions (those are the 72h window's job) and never any
        // other kind.
        if rec.get("kind").and_then(Value::as_str) != Some("reflection") {
            continue;
        }
        let payload = rec.get("payload");
        // Prefer the human-readable reflection `text`; fall back to the terse
        // `summary` tally when the text is absent.
        let digest = payload
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                payload
                    .and_then(|p| p.get("summary"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            });
        let digest = match digest {
            Some(d) => d,
            None => continue,
        };
        items.push((ts, truncate_snippet(digest)));
    }

    if items.is_empty() {
        return None;
    }
    // Newest first, keep at most `days` reflections.
    items.sort_by_key(|item| std::cmp::Reverse(item.0));
    items.truncate(days);

    let mut block = String::from("Earlier days (daily reflections, newest first):");
    for (_, line) in &items {
        block.push_str("\n- ");
        block.push_str(line);
    }
    Some(block)
}

/// Truncate a snippet to keep the injected context compact (single line, capped
/// length so the prompt doesn't bloat).
fn truncate_snippet(s: &str) -> String {
    const MAX: usize = 120;
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > MAX {
        let head: String = one_line.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        one_line
    }
}

/// The "what I said I wanted" side of the reflection: the user's real Todoist
/// goals (使用⑥ #8 — the goal model). Fetches open tasks + projects via the
/// shared [`crate::todoist`] REST client and formats a compact block.
///
/// Best-effort by design: a missing token, a network error, or an empty task
/// list all yield `None`, and [`daily_reflection`] falls back to the
/// "goals-not-connected" line — a reflection must never fail just because
/// Todoist is unreachable. `config` carries the token (or it falls back to the
/// `TODOIST_API_TOKEN` env var). At most `MAX_GOAL_TASKS` tasks are surfaced so
/// the prompt stays small.
const MAX_GOAL_TASKS: usize = 12;

pub async fn fetch_goal_model(config: Option<&crate::config::ToolsConfig>) -> Option<String> {
    let token = crate::todoist::resolve_token(config)?;
    // Fetch tasks first; projects are only used to label tasks, so a projects
    // failure shouldn't sink the goal model.
    let tasks = match crate::todoist::list_tasks(&token, None).await {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!("goal model: Todoist list_tasks failed: {e}");
            return None;
        }
    };
    let projects = crate::todoist::list_projects(&token).await.unwrap_or_default();
    crate::todoist::format_goal_model(&tasks, &projects, MAX_GOAL_TASKS)
}

/// Produce the once-a-day gentle alignment reflection (the proactive half).
///
/// Single trigger: this is fired from `spectyn coach review` via
/// [`crate::life_node::daily_review::run_coach_review`] (which the SPEC-23
/// scheduler runs daily). The events review and this goal-alignment reflection
/// share that one entry point so the two loops never drift apart.
///
/// Tone = **ally, not nanny** (docs/NORTH-STAR.md §5 decision ④): a gentle
/// summary comparing "what I said I wanted vs what actually happened" that
/// *offers* a reflection and leaves the user the decision — it does not nag,
/// correct, or dictate.
///
/// Determinism: `now_unix` is passed through to [`gather_reflection_context`];
/// only the LLM call (`runtime.run`) and the Todoist fetch are
/// non-deterministic, so tests exercise the deterministic context builder and
/// goal-model formatter directly and skip the live model + network.
///
/// Goal model (使用⑥ #8): the "what I said I wanted" side is the user's real
/// Todoist tasks, fetched via [`fetch_goal_model`]. When no token is configured
/// or Todoist is unreachable, the prompt falls back to reflecting on the
/// observed signals only (and does not invent goals).
pub async fn daily_reflection(
    runtime: &AgentRuntime,
    agent: &str,
    now_unix: u64,
    config: Option<&crate::config::ToolsConfig>,
) -> anyhow::Result<String> {
    let ctx = gather_reflection_context(now_unix);

    // "What I said I wanted" = real Todoist goals when available; otherwise an
    // explicit "not connected" line so the model reflects on observed signals
    // only and never fabricates goals.
    let goals = match fetch_goal_model(config).await {
        Some(block) => block,
        None => "<goals not connected — no Todoist token configured or no open tasks; \
                  reflect using the observed signals above only, and don't invent goals I never stated>"
            .to_string(),
    };

    let prompt = format!(
        "You are my life-partner giving me a once-a-day gentle alignment reflection.\n\
         \n\
         Here's my day (from logged activity): {summary}.\n\
         \n\
         What I said I wanted ({goals}).\n\
         \n\
         Gently reflect on whether today seems to align with how I tend to want to \
         spend my time, given those goals. Speak as an ally, not a nanny: offer an \
         observation, don't dictate; give me the agency to decide. No nagging, no \
         correcting, no scolding. Keep it short and warm — two or three sentences.",
        summary = ctx.summary(),
        goals = goals,
    );

    let result = runtime.run(agent, &prompt, &[], None).await?;

    // Record the reflection best-effort — a ledger write failure must not sink
    // the reflection we're about to surface.
    let _ = record_signal(
        "reflection",
        &json!({ "now_unix": now_unix, "summary": ctx.summary(), "text": result.output }),
    );

    Ok(result.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SPECTYN_PARTNER_SIGNALS` is process-global, but `cargo test` runs tests
    /// in parallel — one test's `remove_var` can clobber another's `set_var`
    /// mid-flight (e.g. between two `record_location_behavior` writes), making
    /// `signals_path()` resolve to the wrong ledger. Serialize every test that
    /// mutates this env var behind one lock so they don't interleave.
    ///
    /// Backed by the crate-wide [`crate::env_lock`] mutex (not a module-private
    /// one) so env-touching tests in OTHER modules — notably the
    /// `/partner/message` origin-routing tests in `serve.rs`, which set the SAME
    /// `SPECTYN_PARTNER_SIGNALS` to relocate the ledger — are serialized against
    /// these too; a per-module mutex would let those two groups race on the var.
    /// `lock()` returns an `Ok`-only `Result` so the historic `.unwrap()` /
    /// `.unwrap_or_else(...)` call sites keep compiling unchanged.
    struct EnvLockShim;
    impl EnvLockShim {
        fn lock(
            &self,
        ) -> Result<std::sync::MutexGuard<'static, ()>, std::convert::Infallible> {
            Ok(crate::env_lock::acquire())
        }
    }
    static ENV_LOCK: EnvLockShim = EnvLockShim;

    #[test]
    fn append_then_read_back() {
        let dir =
            std::env::temp_dir().join(format!("spectyn-partner-test-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        append_signal(&path, "sensor", &json!({ "lat": 1.5, "moving": true })).unwrap();
        append_signal(&path, "interaction", &json!({ "user": "hi" })).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "two appended records → two JSONL lines");

        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["kind"], "sensor");
        assert_eq!(first["payload"]["lat"], 1.5);
        assert_eq!(first["payload"]["moving"], true);
        assert!(first["ts"].as_u64().is_some(), "ts is a unix-seconds number");

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["kind"], "interaction");
        assert_eq!(second["payload"]["user"], "hi");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gather_reflection_windows_by_now_unix() {
        // Fixed "now" so the test is deterministic regardless of wall clock.
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-reflect-test-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Seed the ledger directly (controlled `ts`), bypassing append_signal's
        // wall-clock timestamp. Some records inside the 24h window, some older,
        // one in the future relative to NOW.
        let records = [
            json!({ "ts": NOW - 100,        "kind": "interaction", "payload": {} }),
            json!({ "ts": NOW - 200,        "kind": "interaction", "payload": {} }),
            json!({ "ts": NOW - DAY + 10,   "kind": "sensor",      "payload": {} }),
            json!({ "ts": NOW - 300,        "kind": "reflection",  "payload": {} }),
            json!({ "ts": NOW - DAY - 5,    "kind": "interaction", "payload": {} }), // too old
            json!({ "ts": NOW - DAY * 3,    "kind": "sensor",      "payload": {} }), // too old
            json!({ "ts": NOW + 50,         "kind": "interaction", "payload": {} }), // future
        ];
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        // Point the brain at our seeded ledger for the duration of this test.
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = gather_reflection_context(NOW);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        assert_eq!(ctx.now_unix, NOW);
        assert_eq!(ctx.interactions, 2, "two interactions are within the 24h window");
        assert_eq!(ctx.sensor_signals, 1, "one sensor signal is within the window");
        // Machine-generated reflection records are NOT user activity: they must be
        // excluded from the daily tally (moat self-watering fix, 2026-06-06), so
        // the in-window reflection at NOW-300 contributes nothing to other_kinds.
        assert!(
            ctx.other_kinds.is_empty(),
            "reflection records are excluded from the usage tally; got {:?}",
            ctx.other_kinds
        );
        // 2 interactions + 1 sensor = 3; the in-window reflection is excluded,
        // and old + future records are excluded.
        assert_eq!(
            ctx.total, 3,
            "reflection, older-than-24h, and future records are excluded"
        );

        let summary = ctx.summary();
        assert!(summary.contains("2 interactions"), "summary: {summary}");
        assert!(summary.contains("1 sensor signal"), "summary: {summary}");
        assert!(
            !summary.contains("reflection"),
            "reflection must not inflate the daily summary; summary: {summary}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Moat self-watering guard (a): appending more and more machine-generated
    /// `reflection` records must NOT make the usage tally climb. Before the fix,
    /// each day's reflection landed in `other_kinds` and bumped `total`, so the
    /// "I really use this every day" count drifted 1→2→3→4 on its own — pure
    /// machine output masquerading as user activity. After the fix, the count
    /// reflects only the real interaction, regardless of how many reflections
    /// pile up in the ledger.
    #[test]
    fn reflection_records_do_not_inflate_usage_tally() {
        const NOW: u64 = 1_700_000_000;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-reflect-noinflate-{}", std::process::id()));
        let path = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // One genuine user interaction, plus a heap of machine reflections all
        // inside the 24h window — exactly the self-watering scenario.
        let mut records = vec![json!({
            "ts": NOW - 10, "kind": "interaction",
            "payload": { "user": "hey", "reply": "hi" }
        })];
        for i in 1..=5u64 {
            records.push(json!({
                "ts": NOW - 100 - i, "kind": "reflection",
                "payload": { "now_unix": NOW - 100 - i, "summary": "a quiet day",
                             "text": format!("Reflection number {i}.") }
            }));
        }
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = gather_reflection_context(NOW);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // Five reflections present in the ledger, yet none counted.
        assert_eq!(ctx.interactions, 1, "the single real interaction is counted");
        assert!(
            ctx.other_kinds.is_empty(),
            "no reflection leaks into the tally; got {:?}",
            ctx.other_kinds
        );
        assert_eq!(
            ctx.total, 1,
            "tally counts only the real interaction, not the 5 machine reflections"
        );
        let summary = ctx.summary();
        assert!(
            !summary.contains("reflection"),
            "reflections never reach the summary; summary: {summary}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Moat self-watering guard (b): the fix is read-side segregation, not
    /// deletion. The very reflection records excluded from the usage tally above
    /// must STILL be readable by `cross_day_summary_context` (facet ①) so the
    /// partner keeps its cross-day recall. This pins both halves at once: the same
    /// ledger that yields a non-inflating tally still yields the reflection text
    /// to the "Earlier days" memory block — no regression of facet ①.
    #[test]
    fn reflections_excluded_from_tally_still_reach_cross_day_recall() {
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-reflect-recall-{}", std::process::id()));
        let path = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A reflection inside the 24h tally window AND inside the cross-day window.
        let records = [json!({
            "ts": NOW - DAY / 2, "kind": "reflection",
            "payload": { "now_unix": NOW - DAY / 2, "summary": "1 interaction",
                         "text": "You shipped the moat self-watering fix today." }
        })];
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        // facet ⓿ tally: the reflection must NOT be counted.
        let tally = gather_reflection_context(NOW);
        // facet ① cross-day recall: the SAME reflection must still be injected.
        let recall = cross_day_summary_context(NOW, 3);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        assert_eq!(tally.total, 0, "reflection excluded from the usage tally");
        assert!(tally.other_kinds.is_empty(), "no tally leak");

        let recall = recall.expect("facet ① still recalls the reflection");
        assert!(recall.starts_with("Earlier days"), "recall: {recall}");
        assert!(
            recall.contains("moat self-watering fix"),
            "the excluded-from-tally reflection is still surfaced for cross-day \
             recall — facet ① not regressed; recall: {recall}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_location_behavior_is_counted_and_surfaced() {
        // Proactive half (NORTH-STAR §5 ④): a coarse location/behaviour signal
        // recorded via the typed helper must be (a) picked up by
        // gather_reflection_context within its 24h window (counted) and
        // (b) have its place/activity *content* surfaced into the summary.
        //
        // record_location_behavior → record_signal → append_signal stamps the
        // ledger `ts` with the wall clock, so we must window against the *real*
        // now (not a fixed past constant) or the just-written records fall out
        // of the 24h window. We read the clock once and reflect against it.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let dir = std::env::temp_dir()
            .join(format!("spectyn-locbehav-test-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);

        // Two coarse signals; the second (recorded later → newest ledger line)
        // is the one that should surface as "most recent". The `now_unix` arg
        // only populates the payload's own `ts`; ordering here is by ledger ts
        // (wall clock), and the second call is appended last so it wins ties.
        record_location_behavior(now, Some("office"), Some("commuting")).unwrap();
        record_location_behavior(now, Some("home"), Some("working")).unwrap();

        // Window against a now slightly ahead so both wall-clock-stamped records
        // are inside (now, now+...] -> they were stamped at-or-before this.
        let ctx = gather_reflection_context(now + 5);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // (a) Counted as sensor signals.
        assert_eq!(ctx.sensor_signals, 2, "both coarse signals counted");
        assert_eq!(ctx.total, 2);
        // (b) Content surfaced — most recent place/activity, not just a count.
        // Both records share the same wall-clock second; the later-appended
        // ledger line ("home"/"working") wins via the `>=` tie-break.
        assert_eq!(ctx.recent_place.as_deref(), Some("home"));
        assert_eq!(ctx.recent_activity.as_deref(), Some("working"));

        let summary = ctx.summary();
        assert!(summary.contains("2 sensor signals"), "summary: {summary}");
        assert!(
            summary.contains("most recently at home, working"),
            "place/activity surfaced; summary: {summary}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_location_behavior_surfaces_from_seeded_window() {
        // Deterministic variant: seed the ledger directly with controlled `ts`
        // (append_signal stamps wall-clock ts, so we bypass it here) to prove
        // the *newest in-window* place/activity is the one surfaced.
        const NOW: u64 = 1_700_000_000;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-locbehav-seed-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let records = [
            json!({ "ts": NOW - 900, "kind": "sensor",
                    "payload": { "place": "gym", "activity": "exercising", "ts": NOW - 900 } }),
            json!({ "ts": NOW - 50,  "kind": "sensor",
                    "payload": { "place": "home", "activity": "resting", "ts": NOW - 50 } }),
            // Only a place, only an activity — both partial coarse signals.
            json!({ "ts": NOW - 200, "kind": "sensor",
                    "payload": { "activity": "reading", "ts": NOW - 200 } }),
            // Out-of-window: must NOT win even though it is "place"-bearing.
            json!({ "ts": NOW + 100, "kind": "sensor",
                    "payload": { "place": "future", "ts": NOW + 100 } }),
        ];
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = gather_reflection_context(NOW);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // 3 in-window sensors (future one excluded).
        assert_eq!(ctx.sensor_signals, 3);
        // Newest in-window place is "home" (ts NOW-50), beating "gym".
        assert_eq!(ctx.recent_place.as_deref(), Some("home"));
        // Newest in-window activity is "resting" (ts NOW-50), beating "reading".
        assert_eq!(ctx.recent_activity.as_deref(), Some("resting"));
        // The future "place":"future" record is excluded from surfacing.
        assert_ne!(ctx.recent_place.as_deref(), Some("future"));

        let summary = ctx.summary();
        assert!(
            summary.contains("most recently at home, resting"),
            "summary: {summary}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_memory_context_summarizes_recent_interactions() {
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-recent-mem-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Seed: two recent interactions (in the 72h window), one interaction far
        // too old, one in the future, and a non-interaction record.
        let records = [
            json!({ "ts": NOW - 100, "kind": "interaction",
                    "payload": { "user": "remind me to buy milk", "reply": "記下了:remind me to buy milk" } }),
            json!({ "ts": NOW - 5000, "kind": "interaction",
                    "payload": { "user": "what's the weather", "reply": "it's sunny" } }),
            json!({ "ts": NOW - DAY * 5, "kind": "interaction",
                    "payload": { "user": "ancient question", "reply": "old answer" } }), // > 72h
            json!({ "ts": NOW + 50, "kind": "interaction",
                    "payload": { "user": "from the future", "reply": "nope" } }),        // future
            json!({ "ts": NOW - 200, "kind": "sensor", "payload": { "moving": true } }),  // not an interaction
        ];
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = recent_memory_context(NOW, 5);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        let ctx = ctx.expect("two in-window interactions → Some(context)");
        assert!(ctx.starts_with("Recent context"), "ctx: {ctx}");
        // Recent snippets present.
        assert!(ctx.contains("remind me to buy milk"), "ctx: {ctx}");
        assert!(ctx.contains("what's the weather"), "ctx: {ctx}");
        // Out-of-window / non-interaction records excluded.
        assert!(!ctx.contains("ancient question"), "stale excluded; ctx: {ctx}");
        assert!(!ctx.contains("from the future"), "future excluded; ctx: {ctx}");
        assert!(!ctx.contains("moving"), "sensor excluded; ctx: {ctx}");
        // Newest-first ordering.
        let milk = ctx.find("remind me to buy milk").unwrap();
        let weather = ctx.find("what's the weather").unwrap();
        assert!(milk < weather, "newest interaction listed first; ctx: {ctx}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_memory_context_respects_max_items() {
        const NOW: u64 = 1_700_000_000;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-recent-mem-cap-{}", std::process::id()));
        let path = dir.join("sig.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Five in-window interactions; we only want the 2 newest.
        let records: Vec<Value> = (0..5)
            .map(|i| {
                json!({ "ts": NOW - (i as u64) * 10, "kind": "interaction",
                        "payload": { "user": format!("msg{i}"), "reply": "ok" } })
            })
            .collect();
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = recent_memory_context(NOW, 2).expect("Some");
        let none = recent_memory_context(NOW, 0);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // Exactly 2 bullet lines.
        assert_eq!(ctx.matches("\n- ").count(), 2, "capped at max_items=2; ctx: {ctx}");
        // The 2 newest (msg0, msg1) are kept; msg4 (oldest) dropped.
        assert!(ctx.contains("msg0") && ctx.contains("msg1"), "ctx: {ctx}");
        assert!(!ctx.contains("msg4"), "oldest dropped; ctx: {ctx}");
        // max_items == 0 → None (nothing to inject).
        assert!(none.is_none(), "max_items=0 yields None");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_memory_context_empty_ledger_is_none() {
        let path = std::env::temp_dir().join(format!(
            "spectyn-recent-mem-missing-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let missing = recent_memory_context(1_700_000_000, 5);
        // An empty (existing but contentless) ledger is also None.
        std::fs::write(&path, "").unwrap();
        let empty = recent_memory_context(1_700_000_000, 5);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        assert!(missing.is_none(), "no ledger → None");
        assert!(empty.is_none(), "empty ledger → None");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncate_snippet_collapses_whitespace_to_single_spaces() {
        assert_eq!(
            truncate_snippet("alpha \t\t beta\n\n gamma   delta"),
            "alpha beta gamma delta"
        );
    }

    #[test]
    fn truncate_snippet_keeps_short_collapsed_input_without_ellipsis() {
        let snippet = truncate_snippet("short\ninput   under\tlimit");

        assert_eq!(snippet, "short input under limit");
        assert!(!snippet.contains('…'), "short snippet must not be ellipsized");
    }

    #[test]
    fn truncate_snippet_limits_long_input_to_120_chars_plus_ellipsis() {
        let snippet = truncate_snippet(&"a".repeat(121));

        assert_eq!(snippet.chars().count(), 121);
        assert_eq!(snippet.chars().take(120).count(), 120);
        assert!(snippet.ends_with('…'), "long snippet must end with ellipsis");
    }

    #[test]
    fn cross_day_summary_injects_prior_reflections() {
        // Facet ① — cross-day recall: a reflection digest from an earlier day
        // (within the N-day window) is surfaced so a question days later still has
        // a thread to pull. Out-of-window reflections and non-reflection records
        // are excluded; newest-first ordering; each digest truncated.
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-crossday-{}", std::process::id()));
        let path = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let records = [
            // Yesterday's + the day-before's reflections — both in a 3-day window.
            json!({ "ts": NOW - DAY, "kind": "reflection",
                    "payload": { "now_unix": NOW - DAY, "summary": "1 interaction",
                                 "text": "Yesterday you spent time on interview prep." } }),
            json!({ "ts": NOW - DAY * 2, "kind": "reflection",
                    "payload": { "now_unix": NOW - DAY * 2, "summary": "2 interactions",
                                 "text": "Two days ago you rested at home." } }),
            // Too old: outside a 3-day window → excluded.
            json!({ "ts": NOW - DAY * 5, "kind": "reflection",
                    "payload": { "text": "Ancient reflection from last week." } }),
            // Future relative to NOW → excluded.
            json!({ "ts": NOW + 100, "kind": "reflection",
                    "payload": { "text": "A reflection from the future." } }),
            // Not a reflection → excluded (interactions are the 72h window's job).
            json!({ "ts": NOW - 50, "kind": "interaction",
                    "payload": { "user": "raw chat", "reply": "ok" } }),
        ];
        let body: String = records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = cross_day_summary_context(NOW, 3);
        let none_zero = cross_day_summary_context(NOW, 0);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        let ctx = ctx.expect("two in-window reflections → Some(context)");
        assert!(ctx.starts_with("Earlier days"), "ctx: {ctx}");
        // Both in-window reflection digests are present.
        assert!(ctx.contains("interview prep"), "ctx: {ctx}");
        assert!(ctx.contains("rested at home"), "ctx: {ctx}");
        // Out-of-window / non-reflection records are excluded.
        assert!(!ctx.contains("Ancient reflection"), "stale excluded; ctx: {ctx}");
        assert!(!ctx.contains("from the future"), "future excluded; ctx: {ctx}");
        assert!(!ctx.contains("raw chat"), "interaction excluded; ctx: {ctx}");
        // Newest-first ordering: yesterday before the day-before.
        let yest = ctx.find("interview prep").unwrap();
        let prior = ctx.find("rested at home").unwrap();
        assert!(yest < prior, "newest reflection first; ctx: {ctx}");
        // days == 0 → None (nothing to inject).
        assert!(none_zero.is_none(), "days=0 yields None");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_day_summary_recalls_beyond_72h_recent_window() {
        // Facet ① — the load-bearing point of the cross-day path: it must surface a
        // human reflection that is OLDER than the 72h `recent_memory_context`
        // window. We place a single reflection 6 days back (144h) and assert:
        //   - `recent_memory_context` (72h) does NOT see it (proves the recent
        //     window genuinely stops at 72h, so this is the only path to it), and
        //   - `cross_day_summary_context` with a >6-day window DOES surface it
        //     (proves cross-day recall really extends memory past 72h).
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-crossday-72h-{}", std::process::id()));
        let path = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A reflection 6 days ago (144h > 72h) — beyond the recent window.
        let old_reflection = json!({ "ts": NOW - DAY * 6, "kind": "reflection",
            "payload": { "text": "Six days ago you started the job search." } });
        std::fs::write(&path, format!("{old_reflection}\n")).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let recent = recent_memory_context(NOW, 10);
        let cross = cross_day_summary_context(NOW, 10);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // The 72h recent window must NOT reach a 144h-old record — and it's a
        // reflection (not an interaction) so recent_memory_context skips it too.
        assert!(
            recent.is_none(),
            "72h recent window must not see a 144h-old reflection; got {recent:?}"
        );
        // The cross-day path WITH a >6-day window must surface it.
        let cross = cross.expect(">72h reflection within the cross-day window → Some");
        assert!(
            cross.contains("job search"),
            "cross-day recall must surface the >72h human reflection; ctx: {cross}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_day_summary_excludes_machine_origin_ledger() {
        // Machine-origin moat: cross-day recall must read ONLY the human-usage
        // ledger ([`signals_path`]). A reflection-shaped record sitting in the
        // segregated `.machine.jsonl` sibling must NEVER be injected — proving bot
        // traffic cannot leak into the partner's "memory" even if it were shaped
        // like a reflection. (In production, reflections are written via
        // `record_signal`, which always targets the human ledger, so a machine
        // reflection cannot arise; this guards the read side regardless.)
        const NOW: u64 = 1_700_000_000;
        const DAY: u64 = 24 * 60 * 60;

        let dir = std::env::temp_dir()
            .join(format!("spectyn-crossday-moat-{}", std::process::id()));
        let human = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &human);
        // The segregated machine log derives from the same override.
        let machine = machine_signals_path();
        assert_eq!(machine, dir.join("partner-signals.machine.jsonl"));

        // Human ledger: one genuine reflection.
        let human_rec = json!({ "ts": NOW - DAY, "kind": "reflection",
            "payload": { "text": "Human-day: you worked on the partner MVP." } });
        std::fs::write(&human, format!("{human_rec}\n")).unwrap();

        // Machine log: a reflection-SHAPED record from a bot. Must be ignored.
        let machine_rec = json!({ "ts": NOW - DAY, "kind": "reflection",
            "payload": { "text": "BOT-POLLUTION: autonomous loop self-summary." } });
        std::fs::write(&machine, format!("{machine_rec}\n")).unwrap();

        let ctx = cross_day_summary_context(NOW, 3);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        let ctx = ctx.expect("the human reflection is injected");
        assert!(ctx.contains("partner MVP"), "human reflection present; ctx: {ctx}");
        assert!(
            !ctx.contains("BOT-POLLUTION"),
            "machine-origin ledger must NEVER reach cross-day memory; ctx: {ctx}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gather_reflection_empty_ledger_is_quiet_day() {
        let path = std::env::temp_dir().join(format!(
            "spectyn-reflect-missing-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let ctx = gather_reflection_context(1_700_000_000);
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        assert_eq!(ctx.total, 0);
        assert_eq!(ctx.summary(), "a quiet day — no interactions or signals logged");
    }

    #[test]
    fn env_override_wins() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", "/tmp/spectyn-partner-custom.jsonl");
        assert_eq!(
            signals_path(),
            PathBuf::from("/tmp/spectyn-partner-custom.jsonl")
        );
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");
    }

    // ── Dogfood-moat write-guard: bot traffic must not pollute human usage ──────

    #[test]
    fn machine_path_is_segregated_from_human_path() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var(
            "SPECTYN_PARTNER_SIGNALS",
            "/tmp/spectyn-moat/partner-signals.jsonl",
        );
        assert_eq!(
            signals_path(),
            PathBuf::from("/tmp/spectyn-moat/partner-signals.jsonl")
        );
        // The machine log derives from the same override, with a `.machine` stem,
        // so the test override relocates both together and they never collide.
        assert_eq!(
            machine_signals_path(),
            PathBuf::from("/tmp/spectyn-moat/partner-signals.machine.jsonl")
        );
        assert_ne!(signals_path(), machine_signals_path());
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");
    }

    #[test]
    fn machine_path_default_has_machine_stem() {
        // With no override, the machine log sits beside the human ledger under
        // ~/.spectyn-mesh with a `.machine` stem (never the human file itself).
        let _env = ENV_LOCK.lock().unwrap();
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");
        let h = signals_path();
        let m = machine_signals_path();
        assert!(h.ends_with("partner-signals.jsonl"), "human: {h:?}");
        assert!(m.ends_with("partner-signals.machine.jsonl"), "machine: {m:?}");
        assert_eq!(h.parent(), m.parent(), "co-located, different file");
    }

    #[test]
    fn looks_like_machine_prompt_catches_classifier() {
        // The real polluter: the app's intent-classifier prompt (clusterDispatch).
        let classifier_full = "你是 spectyn 的意圖分類器,只回一行 JSON,不要任何其他文字。\n\
             使用者輸入:讓所有機器回答python";
        let classifier_short = "分類這句,只回:{\"intent\":\"chat\",\"machine\":\"\",\"task\":\"\"}\n\
             使用者說:你好";
        assert!(looks_like_machine_prompt(classifier_full), "full classifier prompt");
        assert!(looks_like_machine_prompt(classifier_short), "short classifier prompt");
    }

    #[test]
    fn looks_like_machine_prompt_passes_real_human_text() {
        // Genuine human messages must NOT be misclassified as machine — otherwise
        // we'd silently drop real usage. None of these carry classifier markers.
        for t in [
            "你好",
            "今天天氣如何",
            "記: 買牛奶",
            "用一句話解釋什麼是遞迴",
            "我在桃園中壢準備面試,給我2個現場提醒,簡短",
            "what's the capital of France?",
            "remind me to call mum tomorrow",
        ] {
            assert!(
                !looks_like_machine_prompt(t),
                "human text wrongly flagged as machine: {t:?}"
            );
        }
    }

    #[test]
    fn resolve_origin_precedence() {
        // Explicit marker always wins (even a human asking *about* a classifier).
        assert_eq!(
            resolve_origin(Some(MessageOrigin::Machine), "你好"),
            MessageOrigin::Machine
        );
        assert_eq!(
            resolve_origin(Some(MessageOrigin::Human), "你是 spectyn 的意圖分類器"),
            MessageOrigin::Human,
            "an explicit Human marker is not downgraded by the content heuristic"
        );
        // No marker → content heuristic catches the legacy untagged classifier.
        assert_eq!(
            resolve_origin(None, "你是 spectyn 的意圖分類器,只回一行 JSON"),
            MessageOrigin::Machine
        );
        // No marker + ordinary text → Human (the common, safe default).
        assert_eq!(resolve_origin(None, "今天過得如何"), MessageOrigin::Human);
    }

    #[test]
    fn message_origin_from_wire() {
        assert_eq!(MessageOrigin::from_wire("machine"), Some(MessageOrigin::Machine));
        assert_eq!(MessageOrigin::from_wire("BOT"), Some(MessageOrigin::Machine));
        assert_eq!(MessageOrigin::from_wire(" Classifier "), Some(MessageOrigin::Machine));
        assert_eq!(MessageOrigin::from_wire("human"), Some(MessageOrigin::Human));
        assert_eq!(MessageOrigin::from_wire("user"), Some(MessageOrigin::Human));
        // Unknown values are None so the caller falls back to the heuristic — we
        // never silently upgrade an unknown marker to Human.
        assert_eq!(MessageOrigin::from_wire("wat"), None);
        assert_eq!(MessageOrigin::from_wire(""), None);
    }

    #[test]
    fn record_interaction_routes_by_origin() {
        // A human interaction lands in the human-usage ledger; a machine
        // interaction lands ONLY in the segregated machine log — proving a
        // bot-origin call no longer touches the dogfood moat.
        let dir = std::env::temp_dir()
            .join(format!("spectyn-moat-route-{}", std::process::id()));
        let human = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &human);
        let machine = machine_signals_path();

        record_interaction(MessageOrigin::Human, &json!({ "user": "real human msg" }))
            .unwrap();
        record_interaction(MessageOrigin::Machine, &json!({ "user": "classifier prompt" }))
            .unwrap();
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        let human_content = std::fs::read_to_string(&human).unwrap();
        let machine_content = std::fs::read_to_string(&machine).unwrap();

        // Human ledger has exactly the human record and NOT the machine one.
        assert!(human_content.contains("real human msg"), "human: {human_content}");
        assert!(
            !human_content.contains("classifier prompt"),
            "bot record must NOT be in the human-usage ledger: {human_content}"
        );
        // Machine log has exactly the bot record and NOT the human one.
        assert!(machine_content.contains("classifier prompt"), "machine: {machine_content}");
        assert!(
            !machine_content.contains("real human msg"),
            "human record must NOT be in the machine log: {machine_content}"
        );
        // Each ledger has exactly one line.
        assert_eq!(human_content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
        assert_eq!(machine_content.lines().filter(|l| !l.trim().is_empty()).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pollution hard wall (ACCEL-FRAMEWORK §④, owner option B): a dev-loop /
    /// `spectyn_self` / machine signal MUST be diverted to `dev-loop-log.jsonl`
    /// and must leave `partner-signals.jsonl` (the human-usage moat) untouched.
    #[test]
    fn dev_loop_never_writes_partner_signals() {
        let dir = std::env::temp_dir()
            .join(format!("spectyn-devloop-wall-{}", std::process::id()));
        let human = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &human);
        let dev_loop = dev_loop_log_path();

        // Every dev-loop self-traffic marker resolves to Machine and is diverted.
        for marker in ["spectyn_self", "dev-loop", "dev_loop", "machine", "loop", "smoke"] {
            let origin = resolve_origin(MessageOrigin::from_wire(marker), "");
            assert_eq!(
                origin,
                MessageOrigin::Machine,
                "marker {marker:?} must resolve to Machine"
            );
            record_signal_with_origin(origin, "sensor", &json!({ "src": marker }))
                .unwrap();
        }
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        // The human-usage moat ledger must NOT exist / must have zero lines: not a
        // single dev-loop record leaked in.
        let human_lines = std::fs::read_to_string(&human)
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        assert_eq!(
            human_lines, 0,
            "dev-loop traffic must NOT touch partner-signals.jsonl (found {human_lines} line(s))"
        );

        // The diverted output landed in dev-loop-log.jsonl (one line per marker).
        let dev_loop_content = std::fs::read_to_string(&dev_loop)
            .expect("dev-loop-log.jsonl should exist after machine writes");
        let dev_loop_lines =
            dev_loop_content.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(dev_loop_lines, 6, "dev-loop log: {dev_loop_content}");
        assert!(dev_loop_content.contains("spectyn_self"), "{dev_loop_content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Positive (no-false-kill) half: a genuine human-origin signal still lands in
    /// `partner-signals.jsonl` and does NOT get mis-diverted to the dev-loop log.
    #[test]
    fn human_signal_still_lands_in_partner_signals() {
        let dir = std::env::temp_dir()
            .join(format!("spectyn-human-signal-{}", std::process::id()));
        let human = dir.join("partner-signals.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let _env = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &human);
        let dev_loop = dev_loop_log_path();

        // Explicit human marker AND the no-marker default (a coarse sensor check-in
        // carries no classifier markers) must both be treated as Human.
        let explicit = resolve_origin(MessageOrigin::from_wire("human"), "");
        assert_eq!(explicit, MessageOrigin::Human);
        record_signal_with_origin(explicit, "sensor", &json!({ "place": "home" }))
            .unwrap();

        let defaulted = resolve_origin(None, "at the gym");
        assert_eq!(defaulted, MessageOrigin::Human, "no-marker check-in defaults Human");
        record_signal_with_origin(defaulted, "sensor", &json!({ "activity": "gym" }))
            .unwrap();
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        let human_content = std::fs::read_to_string(&human)
            .expect("human signal must land in partner-signals.jsonl");
        let human_lines =
            human_content.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(human_lines, 2, "both human signals land in the moat: {human_content}");
        assert!(human_content.contains("home"), "{human_content}");
        assert!(human_content.contains("gym"), "{human_content}");

        // Nothing leaked into the dev-loop log for genuine human use.
        let dev_loop_lines = std::fs::read_to_string(&dev_loop)
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        assert_eq!(dev_loop_lines, 0, "human use must NOT be diverted to dev-loop-log");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 記東西 fast-path: intent detection (pure, no LLM/IO) ───────────────────

    #[test]
    fn detect_intent_chinese_record_prefix() {
        assert_eq!(
            detect_intent("記:買牛奶"),
            Intent::Record { body: "買牛奶".to_string() }
        );
        // `記下:` longest-prefix wins (not swallowed by `記`).
        assert_eq!(
            detect_intent("記下:買牛奶"),
            Intent::Record { body: "買牛奶".to_string() }
        );
        // Space form: `記 ` then the body.
        assert_eq!(
            detect_intent("記 打電話給媽媽"),
            Intent::Record { body: "打電話給媽媽".to_string() }
        );
    }

    #[test]
    fn detect_intent_english_record_prefix_case_insensitive() {
        assert_eq!(
            detect_intent("note: call dentist"),
            Intent::Record { body: "call dentist".to_string() }
        );
        // Case-insensitive on the English trigger.
        assert_eq!(
            detect_intent("NOTE: x"),
            Intent::Record { body: "x".to_string() }
        );
        assert_eq!(
            detect_intent("Note: buy milk"),
            Intent::Record { body: "buy milk".to_string() }
        );
    }

    #[test]
    fn detect_intent_general_is_ask() {
        assert_eq!(detect_intent("今天天氣如何"), Intent::Ask);
        assert_eq!(detect_intent("what's the capital of France?"), Intent::Ask);
        // A trigger with an empty body is NOT a record — let the agent handle it.
        assert_eq!(detect_intent("記:"), Intent::Ask);
        assert_eq!(detect_intent("note:"), Intent::Ask);
    }

    // ── 記東西 fast-path: record handler persists a recallable note (IO test) ──

    /// `handle_record` writing into a temp `$HOME`-derived base must be found by
    /// `recall::search_events` on that same base — proving the texted note is
    /// reliably recallable without an LLM. Serialised + `$HOME`-isolated; the
    /// EventStore key cache is process-global, so we install a fixed test key.
    #[ignore = "integration / env-dependent ($HOME + key cache) — run via --ignored"]
    #[test]
    fn record_path_note_is_recallable() {
        use crate::life_node::recall::{search_events, RecallFilter};

        // Crate-wide env lock so this $HOME-mutating test serialises against the
        // rest of the suite, not just sibling partner tests. Acquired first so it
        // drops LAST — after the in-body HOME restore at the end of this fn.
        let _env = crate::env_lock::acquire();
        static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        crate::encryption_wire::clear_event_key_cache();

        let base = spectyn_base_dir();
        std::fs::create_dir_all(&base).unwrap();

        // Drive the real record handler (same path handle_message takes).
        let reply = handle_record(&base, "買牛奶 and call the dentist").unwrap();
        assert_eq!(reply.turns, 0, "record path takes no LLM turn");
        assert!(reply.reply.starts_with("記下了:"), "confirmation: {}", reply.reply);

        // The note is recallable via the same store recall reads.
        let events = base.join("events");
        let hits = search_events(&events, None, &RecallFilter::text("dentist"), 15).unwrap();
        assert_eq!(hits.len(), 1, "the texted note is recallable: {hits:?}");
        assert!(hits[0].summary.contains("買牛奶"), "summary: {}", hits[0].summary);
        // Tagged "partner" so it is attributable to this entrypoint.
        let tagged = search_events(&events, None, &RecallFilter::text("partner"), 15).unwrap();
        assert_eq!(tagged.len(), 1, "note carries the partner tag");

        crate::encryption_wire::clear_event_key_cache();
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    // ── At-most-once ingress: a resent message must not execute twice ──────────

    /// Two identical Record-intent messages sharing a client key: the first runs
    /// (deduped=false), the second is suppressed (deduped=true, turns=0) and the
    /// note is captured only ONCE. The Record fast-path needs no LLM, so this
    /// proves the dedup rail end-to-end without a network call. Serialised +
    /// `$HOME`-isolated (note capture) + a temp idempotency/signals store.
    #[ignore = "integration / env-dependent ($HOME + key cache) — run via --ignored"]
    #[tokio::test]
    async fn idempotent_record_runs_once_then_dedups() {
        use crate::life_node::recall::{search_events, RecallFilter};

        let _env = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        crate::encryption_wire::clear_event_key_cache();
        let idem = tmp.path().join("idem.jsonl");
        let sigs = tmp.path().join("signals.jsonl");
        std::env::set_var("SPECTYN_IDEMPOTENCY_STORE", &idem);
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &sigs);

        let rt = AgentRuntime::default();
        let text = "記: 買牛奶 idempotency-e2e";
        let key = Some("ios-req-abc-123");

        let (first, dup1) = handle_message_idempotent(&rt, "master", text, key, MessageOrigin::Human)
            .await
            .unwrap();
        assert!(!dup1, "first message is not a duplicate");
        assert!(first.reply.starts_with("記下了:"), "first reply: {}", first.reply);

        let (second, dup2) = handle_message_idempotent(&rt, "master", text, key, MessageOrigin::Human)
            .await
            .unwrap();
        assert!(dup2, "the resent message IS a duplicate");
        assert_eq!(second.turns, 0, "deduped reply takes no turn");

        // The note was captured exactly once despite two ingress calls.
        let events = spectyn_base_dir().join("events");
        let hits = search_events(&events, None, &RecallFilter::text("idempotency-e2e"), 15).unwrap();
        assert_eq!(hits.len(), 1, "note captured once, not twice: {hits:?}");

        std::env::remove_var("SPECTYN_IDEMPOTENCY_STORE");
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");
        crate::encryption_wire::clear_event_key_cache();
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
