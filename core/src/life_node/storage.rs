//! Plain-JSON event storage layout. Each event lives under
//! `~/.phantom-mesh/events/<uuid>/` and contains:
//!   - `meta.json` — on-disk event metadata (kind, ts, goal_tags,
//!     source_node, modality_files, user_text). The on-disk shape is the
//!     v0.5.x layout and is kept stable for backward compatibility, but
//!     it is NOT surfaced publicly — `EventStore::write_event` and
//!     `EventStore::read_meta` project it to the SPEC-16 wire type
//!     `event_storage_wire::EventMeta` (slimmer: `{event_id, timestamp,
//!     kind: EventKind, tags}`).
//!   - `modality_<idx>.<ext>` — raw image/audio bytes (one file per
//!     non-text modality)
//!   - `analysis.json` — `AnalysisResult` once provider returns
//!
//! E004 (Plan A2) wraps `EncryptingWriter`/`DecryptingReader` around the
//! file I/O; the on-disk layout is stable across that.
//!
//! Phase G migration (Wave G1, 2026-05-26): the legacy `pub struct
//! EventMeta` was retired in favour of the wire type. The on-disk schema
//! is preserved via the private `OnDiskEventMeta` struct (same serde
//! shape) so already-written `meta.json` files keep loading.

use crate::event_storage_wire::{EventKind, EventMeta};
use crate::life_node::crypto::{decrypt_any, encrypt, looks_like_age};
use crate::life_node::key_derivation::EventKey;
use crate::life_node::multimodal::{AnalysisResult, Modality};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

// ─── On-disk metadata shape (private — schema-stable across v0.5/v0.6) ─────────
//
// This struct is the byte-for-byte equivalent of the retired
// `pub struct EventMeta`. It stays module-private so external callers can't
// depend on the fields the wire shape does not surface
// (`source_node` / `modality_files` / `user_text`). The wire shape is the
// single public type — see `event_storage_wire::EventMeta`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct OnDiskEventMeta {
    event_id: String,
    kind: String,        // "food_log" / "focus_session" / etc — free-form (v0.5.x)
    timestamp: String,   // ISO-8601
    source_node: String, // peer name from agents.toml [cluster].node_name
    goal_tags: Vec<String>,
    modality_files: Vec<String>, // ["modality_0.jpeg", "modality_1.wav"] — text not stored
    user_text: Option<String>,   // text modality stored inline in meta
}

// ─── Projection: on-disk shape → wire shape (replaces the retired From impl) ──
//
// Maps the v0.5.x free-form `kind: String` to the SPEC-16 `EventKind` enum.
// The 3 first-class capture flows (`food_log` / `focus_session` / `habit_log`,
// plus their short aliases `food` / `focus` / `habit`) map directly; every
// other string falls back to `EventKind::Text` — the catch-all variant per
// SPEC-16 §8. Fields not present on the wire shape (`source_node`,
// `modality_files`, `user_text`) are dropped at projection time; they remain
// on disk for forensic / migration tooling.
fn project_to_wire(disk: OnDiskEventMeta) -> EventMeta {
    let kind = match disk.kind.as_str() {
        "food_log" | "food" => EventKind::Food,
        "focus_session" | "focus" => EventKind::Focus,
        "habit_log" | "habit" => EventKind::Habit,
        "dispatch" => EventKind::Dispatch,
        _ => EventKind::Text,
    };
    EventMeta {
        event_id: disk.event_id,
        timestamp: disk.timestamp,
        kind,
        tags: disk.goal_tags,
    }
}

pub struct EventStore {
    root: PathBuf,
    /// When `Some`, all writes are age-encrypted and all reads probe for
    /// age magic (decrypt) or fall back to plaintext (legacy migration).
    key: Option<Arc<EventKey>>,
    /// Optional pre-unification (2026-05-30) EventKey, tried as a DECRYPT
    /// fallback so events written under the old life_node derivation still
    /// decrypt without data loss. Populated only by `with_identity_file`.
    /// Never used for writes (new writes always use the canonical `key`).
    legacy_key: Option<Arc<EventKey>>,
}

impl EventStore {
    /// Plaintext store — for tests and pre-encryption migration. Reads
    /// still transparently handle encrypted files if the directory was
    /// touched by a `with_key` instance, but writes will be plaintext.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            key: None,
            legacy_key: None,
        }
    }

    /// Encrypted store — production constructor. Caller provides an
    /// already-derived `EventKey` (typically via
    /// `key_derivation::load_event_key(home.join("identity.key"))`).
    pub fn with_key(root: impl Into<PathBuf>, key: EventKey) -> Self {
        Self {
            root: root.into(),
            key: Some(Arc::new(key)),
            legacy_key: None,
        }
    }

    /// Encrypted store WITH legacy-key decrypt fallback. Loads BOTH the
    /// canonical and the pre-unification legacy `EventKey` from `identity_path`,
    /// so events written under EITHER derivation decrypt (zero data loss across
    /// the 2026-05-30 EventKey unification). New writes use the canonical key
    /// only. A missing/short identity.key degrades to a keyless (plaintext) store.
    pub fn with_identity_file(root: impl Into<PathBuf>, identity_path: &Path) -> Self {
        let key = crate::life_node::key_derivation::load_event_key(identity_path).ok();
        let legacy = crate::life_node::key_derivation::load_event_key_legacy(identity_path).ok();
        Self {
            root: root.into(),
            key: key.map(Arc::new),
            legacy_key: legacy.map(Arc::new),
        }
    }

    /// Write `bytes` to `path`. If a key is set, encrypt to age v1 binary
    /// format first; otherwise write the bytes verbatim.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let payload = match &self.key {
            None => bytes.to_vec(),
            Some(k) => encrypt(bytes, k.as_ref()).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, format!("encrypt: {}", e))
            })?,
        };
        std::fs::write(path, payload)
    }

    /// Read a file. If its first bytes look like age v1 magic, decrypt
    /// with `self.key`; otherwise return as-is (legacy plaintext support).
    /// Returns InvalidData if the file is encrypted but no key is set.
    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let raw = std::fs::read(path)?;
        if looks_like_age(&raw) {
            // Try the canonical key first, then the pre-unification legacy key
            // (zero data loss across the 2026-05-30 EventKey unification). age
            // selects the matching recipient internally.
            let mut keys: Vec<&EventKey> = Vec::new();
            if let Some(k) = self.key.as_ref() {
                keys.push(k.as_ref());
            }
            if let Some(k) = self.legacy_key.as_ref() {
                keys.push(k.as_ref());
            }
            if keys.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "encrypted file but EventStore has no key",
                ));
            }
            decrypt_any(&raw, &keys).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, format!("decrypt: {}", e))
            })
        } else {
            Ok(raw)
        }
    }

    /// Write a new event. Persists the v0.5.x on-disk shape (including the
    /// per-modality files + inline text) and returns the SPEC-16 wire-shape
    /// `EventMeta` projection.
    pub fn write_event(
        &self,
        kind: &str,
        modalities: &[Modality],
        goal_tags: &[String],
        source_node: &str,
    ) -> std::io::Result<EventMeta> {
        let id = Uuid::new_v4().to_string();
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir)?;

        let mut user_text: Option<String> = None;
        let mut modality_files: Vec<String> = Vec::new();
        for (i, m) in modalities.iter().enumerate() {
            match m {
                Modality::Image { bytes, mime } => {
                    let ext = match mime.as_str() {
                        "image/jpeg" => "jpeg",
                        "image/png" => "png",
                        "image/webp" => "webp",
                        _ => "bin",
                    };
                    let name = format!("modality_{}.{}", i, ext);
                    self.write_file(&dir.join(&name), bytes)?;
                    modality_files.push(name);
                }
                Modality::Audio { bytes, mime } => {
                    let ext = match mime.as_str() {
                        "audio/wav" => "wav",
                        "audio/mp3" | "audio/mpeg" => "mp3",
                        "audio/ogg" => "ogg",
                        _ => "bin",
                    };
                    let name = format!("modality_{}.{}", i, ext);
                    self.write_file(&dir.join(&name), bytes)?;
                    modality_files.push(name);
                }
                Modality::Text(s) => {
                    // Combine multiple text modalities by concatenation.
                    user_text = Some(match user_text {
                        Some(prev) => format!("{}\n{}", prev, s),
                        None => s.clone(),
                    });
                }
            }
        }

        let disk = OnDiskEventMeta {
            event_id: id.clone(),
            kind: kind.to_string(),
            // SPEC-16 T-STOR-01: store UTC (`+00:00`), never local time. Local
            // strings made per-day queries timezone-ambiguous; readers convert
            // back to the user's local date at query time via
            // `event_storage_wire::ts_local_date`.
            timestamp: chrono::Utc::now().to_rfc3339(),
            source_node: source_node.to_string(),
            goal_tags: goal_tags.to_vec(),
            modality_files,
            user_text,
        };
        self.write_file(&dir.join("meta.json"), &serde_json::to_vec_pretty(&disk)?)?;
        Ok(project_to_wire(disk))
    }

    /// Resolve an event's on-disk directory, rejecting ids that could escape
    /// the events root via path traversal. Real ids are `Uuid`s (server-
    /// generated in `write_event`); any id with a path separator, `..`, or
    /// that is empty is hostile input from a `:id` route param (e.g. the
    /// unauthenticated `GET /api/events/:id/analysis`, where axum percent-
    /// decodes `..%2F..` into `../..` *after* routing) and is refused before
    /// it reaches the filesystem — `root.join("../../secret")` would otherwise
    /// read outside `~/.phantom-mesh/events`.
    fn event_dir(&self, event_id: &str) -> std::io::Result<PathBuf> {
        if event_id.is_empty()
            || event_id.contains('/')
            || event_id.contains('\\')
            || event_id.contains("..")
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid event id",
            ));
        }
        Ok(self.root.join(event_id))
    }

    /// Read meta from disk and project to the SPEC-16 wire shape. Fields
    /// not on the wire surface (`source_node`, `modality_files`,
    /// `user_text`) are dropped; the on-disk file itself is untouched.
    pub fn read_meta(&self, event_id: &str) -> std::io::Result<EventMeta> {
        let p = self.event_dir(event_id)?.join("meta.json");
        let b = self.read_file(&p)?;
        let disk: OnDiskEventMeta = serde_json::from_slice(&b)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(project_to_wire(disk))
    }

    pub fn write_analysis(&self, event_id: &str, result: &AnalysisResult) -> std::io::Result<()> {
        let p = self.event_dir(event_id)?.join("analysis.json");
        self.write_file(&p, &serde_json::to_vec_pretty(result)?)?;
        Ok(())
    }

    pub fn read_analysis(&self, event_id: &str) -> std::io::Result<AnalysisResult> {
        let p = self.event_dir(event_id)?.join("analysis.json");
        let b = self.read_file(&p)?;
        serde_json::from_slice(&b)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::multimodal::Modality;
    use tempfile::TempDir;

    #[test]
    fn write_event_creates_dir_with_meta_and_modality_files() {
        let dir = TempDir::new().unwrap();
        let store = EventStore::new(dir.path());
        let m = store
            .write_event(
                "food_log",
                &[
                    Modality::Image {
                        bytes: vec![1, 2, 3],
                        mime: "image/jpeg".into(),
                    },
                    Modality::Audio {
                        bytes: vec![4, 5, 6, 7],
                        mime: "audio/wav".into(),
                    },
                    Modality::Text("lunch — chicken bento".into()),
                ],
                &["fat_loss".to_string()],
                "node-a",
            )
            .unwrap();
        // Wire-shape assertions: kind enum bridged from "food_log", tags
        // mirror the goal_tags input.
        assert_eq!(m.kind, EventKind::Food);
        assert_eq!(m.tags, vec!["fat_loss".to_string()]);

        // On-disk shape still carries the legacy fields (source_node /
        // modality_files / user_text). Read the raw plaintext JSON back
        // and check via the private `OnDiskEventMeta` projection.
        let event_dir = dir.path().join(&m.event_id);
        assert!(event_dir.join("meta.json").exists());
        assert!(event_dir.join("modality_0.jpeg").exists());
        assert!(event_dir.join("modality_1.wav").exists());

        let img = std::fs::read(event_dir.join("modality_0.jpeg")).unwrap();
        assert_eq!(img, vec![1, 2, 3]);

        let raw = std::fs::read(event_dir.join("meta.json")).unwrap();
        let disk: OnDiskEventMeta = serde_json::from_slice(&raw).unwrap();
        assert_eq!(disk.kind, "food_log");
        assert_eq!(disk.source_node, "node-a");
        assert_eq!(disk.modality_files, vec!["modality_0.jpeg", "modality_1.wav"]);
        assert_eq!(disk.user_text.as_deref(), Some("lunch — chicken bento"));
    }

    #[test]
    fn read_meta_round_trip() {
        let dir = TempDir::new().unwrap();
        let store = EventStore::new(dir.path());
        let m = store
            .write_event(
                "focus_session",
                &[Modality::Text("notes".into())],
                &[],
                "node-b",
            )
            .unwrap();
        let m2 = store.read_meta(&m.event_id).unwrap();
        // Wire-shape PartialEq is field-by-field (event_id / timestamp /
        // kind / tags) — same projection, same bytes.
        assert_eq!(m2.event_id, m.event_id);
        assert_eq!(m2.timestamp, m.timestamp);
        assert_eq!(m2.kind, m.kind);
        assert_eq!(m2.tags, m.tags);
    }

    #[test]
    fn write_then_read_analysis() {
        let dir = TempDir::new().unwrap();
        let store = EventStore::new(dir.path());
        let m = store
            .write_event("food_log", &[Modality::Text("x".into())], &[], "node-a")
            .unwrap();
        let result = AnalysisResult {
            summary: "test summary".into(),
            goal_impact: None,
            suggestion: None,
            confidence: None,
            raw_response: serde_json::json!({}),
            model_id: "gemini-2.5-flash".into(),
            latency_ms: 100,
            cost_usd: None,
        };
        store.write_analysis(&m.event_id, &result).unwrap();
        let back = store.read_analysis(&m.event_id).unwrap();
        assert_eq!(back.summary, "test summary");
        assert_eq!(back.model_id, "gemini-2.5-flash");
    }

    // ── E004 F303: encrypted-store tests ─────────────────────────────────────

    use crate::life_node::key_derivation::derive_event_key;

    #[test]
    fn encrypted_store_writes_age_format_on_disk() {
        let dir = TempDir::new().unwrap();
        let key = derive_event_key(&[0x42u8; 64]).unwrap();
        let store = EventStore::with_key(dir.path(), key);
        let m = store
            .write_event(
                "food_log",
                &[Modality::Text("private note".into())],
                &[],
                "test-node",
            )
            .unwrap();
        let meta_bytes = std::fs::read(dir.path().join(&m.event_id).join("meta.json")).unwrap();
        assert!(
            looks_like_age(&meta_bytes),
            "meta.json must be age-encrypted; first bytes = {:?}",
            &meta_bytes[..meta_bytes.len().min(32)]
        );
        // A plain serde_json parse of the ciphertext must fail — the on-disk
        // payload is not JSON anymore (probe via the private OnDiskEventMeta).
        assert!(serde_json::from_slice::<OnDiskEventMeta>(&meta_bytes).is_err());
    }

    /// P2 (multimodal_roundtrip) data-integrity invariant — modality bytes
    /// written through the encrypted store come back byte-identical. Uses
    /// binary-unsafe payloads (NUL, 0xFF/0xFE, a PNG-ish header) so any
    /// re-encoding / truncation would be caught. Reads back via the private
    /// `read_file` decrypt path (in-module test), and confirms the on-disk
    /// bytes are actually age-encrypted (≠ plaintext) and meta tags survive.
    #[test]
    fn multimodal_event_round_trips_byte_identical() {
        use crate::life_node::key_derivation::derive_event_key;

        let dir = TempDir::new().unwrap();
        let key = derive_event_key(&[0x5Au8; 64]).unwrap();
        let store = EventStore::with_key(dir.path(), key);

        let img: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x00, 0xFF, 0xFE, 0x01, 0x02];
        let aud: Vec<u8> = vec![0x00, 0x01, 0xFF, 0x7F, 0x80, 0xAA, 0x55, 0x00];

        let m = store
            .write_event(
                "food_log",
                &[
                    Modality::Image { bytes: img.clone(), mime: "image/png".into() },
                    Modality::Audio { bytes: aud.clone(), mime: "audio/wav".into() },
                    Modality::Text("水 multimodal note\u{0}end".into()),
                ],
                &["fat_loss".to_string(), "水".to_string()],
                "node-mm",
            )
            .unwrap();

        let ev = dir.path().join(&m.event_id);

        // On-disk modality files are age-encrypted, not the raw bytes.
        let raw_img = std::fs::read(ev.join("modality_0.png")).unwrap();
        assert!(looks_like_age(&raw_img), "image stored encrypted");
        assert_ne!(raw_img, img, "on-disk image must differ from plaintext");

        // Byte-for-byte round-trip through the decrypt read path.
        let got_img = store.read_file(&ev.join("modality_0.png")).unwrap();
        assert_eq!(got_img, img, "image bytes must round-trip byte-identical");
        let got_aud = store.read_file(&ev.join("modality_1.wav")).unwrap();
        assert_eq!(got_aud, aud, "audio bytes must round-trip byte-identical");

        // Meta survives the encrypt→disk→decrypt round-trip.
        let meta = store.read_meta(&m.event_id).unwrap();
        assert_eq!(
            meta.tags,
            vec!["fat_loss".to_string(), "水".to_string()],
            "tags preserved through round-trip"
        );
    }

    /// G3 / MAC-INV-P4-003 (P4 core privacy invariant) — neither the raw
    /// identity-key IKM nor the derived EventKey may EVER appear in plaintext
    /// anywhere under the events dir. The mac.md row claimed this with a shell
    /// `grep -r` but had NO real test; this pins it hermetically.
    ///
    /// We write through the FULL modality set (text + image + audio), then walk
    /// every file on disk and assert the secret byte windows occur 0 times.
    #[test]
    fn no_key_material_leaks_into_events_dir() {
        use crate::life_node::key_derivation::derive_event_key;

        let dir = TempDir::new().unwrap();
        // Distinctive IKM so a leak can't be confused with incidental bytes.
        let ikm = [0xABu8; 64];
        let key = derive_event_key(&ikm).unwrap();
        // Capture the derived 32-byte EventKey BEFORE moving it into the store.
        let event_key_bytes: Vec<u8> = key.as_bytes().to_vec();

        let store = EventStore::with_key(dir.path(), key);
        store
            .write_event(
                "food_log",
                &[
                    Modality::Text("私密午餐紀錄 — 雞肉便當 + 祕密筆記".into()),
                    Modality::Image { bytes: vec![9, 8, 7, 6, 5], mime: "image/jpeg".into() },
                    Modality::Audio { bytes: vec![1, 1, 2, 3, 5, 8], mime: "audio/wav".into() },
                ],
                &["fat_loss".to_string()],
                "node-secret",
            )
            .unwrap();

        // Collect every byte of every file under the events dir.
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if let Ok(b) = std::fs::read(&p) {
                        out.push((p, b));
                    }
                }
            }
        }
        fn contains_window(hay: &[u8], needle: &[u8]) -> bool {
            !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
        }

        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        walk(dir.path(), &mut files);
        assert!(!files.is_empty(), "no files written under events dir");

        for (path, bytes) in &files {
            assert!(
                !contains_window(bytes, &ikm),
                "identity IKM (64B) leaked into {}",
                path.display()
            );
            assert!(
                !contains_window(bytes, &event_key_bytes),
                "derived EventKey (32B) leaked into {}",
                path.display()
            );
            // Also catch a partial-prefix leak (mac.md row grepped only the
            // first 16 bytes of identity.key — match that intent too).
            assert!(
                !contains_window(bytes, &ikm[..16]),
                "identity IKM 16-byte prefix leaked into {}",
                path.display()
            );
        }
    }

    #[test]
    fn encrypted_store_round_trip_via_read_meta() {
        let dir = TempDir::new().unwrap();
        let key = derive_event_key(&[0x42u8; 64]).unwrap();
        let store = EventStore::with_key(dir.path(), key);
        let m = store
            .write_event(
                "focus_session",
                &[Modality::Text("48-min deep work".into())],
                &["focus".to_string()],
                "test-node",
            )
            .unwrap();
        let recovered = store.read_meta(&m.event_id).unwrap();
        // Wire-shape: `focus_session` bridges to `EventKind::Focus`; tags
        // survive. The `user_text` field is no longer surfaced (lives on
        // disk only); read it back via the private struct if needed.
        assert_eq!(recovered.kind, EventKind::Focus);
        assert_eq!(recovered.tags, vec!["focus".to_string()]);
        assert_eq!(recovered.event_id, m.event_id);
    }

    #[test]
    fn migration_reads_old_plaintext_writes_new_encrypted() {
        let dir = TempDir::new().unwrap();
        // 1. Write a plaintext event via the legacy `new(root)` constructor.
        let legacy = EventStore::new(dir.path());
        let m = legacy
            .write_event(
                "food_log",
                &[Modality::Text("legacy lunch".into())],
                &["fat_loss".to_string()],
                "legacy-node",
            )
            .unwrap();
        // 2. Open the same root with an encrypted store.
        let key = derive_event_key(&[0x42u8; 64]).unwrap();
        let encrypted = EventStore::with_key(dir.path(), key);
        // 3. The encrypted store must still read the legacy plaintext event
        //    (looks_like_age returns false → fall through to raw bytes).
        let recovered = encrypted.read_meta(&m.event_id).unwrap();
        assert_eq!(recovered.event_id, m.event_id);
        assert_eq!(recovered.kind, EventKind::Food);
        assert_eq!(recovered.tags, vec!["fat_loss".to_string()]);
        // 4. A NEW event written via the encrypted store must be age-encrypted
        //    on disk — this is the "migration writes new format" half.
        let key2 = derive_event_key(&[0x42u8; 64]).unwrap();
        let store2 = EventStore::with_key(dir.path(), key2);
        let m2 = store2
            .write_event(
                "food_log",
                &[Modality::Text("new lunch".into())],
                &[],
                "new-node",
            )
            .unwrap();
        let new_bytes = std::fs::read(dir.path().join(&m2.event_id).join("meta.json")).unwrap();
        assert!(
            looks_like_age(&new_bytes),
            "post-migration meta.json must be encrypted"
        );
    }

    #[test]
    fn encrypted_store_image_modality_is_encrypted_on_disk() {
        let dir = TempDir::new().unwrap();
        let key = derive_event_key(&[0x42u8; 64]).unwrap();
        let store = EventStore::with_key(dir.path(), key);
        let raw_jpeg = vec![0xffu8, 0xd8, 0xff, 0xd9]; // JPEG SOI/EOI
        let m = store
            .write_event(
                "food_log",
                &[Modality::Image {
                    bytes: raw_jpeg.clone(),
                    mime: "image/jpeg".into(),
                }],
                &[],
                "test-node",
            )
            .unwrap();
        let on_disk = std::fs::read(dir.path().join(&m.event_id).join("modality_0.jpeg")).unwrap();
        assert!(looks_like_age(&on_disk), "modality bytes must be encrypted");
        assert_ne!(
            on_disk, raw_jpeg,
            "raw modality bytes must not leak to disk"
        );
    }

    #[test]
    fn plaintext_store_rejects_encrypted_file_with_clear_error() {
        let dir = TempDir::new().unwrap();
        // First, write an encrypted event.
        let key = derive_event_key(&[0x42u8; 64]).unwrap();
        let encrypted = EventStore::with_key(dir.path(), key);
        let m = encrypted
            .write_event("food_log", &[Modality::Text("secret".into())], &[], "n")
            .unwrap();
        // Then try to read it with a plaintext-only store.
        let plain = EventStore::new(dir.path());
        let err = plain.read_meta(&m.event_id).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string()
                .contains("encrypted file but EventStore has no key"),
            "error should explain key is missing; got: {}",
            err
        );
    }

    // ── G1 migration: legacy → wire projection ───────────────────────────────

    #[test]
    fn project_to_wire_maps_known_kinds() {
        let mk = |k: &str| OnDiskEventMeta {
            event_id: "id".into(),
            kind: k.into(),
            timestamp: "2026-05-26T00:00:00Z".into(),
            source_node: "n".into(),
            goal_tags: vec!["t".into()],
            modality_files: vec![],
            user_text: None,
        };
        assert_eq!(project_to_wire(mk("food_log")).kind, EventKind::Food);
        assert_eq!(project_to_wire(mk("food")).kind, EventKind::Food);
        assert_eq!(project_to_wire(mk("focus_session")).kind, EventKind::Focus);
        assert_eq!(project_to_wire(mk("focus")).kind, EventKind::Focus);
        assert_eq!(project_to_wire(mk("habit_log")).kind, EventKind::Habit);
        assert_eq!(project_to_wire(mk("habit")).kind, EventKind::Habit);
        // Cross-node dispatch events (written by `phantom dispatch`) project to
        // the first-class Dispatch kind so they're filterable via recall --kind.
        assert_eq!(project_to_wire(mk("dispatch")).kind, EventKind::Dispatch);
        // Unknown / empty → Text fallback per SPEC-16 §8 catch-all.
        assert_eq!(project_to_wire(mk("totally_unknown")).kind, EventKind::Text);
        assert_eq!(project_to_wire(mk("")).kind, EventKind::Text);
    }

    #[test]
    fn project_to_wire_preserves_id_timestamp_and_tags() {
        let disk = OnDiskEventMeta {
            event_id: "01890000-0000-7000-8000-000000000001".into(),
            kind: "food_log".into(),
            timestamp: "2026-05-26T12:00:00+08:00".into(),
            source_node: "node-a".into(),
            goal_tags: vec!["fat_loss".into(), "habit:water".into()],
            modality_files: vec!["modality_0.jpeg".into()],
            user_text: Some("oatmeal".into()),
        };
        let wire = project_to_wire(disk);
        assert_eq!(wire.event_id, "01890000-0000-7000-8000-000000000001");
        assert_eq!(wire.timestamp, "2026-05-26T12:00:00+08:00");
        assert_eq!(wire.tags, vec!["fat_loss".to_string(), "habit:water".into()]);
    }

    #[test]
    fn read_rejects_path_traversal_event_ids() {
        // Regression: `GET /api/events/:id/analysis` (unauthenticated) passes
        // the percent-decoded `:id` straight to `read_analysis`. A traversal
        // id like `../../secret` must be refused before `root.join(id)` can
        // escape the events root — not used to read an arbitrary file.
        let dir = TempDir::new().unwrap();
        // Plant a file one level ABOVE the events root that an escape would hit.
        std::fs::write(dir.path().join("analysis.json"), b"{\"summary\":\"x\"}").unwrap();
        let root = dir.path().join("events");
        std::fs::create_dir_all(&root).unwrap();
        let store = EventStore::new(&root);

        for bad in [
            "..",
            "../analysis-dir",        // ../<root-sibling>/analysis.json
            "../../etc",
            "a/../../b",
            "sub/dir",
            "back\\slash",
            "",
        ] {
            let e = store.read_analysis(bad).unwrap_err();
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::InvalidInput,
                "traversal id {:?} must be rejected as InvalidInput, not read off-root",
                bad
            );
            assert_eq!(store.read_meta(bad).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(
                store
                    .write_analysis(bad, &AnalysisResult {
                        summary: "x".into(),
                        goal_impact: None,
                        suggestion: None,
                        confidence: None,
                        raw_response: serde_json::json!({}),
                        model_id: "m".into(),
                        latency_ms: 0,
                        cost_usd: None,
                    })
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }

        // A legitimate UUID id is NOT rejected by the guard: it passes
        // validation and fails only because the event doesn't exist (NotFound),
        // proving the guard doesn't break the real server-generated id shape.
        let uuid = Uuid::new_v4().to_string();
        assert_eq!(
            store.read_analysis(&uuid).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
