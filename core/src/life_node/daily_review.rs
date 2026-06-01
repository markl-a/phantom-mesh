//! Daily review aggregator — first slice of E003 Coach Node.
//!
//! Reads (EventMeta, AnalysisResult) pairs for a given date and produces
//! a Markdown brief. Pure formatting; no LLM call. The LLM-driven
//! "tomorrow-action planner" pass lives in a separate function (later slice).
//!
//! Spec: `docs/superpowers/specs/_current/E003-coach-node-daily-review.md`

use crate::event_storage_wire::EventMeta;
use crate::life_node::key_derivation::EventKey;
use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderError, ResponseFormat,
};
use crate::life_node::storage::EventStore;
use std::collections::BTreeMap;
use std::path::Path;

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

/// Scan an events directory (typically `~/.phantom-mesh/events/`) and
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
}

/// Run a full coach review for `date`, shared by the CLI (`phantom coach
/// review`) and the app's `daily_review_generate` command so the aggregate +
/// LLM "Tomorrow's one action" + save/encrypt logic stays in one place.
///
/// Aggregates the day's events, runs the shame-free lint (warns, never blocks),
/// then appends the Gemini-driven "Tomorrow's one action" — degrading
/// gracefully to a `(skipped: …)` footer when there's no `GEMINI_API_KEY` or
/// the call fails, so the review is always produced. When `save` is set, writes
/// to `~/.phantom-mesh/reviews/{date}.md`, age-encrypted with the identity key
/// when present (else plaintext, matching the events store).
pub async fn run_coach_review(
    home: &Path,
    date: &str,
    save: bool,
) -> anyhow::Result<CoachReviewResult> {
    let events_dir = home.join(".phantom-mesh/events");
    let identity_path = home.join(".phantom-mesh/identity.key");
    let event_key = crate::life_node::key_derivation::load_event_key(&identity_path).ok();
    let pairs = load_events_for_date(&events_dir, date, event_key)?;
    let event_count = pairs.len();
    let mut md = aggregate(date, &pairs);
    if let Err(e) = crate::life_node::coach_prompts::lint::check(&md) {
        eprintln!("warning: shame-free lint on review output: {}", e);
    }

    // Tomorrow-action second pass over a provider FALLBACK CHAIN (SPEC-23):
    // Gemini first, then Groq, so a rate-limited / unavailable primary provider
    // falls back instead of producing no action. Graceful footer note when no
    // provider key is configured so offline / cost-sensitive runs still work.
    let mut chain: Vec<Box<dyn MultimodalProvider>> = Vec::new();
    if let Ok(p) = crate::life_node::providers::gemini::GeminiMultimodalProvider::from_env() {
        chain.push(Box::new(p));
    }
    if let Ok(p) = crate::life_node::providers::groq::GroqTextProvider::from_env() {
        chain.push(Box::new(p));
    }
    if chain.is_empty() {
        md.push_str(
            "\n## Tomorrow's one action\n\n(skipped: no GEMINI_API_KEY / GROQ_API_KEY in env)\n",
        );
    } else {
        match propose_tomorrow_action_chain(&md, &chain).await {
            Ok(action) => md.push_str(&format!("\n## Tomorrow's one action\n\n{}\n", action)),
            Err(e) => md.push_str(&format!("\n## Tomorrow's one action\n\n(skipped: {})\n", e)),
        }
    }

    let mut saved_to = None;
    let mut saved_encrypted = false;
    if save {
        let reviews_dir = home.join(".phantom-mesh/reviews");
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
}
