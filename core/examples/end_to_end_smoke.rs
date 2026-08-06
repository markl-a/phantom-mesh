//! End-to-end smoke demo — the first full skillbank + remote-control closed-loop.
//!
//! Goal: prove the integration façade can drive a single request from
//! "message-arrives" → "curator scores work" → "skill extracted into FTS5
//! memory" → "next time the same intent shows up, recall is fast" without
//! any real network traffic.
//!
//! ## DEMO-2 + A8 update (over DEMO-1 PR #115 and DEMO-2 PR #157)
//!
//! DEMO-1 disclosed five wiring gaps. Three of those are still real and out
//! of scope (no `LlmProvider` trait, no remote-control inbound surface, no
//! `core::cluster ↔ SkillbankRuntime` bridge). The remaining two are wired:
//!
//! * **Gap 4 — hard-coded skill text.** DEMO-1 carried a `const
//!   EXTRACTED_SKILL_MD: &str = "..."`. DEMO-2 + A8 calls the real
//!   `skillbank::extract::extract_skill(&verdict, &checkpoint)` (A1 / PR #144
//!   + A8) to synthesise the skill from the verdict + checkpoint signals.
//! * **Gap 5 — second `SkillMemory` handle.** DEMO-1 had to open a fresh
//!   `SkillMemory::open_at(db_path)` because the runtime did not expose a
//!   `register_skill` helper. DEMO-2 + A8 calls
//!   `SkillbankRuntime::set_skill_extractor` + `judge_and_auto_register`
//!   (A2 / PR #146) — one handle, one call, one persisted row.
//!
//! **A8 specifically retires the `VerdictBackedExtractor` workaround.**
//! DEMO-2 had to install an adapter that disabled A1's score gate
//! (`extract_skill_with_threshold(_, _, u8::MAX)`) because A1 only emitted
//! on score < θ and A2 only invoked on score ≥ θ — the two never overlapped.
//! A8 makes A1's `extract_skill` polarity-aware (routes to success or
//! failure classifier based on score) so the adapter collapses to a thin
//! 3-line trait impl that just forwards `(checkpoint.judge_score, checkpoint)`
//! to `extract_skill`.
//!
//! Hot-path recall (step 7) goes through
//! `SkillbankRuntime::recall_context_for(prompt, k)` (A4 / PR #154).
//!
//! ## What is mocked
//! - **LLM provider.** The Curator only knows how to POST to an Anthropic-
//!   shaped `/v1/messages` endpoint, so we stand up a `wiremock` server on
//!   localhost and point the Curator at it. No `api.anthropic.com` traffic.
//! - **Telegram inbound.** `core::remote_control` exposes a send-only `Channel`
//!   trait. There is NO inbound polling / webhook surface in tree today,
//!   so the demo synthesises a `SimulatedInboundMessage` and feeds it
//!   straight into the dispatch helper below. No `api.telegram.org`
//!   traffic.
//! - **Worker dispatch.** `core::cluster` does not exist in tree (the
//!   `worker_caps` concept lives in `mesh.rs` but is not wired to
//!   skillbank). The demo invokes one of the skill tools (`skill_calculator`)
//!   in-process to stand in for "agent did work".
//! - **Disk.** FTS5 SQLite lives under `tempfile::tempdir()`; nothing
//!   persists past the run.
//!
//! ## What is NOT mocked
//! - `SkillbankRuntime::new` — runs the real seeding pipeline (30 tools + the
//!   sample skill) against the real FTS5 schema.
//! - `Curator::judge` — runs the real Anthropic request/response path
//!   (URL just points at wiremock instead of api.anthropic.com).
//! - `SkillMemory::insert` + `search` — real `rusqlite` + FTS5 BM25.
//! - `skillbank::extract::extract_skill` — real A1 + A8 polarity-aware
//!   extractor, fed real `JudgeVerdict` + `EvolveCheckpoint` signals.
//! - `SkillbankRuntime::judge_and_auto_register` — real A2 orchestration call.
//! - `SkillbankRuntime::recall_context_for` — real A4 FTS5 lookup.
//!
//! ## Run
//! ```
//! set CARGO_TARGET_DIR=C:/tmp/a8-target
//! cargo run --example end_to_end_smoke \
//!     --features experimental-skillbank,experimental-remote-control
//! ```
//!
//! Expected last line: `END-TO-END SMOKE COMPLETE`. Exit code 0.

use std::path::PathBuf;
use std::time::Instant;

use spectyn_mesh::evolve_checkpoint::{EvolveCheckpoint, JudgeVerdict};
use spectyn_mesh::skillbank::curator::{Curator, SkillExtractor, DEFAULT_JUDGE_MODEL};
use spectyn_mesh::skillbank::extract::extract_skill;
use spectyn_mesh::skillbank::skill::SkillDocument;
use spectyn_mesh::skillbank::tools::calculator::Calculator;
use spectyn_mesh::skillbank::tools::SkillTool;
use spectyn_mesh::skillbank::SkillbankRuntime;
use spectyn_mesh::remote_control::slack::SlackStub;
use spectyn_mesh::remote_control::Channel;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Simulated inbound message — stand-in for what a Telegram poller would
/// hand the dispatcher if one existed. Mirrors the minimum field set
/// (user, chat, text) the real remote-control track was scoped to deliver.
struct SimulatedInboundMessage<'a> {
    user_id: i64,
    chat_id: i64,
    text: &'a str,
    channel: &'static str,
}

/// Build a mock Anthropic /v1/messages reply that the Curator's parser
/// will accept. The Curator pulls the first `text` block out of `content`.
fn anthropic_text_response(text: &str) -> serde_json::Value {
    json!({
        "id": "msg_smoke",
        "type": "message",
        "role": "assistant",
        "model": DEFAULT_JUDGE_MODEL,
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 50, "output_tokens": 25}
    })
}

/// Thin adapter that plugs A1's `extract_skill` (which takes
/// `&JudgeVerdict + &EvolveCheckpoint`) into A2's `SkillExtractor` trait
/// (which takes `&EvolveCheckpoint + agent_output: &str`).
///
/// **A8 simplification.** DEMO-2 carried a `VerdictBackedExtractor` that
/// also had to disable A1's score gate (`extract_skill_with_threshold(_,
/// _, u8::MAX)`) because A1 historically only emitted on LOW scores while
/// A2 only invoked on HIGH scores. A8 makes A1 polarity-aware
/// (`extract_skill` itself now routes to the success-side or failure-side
/// classifier based on score), so the adapter is now a 3-line forward.
/// No threshold trick.
struct DemoSkillExtractor;

impl SkillExtractor for DemoSkillExtractor {
    fn extract_skill(
        &self,
        checkpoint: &EvolveCheckpoint,
        _agent_output: &str,
    ) -> Result<Option<SkillDocument>, String> {
        let verdict = checkpoint
            .judge_score
            .as_ref()
            .ok_or_else(|| "extractor invoked before judge stamped a verdict".to_string())?;
        Ok(extract_skill(verdict, checkpoint))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("==== spectyn-mesh end-to-end smoke (DEMO-2 + A8: A1↔A2 polarity aligned) ====");
    println!("(mocks: LLM=wiremock, Telegram=synthetic, worker=in-process)");
    println!();

    // ── [1/7] Mock LLM provider ─────────────────────────────────────────
    println!("[1/7] Setting up mock LLM provider (wiremock Anthropic /v1/messages)");
    let llm = MockServer::start().await;
    // Judge-shaped reply for the curator step. Score 8 — A1's success-side
    // classifier (A8) will fire because the checkpoint we send to
    // judge_and_auto_register has at least one successful step.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                r#"{"score": 8, "rationale": "agent summarised the PRs cleanly and persisted the skill"}"#,
            )),
        )
        .mount(&llm)
        .await;
    println!("      mock LLM ready at {}", llm.uri());
    println!();

    // ── [2/7] SkillbankRuntime boot ────────────────────────────────────────
    println!("[2/7] Initializing SkillbankRuntime (FTS5 memory + 30-tool catalog + sample skill)");
    let tmp = tempfile::tempdir()?;
    let db_path: PathBuf = tmp.path().join("skill-smoke.db");
    let boot_t0 = Instant::now();
    let mut runtime = SkillbankRuntime::new(db_path.clone()).await?;
    let boot_ms = boot_t0.elapsed().as_millis();
    runtime.set_curator(Curator {
        api_base: llm.uri(),
        api_key: "smoke-test-key".into(),
        model: DEFAULT_JUDGE_MODEL.into(),
        timeout_secs: 5,
    });
    // DEMO-2 (gap 5 closure) + A8: inject the polarity-aware A1 extractor
    // through A2's injection point. No threshold workaround — the adapter
    // is a straight forward to `extract_skill(verdict, checkpoint)`.
    runtime.set_skill_extractor(Box::new(DemoSkillExtractor));
    println!(
        "      runtime up in {} ms — db at {}",
        boot_ms,
        db_path.display()
    );
    println!("      A1 extractor wired into runtime via set_skill_extractor (real, no threshold workaround)");
    println!();

    // ── [3/7] Simulated remote-control inbound ──────────────────────────
    println!("[3/7] Receiving simulated Telegram-like message");
    let channel = SlackStub::with_allowed_users(vec![42, 1001]);
    let inbound = SimulatedInboundMessage {
        user_id: 42,
        chat_id: 9000,
        text: "summarize the last 3 PRs",
        channel: "telegram-simulated-via-slack-stub",
    };
    assert!(
        channel.is_user_allowed(inbound.user_id),
        "smoke pre-condition: user must be on the allowlist"
    );
    println!(
        "      {} → user_id={}, chat_id={}, text={:?}",
        inbound.channel, inbound.user_id, inbound.chat_id, inbound.text
    );
    println!();

    // ── [4/7] Dispatch: find a matching skill + tool, run the work ──────
    println!("[4/7] Dispatching to spectyn agent (in-process — no real cluster)");
    let prior = runtime.find_tool_by_intent(inbound.text).await?;
    println!(
        "      FTS5 tool lookup (cold): hits={}, top={:?}",
        prior.len(),
        prior.first()
    );
    let calc = Calculator;
    let work = calc.call(&json!({"expression": "3 + 0"})).await?;
    let agent_output_for_judge = format!(
        "ran skill_calculator → {} (PR-summary work simulated)",
        work
    );
    println!(
        "      worker_caps=[skill:summarize, tool:skill_calculator] → {}",
        work
    );
    println!();

    // ── [5/7] Pre-judge sanity: A1 extractor returns Some on either polarity ─
    println!("[5/7] Pre-flight: prove A1 `extract_skill` returns Some on BOTH polarities");
    // Failure-side: low score + populated checkpoint.
    let failure_verdict = JudgeVerdict {
        score: 2,
        rubric_version: spectyn_mesh::skillbank::RUBRIC_VERSION.to_string(),
        model: DEFAULT_JUDGE_MODEL.to_string(),
        rationale: "agent kept retrying the failing build".into(),
        judged_at_ms: 1_715_000_000_000,
    };
    let mut failure_ck =
        EvolveCheckpoint::new("restore green build", "check", "a8-preflight-failure");
    failure_ck.append_step("cargo check", Some("cargo".into()), false);
    failure_ck.append_step("cargo check", Some("cargo".into()), false);
    failure_ck.record_dead_end("missing import", "import exists but feature gated");
    let failure_skill = extract_skill(&failure_verdict, &failure_ck)
        .expect("A1 must emit a skill on a populated low-score case");
    println!(
        "      skill auto-extracted (real, failure-side) — name={} tags={:?}",
        failure_skill.frontmatter.name, failure_skill.frontmatter.tags
    );
    // Success-side: high score + successful step.
    let success_verdict = JudgeVerdict {
        score: 9,
        rubric_version: spectyn_mesh::skillbank::RUBRIC_VERSION.to_string(),
        model: DEFAULT_JUDGE_MODEL.to_string(),
        rationale: "shipped clean fix, tests green".into(),
        judged_at_ms: 1_715_000_000_000,
    };
    let mut success_ck = EvolveCheckpoint::new("ship a fix", "check", "a8-preflight-success");
    success_ck.append_step("applied the patch", Some("apply_patch".into()), true);
    let success_skill = extract_skill(&success_verdict, &success_ck)
        .expect("A1 must emit a skill on a populated high-score case (A8 success path)");
    println!(
        "      skill auto-extracted (real, success-side) — name={} tags={:?}",
        success_skill.frontmatter.name, success_skill.frontmatter.tags
    );
    println!();

    // ── [6/7] Curator judges + auto-registers in ONE call ───────────────
    println!("[6/7] SkillbankRuntime.judge_and_auto_register (A2) — judge + register, one call");
    let mut checkpoint = EvolveCheckpoint::new(inbound.text, "check", "end-to-end-smoke-node");
    // Seed at least one successful step so A1's success-side classifier
    // (the path A2 will route the mocked score=8 verdict through) has a
    // signal to latch onto. The agent output above mirrors what an LLM-
    // driven loop would record as the calculator returns.
    checkpoint.append_step(
        "ran skill_calculator and produced the PR summary",
        Some("skill_calculator".into()),
        true,
    );
    let skills_before = runtime
        .recall_context_for("auto-extracted", 50)
        .await?
        .len();
    let judge_t0 = Instant::now();
    let (verdict, registered_name) = runtime
        .judge_and_auto_register(&mut checkpoint, &agent_output_for_judge)
        .await?;
    let judge_ms = judge_t0.elapsed().as_millis();
    let normalized = verdict.score as f64 / 10.0;
    println!(
        "      verdict: score={}/10 (normalised={:.2}) rubric={} in {} ms",
        verdict.score, normalized, verdict.rubric_version, judge_ms
    );
    println!("      rationale: {}", verdict.rationale);
    let registered = registered_name.as_deref().expect(
        "A8 invariant: high-score path with polarity-aware extractor must register a skill",
    );
    println!(
        "      auto-registered (real) — skill name={} (single runtime, single memory handle)",
        registered
    );
    let recalled = runtime.recall_verdicts_for(&checkpoint.session_id).await?;
    assert_eq!(
        recalled.len(),
        1,
        "smoke invariant: verdict must be recallable by session_id"
    );
    println!(
        "      verdict recallable by session_id={} (rows={})",
        checkpoint.session_id,
        recalled.len()
    );
    let skills_after = runtime
        .recall_context_for("auto-extracted", 50)
        .await?
        .len();
    assert!(
        skills_after > skills_before,
        "smoke invariant: auto-register must add at least one row (before={skills_before}, after={skills_after})"
    );
    println!(
        "      auto-extracted memory rows: {} → {} (Δ={})",
        skills_before,
        skills_after,
        skills_after - skills_before
    );
    println!();

    // ── [7/7] Hot-path replay via A4 recall_context_for ─────────────────
    println!(
        "[7/7] SkillbankRuntime.recall_context_for (A4) — freshly-registered skill is recallable"
    );
    let warm_t0 = Instant::now();
    // The success-side skill body contains tokens like "Recipe",
    // "Successful steps", "replay", etc. We probe on one of those so the
    // recall surfaces the just-registered row independent of its exact name.
    let warm_hits = runtime
        .recall_context_for("replay successful steps recipe", 5)
        .await?;
    let warm_us = warm_t0.elapsed().as_micros();
    assert!(
        !warm_hits.is_empty(),
        "A8 invariant: A4 recall_context_for must surface the freshly-extracted skill; got {:?}",
        warm_hits
    );
    println!(
        "      recalled via FTS5 (real) — {} hit(s) in {} µs",
        warm_hits.len(),
        warm_us
    );
    println!(
        "      first match (≤{} chars): {}",
        spectyn_mesh::skillbank::MEMORY_ROW_CHAR_CAP,
        warm_hits[0]
    );
    let reply_err = channel
        .send_message(inbound.chat_id, "PR summary: ...")
        .await
        .expect_err("smoke invariant: stub channel must reject sends");
    println!(
        "      reply-channel contract holds (stub returned: {})",
        reply_err
    );
    println!();

    println!("END-TO-END SMOKE COMPLETE");
    println!("(A8: A1+A2 polarity aligned; VerdictBackedExtractor workaround retired)");
    Ok(())
}
