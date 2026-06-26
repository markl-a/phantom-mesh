//! Flight-recorder: every governed-run event + governance decision is appended to
//! a RunRecorder. Production records governance to the S0 task EventStore
//! (replayable) and the raw event stream to a per-run jsonl transcript; tests use
//! MemRecorder.

use crate::cli_session::event::CliEvent;
use crate::execution_contract::{ContractState, RiskLevel};
use crate::life_node::key_derivation::{EventKey, load_event_key};
use crate::tasks::events::{EventStore, TaskEventKind};
use hmac::{Hmac, Mac};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::io::Write as _;
use subtle::ConstantTimeEq;
use tokio::runtime::Handle;
use uuid::Uuid;
use zeroize::Zeroize;

const TRANSCRIPT_MAC_LABEL: &[u8] = b"phantom-mesh.governed-run.transcript-hmac-v1";
const TRANSCRIPT_LINE_DOMAIN: &[u8] = b"phantom-mesh.governed-run.transcript-line-v1";
const TRANSCRIPT_ALG: &str = "HKDF-SHA256+HMAC-SHA256";
const TRANSCRIPT_VERSION: u8 = 1;
const ZERO_MAC_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000000";

type HmacSha256 = Hmac<Sha256>;

/// One append-only signed transcript line. `event` is the exact compact JSON
/// bytes that were MACed; keeping it as a string avoids verifier-dependent JSON
/// re-serialization.
#[derive(Debug, Serialize, Deserialize)]
struct SignedTranscriptLine {
    v: u8,
    alg: String,
    seq: u64,
    prev: String,
    event: String,
    hmac: String,
}

#[derive(Clone)]
struct TranscriptMacKey {
    bytes: [u8; 32],
}

impl TranscriptMacKey {
    fn derive(event_key: &EventKey) -> Result<Self, String> {
        let hk = Hkdf::<Sha256>::new(None, event_key.as_bytes());
        let mut bytes = [0u8; 32];
        hk.expand(TRANSCRIPT_MAC_LABEL, &mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(Self { bytes })
    }
}

impl Drop for TranscriptMacKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Debug)]
struct TranscriptVerifyState {
    events: Vec<CliEvent>,
    next_seq: u64,
    last_mac_hex: String,
}

#[derive(thiserror::Error, Debug)]
pub enum TranscriptVerifyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("key derivation: {0}")]
    Key(String),
    #[error("line {line}: malformed signed transcript JSON: {source}")]
    Json { line: usize, source: serde_json::Error },
    #[error("line {line}: unsupported transcript signature envelope")]
    BadEnvelope { line: usize },
    #[error("line {line}: sequence mismatch, expected {expected}, got {got}")]
    BadSequence { line: usize, expected: u64, got: u64 },
    #[error("line {line}: previous-MAC mismatch")]
    BadPreviousMac { line: usize },
    #[error("line {line}: invalid HMAC hex")]
    BadMacHex { line: usize },
    #[error("line {line}: HMAC mismatch")]
    BadMac { line: usize },
    #[error("line {line}: signed event JSON does not decode: {source}")]
    BadEventJson { line: usize, source: serde_json::Error },
}

/// A recorded governance moment (a CliEvent or a governance decision).
#[derive(Clone, Debug, PartialEq)]
pub enum RunRecord {
    Event(CliEvent),
    Governance {
        approval_id: String,
        risk: RiskLevel,
        state: ContractState,
        enforcement: &'static str,
    },
}

pub trait RunRecorder: Send {
    fn record(&mut self, rec: RunRecord);
}

/// In-memory recorder for tests + replay inspection.
#[derive(Default)]
pub struct MemRecorder {
    pub records: Vec<RunRecord>,
}
impl RunRecorder for MemRecorder {
    fn record(&mut self, rec: RunRecord) {
        self.records.push(rec);
    }
}

/// Production recorder: governance moments → the replayable S0 task `EventStore`;
/// raw `CliEvent`s → a per-run append-only jsonl transcript (the full flight
/// recording). Bridges the sync `RunRecorder` trait to the async `EventStore` via
/// a tokio runtime `Handle`, so the `drive` loop MUST run on a blocking
/// (non-worker) thread (see `phantom govern`).
pub struct EventStoreRecorder {
    store: EventStore,
    task_id: Uuid,
    handle: Handle,
    transcript: PathBuf,
    transcript_key: TranscriptMacKey,
    next_seq: u64,
    prev_mac_hex: String,
}

impl EventStoreRecorder {
    /// `transcript_dir` is created if missing; the raw stream lands in
    /// `<transcript_dir>/<task_id>.jsonl`.
    pub fn new(
        store: EventStore,
        task_id: Uuid,
        handle: Handle,
        transcript_dir: PathBuf,
    ) -> std::io::Result<Self> {
        Self::new_with_identity_path(
            store,
            task_id,
            handle,
            transcript_dir,
            crate::identity::root_identity_key_path(),
        )
    }

    pub fn new_with_identity_path(
        store: EventStore,
        task_id: Uuid,
        handle: Handle,
        transcript_dir: PathBuf,
        identity_path: PathBuf,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(&transcript_dir)?;
        let transcript = transcript_dir.join(format!("{task_id}.jsonl"));
        let event_key = ensure_and_load_event_key(&identity_path)?;
        let transcript_key = TranscriptMacKey::derive(&event_key)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let state = if transcript.exists() {
            verify_transcript_with_key(&transcript, &transcript_key).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
            })?
        } else {
            TranscriptVerifyState {
                events: Vec::new(),
                next_seq: 0,
                last_mac_hex: ZERO_MAC_HEX.to_string(),
            }
        };
        Ok(Self {
            store,
            task_id,
            handle,
            transcript,
            transcript_key,
            next_seq: state.next_seq,
            prev_mac_hex: state.last_mac_hex,
        })
    }

    /// Map a contract state onto the EventStore's approval-aware event kind.
    fn kind_for(state: ContractState) -> TaskEventKind {
        match state {
            ContractState::Pending => TaskEventKind::ApprovalRequested,
            ContractState::Approved => TaskEventKind::Approved,
            ContractState::Denied | ContractState::Cancelled | ContractState::Expired => {
                TaskEventKind::Denied
            }
        }
    }

    fn signed_line(&self, event_json: String) -> SignedTranscriptLine {
        let mac = transcript_mac_hex(
            &self.transcript_key,
            self.next_seq,
            &self.prev_mac_hex,
            event_json.as_bytes(),
        );
        SignedTranscriptLine {
            v: TRANSCRIPT_VERSION,
            alg: TRANSCRIPT_ALG.to_string(),
            seq: self.next_seq,
            prev: self.prev_mac_hex.clone(),
            event: event_json,
            hmac: mac,
        }
    }

    fn advance_signed_chain(&mut self, hmac: String) {
        self.next_seq = self.next_seq.saturating_add(1);
        self.prev_mac_hex = hmac;
    }
}

fn ensure_and_load_event_key(identity_path: &Path) -> std::io::Result<EventKey> {
    if !identity_path.exists() {
        let dir = identity_path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("identity path has no parent: {}", identity_path.display()),
            )
        })?;
        crate::identity::ensure_root_identity_key_in(dir)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }
    load_event_key(identity_path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

fn transcript_mac_hex(
    key: &TranscriptMacKey,
    seq: u64,
    prev_mac_hex: &str,
    event_json: &[u8],
) -> String {
    let mut mac =
        HmacSha256::new_from_slice(&key.bytes).expect("HMAC accepts keys of any length");
    mac.update(TRANSCRIPT_LINE_DOMAIN);
    mac.update(&seq.to_be_bytes());
    mac.update(b"\0");
    mac.update(prev_mac_hex.as_bytes());
    mac.update(b"\0");
    mac.update(event_json);
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_transcript(path: &Path) -> Result<Vec<CliEvent>, TranscriptVerifyError> {
    verify_transcript_with_identity(path, &crate::identity::root_identity_key_path())
}

pub fn verify_transcript_with_identity(
    path: &Path,
    identity_path: &Path,
) -> Result<Vec<CliEvent>, TranscriptVerifyError> {
    let event_key = load_event_key(identity_path).map_err(|e| TranscriptVerifyError::Key(e.to_string()))?;
    let transcript_key =
        TranscriptMacKey::derive(&event_key).map_err(TranscriptVerifyError::Key)?;
    Ok(verify_transcript_with_key(path, &transcript_key)?.events)
}

fn verify_transcript_with_key(
    path: &Path,
    key: &TranscriptMacKey,
) -> Result<TranscriptVerifyState, TranscriptVerifyError> {
    let body = std::fs::read_to_string(path)?;
    let mut events = Vec::new();
    let mut expected_seq = 0u64;
    let mut expected_prev = ZERO_MAC_HEX.to_string();

    for (idx, line) in body.lines().enumerate() {
        let line_no = idx + 1;
        if line.trim().is_empty() {
            continue;
        }
        let signed: SignedTranscriptLine =
            serde_json::from_str(line).map_err(|source| TranscriptVerifyError::Json {
                line: line_no,
                source,
            })?;
        if signed.v != TRANSCRIPT_VERSION || signed.alg != TRANSCRIPT_ALG {
            return Err(TranscriptVerifyError::BadEnvelope { line: line_no });
        }
        if signed.seq != expected_seq {
            return Err(TranscriptVerifyError::BadSequence {
                line: line_no,
                expected: expected_seq,
                got: signed.seq,
            });
        }
        if signed.prev != expected_prev {
            return Err(TranscriptVerifyError::BadPreviousMac { line: line_no });
        }

        let expected_mac =
            transcript_mac_hex(key, signed.seq, &signed.prev, signed.event.as_bytes());
        let supplied = hex::decode(&signed.hmac)
            .map_err(|_| TranscriptVerifyError::BadMacHex { line: line_no })?;
        let expected = hex::decode(&expected_mac)
            .map_err(|_| TranscriptVerifyError::BadMacHex { line: line_no })?;
        if supplied.len() != expected.len()
            || supplied.ct_eq(expected.as_slice()).unwrap_u8() != 1
        {
            return Err(TranscriptVerifyError::BadMac { line: line_no });
        }

        let ev: CliEvent = serde_json::from_str(&signed.event).map_err(|source| {
            TranscriptVerifyError::BadEventJson {
                line: line_no,
                source,
            }
        })?;
        events.push(ev);
        expected_prev = signed.hmac;
        expected_seq = expected_seq.saturating_add(1);
    }

    Ok(TranscriptVerifyState {
        events,
        next_seq: expected_seq,
        last_mac_hex: expected_prev,
    })
}

impl RunRecorder for EventStoreRecorder {
    fn record(&mut self, rec: RunRecord) {
        match rec {
            RunRecord::Event(ev) => {
                // Raw flight-recording (best-effort): append one signed jsonl line.
                // P4: route the serialized event through the central redactor so
                // an API key / token captured in a command-execution arg, tool
                // output, or assistant text never reaches disk in CLEAR. Redact
                // BEFORE signing so the HMAC covers the on-disk (redacted) bytes
                // and `verify_transcript` still validates.
                if let Ok(event_json) = serde_json::to_string(&ev) {
                    let event_json = crate::redact::redact(&event_json);
                    let signed = self.signed_line(event_json);
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&self.transcript)
                    {
                        let hmac = signed.hmac.clone();
                        if let Ok(line) = serde_json::to_string(&signed) {
                            if writeln!(f, "{line}").is_ok() {
                                self.advance_signed_chain(hmac);
                            }
                        }
                    }
                }
            }
            RunRecord::Governance { approval_id, risk, state, enforcement } => {
                let kind = Self::kind_for(state);
                let detail = serde_json::json!({
                    "approval_id": approval_id,
                    "risk": risk.as_str(),
                    "enforcement": enforcement,
                })
                .to_string();
                let store = self.store.clone();
                let task_id = self.task_id;
                // Bridge sync -> async. Safe because `drive` runs on a blocking thread.
                self.handle.block_on(async move {
                    let _ = store.append(task_id, kind, Some(&detail)).await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};

    #[test]
    fn mem_recorder_keeps_order() {
        let mut r = MemRecorder::default();
        r.record(RunRecord::Event(CliEvent::new(
            EventKind::SessionStarted { id: "x".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )));
        r.record(RunRecord::Governance {
            approval_id: "a1".into(),
            risk: RiskLevel::ExecuteHigh,
            state: ContractState::Pending,
            enforcement: "pre_action_blocking",
        });
        assert_eq!(r.records.len(), 2);
        assert!(matches!(r.records[1], RunRecord::Governance { .. }));
    }

    #[test]
    fn event_store_recorder_writes_governance_and_transcript() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let raw = rusqlite::Connection::open_in_memory().unwrap();
        EventStore::init_schema(&raw).unwrap();
        let store = EventStore::from_conn(std::sync::Arc::new(tokio::sync::Mutex::new(raw)));
        let task_id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("gr-rec-{task_id}"));
        let identity_path = dir.join("identity.key");
        let mut rec = EventStoreRecorder::new_with_identity_path(
            store.clone(),
            task_id,
            rt.handle().clone(),
            dir.clone(),
            identity_path.clone(),
        )
        .unwrap();

        rec.record(RunRecord::Event(CliEvent::new(
            EventKind::SessionStarted { id: "s".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )));
        rec.record(RunRecord::Governance {
            approval_id: "a1".into(),
            risk: RiskLevel::ExecuteHigh,
            state: ContractState::Pending,
            enforcement: "pre_action_blocking",
        });
        rec.record(RunRecord::Governance {
            approval_id: "a1".into(),
            risk: RiskLevel::ExecuteHigh,
            state: ContractState::Approved,
            enforcement: "pre_action_blocking",
        });

        // Only governance reaches the replayable EventStore, in order.
        let events = rt.block_on(store.events_for(task_id)).unwrap();
        assert_eq!(events.len(), 2, "only governance goes to the EventStore");
        assert_eq!(events[0].kind, TaskEventKind::ApprovalRequested);
        assert_eq!(events[1].kind, TaskEventKind::Approved);

        // The raw event stream is captured in the jsonl transcript.
        let transcript = dir.join(format!("{task_id}.jsonl"));
        let body = std::fs::read_to_string(&transcript).unwrap();
        assert!(body.contains("session_started"), "raw event in transcript: {body}");
        let verified = verify_transcript_with_identity(&transcript, &identity_path).unwrap();
        assert_eq!(verified.len(), 1, "signed transcript verifies");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signed_transcript_verifies_and_detects_tamper() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let raw = rusqlite::Connection::open_in_memory().unwrap();
        EventStore::init_schema(&raw).unwrap();
        let store = EventStore::from_conn(std::sync::Arc::new(tokio::sync::Mutex::new(raw)));
        let task_id = Uuid::new_v4();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("governed_runs");
        let identity_path = tmp.path().join("identity.key");
        let mut rec = EventStoreRecorder::new_with_identity_path(
            store,
            task_id,
            rt.handle().clone(),
            dir.clone(),
            identity_path.clone(),
        )
        .unwrap();

        rec.record(RunRecord::Event(CliEvent::new(
            EventKind::AssistantText { delta: "first".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )));
        rec.record(RunRecord::Event(CliEvent::new(
            EventKind::AssistantText { delta: "second".into() },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )));

        let transcript = dir.join(format!("{task_id}.jsonl"));
        let verified = verify_transcript_with_identity(&transcript, &identity_path).unwrap();
        assert_eq!(verified.len(), 2);

        let mut bytes = std::fs::read(&transcript).unwrap();
        let pos = bytes
            .windows(b"second".len())
            .position(|w| w == b"second")
            .expect("test transcript contains second event");
        bytes[pos] = b'S';
        std::fs::write(&transcript, bytes).unwrap();
        assert!(
            verify_transcript_with_identity(&transcript, &identity_path).is_err(),
            "flipping one event byte must fail verification"
        );
    }

    #[test]
    fn transcript_redacts_secrets_before_signing_and_still_verifies() {
        // P4 + flight-recorder integrity: a secret embedded in a recorded event
        // must be REDACTED on disk (never stored in clear) AND the signed
        // transcript must still verify — proving redaction happens BEFORE
        // signing (the HMAC covers the on-disk redacted bytes). A regression
        // that signed before redacting would either leak the secret to disk or
        // break verification.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let raw = rusqlite::Connection::open_in_memory().unwrap();
        EventStore::init_schema(&raw).unwrap();
        let store = EventStore::from_conn(std::sync::Arc::new(tokio::sync::Mutex::new(raw)));
        let task_id = Uuid::new_v4();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("governed_runs");
        let identity_path = tmp.path().join("identity.key");
        let mut rec = EventStoreRecorder::new_with_identity_path(
            store,
            task_id,
            rt.handle().clone(),
            dir.clone(),
            identity_path.clone(),
        )
        .unwrap();

        let secret = "sk-LIVEKEY123abcDEF456ghiJKL789mno";
        rec.record(RunRecord::Event(CliEvent::new(
            EventKind::AssistantText { delta: format!("the key is {secret}") },
            Fidelity::StructuredVerified,
            Source::LiveStream,
        )));
        // Drop the recorder before reading the transcript: record() reopens +
        // closes the file per call so this isn't strictly required, but it
        // guarantees flush/handle-release even if that internal detail changes.
        drop(rec);

        let transcript = dir.join(format!("{task_id}.jsonl"));
        let body = std::fs::read_to_string(&transcript).unwrap();
        assert!(!body.contains(secret), "secret leaked to transcript in clear: {body}");
        assert!(body.contains("[REDACTED]"), "secret was not redacted: {body}");

        // Redaction happened BEFORE signing → the HMAC chain still verifies.
        let verified = verify_transcript_with_identity(&transcript, &identity_path).unwrap();
        assert_eq!(verified.len(), 1, "redacted transcript still verifies");
    }
}
