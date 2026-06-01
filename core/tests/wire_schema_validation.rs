//! V3 wire schema validation tests (Phase E Integration Test Plan §V3).
//!
//! Companion to `wire_round_trip.rs` (V1 gate). For each of the 18 `*_wire.rs`
//! modules this binary asserts four invariants:
//!   1. A hand-written camelCase JSON fixture deserialises cleanly.
//!   2. `to_string(from_str(X))` is byte-identical on the 2nd round trip
//!      (re-serialise idempotence — catches non-deterministic ordering).
//!   3. The ts-rs `.ts` file under `app/src/lib/generated/<spec>/` exists
//!      and its text contains every camelCase field name in the sample.
//!   4. (Forward-compat probe) extra JSON keys are tolerated on parse and
//!      dropped on re-serialise — flipping `deny_unknown_fields` in any
//!      future spec wave will break this and force a deliberate review.

use std::path::PathBuf;

/// Resolve `app/src/lib/generated/<spec>/<Type>.ts` via `CARGO_MANIFEST_DIR`.
/// Helper duplicated from `wire_round_trip.rs` — cargo builds each tests
/// binary as a separate crate so cross-file `mod` reuse is awkward.
fn ts_path(spec_dir: &str, type_name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.push("app");
    p.push("src");
    p.push("lib");
    p.push("generated");
    p.push(spec_dir);
    p.push(format!("{type_name}.ts"));
    p
}

/// Read the ts-rs binding file into a String. Panics loudly if missing —
/// a missing file usually means `cargo test --features ts-export` was not run.
fn read_ts(spec_dir: &str, type_name: &str) -> String {
    let p = ts_path(spec_dir, type_name);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("missing TS binding {}: {e}", p.display()))
}

/// Assert every name in `fields` is present in the TS file text. Uses
/// `String::contains` — good enough because ts-rs emits unique `fieldName:`.
fn assert_ts_has_fields(ts: &str, fields: &[&str], type_name: &str) {
    for f in fields {
        assert!(
            ts.contains(f),
            "TS binding for {type_name} missing field `{f}` — ts-rs export drift?"
        );
    }
}

/// Parse → serialise → parse → serialise; assert the two output strings
/// match (re-serialisation is idempotent).
fn assert_reserialize_stable<T>(sample: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let first: T = serde_json::from_str(sample).expect("parse sample JSON");
    let s1 = serde_json::to_string(&first).expect("serialize round 1");
    let second: T = serde_json::from_str(&s1).expect("parse round-1 output");
    let s2 = serde_json::to_string(&second).expect("serialize round 2");
    assert_eq!(s1, s2, "re-serialisation is not idempotent");
}

// ─── 1/18 — rpc_wire / PingResponse ──────────────────────────────────────────

#[test]
fn v3_rpc_wire_schema_stable() {
    use phantom_mesh::rpc_wire::PingResponse;
    const SAMPLE: &str = r#"{"peerName":"alpha","os":"mac","version":"0.6.0","capabilities":["P1.mdns"],"clusterFingerprint":"abcd1234","uptimeS":42}"#;
    assert_reserialize_stable::<PingResponse>(SAMPLE);
    let ts = read_ts("rpc", "PingResponse");
    assert_ts_has_fields(
        &ts,
        &["peerName", "os", "version", "capabilities", "clusterFingerprint", "uptimeS"],
        "PingResponse",
    );
}

// ─── 2/18 — identity_wire / IdentityPublic ───────────────────────────────────

#[test]
fn v3_identity_wire_schema_stable() {
    use phantom_mesh::identity_wire::IdentityPublic;
    const SAMPLE: &str = r#"{"publicKey":"0000000000000000000000000000000000000000000000000000000000000000","fingerprint":"abc123def456","createdAt":"2026-05-25T00:00:00Z"}"#;
    assert_reserialize_stable::<IdentityPublic>(SAMPLE);
    let ts = read_ts("identity", "IdentityPublic");
    assert_ts_has_fields(
        &ts,
        &["publicKey", "fingerprint", "createdAt"],
        "IdentityPublic",
    );
}

// ─── 3/18 — mdns_wire / PeerAdvertisement ────────────────────────────────────

#[test]
fn v3_mdns_wire_schema_stable() {
    use phantom_mesh::mdns_wire::PeerAdvertisement;
    const SAMPLE: &str = r#"{"v":1,"pf":"a1b2c3d4","cl":"0123456789abcdef","ca":["role-coder"],"os":"mac","na":"test-host","port":7878,"addrs":["127.0.0.1"]}"#;
    assert_reserialize_stable::<PeerAdvertisement>(SAMPLE);
    let ts = read_ts("mdns", "PeerAdvertisement");
    assert_ts_has_fields(
        &ts,
        &["v", "pf", "cl", "ca", "os", "na", "port", "addrs"],
        "PeerAdvertisement",
    );
}

// ─── 4/18 — encryption_wire / EncryptionEnvelope ─────────────────────────────

#[test]
fn v3_encryption_wire_schema_stable() {
    use phantom_mesh::encryption_wire::EncryptionEnvelope;
    const SAMPLE: &str = r#"{"algorithm":"age_v1","recipient":"age1example","ciphertextB64":"Zm9vYmFy","createdAt":"2026-05-25T00:00:00Z"}"#;
    assert_reserialize_stable::<EncryptionEnvelope>(SAMPLE);
    let ts = read_ts("encryption", "EncryptionEnvelope");
    assert_ts_has_fields(
        &ts,
        &["algorithm", "recipient", "ciphertextB64", "createdAt"],
        "EncryptionEnvelope",
    );
}

// ─── 5/18 — providers_wire / ProviderConfig ──────────────────────────────────

#[test]
fn v3_providers_wire_schema_stable() {
    use phantom_mesh::providers_wire::ProviderConfig;
    const SAMPLE: &str = r#"{"slug":"groq","apiKeyRef":"secrets.age#providers.groq.api_key","defaultModel":"llama-3.1-8b-instant","baseUrl":null,"timeoutMs":30000}"#;
    assert_reserialize_stable::<ProviderConfig>(SAMPLE);
    let ts = read_ts("providers", "ProviderConfig");
    assert_ts_has_fields(
        &ts,
        &["slug", "apiKeyRef", "defaultModel", "baseUrl", "timeoutMs"],
        "ProviderConfig",
    );
}

// ─── 6/18 — broker_vault_wire / BrokerJwt ────────────────────────────────────

#[test]
fn v3_broker_vault_wire_schema_stable() {
    use phantom_mesh::broker_vault_wire::BrokerJwt;
    const SAMPLE: &str =
        r#"{"token":"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.sig","expiresAtTs":1700000000000}"#;
    assert_reserialize_stable::<BrokerJwt>(SAMPLE);
    let ts = read_ts("broker_vault", "BrokerJwt");
    assert_ts_has_fields(&ts, &["token", "expiresAtTs"], "BrokerJwt");
}

// ─── 7/18 — event_storage_wire / EventMeta ───────────────────────────────────

#[test]
fn v3_event_storage_wire_schema_stable() {
    use phantom_mesh::event_storage_wire::EventMeta;
    // `EventKind` re-exported from rpc_wire uses snake_case → wire value `"food"`.
    const SAMPLE: &str = r#"{"eventId":"01890000-0000-7000-8000-000000000000","timestamp":"2026-05-25T00:00:00Z","kind":"food","tags":["fat_loss"]}"#;
    assert_reserialize_stable::<EventMeta>(SAMPLE);
    let ts = read_ts("event_storage", "EventMeta");
    assert_ts_has_fields(&ts, &["eventId", "timestamp", "kind", "tags"], "EventMeta");
}

// ─── 8/18 — tauri_wire / ClusterStatusResponse ───────────────────────────────

#[test]
fn v3_tauri_wire_schema_stable() {
    use phantom_mesh::tauri_wire::ClusterStatusResponse;
    const SAMPLE: &str =
        r#"{"state":"healthy","peerCount":3,"lastHeartbeatTsMs":1700000000000}"#;
    assert_reserialize_stable::<ClusterStatusResponse>(SAMPLE);
    let ts = read_ts("tauri", "ClusterStatusResponse");
    assert_ts_has_fields(&ts, &["state", "peerCount", "lastHeartbeatTsMs"], "ClusterStatusResponse");
}

// ─── 9/18 — capture_food_wire / MacroEstimate ────────────────────────────────

#[test]
fn v3_capture_food_wire_schema_stable() {
    use phantom_mesh::capture_food_wire::MacroEstimate;
    const SAMPLE: &str =
        r#"{"calories":520,"proteinG":32,"carbsG":60,"fatG":18,"fiberG":6}"#;
    assert_reserialize_stable::<MacroEstimate>(SAMPLE);
    let ts = read_ts("capture_food", "MacroEstimate");
    assert_ts_has_fields(&ts, &["calories", "proteinG", "carbsG", "fatG", "fiberG"], "MacroEstimate");
}

// ─── 10/18 — capture_focus_wire / FocusInterruption ──────────────────────────

#[test]
fn v3_capture_focus_wire_schema_stable() {
    use phantom_mesh::capture_focus_wire::FocusInterruption;
    // `InterruptionKind` uses snake_case → wire value `"notification"`.
    const SAMPLE: &str =
        r#"{"timestampMs":1700000000000,"kind":"notification","durationMs":1200}"#;
    assert_reserialize_stable::<FocusInterruption>(SAMPLE);
    let ts = read_ts("capture_focus", "FocusInterruption");
    assert_ts_has_fields(&ts, &["timestampMs", "kind", "durationMs"], "FocusInterruption");
}

// ─── 11/18 — capture_habit_wire / HabitStreak ────────────────────────────────

#[test]
fn v3_capture_habit_wire_schema_stable() {
    use phantom_mesh::capture_habit_wire::HabitStreak;
    const SAMPLE: &str = r#"{"habitSlug":"drink_water","currentStreak":7,"longestStreak":21,"lastCheckinAt":"2026-05-25T08:30:00Z"}"#;
    assert_reserialize_stable::<HabitStreak>(SAMPLE);
    let ts = read_ts("capture_habit", "HabitStreak");
    assert_ts_has_fields(
        &ts,
        &["habitSlug", "currentStreak", "longestStreak", "lastCheckinAt"],
        "HabitStreak",
    );
}

// ─── 12/18 — coach_wire / CoachReviewReadyPayload ────────────────────────────

#[test]
fn v3_coach_wire_schema_stable() {
    use phantom_mesh::coach_wire::CoachReviewReadyPayload;
    const SAMPLE: &str = r#"{"reviewId":"01890000-0000-7000-8000-000000000001","eventId":"01890000-0000-7000-8000-000000000001","date":"2026-05-25","takeawaysCount":4,"markdownPath":"/home/user/.phantom-mesh/coach/2026-05-25.md.age"}"#;
    assert_reserialize_stable::<CoachReviewReadyPayload>(SAMPLE);
    let ts = read_ts("coach", "CoachReviewReadyPayload");
    assert_ts_has_fields(
        &ts,
        &["reviewId", "eventId", "date", "takeawaysCount", "markdownPath"],
        "CoachReviewReadyPayload",
    );
}

// ─── 13/18 — coach_delivery_wire / DeliveryReceipt ───────────────────────────

#[test]
fn v3_coach_delivery_wire_schema_stable() {
    use phantom_mesh::coach_delivery_wire::DeliveryReceipt;
    // `DeliveryChannel` + `DeliveryStatus` both serialise lowercase
    // (`"markdown"`, `"sent"`) per the SPEC-24 §7 wire table.
    const SAMPLE: &str = r#"{"reviewId":"01890000-0000-7000-8000-000000000002","channel":"markdown","attemptedAtMs":1700000000000,"status":"sent","errorMessage":null}"#;
    assert_reserialize_stable::<DeliveryReceipt>(SAMPLE);
    let ts = read_ts("coach_delivery", "DeliveryReceipt");
    assert_ts_has_fields(
        &ts,
        &["reviewId", "channel", "attemptedAtMs", "status", "errorMessage"],
        "DeliveryReceipt",
    );
}

// ─── 14/18 — skill_wire / SkillExample ───────────────────────────────────────

#[test]
fn v3_skill_wire_schema_stable() {
    use phantom_mesh::skill_wire::SkillExample;
    const SAMPLE: &str = r#"{"eventIdHash":"abcdef0123456789","redactedSnippet":"[redacted] morning routine started"}"#;
    assert_reserialize_stable::<SkillExample>(SAMPLE);
    let ts = read_ts("skill", "SkillExample");
    assert_ts_has_fields(&ts, &["eventIdHash", "redactedSnippet"], "SkillExample");
}

// ─── 15/18 — cluster_dispatch_wire / CapabilityTag ───────────────────────────

#[test]
fn v3_cluster_dispatch_wire_schema_stable() {
    use phantom_mesh::cluster_dispatch_wire::CapabilityTag;
    const SAMPLE: &str = r#"{"slug":"role-coder","value":"rust"}"#;
    assert_reserialize_stable::<CapabilityTag>(SAMPLE);
    let ts = read_ts("cluster_dispatch", "CapabilityTag");
    assert_ts_has_fields(&ts, &["slug", "value"], "CapabilityTag");
}

// ─── 16/18 — smart_decompose_wire / ExecutionProgress ────────────────────────

#[test]
fn v3_smart_decompose_wire_schema_stable() {
    use phantom_mesh::smart_decompose_wire::ExecutionProgress;
    // `DecomposeStatus` uses snake_case → wire value `"running"`.
    const SAMPLE: &str = r#"{"parentTaskId":"01890000-0000-7000-8000-000000000003","completedSubtasks":2,"totalSubtasks":5,"failedSubtasks":0,"currentStatus":"running"}"#;
    assert_reserialize_stable::<ExecutionProgress>(SAMPLE);
    let ts = read_ts("smart_decompose", "ExecutionProgress");
    assert_ts_has_fields(
        &ts,
        &["parentTaskId", "completedSubtasks", "totalSubtasks", "failedSubtasks", "currentStatus"],
        "ExecutionProgress",
    );
}

// ─── 17/18 — onboarding_wire / TTFRMetric ────────────────────────────────────

#[test]
fn v3_onboarding_wire_schema_stable() {
    use phantom_mesh::onboarding_wire::TTFRMetric;
    const SAMPLE: &str = r#"{"installCompleteAtMs":1700000000000,"firstReplyAtMs":1700000012500,"totalMs":12500}"#;
    assert_reserialize_stable::<TTFRMetric>(SAMPLE);
    let ts = read_ts("onboarding", "TTFRMetric");
    assert_ts_has_fields(&ts, &["installCompleteAtMs", "firstReplyAtMs", "totalMs"], "TTFRMetric");
}

// ─── 18/18 — release_pipeline_wire / ReleaseArtifact ─────────────────────────

#[test]
fn v3_release_pipeline_wire_schema_stable() {
    use phantom_mesh::release_pipeline_wire::ReleaseArtifact;
    // `ArtifactOs` + `ArtifactArch` ts-rs emit PascalCase variants
    // (`"Macos"`, `"Aarch64"`) per the generated `.ts` enums.
    const SAMPLE: &str = r#"{"os":"Macos","arch":"Aarch64","fileName":"phantom-mesh-0.6.0-darwin-aarch64.dmg","sha256Hex":"0000000000000000000000000000000000000000000000000000000000000000","sizeBytes":50000000,"signatureUrl":null,"downloadUrl":"https://example.invalid/phantom-mesh.dmg"}"#;
    assert_reserialize_stable::<ReleaseArtifact>(SAMPLE);
    let ts = read_ts("release_pipeline", "ReleaseArtifact");
    assert_ts_has_fields(
        &ts,
        &["os", "arch", "fileName", "sha256Hex", "sizeBytes", "signatureUrl", "downloadUrl"],
        "ReleaseArtifact",
    );
}

// ─── Forward-compat boundary probe ───────────────────────────────────────────

/// Documents the *current* v0.6.0 contract: none of the 18 wire structs use
/// `#[serde(deny_unknown_fields)]`, so an extra JSON field is silently
/// tolerated on parse and dropped on re-serialise. The expectation here is
/// "extra field disappears after a round trip" — if a future spec wave flips
/// `deny_unknown_fields` on `PingResponse`, this test will fail at the parse
/// step and force the change to be reviewed across every consumer.
#[test]
fn v3_forward_compat_unknown_field_tolerated() {
    use phantom_mesh::rpc_wire::PingResponse;
    const SAMPLE_WITH_EXTRA: &str = r#"{"peerName":"alpha","os":"mac","version":"0.6.0","capabilities":[],"clusterFingerprint":"abcd1234","uptimeS":1,"futureFieldNotYetDefined":"ignored-by-v0.6.0"}"#;
    let parsed: PingResponse =
        serde_json::from_str(SAMPLE_WITH_EXTRA).expect("v0.6.0 must tolerate unknown fields");
    let reserialised = serde_json::to_string(&parsed).expect("serialize back");
    assert!(
        !reserialised.contains("futureFieldNotYetDefined"),
        "unknown field must be dropped on re-serialise (not echoed back)"
    );
    // Round-trip stable AFTER the unknown field is dropped.
    let again: PingResponse =
        serde_json::from_str(&reserialised).expect("post-drop reparse");
    assert_eq!(parsed.peer_name, again.peer_name);
    assert_eq!(parsed.uptime_s, again.uptime_s);
}
