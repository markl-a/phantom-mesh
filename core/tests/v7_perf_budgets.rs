//! Phase E §V7 — pure-CPU wall-clock perf budgets.
//!
//! Order-of-magnitude regression guard. Budgets are intentionally 5-10× looser
//! than the SPEC target so CI noise (loaded runners, debug builds, cold caches)
//! cannot fail the suite spuriously. On failure the printed measurement is the
//! diagnostic. No network, no filesystem, no async.

use std::time::Instant;

fn assert_budget(label: &str, avg_ns: u128, budget_ns: u128, iters: u32) {
    assert!(
        avg_ns < budget_ns,
        "{label} avg = {avg_ns}ns over {iters} iters (budget: < {budget_ns}ns)"
    );
    println!("{label} avg = {avg_ns}ns / iter ({iters} iters)");
}

// V7.1 — SPEC-10 HMAC sign + verify ≤ 1ms avg per op.
#[test]
fn v7_hmac_sign_verify_under_1ms_avg() {
    use phantom_mesh::rpc_wire::{sign_hmac, verify_hmac};
    let secret: &[u8] = b"perf-budget-cluster-secret-32-by";
    assert_eq!(secret.len(), 32);
    let canonical = "GET\n/rpc/ping\n\nXXX\n";
    const ITERS: u32 = 10_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let sig = sign_hmac(secret, canonical);
        let _ = verify_hmac(secret, canonical, &sig);
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.1 hmac sign+verify", avg_ns, 1_000_000, ITERS);
}

// V7.2 — SPEC-11 mdns TXT parse ≤ 100µs avg.
#[test]
fn v7_mdns_parse_txt_under_100us_avg() {
    use phantom_mesh::mdns_wire::parse_txt_records;
    let raw: Vec<(String, String)> = vec![
        ("v".into(), "1".into()),
        ("pf".into(), "deadbeef".into()),
        ("cl".into(), "0123456789abcdef".into()),
        ("ca".into(), "role-coder,cargo,git".into()),
        ("os".into(), "mac".into()),
        ("na".into(), "perf-budget-peer".into()),
    ];
    const ITERS: u32 = 1_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let _ = parse_txt_records(&raw).expect("perf-budget fixture must parse");
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.2 mdns parse_txt_records", avg_ns, 100_000, ITERS);
}

// V7.3 — SPEC-12 identity fingerprint_short ≤ 50µs avg.
#[test]
fn v7_identity_fingerprint_short_under_50us_avg() {
    use phantom_mesh::identity_wire::fingerprint_short;
    let verifying: [u8; 32] = [0x42; 32];
    const ITERS: u32 = 1_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let _ = fingerprint_short(&verifying);
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.3 identity fingerprint_short", avg_ns, 50_000, ITERS);
}

// V7.4 — SPEC-23 coach aggregate of 50 events ≤ 5ms.
#[test]
fn v7_coach_aggregate_50_events_under_5ms_avg() {
    use phantom_mesh::coach_wire::aggregate;
    use phantom_mesh::event_storage_wire::{AnalysisResult, EventKind, EventMeta};
    let events: Vec<(EventMeta, AnalysisResult)> = (0..50)
        .map(|i| {
            let meta = EventMeta {
                event_id: format!("evt-{i:03}"),
                timestamp: "2026-05-25T12:00:00Z".to_string(),
                kind: if i % 2 == 0 { EventKind::Food } else { EventKind::Focus },
                tags: vec![if i % 3 == 0 { "fat_loss" } else { "work" }.to_string()],
            };
            let analysis = AnalysisResult {
                summary: format!("event {i} brief summary"),
                confidence: 0.8,
                goal_impact: "+0kcal vs target".to_string(),
                suggestion: "stay the course".to_string(),
                cost_usd: 0.0,
                latency_ms: 12,
                model_id: "groq:llama-3.1-8b-instant".to_string(),
                raw_response: "{}".to_string(),
            };
            (meta, analysis)
        })
        .collect();
    const ITERS: u32 = 100;
    let start = Instant::now();
    for _ in 0..ITERS {
        let _ = aggregate(&events);
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.4 coach aggregate(50)", avg_ns, 5_000_000, ITERS);
}

// V7.5 — SPEC-25 apply_skill_to_prompt with 10 recalled skills ≤ 1ms avg.
#[test]
fn v7_skill_apply_to_prompt_10_skills_under_1ms_avg() {
    use phantom_mesh::skill_wire::{apply_skill_to_prompt, Skill, SkillExample};
    let recalled: Vec<Skill> = (0..10)
        .map(|i| Skill {
            id: format!("skill-{i:02}"),
            name: format!("skill name {i}"),
            trigger_pattern: format!("when user asks about topic {i}"),
            steps: vec![
                format!("step a for skill {i}"),
                format!("step b for skill {i}"),
                format!("step c for skill {i}"),
            ],
            examples: vec![SkillExample {
                event_id_hash: format!("hash{i:08x}"),
                redacted_snippet: format!("snippet {i}"),
            }],
            version: 1,
            quality_score: 0.7,
            last_applied_at: 0,
            source_event_count: 5,
        })
        .collect();
    let prompt = "user prompt body for the perf budget test";
    const ITERS: u32 = 1_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let _ = apply_skill_to_prompt(prompt, &recalled);
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.5 skill apply_to_prompt(10)", avg_ns, 1_000_000, ITERS);
}

// V7.6 — SPEC-26 score_peer over 20 peers ≤ 500µs per sweep.
#[test]
fn v7_cluster_score_peer_20_peers_under_500us_avg() {
    use phantom_mesh::cluster_dispatch_wire::{
        score_peer, CapabilityTag, DispatchTask, PeerCapabilities,
    };
    let now_ms = 1_700_000_000_000u64;
    let task = DispatchTask {
        task_id: "task-perf".to_string(),
        required_caps: vec![CapabilityTag { slug: "role-coder".to_string(), value: None }],
        preferred_caps: vec![
            CapabilityTag { slug: "cargo".to_string(), value: None },
            CapabilityTag { slug: "git".to_string(), value: None },
        ],
        payload: "null".to_string(),
        deadline_ms: None,
    };
    let peers: Vec<PeerCapabilities> = (0..20)
        .map(|i| PeerCapabilities {
            peer_id: format!("peer-{i:02}"),
            tags: vec![
                CapabilityTag { slug: "role-coder".to_string(), value: None },
                CapabilityTag { slug: "cargo".to_string(), value: None },
            ],
            last_reported_at: now_ms,
        })
        .collect();
    const ITERS: u32 = 1_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        for p in &peers {
            let _ = score_peer(p, &task);
        }
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.6 cluster score_peer(20)", avg_ns, 500_000, ITERS);
}

// V7.7 — SPEC-28 onboarding FSM decision predicate ≤ 100µs avg.
// `advance`/`rollback`/`compute_ttfr` are still Stage 3 `unimplemented!()`;
// `should_fallback_to_demo_relay` is the one pure-CPU FSM helper available, so
// it stands in as the V7.7 onboarding-wire budget sentinel.
#[test]
fn v7_onboarding_fsm_decision_under_100us_avg() {
    use phantom_mesh::onboarding_wire::{should_fallback_to_demo_relay, OnboardingContext};
    let ctx_needs_fallback = OnboardingContext::default();
    let ctx_has_provider = OnboardingContext {
        cluster_id_hash: Some("ab".repeat(32)),
        identity_fingerprint: Some("0123456789ab".to_string()),
        provider_slug: Some("groq".to_string()),
        demo_relay_used: false,
    };
    const ITERS: u32 = 1_000;
    let start = Instant::now();
    for i in 0..ITERS {
        let ctx = if i % 2 == 0 { &ctx_needs_fallback } else { &ctx_has_provider };
        let _ = should_fallback_to_demo_relay(ctx);
    }
    let avg_ns = start.elapsed().as_nanos() / u128::from(ITERS);
    assert_budget("V7.7 onboarding fsm decision", avg_ns, 100_000, ITERS);
}
