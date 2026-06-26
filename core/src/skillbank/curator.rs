//! H1: Curator + LLM-as-judge.
//!
//! See docs/superpowers/specs/2026-05-15-weekend-multi-agent-push-design.md §4 H1.
//!
//! The Curator takes a completed (or in-progress) `EvolveCheckpoint` and asks
//! a small Claude model to score the agent's work against a fixed rubric.
//! The verdict is persisted onto the checkpoint via `record_judge_verdict`.
//!
//! LLM-as-judge scoring over an internal `EvolveCheckpoint`.

use crate::evolve_checkpoint::{EvolveCheckpoint, JudgeVerdict};
use crate::skillbank::skill::SkillDocument;

/// Frozen rubric id. Bumped (e.g. "h1-v2") only if the prompt or scoring
/// scale changes in a way that makes old verdicts non-comparable.
pub const RUBRIC_VERSION: &str = "h1-v1";

/// Default judge model — small + fast + cheap; we are scoring, not generating.
pub const DEFAULT_JUDGE_MODEL: &str = "claude-haiku-4-5-20251001";

/// Maximum characters the rationale we persist may contain. Anthropic
/// completions can be verbose; we don't need a novel — a 2 sentence reason.
pub const MAX_RATIONALE_CHARS: usize = 800;

/// A2/T92: confidence threshold (on the normalized 0.0..1.0 scale) that
/// historically gated `judge_and_maybe_extract`. The 0..=10 rubric score
/// is divided by 10 before comparison, so 0.5 means "score >= 5 on the
/// 0..=10 rubric".
///
/// **A8 (PR-aligned with A1):** the gate is no longer applied here.
/// A1's [`crate::skillbank::extract::extract_skill`] now routes the verdict
/// internally (score ≥ θ → success-side extractor; score < θ → failure-side
/// extractor) and returns `None` when neither side fits. A2 therefore calls
/// the extractor unconditionally and trusts A1's routing.
///
/// The constant is **retained** so:
///   - Existing public API consumers don't break (the symbol is re-exported
///     from `skillbank::mod`).
///   - Future calibration work (e.g., per-task thresholds) has an anchor.
pub const CONFIDENCE_THRESHOLD: f32 = 0.5;

/// A2/T92: pluggable skill extractor used by `judge_and_maybe_extract`.
///
/// The real implementation lives in `skillbank::extract` once A1 lands.
/// Until then, the runtime composes a `NoopSkillExtractor` so the orchestration
/// path is exercised end-to-end without the extractor doing anything. Tests
/// inject `MockSkillExtractor` to assert which inputs the extractor saw.
///
/// `Send + Sync` so the trait object can live inside an `Arc` and be cloned
/// across worker tasks. Extraction is intentionally synchronous: the current
/// (A1) plan does the heavy lifting at the model layer (which is async via
/// the Curator); the extractor's job is the pure-data step of turning agent
/// transcript + verdict into a `SkillDocument`.
pub trait SkillExtractor: Send + Sync {
    /// Try to distil `agent_output` (raw transcript text) into a `SkillDocument`.
    ///
    /// Returns `Ok(None)` to mean "nothing worth registering"; returns `Err`
    /// only for hard failures (malformed input the extractor couldn't tolerate).
    /// The default A2 wiring treats `Err` and `Ok(None)` identically — no
    /// skill is written — so implementations may choose either signal.
    fn extract_skill(
        &self,
        checkpoint: &EvolveCheckpoint,
        agent_output: &str,
    ) -> Result<Option<SkillDocument>, String>;
}

/// A2/T92: default extractor when A1 has not landed (or is feature-disabled).
/// Always returns `Ok(None)` so the auto-register path is a no-op.
pub struct NoopSkillExtractor;

impl SkillExtractor for NoopSkillExtractor {
    fn extract_skill(
        &self,
        _checkpoint: &EvolveCheckpoint,
        _agent_output: &str,
    ) -> Result<Option<SkillDocument>, String> {
        Ok(None)
    }
}

/// Build the user prompt the judge sees. Pure function — no I/O. Easy to
/// snapshot-test.
pub fn build_judge_user_prompt(checkpoint: &EvolveCheckpoint) -> String {
    let mut s = String::new();
    s.push_str("You are scoring an autonomous agent's work on a development task.\n\n");
    s.push_str("RUBRIC (rubric_version=");
    s.push_str(RUBRIC_VERSION);
    s.push_str("):\n");
    s.push_str("  10 = goal fully achieved, tests green, no dead-ends wasted\n");
    s.push_str("   8 = goal achieved with minor inefficiency or one dead-end\n");
    s.push_str("   6 = goal partially achieved, real progress, more rounds needed\n");
    s.push_str("   4 = some progress but agent got stuck on a hypothesis\n");
    s.push_str("   2 = minimal/no useful progress\n");
    s.push_str("   0 = wrong direction or destructive\n\n");
    s.push_str("Reply with EXACTLY one JSON object on a single line, no prose, no code fences:\n");
    s.push_str("  {\"score\": <0-10 integer>, \"rationale\": \"<one short sentence>\"}\n\n");
    s.push_str("--- evolve session timeline ---\n");
    s.push_str(&checkpoint.render_markdown());
    s
}

/// Parse the judge's reply into a numeric score + rationale. Tolerates
/// the model wrapping the JSON in ```json ... ``` fences or leading/trailing prose.
pub fn parse_judge_reply(raw: &str) -> Result<(u8, String), String> {
    // Strip common code-fence wrappers.
    let stripped = raw.trim();
    let stripped = stripped.strip_prefix("```json").unwrap_or(stripped);
    let stripped = stripped.strip_prefix("```").unwrap_or(stripped);
    let stripped = stripped.strip_suffix("```").unwrap_or(stripped);

    // Find the first '{' and last '}' — the substring between is our candidate.
    let start = stripped
        .find('{')
        .ok_or_else(|| "no '{' in reply".to_string())?;
    let end = stripped
        .rfind('}')
        .ok_or_else(|| "no '}' in reply".to_string())?;
    if end < start {
        return Err("malformed braces".into());
    }
    let json_str = &stripped[start..=end];

    let v: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("not valid json: {} (input: {})", e, json_str))?;

    let score_raw = v
        .get("score")
        .ok_or_else(|| "missing 'score' field".to_string())?;
    let score_i64 = score_raw
        .as_i64()
        .ok_or_else(|| "'score' is not an integer".to_string())?;
    if !(0..=10).contains(&score_i64) {
        return Err(format!("'score' {} out of range 0..=10", score_i64));
    }
    let score = score_i64 as u8;

    let rationale = v
        .get("rationale")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let rationale = truncate_chars(&rationale, MAX_RATIONALE_CHARS);

    Ok((score, rationale))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// V2 (T28): strict JSON-schema parser. Same fence/prose tolerance as
/// `parse_judge_reply`, then enforces:
///   - object shape `{score, rationale}` with NO other fields
///   - score is an integer in 0..=10 (rejects floats, strings, missing)
///   - rationale is a string (rejects missing/null/non-string)
///
/// Returns the same `(u8, String)` tuple as `parse_judge_reply` so callers can
/// swap parsers without restructuring downstream code.
pub fn parse_judge_reply_strict(raw: &str) -> Result<(u8, String), String> {
    let stripped = raw.trim();
    let stripped = stripped.strip_prefix("```json").unwrap_or(stripped);
    let stripped = stripped.strip_prefix("```").unwrap_or(stripped);
    let stripped = stripped.strip_suffix("```").unwrap_or(stripped);

    let start = stripped
        .find('{')
        .ok_or_else(|| "no '{' in reply".to_string())?;
    let end = stripped
        .rfind('}')
        .ok_or_else(|| "no '}' in reply".to_string())?;
    if end < start {
        return Err("malformed braces".into());
    }
    let json_str = &stripped[start..=end];

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictVerdict {
        score: i64,
        rationale: String,
    }

    let v: StrictVerdict = serde_json::from_str(json_str)
        .map_err(|e| format!("schema violation: {} (input: {})", e, json_str))?;

    if !(0..=10).contains(&v.score) {
        return Err(format!("'score' {} out of range 0..=10", v.score));
    }

    let rationale = truncate_chars(&v.rationale, MAX_RATIONALE_CHARS);
    Ok((v.score as u8, rationale))
}

/// Construct a fully-formed `JudgeVerdict` ready to feed to
/// `EvolveCheckpoint::record_judge_verdict`.
pub fn verdict_from_parsed(
    score: u8,
    rationale: String,
    model: String,
    judged_at_ms: i64,
) -> JudgeVerdict {
    JudgeVerdict {
        score,
        rubric_version: RUBRIC_VERSION.to_string(),
        model,
        rationale,
        judged_at_ms,
    }
}

/// HTTP-talking judge. Holds Anthropic API base + key + model + timeout.
/// One Curator instance per `phantom evolve --judge` invocation.
pub struct Curator {
    /// Base URL, e.g. "https://api.anthropic.com" or a wiremock URI in tests.
    pub api_base: String,
    /// Anthropic API key (NEVER logged).
    pub api_key: String,
    /// Canonical model id; `DEFAULT_JUDGE_MODEL` recommended for cost.
    pub model: String,
    /// HTTP timeout per request (seconds).
    pub timeout_secs: u64,
}

impl Curator {
    /// Score the supplied checkpoint via a real Anthropic round-trip.
    /// On success, persists a `JudgeVerdict` onto the checkpoint and returns Ok(()).
    /// On any HTTP / parse failure, returns an Err and the checkpoint is unmodified.
    /// Caller is responsible for calling `checkpoint.save()` afterwards.
    pub async fn judge(
        &self,
        checkpoint: &mut EvolveCheckpoint,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let user_prompt = build_judge_user_prompt(checkpoint);
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 256,
            "messages": [{"role": "user", "content": user_prompt}]
        });

        let url = format!("{}/v1/messages", self.api_base.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()?;

        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            // Read body for diag — but never echo the api key in logs.
            let txt = resp.text().await.unwrap_or_default();
            return Err(format!(
                "anthropic returned status {}: {}",
                status,
                truncate_chars(&txt, 200)
            )
            .into());
        }

        let value: serde_json::Value = resp.json().await?;
        // Extract the text block from the messages-shaped reply.
        let text = value
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|b| {
                    let t = b.get("type")?.as_str()?;
                    if t == "text" {
                        b.get("text")?.as_str()
                    } else {
                        None
                    }
                })
            })
            .ok_or("no text block in anthropic reply")?
            .to_string();

        let (score, rationale) =
            parse_judge_reply(&text).map_err(|e| format!("parse_judge_reply: {}", e))?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        checkpoint.record_judge_verdict(verdict_from_parsed(
            score,
            rationale,
            self.model.clone(),
            now_ms,
        ));
        Ok(())
    }

    /// A2/T92 (A8-revised): judge `checkpoint` and ask the supplied
    /// extractor to distil the agent's output into a `SkillDocument`.
    ///
    /// **A8 polarity alignment.** The original (PR #146) implementation
    /// gated extractor invocation on `score / 10.0 >= CONFIDENCE_THRESHOLD`.
    /// That gate is **removed** because A1 (PR #144, expanded in A8) now
    /// owns the polarity decision: it routes high-score verdicts to the
    /// success-side classifier and low-score verdicts to the failure-side
    /// classifier internally. A2's job is pure orchestration: call the
    /// extractor on every verdict, let A1 decide whether to emit, return
    /// the result.
    ///
    /// This is a pure orchestration layer:
    ///   - calls `self.judge()` (which writes the verdict onto `checkpoint`),
    ///   - reads the verdict back,
    ///   - calls `extractor.extract_skill(checkpoint, agent_output)`,
    ///   - returns `(verdict, maybe_skill)`.
    ///
    /// The CALLER is responsible for actually inserting the returned
    /// `SkillDocument` into FTS5 memory (with the idempotency probe at
    /// `integration.rs:99-109`). Splitting "extract" from "register" keeps
    /// this function feature-flag agnostic — `experimental-memory`
    /// is not required to compile or call it.
    ///
    /// `Err` is returned only if the judge HTTP round-trip fails. Extractor
    /// failures are swallowed (logged via debug only) so a flaky extractor
    /// cannot prevent verdict recording — the verdict on the checkpoint is
    /// the canonical durable artifact.
    pub async fn judge_and_maybe_extract<E: SkillExtractor + ?Sized>(
        &self,
        checkpoint: &mut EvolveCheckpoint,
        agent_output: &str,
        extractor: &E,
    ) -> Result<(JudgeVerdict, Option<SkillDocument>), Box<dyn std::error::Error + Send + Sync>>
    {
        self.judge(checkpoint).await?;
        let verdict = checkpoint
            .judge_score
            .as_ref()
            .ok_or("judge() returned ok but no verdict was recorded on checkpoint")?
            .clone();

        // A8: no score gate here. The extractor is the authority on polarity.
        let maybe_skill = match extractor.extract_skill(checkpoint, agent_output) {
            Ok(s) => s,
            Err(_e) => None, // see doc comment — extractor failures are non-fatal
        };

        Ok((verdict, maybe_skill))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolve_checkpoint::EvolveCheckpoint;

    fn fixture_checkpoint() -> EvolveCheckpoint {
        EvolveCheckpoint::new("fix the lint", "check", "test-node")
    }

    #[test]
    fn build_judge_user_prompt_contains_rubric_and_timeline() {
        let c = fixture_checkpoint();
        let prompt = build_judge_user_prompt(&c);
        assert!(prompt.contains("RUBRIC (rubric_version=h1-v1)"));
        assert!(prompt.contains("evolve session"));
        assert!(prompt.contains(&c.session_id));
    }

    #[test]
    fn parse_judge_reply_accepts_clean_json() {
        let raw = r#"{"score": 7, "rationale": "ok progress"}"#;
        let (score, rationale) = parse_judge_reply(raw).unwrap();
        assert_eq!(score, 7);
        assert_eq!(rationale, "ok progress");
    }

    #[test]
    fn parse_judge_reply_strips_code_fences() {
        let raw = "```json\n{\"score\": 5, \"rationale\": \"meh\"}\n```";
        let (score, rationale) = parse_judge_reply(raw).unwrap();
        assert_eq!(score, 5);
        assert_eq!(rationale, "meh");
    }

    #[test]
    fn parse_judge_reply_tolerates_leading_prose() {
        let raw =
            "Sure, here's the verdict:\n{\"score\": 9, \"rationale\": \"clean fix\"}\nThanks!";
        let (score, rationale) = parse_judge_reply(raw).unwrap();
        assert_eq!(score, 9);
        assert_eq!(rationale, "clean fix");
    }

    #[test]
    fn parse_judge_reply_rejects_out_of_range() {
        let raw = r#"{"score": 11, "rationale": "x"}"#;
        let err = parse_judge_reply(raw).unwrap_err();
        assert!(err.contains("out of range"), "got: {}", err);
    }

    #[test]
    fn parse_judge_reply_rejects_non_integer_score() {
        let raw = r#"{"score": "high", "rationale": "x"}"#;
        let err = parse_judge_reply(raw).unwrap_err();
        assert!(err.contains("not an integer"), "got: {}", err);
    }

    #[test]
    fn parse_judge_reply_rejects_no_json_at_all() {
        let raw = "the agent did fine I guess";
        let err = parse_judge_reply(raw).unwrap_err();
        assert!(err.contains("no '{'"), "got: {}", err);
    }

    #[test]
    fn parse_judge_reply_truncates_long_rationale() {
        let long = "x".repeat(MAX_RATIONALE_CHARS + 100);
        let raw = format!(r#"{{"score": 5, "rationale": "{}"}}"#, long);
        let (_, rationale) = parse_judge_reply(&raw).unwrap();
        // truncate_chars adds a single '…' marker
        assert_eq!(rationale.chars().count(), MAX_RATIONALE_CHARS + 1);
        assert!(rationale.ends_with('…'));
    }

    #[test]
    fn verdict_from_parsed_freezes_rubric_version() {
        let v = verdict_from_parsed(8, "good".into(), "claude-haiku-4-5-20251001".into(), 12345);
        assert_eq!(v.rubric_version, "h1-v1");
        assert_eq!(v.model, "claude-haiku-4-5-20251001");
        assert_eq!(v.judged_at_ms, 12345);
    }

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a mock Anthropic /v1/messages response that contains the canonical
    /// reply shape: {content: [{type: "text", text: "..."}]}
    fn anthropic_text_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": DEFAULT_JUDGE_MODEL,
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 100, "output_tokens": 20}
        })
    }

    #[tokio::test]
    async fn judge_round_trip_writes_score_into_checkpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key-123"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 8, "rationale": "good"}"#,
                )),
            )
            .mount(&server)
            .await;

        let curator = Curator {
            api_base: server.uri(),
            api_key: "test-key-123".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 10,
        };

        let mut c = EvolveCheckpoint::new("test goal", "check", "test-node");
        curator.judge(&mut c).await.expect("judge should succeed");

        let v = c.judge_score.as_ref().expect("verdict must be persisted");
        assert_eq!(v.score, 8);
        assert_eq!(v.rationale, "good");
        assert_eq!(v.rubric_version, "h1-v1");
        assert_eq!(v.model, DEFAULT_JUDGE_MODEL);
        assert!(v.judged_at_ms > 0);
    }

    #[tokio::test]
    async fn judge_returns_error_on_http_500_and_does_not_record_verdict() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let curator = Curator {
            api_base: server.uri(),
            api_key: "test-key-123".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 10,
        };
        let mut c = EvolveCheckpoint::new("g", "check", "n");
        let err = curator.judge(&mut c).await.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("500") || msg.to_lowercase().contains("status"),
            "error should mention status: {}",
            msg
        );
        assert!(c.judge_score.is_none(), "no verdict on failed call");
    }

    #[tokio::test]
    async fn judge_returns_error_on_unparseable_reply_and_does_not_record_verdict() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    "the model wrote prose instead of json",
                )),
            )
            .mount(&server)
            .await;

        let curator = Curator {
            api_base: server.uri(),
            api_key: "test-key-123".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 10,
        };
        let mut c = EvolveCheckpoint::new("g", "check", "n");
        let err = curator.judge(&mut c).await.unwrap_err();
        assert!(format!("{}", err).contains("no '{'"));
        assert!(c.judge_score.is_none());
    }

    // ─── V2 (T28): strict parser ─────────────────────────────────────────

    #[test]
    fn strict_parser_accepts_clean_minimal_json() {
        let raw = r#"{"score": 7, "rationale": "ok"}"#;
        let (score, rationale) = parse_judge_reply_strict(raw).unwrap();
        assert_eq!(score, 7);
        assert_eq!(rationale, "ok");
    }

    #[test]
    fn strict_parser_rejects_missing_rationale() {
        let raw = r#"{"score": 7}"#;
        let err = parse_judge_reply_strict(raw).unwrap_err();
        assert!(
            err.contains("rationale"),
            "expected rationale-missing error, got: {}",
            err
        );
    }

    #[test]
    fn strict_parser_rejects_unknown_top_level_field() {
        let raw = r#"{"score": 7, "rationale": "x", "verdict_color": "green"}"#;
        let err = parse_judge_reply_strict(raw).unwrap_err();
        assert!(
            err.contains("unknown field") || err.contains("verdict_color"),
            "expected unknown-field rejection, got: {}",
            err
        );
    }

    #[test]
    fn strict_parser_rejects_score_as_float() {
        let raw = r#"{"score": 7.5, "rationale": "x"}"#;
        let err = parse_judge_reply_strict(raw).unwrap_err();
        assert!(
            err.to_lowercase().contains("integer") || err.contains("invalid type"),
            "expected integer-only rejection, got: {}",
            err
        );
    }

    #[test]
    fn strict_parser_still_strips_code_fences() {
        // The V1 code-fence tolerance is preserved — only the *post-fence* JSON
        // body is validated strictly. This is what the judge models actually emit.
        let raw = "```json\n{\"score\": 5, \"rationale\": \"meh\"}\n```";
        let (score, rationale) = parse_judge_reply_strict(raw).unwrap();
        assert_eq!(score, 5);
        assert_eq!(rationale, "meh");
    }

    // ─── A2/T92: judge_and_maybe_extract ─────────────────────────────────
    mod auto_register {
        use super::*;
        use crate::skillbank::skill::{SkillDocument, SkillFrontmatter};
        use std::collections::BTreeMap;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        /// Test extractor that records how many times it was called and
        /// returns a canned `Option<SkillDocument>`.
        struct MockExtractor {
            call_count: Arc<AtomicUsize>,
            ret: Option<SkillDocument>,
        }

        impl SkillExtractor for MockExtractor {
            fn extract_skill(
                &self,
                _checkpoint: &EvolveCheckpoint,
                _agent_output: &str,
            ) -> Result<Option<SkillDocument>, String> {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Ok(self.ret.clone())
            }
        }

        fn fake_skill(name: &str) -> SkillDocument {
            SkillDocument {
                frontmatter: SkillFrontmatter {
                    name: name.into(),
                    version: "0.1.0".into(),
                    description: "extracted by test".into(),
                    triggers: vec!["test trigger".into()],
                    tools: vec![],
                    inputs: BTreeMap::new(),
                    outputs: vec![],
                    tags: vec![],
                    created_at: None,
                    author: None,
                },
                body: "extracted body\n".into(),
            }
        }

        fn mock_curator(uri: String) -> Curator {
            Curator {
                api_base: uri,
                api_key: "k".into(),
                model: DEFAULT_JUDGE_MODEL.into(),
                timeout_secs: 5,
            }
        }

        #[tokio::test]
        async fn high_score_calls_extractor_and_returns_skill() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                        r#"{"score": 8, "rationale": "good"}"#,
                    )),
                )
                .mount(&server)
                .await;

            let count = Arc::new(AtomicUsize::new(0));
            let ext = MockExtractor {
                call_count: count.clone(),
                ret: Some(fake_skill("learned-skill")),
            };
            let curator = mock_curator(server.uri());
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, skill) = curator
                .judge_and_maybe_extract(&mut c, "agent transcript", &ext)
                .await
                .expect("ok");

            assert_eq!(verdict.score, 8);
            assert_eq!(
                count.load(Ordering::SeqCst),
                1,
                "extractor must be invoked once"
            );
            assert!(
                skill.is_some(),
                "high-score path must surface the extracted skill"
            );
            assert_eq!(skill.unwrap().frontmatter.name, "learned-skill");
        }

        #[tokio::test]
        async fn low_score_still_invokes_extractor_a8_a1_owns_polarity() {
            // A8: the old gate (score < CONFIDENCE_THRESHOLD ⇒ skip extractor)
            // is removed. A1's extract_skill now routes the failure-side
            // classifier internally, so A2 always calls the extractor and
            // trusts whatever the extractor returns. A mock that hands back
            // Some(_) thus surfaces a skill even on a low score.
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                        r#"{"score": 3, "rationale": "stuck"}"#,
                    )),
                )
                .mount(&server)
                .await;

            let count = Arc::new(AtomicUsize::new(0));
            let ext = MockExtractor {
                call_count: count.clone(),
                ret: Some(fake_skill("low-score-lesson")),
            };
            let curator = mock_curator(server.uri());
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, skill) = curator
                .judge_and_maybe_extract(&mut c, "agent transcript", &ext)
                .await
                .expect("ok");

            assert_eq!(verdict.score, 3);
            assert_eq!(
                count.load(Ordering::SeqCst),
                1,
                "A8: extractor MUST run regardless of score; A1 decides polarity"
            );
            assert!(
                skill.is_some(),
                "low-score path now surfaces failure-side skill if extractor returns Some"
            );
            assert_eq!(skill.unwrap().frontmatter.name, "low-score-lesson");
        }

        #[tokio::test]
        async fn boundary_score_at_threshold_calls_extractor() {
            // A8: invocation is unconditional, so threshold semantics shift
            // from "did A2 invoke?" to "did A1 route to success-or-failure?".
            // From A2's POV we only assert: invocation happened, returned None
            // ⇒ no skill propagated.
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                        r#"{"score": 5, "rationale": "ok"}"#,
                    )),
                )
                .mount(&server)
                .await;

            let count = Arc::new(AtomicUsize::new(0));
            let ext = MockExtractor {
                call_count: count.clone(),
                ret: None, // extractor decides nothing was worth saving
            };
            let curator = mock_curator(server.uri());
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (_v, skill) = curator
                .judge_and_maybe_extract(&mut c, "x", &ext)
                .await
                .expect("ok");
            assert_eq!(
                count.load(Ordering::SeqCst),
                1,
                "extractor invoked unconditionally"
            );
            assert!(
                skill.is_none(),
                "extractor returned None ⇒ no skill propagated"
            );
        }

        #[tokio::test]
        async fn extractor_error_is_swallowed_and_verdict_still_returned() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                        r#"{"score": 9, "rationale": "great"}"#,
                    )),
                )
                .mount(&server)
                .await;

            struct FlakyExtractor;
            impl SkillExtractor for FlakyExtractor {
                fn extract_skill(
                    &self,
                    _: &EvolveCheckpoint,
                    _: &str,
                ) -> Result<Option<SkillDocument>, String> {
                    Err("extractor boom".into())
                }
            }

            let curator = mock_curator(server.uri());
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, skill) = curator
                .judge_and_maybe_extract(&mut c, "x", &FlakyExtractor)
                .await
                .expect("verdict path must not propagate extractor failure");
            assert_eq!(verdict.score, 9);
            assert!(
                skill.is_none(),
                "flaky extractor ⇒ no skill but verdict still written"
            );
        }

        #[tokio::test]
        async fn judge_http_failure_propagates_and_skips_extractor() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
                .mount(&server)
                .await;

            let count = Arc::new(AtomicUsize::new(0));
            let ext = MockExtractor {
                call_count: count.clone(),
                ret: Some(fake_skill("never")),
            };
            let curator = mock_curator(server.uri());
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let err = curator
                .judge_and_maybe_extract(&mut c, "x", &ext)
                .await
                .unwrap_err();
            assert!(
                format!("{}", err).to_lowercase().contains("500")
                    || format!("{}", err).to_lowercase().contains("status")
            );
            assert_eq!(
                count.load(Ordering::SeqCst),
                0,
                "extractor must not run when judge fails"
            );
            assert!(c.judge_score.is_none(), "no verdict on failed judge");
        }

        #[test]
        fn noop_extractor_returns_none() {
            let ext = NoopSkillExtractor;
            let c = EvolveCheckpoint::new("g", "check", "n");
            let r = ext
                .extract_skill(&c, "anything")
                .expect("noop never errors");
            assert!(r.is_none());
        }
    }

    #[tokio::test]
    async fn judge_request_body_includes_user_prompt_with_timeline() {
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        // Match on the body containing `model` + a messages array shape.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(
                serde_json::json!({"model": DEFAULT_JUDGE_MODEL}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(anthropic_text_response(r#"{"score": 4, "rationale": "x"}"#)),
            )
            .mount(&server)
            .await;

        let curator = Curator {
            api_base: server.uri(),
            api_key: "k".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 5,
        };
        let mut c = EvolveCheckpoint::new("a goal that should appear in the prompt", "check", "n");
        curator.judge(&mut c).await.expect("ok");
        assert_eq!(c.judge_score.as_ref().unwrap().score, 4);
    }
}
