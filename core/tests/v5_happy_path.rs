//! Phase E V5 — happy-path integration tests. Pure-logic flows that exercise
//! **multiple wire modules together**, proving cross-module contracts hold when
//! real Stage 3/4 impls (HMAC, HKDF, mDNS TXT parse, phf FSM table) chain into
//! each other. No network / filesystem / async — V11 smoke covers those.
//! See `docs/superpowers/PHASE-E-INTEGRATION-TEST-PLAN.md` §V5.

// ─── 1/7 — rpc envelope ↔ HMAC sign ↔ verify (SPEC-10 Stage 3 real crypto) ──

/// Real HMAC-SHA256 round-trip through `build_canonical_string`. Catches
/// drift in line order / separator and any regression in the `sha2`/`hmac`/
/// `subtle` swap that landed Wave 13.
#[test]
fn v5_rpc_sign_verify_round_trip() {
    use spectyn_mesh::rpc_wire::{build_canonical_string, sign_hmac, verify_hmac};

    // Both secrets MUST be exactly 32 bytes (matches SPEC-12 §7.2
    // `KeyPurpose::ClusterHmac` HKDF output length).
    let secret: &[u8; 32] = b"test-cluster-secret-32-bytes-len";
    let wrong: &[u8; 32] = b"wrong-secret-32-bytes-padding!!!";
    const _: () = assert!(b"test-cluster-secret-32-bytes-len".len() == 32);
    const _: () = assert!(b"wrong-secret-32-bytes-padding!!!".len() == 32);

    let canonical =
        build_canonical_string("POST", "/rpc/task/assign", "", b"hello", None);
    let sig = sign_hmac(secret, &canonical);
    assert_eq!(sig.len(), 64, "HMAC-SHA256 hex tag MUST be 64 chars");

    verify_hmac(secret, &canonical, &sig).expect("good secret MUST verify");
    let bad = verify_hmac(wrong, &canonical, &sig)
        .expect_err("wrong secret MUST fail verification");
    assert_eq!(bad.code, "auth_invalid", "mismatch → SPEC-04 auth_invalid");
}

// ─── 2/7 — identity HKDF subkey → encryption EventKey (SPEC-12 → SPEC-13) ──

/// SPEC-12 `KeyPurpose::EventEncrypt` and SPEC-13 `derive_event_key_from_identity`
/// share the canonical info-string `spectyn-mesh.v1.event-encrypt`. Determinism
/// on repeat call is load-bearing: without it E004 encrypted-events lose data
/// across restarts.
#[test]
fn v5_identity_hkdf_feeds_encryption_event_key() {
    use spectyn_mesh::encryption_wire::derive_event_key_from_identity;
    use spectyn_mesh::identity_wire::KeyPurpose;

    assert_eq!(
        KeyPurpose::EventEncrypt.info_string(),
        "spectyn-mesh.v1.event-encrypt",
        "SPEC-12 §7.2 reserved prefix invariant",
    );

    let seed: [u8; 32] = [0x42u8; 32];
    let k1 = derive_event_key_from_identity(&seed).expect("hkdf must succeed");
    let k2 = derive_event_key_from_identity(&seed).expect("hkdf must succeed");
    assert_eq!(
        k1.as_bytes(),
        k2.as_bytes(),
        "EventKey HKDF MUST be deterministic on the same seed",
    );

    let k3 = derive_event_key_from_identity(&[0x99u8; 32]).expect("hkdf");
    assert_ne!(
        k1.as_bytes(),
        k3.as_bytes(),
        "different seeds MUST yield different EventKey bytes",
    );
}

// ─── 3/7 — smart_decompose validate_dag → cluster_dispatch plan shape ──────

/// Linear DAG (A→B→C) through real Stage 4 `validate_dag` → Sequential, then
/// assemble a matching `DispatchPlan` shape. The shared `CapabilityTag` type
/// used by `SubTask.required_caps` is the load-bearing cross-module symbol.
#[test]
fn v5_decompose_dag_validates_then_dispatch_plan_shape() {
    use spectyn_mesh::cluster_dispatch_wire::{CapabilityTag, DispatchPlan};
    use spectyn_mesh::smart_decompose_wire::{
        validate_dag, DecomposeRequest, SubTask, TopologyHint,
    };

    let req = DecomposeRequest {
        task_text: "fix the login bug and write a regression test".to_string(),
        max_subtasks: 8,
        target_caps: None,
        deadline_ms: None,
    };
    let req_json = serde_json::to_string(&req).expect("DecomposeRequest serializes");
    let _: DecomposeRequest =
        serde_json::from_str(&req_json).expect("DecomposeRequest round-trips");

    let parent = "01890000-0000-7000-8000-00000000abcd".to_string();
    let cap = CapabilityTag {
        slug: "role-coder".to_string(),
        value: None,
    };
    let mk = |id: &str, dep: Vec<&str>| SubTask {
        subtask_id: id.to_string(),
        parent_task_id: parent.clone(),
        text: format!("step {id}"),
        required_caps: vec![cap.clone()],
        depends_on: dep.into_iter().map(str::to_string).collect(),
        priority: 5,
        estimated_duration_ms: None,
    };
    let subtasks = vec![mk("a", vec![]), mk("b", vec!["a"]), mk("c", vec!["b"])];

    let topo = validate_dag(&subtasks).expect("linear DAG must validate");
    assert_eq!(
        topo,
        TopologyHint::Sequential,
        "single-chain DAG must classify as Sequential",
    );

    let plan = DispatchPlan {
        task_id: parent,
        selected_peer_id: "peer-mac".to_string(),
        fallback_peer_ids: vec!["peer-win".to_string()],
        scoring_reason: "role-coder present; rtt 45ms".to_string(),
        planned_at_ms: 1_700_000_000_000,
    };
    let j = serde_json::to_string(&plan).expect("DispatchPlan serializes");
    let back: DispatchPlan = serde_json::from_str(&j).expect("round-trips");
    assert_eq!(back.fallback_peer_ids.len(), 1);
}

// ─── 4/7 — coach review payload ↔ coach_delivery DeliveryReceipt schema ─────

/// SPEC-23 §9.7 + SPEC-24 §20.1 cycle-break invariant: payload MUST stay
/// 5 fields and SPEC-24 MUST be able to build a `DeliveryReceipt` keyed on
/// the same `review_id`. Drift here breaks all coach → delivery dispatch.
#[test]
fn v5_coach_review_payload_matches_delivery_schema() {
    use spectyn_mesh::coach_delivery_wire::{
        DeliveryChannel, DeliveryReceipt, DeliveryStatus,
    };
    use spectyn_mesh::coach_wire::CoachReviewReadyPayload;

    let review_id = "01890000-0000-7000-8000-000000000099".to_string();
    let payload = CoachReviewReadyPayload {
        review_id: review_id.clone(),
        event_id: review_id.clone(),
        date: "2026-05-25".to_string(),
        takeaways_count: 4,
        markdown_path: "/home/user/.spectyn-mesh/coach/2026-05-25.md.age".to_string(),
    };

    let j = serde_json::to_string(&payload).expect("payload serializes");
    let parsed: serde_json::Value = serde_json::from_str(&j).expect("re-parse");
    let map = parsed.as_object().expect("payload is JSON object");
    assert_eq!(
        map.len(),
        5,
        "CoachReviewReadyPayload MUST stay 5 fields (SPEC-23 §9.7 + SPEC-24 §20.1); got keys: {:?}",
        map.keys().collect::<Vec<_>>(),
    );
    for required in ["reviewId", "eventId", "date", "takeawaysCount", "markdownPath"] {
        assert!(
            map.contains_key(required),
            "payload missing required field `{}`",
            required,
        );
    }

    let receipt = DeliveryReceipt {
        review_id: payload.review_id.clone(),
        channel: DeliveryChannel::Markdown,
        attempted_at_ms: 1_700_000_000_000,
        status: DeliveryStatus::Sent,
        error_message: None,
    };
    assert_eq!(receipt.review_id, payload.review_id);
}

// ─── 5/7 — RETIRED (Phase G1, 2026-05-26) ──────────────────────────────────
//
// The legacy `life_node::storage::EventMeta` struct + its `From` bridge to the
// wire `event_storage_wire::EventMeta` were retired in Wave G1. The on-disk
// projection now lives entirely inside `EventStore::read_meta` /
// `EventStore::write_event` (private `OnDiskEventMeta` → wire `EventMeta`),
// covered by unit tests in `core/src/life_node/storage.rs`
// (`project_to_wire_maps_known_kinds` + `project_to_wire_preserves_id_timestamp_and_tags`).
// V8 (`core/tests/event_migration_v8.rs`) was deleted in the same wave.

// ─── 6/7 — mDNS TXT records → PeerAdvertisement (SPEC-11 Stage 4 parser) ───

/// Real Stage 4 `parse_txt_records` consumes the 6-key TXT schema and emits a
/// fully-populated `PeerAdvertisement` (port + addrs left empty for the
/// SRV / A lookup at the caller layer).
#[test]
fn v5_mdns_txt_records_parse_to_advertisement() {
    use spectyn_mesh::mdns_wire::{parse_txt_records, PeerOs};

    let raw: Vec<(String, String)> = vec![
        ("v".to_string(), "1".to_string()),
        ("pf".to_string(), "a1b2c3d4".to_string()),
        ("cl".to_string(), "0123456789abcdef".to_string()),
        ("ca".to_string(), "role-coder,cargo".to_string()),
        ("os".to_string(), "mac".to_string()),
        ("na".to_string(), "test-host".to_string()),
    ];

    let ad = parse_txt_records(&raw).expect("valid 6-key TXT set must parse");

    assert_eq!(ad.v, 1, "version sentinel pinned at 1 for v0.6.0");
    assert_eq!(ad.pf, "a1b2c3d4");
    assert_eq!(ad.cl, "0123456789abcdef");
    assert_eq!(ad.ca, vec!["role-coder".to_string(), "cargo".to_string()]);
    assert_eq!(ad.os, PeerOs::Mac);
    assert_eq!(ad.na, "test-host");
    assert_eq!(ad.port, 0, "port placeholder zero — SRV fills at caller");
    assert!(ad.addrs.is_empty(), "addrs placeholder empty — A/AAAA fills at caller");
}

// ─── 7/7 — onboarding FSM forward-chain coverage (SPEC-28 §7.1 / §8) ────────

/// SPEC-28 §7.1 forward chain — all 6 states MUST serialize with their
/// canonical snake_case slugs + survive a snapshot round-trip. The Stage 4
/// `phf::Map`-backed FSM table is the source of truth for transitions; this
/// integration test asserts the **observable** wire surface (slug strings +
/// state ordering) that UI + persisted `~/.spectyn-mesh/onboarding.json`
/// depend on. We do NOT call `advance()` here — Stage 3 leaves it
/// `unimplemented!()` until V11 wave (see TODO at top of onboarding_wire.rs).
#[test]
fn v5_onboarding_forward_chain_states_round_trip() {
    use spectyn_mesh::onboarding_wire::{
        should_fallback_to_demo_relay, OnboardingContext, OnboardingState,
        OnboardingStateSnapshot,
    };

    let forward_chain = [
        (OnboardingState::FreshInstall, "fresh_install"),
        (OnboardingState::PickedLanguage, "picked_language"),
        (OnboardingState::CreatedIdentity, "created_identity"),
        (OnboardingState::JoinedCluster, "joined_cluster"),
        (OnboardingState::SetProvider, "set_provider"),
        (OnboardingState::FirstReplyReceived, "first_reply_received"),
    ];

    for (state, expected_slug) in forward_chain {
        let snap = OnboardingStateSnapshot {
            current_state: state,
            entered_at_ms: 1_716_563_400_000,
            retry_count: 0,
            last_error: None,
        };
        let j = serde_json::to_string(&snap).expect("snapshot serializes");
        assert!(
            j.contains(&format!("\"{}\"", expected_slug)),
            "state {:?} MUST serialize as `{}` (got: {})",
            state,
            expected_slug,
            j,
        );
        let back: OnboardingStateSnapshot =
            serde_json::from_str(&j).expect("round-trips");
        assert_eq!(back.current_state, state, "state survives round-trip");
    }

    // Cross-module: fresh context (no cluster, no provider) MUST trigger
    // demo-relay fallback per SPEC-28 §10.4. The decision function consumed
    // by the wizard between forward-chain steps.
    let fresh_ctx = OnboardingContext::default();
    assert!(
        should_fallback_to_demo_relay(&fresh_ctx),
        "fresh install MUST fall back to demo-relay",
    );
    let configured_ctx = OnboardingContext {
        cluster_id_hash: None,
        identity_fingerprint: Some("abcdef012345".to_string()),
        provider_slug: Some("groq".to_string()),
        demo_relay_used: false,
        // D1 login-first fields added to OnboardingContext after this test was
        // written; this case exercises provider-config (not login), and the
        // fallback decision ignores them, so None keeps the original intent.
        identity_provider: None,
        identity_sub: None,
    };
    assert!(
        !should_fallback_to_demo_relay(&configured_ctx),
        "provider configured MUST NOT trigger demo-relay fallback",
    );
}
