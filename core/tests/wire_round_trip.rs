//! V1 wire round-trip integration tests (Phase E Wave 16 / SPEC-60 V1 gate).
//!
//! For each of the 18 `*_wire.rs` modules under `core/src/`:
//!   1. Construct one representative (non-generic) primary struct.
//!   2. Round-trip it through `serde_json::to_string` → `from_str` and assert
//!      a field survives the round trip.
//!   3. Verify the ts-rs generated `.ts` binding file exists at
//!      `app/src/lib/generated/<spec>/<Type>.ts`.
//!
//! Path resolution: `CARGO_MANIFEST_DIR` is the `core/` crate directory; the
//! TS bindings live one level up under `app/src/lib/generated/`.

use std::path::PathBuf;

/// Resolve the `app/src/lib/generated/<spec>/<Type>.ts` path and return
/// whether the file exists. Uses `CARGO_MANIFEST_DIR` so the test is
/// invariant to the current working directory at `cargo test` invocation.
fn ts_file_exists(spec_dir: &str, type_name: &str) -> bool {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.push("app");
    p.push("src");
    p.push("lib");
    p.push("generated");
    p.push(spec_dir);
    p.push(format!("{type_name}.ts"));
    p.exists()
}

// ─── 1/18 — rpc_wire ─────────────────────────────────────────────────────────

#[test]
fn v1_rpc_wire_round_trip() {
    use spectyn_mesh::rpc_wire::{ClientOs, PingResponse};
    let req = PingResponse {
        peer_name: "test-peer".to_string(),
        os: ClientOs::Mac,
        version: "0.6.0".to_string(),
        capabilities: vec!["P1.mdns".to_string()],
        cluster_fingerprint: "abcd1234".to_string(),
        uptime_s: 42,
    };
    let json = serde_json::to_string(&req).expect("serialize PingResponse");
    let back: PingResponse = serde_json::from_str(&json).expect("deserialize PingResponse");
    assert_eq!(req.peer_name, back.peer_name);
    assert_eq!(req.uptime_s, back.uptime_s);
    assert!(ts_file_exists("rpc", "PingResponse"));
}

// ─── 2/18 — identity_wire ────────────────────────────────────────────────────

#[test]
fn v1_identity_wire_round_trip() {
    use spectyn_mesh::identity_wire::IdentityPublic;
    let req = IdentityPublic {
        public_key: "00".repeat(32),
        fingerprint: "abc123def456".to_string(),
        created_at: "2026-05-25T00:00:00Z".to_string(),
    };
    let json = serde_json::to_string(&req).expect("serialize IdentityPublic");
    let back: IdentityPublic = serde_json::from_str(&json).expect("deserialize IdentityPublic");
    assert_eq!(req.fingerprint, back.fingerprint);
    assert_eq!(req.public_key, back.public_key);
    assert!(ts_file_exists("identity", "IdentityPublic"));
}

// ─── 3/18 — mdns_wire ────────────────────────────────────────────────────────

#[test]
fn v1_mdns_wire_round_trip() {
    use spectyn_mesh::mdns_wire::{PeerAdvertisement, PeerOs};
    use std::net::{IpAddr, Ipv4Addr};
    let req = PeerAdvertisement {
        v: 1,
        pf: "a1b2c3d4".to_string(),
        cl: "0123456789abcdef".to_string(),
        ca: vec!["role-coder".to_string()],
        os: PeerOs::Mac,
        na: "test-host".to_string(),
        port: 7878,
        addrs: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
    };
    let json = serde_json::to_string(&req).expect("serialize PeerAdvertisement");
    let back: PeerAdvertisement =
        serde_json::from_str(&json).expect("deserialize PeerAdvertisement");
    assert_eq!(req.pf, back.pf);
    assert_eq!(req.port, back.port);
    assert!(ts_file_exists("mdns", "PeerAdvertisement"));
}

// ─── 4/18 — encryption_wire ──────────────────────────────────────────────────

#[test]
fn v1_encryption_wire_round_trip() {
    use spectyn_mesh::encryption_wire::{
        EncryptionAlgorithm, EncryptionEnvelope, X25519Recipient,
    };
    let req = EncryptionEnvelope {
        algorithm: EncryptionAlgorithm::AgeV1,
        recipient: X25519Recipient("age1example".to_string()),
        ciphertext_b64: "Zm9vYmFy".to_string(),
        created_at: "2026-05-25T00:00:00Z".to_string(),
    };
    let json = serde_json::to_string(&req).expect("serialize EncryptionEnvelope");
    let back: EncryptionEnvelope =
        serde_json::from_str(&json).expect("deserialize EncryptionEnvelope");
    assert_eq!(req.ciphertext_b64, back.ciphertext_b64);
    assert_eq!(req.recipient.0, back.recipient.0);
    assert!(ts_file_exists("encryption", "EncryptionEnvelope"));
}

// ─── 5/18 — providers_wire ───────────────────────────────────────────────────

#[test]
fn v1_providers_wire_round_trip() {
    use spectyn_mesh::providers_wire::ProviderConfig;
    let req = ProviderConfig {
        slug: "groq".to_string(),
        api_key_ref: "secrets.age#providers.groq.api_key".to_string(),
        default_model: "llama-3.1-8b-instant".to_string(),
        base_url: None,
        timeout_ms: 30_000,
    };
    let json = serde_json::to_string(&req).expect("serialize ProviderConfig");
    let back: ProviderConfig = serde_json::from_str(&json).expect("deserialize ProviderConfig");
    assert_eq!(req.slug, back.slug);
    assert_eq!(req.timeout_ms, back.timeout_ms);
    assert!(ts_file_exists("providers", "ProviderConfig"));
}

// ─── 6/18 — broker_vault_wire ────────────────────────────────────────────────

#[test]
fn v1_broker_vault_wire_round_trip() {
    use spectyn_mesh::broker_vault_wire::BrokerJwt;
    let req = BrokerJwt {
        token: "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.sig".to_string(),
        expires_at_ts: 1_700_000_000_000,
    };
    let json = serde_json::to_string(&req).expect("serialize BrokerJwt");
    let back: BrokerJwt = serde_json::from_str(&json).expect("deserialize BrokerJwt");
    assert_eq!(req.token, back.token);
    assert_eq!(req.expires_at_ts, back.expires_at_ts);
    assert!(ts_file_exists("broker_vault", "BrokerJwt"));
}

// ─── 7/18 — event_storage_wire ───────────────────────────────────────────────

#[test]
fn v1_event_storage_wire_round_trip() {
    use spectyn_mesh::event_storage_wire::{EventKind, EventMeta};
    let req = EventMeta {
        event_id: "01890000-0000-7000-8000-000000000000".to_string(),
        timestamp: "2026-05-25T00:00:00Z".to_string(),
        kind: EventKind::Food,
        tags: vec!["fat_loss".to_string()],
    };
    let json = serde_json::to_string(&req).expect("serialize EventMeta");
    let back: EventMeta = serde_json::from_str(&json).expect("deserialize EventMeta");
    assert_eq!(req.event_id, back.event_id);
    assert_eq!(req.tags, back.tags);
    assert!(ts_file_exists("event_storage", "EventMeta"));
}

// ─── 8/18 — tauri_wire ───────────────────────────────────────────────────────

#[test]
fn v1_tauri_wire_round_trip() {
    use spectyn_mesh::tauri_wire::ClusterStatusResponse;
    let req = ClusterStatusResponse {
        state: "healthy".to_string(),
        peer_count: 3,
        last_heartbeat_ts_ms: Some(1_700_000_000_000),
    };
    let json = serde_json::to_string(&req).expect("serialize ClusterStatusResponse");
    let back: ClusterStatusResponse =
        serde_json::from_str(&json).expect("deserialize ClusterStatusResponse");
    assert_eq!(req.state, back.state);
    assert_eq!(req.peer_count, back.peer_count);
    assert_eq!(req.last_heartbeat_ts_ms, back.last_heartbeat_ts_ms);
    assert!(ts_file_exists("tauri", "ClusterStatusResponse"));
}

// ─── 9/18 — capture_food_wire ────────────────────────────────────────────────

#[test]
fn v1_capture_food_wire_round_trip() {
    use spectyn_mesh::capture_food_wire::MacroEstimate;
    let req = MacroEstimate {
        calories: 520,
        protein_g: 32,
        carbs_g: 60,
        fat_g: 18,
        fiber_g: 6,
    };
    let json = serde_json::to_string(&req).expect("serialize MacroEstimate");
    let back: MacroEstimate = serde_json::from_str(&json).expect("deserialize MacroEstimate");
    assert_eq!(req.calories, back.calories);
    assert_eq!(req.protein_g, back.protein_g);
    assert!(ts_file_exists("capture_food", "MacroEstimate"));
}

// ─── 10/18 — capture_focus_wire ──────────────────────────────────────────────

#[test]
fn v1_capture_focus_wire_round_trip() {
    use spectyn_mesh::capture_focus_wire::{FocusInterruption, InterruptionKind};
    let req = FocusInterruption {
        timestamp_ms: 1_700_000_000_000,
        kind: InterruptionKind::Notification,
        duration_ms: 1_200,
    };
    let json = serde_json::to_string(&req).expect("serialize FocusInterruption");
    let back: FocusInterruption =
        serde_json::from_str(&json).expect("deserialize FocusInterruption");
    assert_eq!(req.timestamp_ms, back.timestamp_ms);
    assert_eq!(req.duration_ms, back.duration_ms);
    assert!(ts_file_exists("capture_focus", "FocusInterruption"));
}

// ─── 11/18 — capture_habit_wire ──────────────────────────────────────────────

#[test]
fn v1_capture_habit_wire_round_trip() {
    use spectyn_mesh::capture_habit_wire::HabitStreak;
    let req = HabitStreak {
        habit_slug: "drink_water".to_string(),
        current_streak: 7,
        longest_streak: 21,
        last_checkin_at: Some("2026-05-25T08:30:00Z".to_string()),
    };
    let json = serde_json::to_string(&req).expect("serialize HabitStreak");
    let back: HabitStreak = serde_json::from_str(&json).expect("deserialize HabitStreak");
    assert_eq!(req.habit_slug, back.habit_slug);
    assert_eq!(req.current_streak, back.current_streak);
    assert_eq!(req.last_checkin_at, back.last_checkin_at);
    assert!(ts_file_exists("capture_habit", "HabitStreak"));
}

// ─── 12/18 — coach_wire ──────────────────────────────────────────────────────

#[test]
fn v1_coach_wire_round_trip() {
    use spectyn_mesh::coach_wire::CoachReviewReadyPayload;
    let req = CoachReviewReadyPayload {
        review_id: "01890000-0000-7000-8000-000000000001".to_string(),
        event_id: "01890000-0000-7000-8000-000000000001".to_string(),
        date: "2026-05-25".to_string(),
        takeaways_count: 4,
        markdown_path: "/home/user/.spectyn-mesh/coach/2026-05-25.md.age".to_string(),
    };
    let json = serde_json::to_string(&req).expect("serialize CoachReviewReadyPayload");
    let back: CoachReviewReadyPayload =
        serde_json::from_str(&json).expect("deserialize CoachReviewReadyPayload");
    assert_eq!(req.review_id, back.review_id);
    assert_eq!(req.event_id, back.event_id);
    assert_eq!(req.takeaways_count, back.takeaways_count);
    assert!(ts_file_exists("coach", "CoachReviewReadyPayload"));
}

// ─── 13/18 — coach_delivery_wire ─────────────────────────────────────────────

#[test]
fn v1_coach_delivery_wire_round_trip() {
    use spectyn_mesh::coach_delivery_wire::{DeliveryChannel, DeliveryReceipt, DeliveryStatus};
    let req = DeliveryReceipt {
        review_id: "01890000-0000-7000-8000-000000000002".to_string(),
        channel: DeliveryChannel::Markdown,
        attempted_at_ms: 1_700_000_000_000,
        status: DeliveryStatus::Sent,
        error_message: None,
    };
    let json = serde_json::to_string(&req).expect("serialize DeliveryReceipt");
    let back: DeliveryReceipt =
        serde_json::from_str(&json).expect("deserialize DeliveryReceipt");
    assert_eq!(req.review_id, back.review_id);
    assert_eq!(req.attempted_at_ms, back.attempted_at_ms);
    assert!(ts_file_exists("coach_delivery", "DeliveryReceipt"));
}

// ─── 14/18 — skill_wire ──────────────────────────────────────────────────────

#[test]
fn v1_skill_wire_round_trip() {
    use spectyn_mesh::skill_wire::SkillExample;
    let req = SkillExample {
        event_id_hash: "abcdef0123456789".to_string(),
        redacted_snippet: "[redacted] morning routine started".to_string(),
    };
    let json = serde_json::to_string(&req).expect("serialize SkillExample");
    let back: SkillExample = serde_json::from_str(&json).expect("deserialize SkillExample");
    assert_eq!(req.event_id_hash, back.event_id_hash);
    assert_eq!(req.redacted_snippet, back.redacted_snippet);
    assert!(ts_file_exists("skill", "SkillExample"));
}

// ─── 15/18 — cluster_dispatch_wire ───────────────────────────────────────────

#[test]
fn v1_cluster_dispatch_wire_round_trip() {
    use spectyn_mesh::cluster_dispatch_wire::CapabilityTag;
    let req = CapabilityTag {
        slug: "role-coder".to_string(),
        value: Some("rust".to_string()),
    };
    let json = serde_json::to_string(&req).expect("serialize CapabilityTag");
    let back: CapabilityTag = serde_json::from_str(&json).expect("deserialize CapabilityTag");
    assert_eq!(req.slug, back.slug);
    assert_eq!(req.value, back.value);
    assert!(ts_file_exists("cluster_dispatch", "CapabilityTag"));
}

// ─── 16/18 — smart_decompose_wire ────────────────────────────────────────────

#[test]
fn v1_smart_decompose_wire_round_trip() {
    use spectyn_mesh::smart_decompose_wire::{DecomposeStatus, ExecutionProgress};
    let req = ExecutionProgress {
        parent_task_id: "01890000-0000-7000-8000-000000000003".to_string(),
        completed_subtasks: 2,
        total_subtasks: 5,
        failed_subtasks: 0,
        current_status: DecomposeStatus::Running,
    };
    let json = serde_json::to_string(&req).expect("serialize ExecutionProgress");
    let back: ExecutionProgress =
        serde_json::from_str(&json).expect("deserialize ExecutionProgress");
    assert_eq!(req.parent_task_id, back.parent_task_id);
    assert_eq!(req.completed_subtasks, back.completed_subtasks);
    assert_eq!(req.total_subtasks, back.total_subtasks);
    assert!(ts_file_exists("smart_decompose", "ExecutionProgress"));
}

// ─── 17/18 — onboarding_wire ─────────────────────────────────────────────────

#[test]
fn v1_onboarding_wire_round_trip() {
    use spectyn_mesh::onboarding_wire::TTFRMetric;
    let req = TTFRMetric {
        install_complete_at_ms: 1_700_000_000_000,
        first_reply_at_ms: 1_700_000_012_500,
        total_ms: 12_500,
    };
    let json = serde_json::to_string(&req).expect("serialize TTFRMetric");
    let back: TTFRMetric = serde_json::from_str(&json).expect("deserialize TTFRMetric");
    assert_eq!(req.install_complete_at_ms, back.install_complete_at_ms);
    assert_eq!(req.first_reply_at_ms, back.first_reply_at_ms);
    assert_eq!(req.total_ms, back.total_ms);
    assert_eq!(
        req.total_ms,
        req.first_reply_at_ms - req.install_complete_at_ms
    );
    assert!(ts_file_exists("onboarding", "TTFRMetric"));
}

// ─── 18/18 — release_pipeline_wire ───────────────────────────────────────────

#[test]
fn v1_release_pipeline_wire_round_trip() {
    use spectyn_mesh::release_pipeline_wire::{ArtifactArch, ArtifactOs, ReleaseArtifact};
    let req = ReleaseArtifact {
        os: ArtifactOs::Macos,
        arch: ArtifactArch::Aarch64,
        file_name: "spectyn-mesh-0.6.0-darwin-aarch64.dmg".to_string(),
        sha256_hex: "0".repeat(64),
        size_bytes: 50_000_000,
        signature_url: None,
        download_url: "https://example.invalid/spectyn-mesh.dmg".to_string(),
    };
    let json = serde_json::to_string(&req).expect("serialize ReleaseArtifact");
    let back: ReleaseArtifact =
        serde_json::from_str(&json).expect("deserialize ReleaseArtifact");
    assert_eq!(req.file_name, back.file_name);
    assert_eq!(req.sha256_hex, back.sha256_hex);
    assert_eq!(req.size_bytes, back.size_bytes);
    assert!(ts_file_exists("release_pipeline", "ReleaseArtifact"));
}
