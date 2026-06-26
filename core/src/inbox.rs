//! Node inbox — persistent coordination messages for dev sessions (S1).
//!
//! A peer POSTs a small JSON message to this node's `/rpc/inbox`; the serve
//! daemon drops it as one file under `~/.phantom-mesh/inbox/`. The local dev
//! session (Claude Code / codex loop) reads the files on its next tick and
//! acks them by moving them to `inbox/done/`. Transport is HTTP over the
//! tailnet (HMAC-gated) — deliberately NOT SSH stdin (Windows OpenSSH
//! deadlocks above ~4 KB) and NOT git (messages are runtime traffic, not
//! versioned artifacts). Large payloads must travel as git branch/commit
//! refs inside `text`, never inline — hence the hard size cap.
//!
//! 中文: 節點信箱 — 跨機 dev session 的小協調訊息。serve 收到就落檔,
//! 本機 session 下個 tick 讀取;大東西走 git ref,不走訊息本體。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Process-local monotonic tiebreaker for message ids. The unix-ms prefix alone
/// does NOT preserve arrival order for two messages persisted in the same
/// millisecond (and a coarse-resolution clock makes that common), so a bare
/// random-uuid suffix would order same-ms messages non-deterministically. This
/// counter guarantees `id` lexicographic order == arrival order within a process.
static INBOX_SEQ: AtomicU64 = AtomicU64::new(0);

/// Hard cap on `text` bytes. Mirrors the SPEC-26 §3.1 G1 16 KB task-payload
/// posture: coordination messages carry directives + refs, not diffs.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

/// Maximum bytes of `from` / `topic` fields (defensive — these end up in
/// filenames-adjacent metadata and status output).
const MAX_META_BYTES: usize = 256;

/// One persisted inbox message. `id` doubles as the filename stem and sorts
/// chronologically (unix-ms prefix).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    pub id: String,
    /// Sender node name (self-reported, HMAC already proved cluster
    /// membership — this is attribution, not authentication).
    pub from: String,
    /// The directive / message body. ≤ MAX_TEXT_BYTES.
    pub text: String,
    /// Optional routing hint (e.g. "backlog", "review", "status").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Unix seconds when this node persisted the message.
    pub received_at: u64,
}

/// `~/.phantom-mesh/inbox` under the given home.
pub fn inbox_dir(home: &Path) -> PathBuf {
    crate::cli_config::phantom_dir_under(home).join("inbox")
}

/// `~/.phantom-mesh/inbox/done` — acked messages are moved, not deleted,
/// so a routine crash mid-handling never loses a directive silently.
pub fn done_dir(home: &Path) -> PathBuf {
    inbox_dir(home).join("done")
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Persist one message; returns its id. Write is atomic (tmp + rename) so a
/// reader listing the directory never observes a half-written JSON file.
pub fn write_message(
    home: &Path,
    from: &str,
    text: &str,
    topic: Option<&str>,
) -> anyhow::Result<String> {
    if text.trim().is_empty() {
        anyhow::bail!("inbox message text is empty");
    }
    if text.len() > MAX_TEXT_BYTES {
        anyhow::bail!(
            "inbox message too large: {} bytes > {} cap — send a git branch/commit ref instead",
            text.len(),
            MAX_TEXT_BYTES
        );
    }
    if from.len() > MAX_META_BYTES || topic.is_some_and(|t| t.len() > MAX_META_BYTES) {
        anyhow::bail!("inbox from/topic field too large (> {} bytes)", MAX_META_BYTES);
    }
    let dir = inbox_dir(home);
    fs::create_dir_all(&dir)?;
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // id = {unix_ms}-{seq}-{uuid}. The zero-padded unix-ms prefix keeps
    // chronological order across restarts; the process-local monotonic seq
    // breaks same-millisecond ties so lexicographic order == arrival order
    // (fixed-width so the string compare matches the numeric compare); the uuid
    // suffix prevents filename collisions between concurrent senders/processes.
    let seq = INBOX_SEQ.fetch_add(1, Ordering::Relaxed);
    // Pad seq to the full u64 width (20 digits) so lexicographic order == numeric
    // order for every possible value (a narrower min-width would break ordering
    // once seq crosses a power-of-ten boundary).
    let id = format!(
        "{:013}-{:020}-{}",
        unix_ms,
        seq,
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let msg = InboxMessage {
        id: id.clone(),
        from: if from.trim().is_empty() { "unknown".into() } else { from.to_string() },
        text: text.to_string(),
        topic: topic.map(|t| t.to_string()),
        received_at: now_unix(),
    };
    let tmp = dir.join(format!(".{id}.json.tmp"));
    let dest = dir.join(format!("{id}.json"));
    fs::write(&tmp, serde_json::to_vec_pretty(&msg)?)?;
    fs::rename(&tmp, &dest)?;
    Ok(id)
}

/// List pending (un-acked) messages, oldest first. Skips unreadable or
/// half-foreign files rather than failing the whole listing.
pub fn list_messages(home: &Path) -> anyhow::Result<Vec<InboxMessage>> {
    let dir = inbox_dir(home);
    let mut out: Vec<InboxMessage> = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(out), // no inbox yet == no messages
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read(&path) {
            if let Ok(msg) = serde_json::from_slice::<InboxMessage>(&raw) {
                out.push(msg);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Ack one message by id: move it to `inbox/done/`. Errors if the id has no
/// pending file (already acked or never existed).
pub fn ack_message(home: &Path, id: &str) -> anyhow::Result<()> {
    // ids are generated by write_message (unix-ms + uuid hex). Reject path
    // separators so a crafted id can't traverse out of the inbox dir.
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        anyhow::bail!("invalid inbox id: {id}");
    }
    let src = inbox_dir(home).join(format!("{id}.json"));
    if !src.exists() {
        anyhow::bail!("no pending inbox message with id {id}");
    }
    let done = done_dir(home);
    fs::create_dir_all(&done)?;
    fs::rename(&src, done.join(format!("{id}.json")))?;
    Ok(())
}

/// Ack everything pending; returns how many messages were moved.
pub fn ack_all(home: &Path) -> anyhow::Result<usize> {
    let msgs = list_messages(home)?;
    let mut n = 0;
    for m in &msgs {
        ack_message(home, &m.id)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_list_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let id = write_message(tmp.path(), "m1", "run the backlog item demo-1", Some("backlog"))
            .unwrap();
        let msgs = list_messages(tmp.path()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, id);
        assert_eq!(msgs[0].from, "m1");
        assert_eq!(msgs[0].text, "run the backlog item demo-1");
        assert_eq!(msgs[0].topic.as_deref(), Some("backlog"));
        assert!(msgs[0].received_at > 0);
    }

    #[test]
    fn list_orders_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let a = write_message(tmp.path(), "m1", "first", None).unwrap();
        let b = write_message(tmp.path(), "z13", "second", None).unwrap();
        let msgs = list_messages(tmp.path()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, a);
        assert_eq!(msgs[1].id, b);
    }

    #[test]
    fn ack_moves_to_done_and_clears_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let id = write_message(tmp.path(), "m1", "ack me", None).unwrap();
        ack_message(tmp.path(), &id).unwrap();
        assert!(list_messages(tmp.path()).unwrap().is_empty());
        assert!(done_dir(tmp.path()).join(format!("{id}.json")).exists());
        // double-ack is an error, not a silent no-op
        assert!(ack_message(tmp.path(), &id).is_err());
    }

    #[test]
    fn ack_all_drains_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        write_message(tmp.path(), "m1", "one", None).unwrap();
        write_message(tmp.path(), "m1", "two", None).unwrap();
        assert_eq!(ack_all(tmp.path()).unwrap(), 2);
        assert!(list_messages(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn rejects_empty_and_oversized_text() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(write_message(tmp.path(), "m1", "   ", None).is_err());
        let big = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(write_message(tmp.path(), "m1", &big, None).is_err());
        // boundary: exactly at cap is accepted
        let max = "x".repeat(MAX_TEXT_BYTES);
        assert!(write_message(tmp.path(), "m1", &max, None).is_ok());
    }

    #[test]
    fn ack_rejects_path_traversal_ids() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(ack_message(tmp.path(), "../evil").is_err());
        assert!(ack_message(tmp.path(), "a/b").is_err());
        assert!(ack_message(tmp.path(), "a\\b").is_err());
    }

    #[test]
    fn half_written_tmp_files_are_invisible_to_list() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = inbox_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".123-abc.json.tmp"), b"{not json").unwrap();
        std::fs::write(dir.join("garbage.json"), b"{not json").unwrap();
        assert!(list_messages(tmp.path()).unwrap().is_empty());
    }
}
