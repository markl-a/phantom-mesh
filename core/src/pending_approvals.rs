//! Pending-approvals store — a filesystem mirror of the high-risk approvals a
//! governed run is currently BLOCKED on, so a phone app can LIST what needs a
//! decision (apex-④ phone approval UI).
//!
//! Decision *submission* already works: the phone POSTs to `/rpc/inbox` and
//! `PhoneEscalator::await_decision` correlates + parses the reply. The only gap
//! this module closes is *listing* what is pending. When `await_decision` starts
//! awaiting it drops one `PendingCard` file here; on every return path (operator
//! decided OR timeout fallback) it removes the card. A `/rpc/approvals/list`
//! endpoint reads them back. This deliberately mirrors `inbox.rs`: one JSON file
//! per item, atomic temp+rename writes, list-skips-unreadable-files.
//!
//! 中文: 待批准清單 — 把治理執行當下卡住等待的高風險動作落檔, 讓手機 App 能
//! 列出「現在有什麼需要我批准」。提交批准走既有的 /rpc/inbox; 這裡只補「列出」。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One pending high-risk approval, mirroring what the phone needs to render a
/// decision card. `approval_id` doubles as the filename stem and is the topic the
/// phone replies with to `/rpc/inbox`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCard {
    /// Correlation id the operator's reply must reference (inbox topic/text).
    pub approval_id: String,
    /// The governed run (task) this approval belongs to.
    pub task_id: String,
    /// Tool/action awaiting approval (e.g. "Bash").
    pub tool: String,
    /// Risk level string (e.g. "execute_high").
    pub risk: String,
    /// Short human reason for the card (e.g. "pre-action approval").
    pub reason: String,
    /// Unix milliseconds when the card was created (sort key).
    pub created_ms: u64,
}

/// `~/.phantom-mesh/pending` under the given home.
pub fn pending_dir(home: &Path) -> PathBuf {
    crate::cli_config::phantom_dir_under(home).join("pending")
}

/// Persist one pending card atomically (tmp + rename) as
/// `<pending_dir>/<approval_id>.json`, so a lister never sees a half-written
/// file. Rejects an approval_id containing path separators so a crafted id can't
/// traverse out of the pending dir.
pub fn write_pending(home: &Path, card: &PendingCard) -> anyhow::Result<()> {
    let id = card.approval_id.as_str();
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        anyhow::bail!("invalid approval_id: {id}");
    }
    let dir = pending_dir(home);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".{id}.json.tmp"));
    let dest = dir.join(format!("{id}.json"));
    fs::write(&tmp, serde_json::to_vec_pretty(card)?)?;
    fs::rename(&tmp, &dest)?;
    Ok(())
}

/// List all pending cards, oldest first (by `created_ms`). Skips unreadable or
/// non-JSON files rather than failing the whole listing; a missing pending dir
/// is simply an empty list.
pub fn list_pending(home: &Path) -> anyhow::Result<Vec<PendingCard>> {
    let dir = pending_dir(home);
    let mut out: Vec<PendingCard> = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(out), // no pending dir yet == nothing pending
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read(&path) {
            if let Ok(card) = serde_json::from_slice::<PendingCard>(&raw) {
                out.push(card);
            }
        }
    }
    out.sort_by(|a, b| a.created_ms.cmp(&b.created_ms).then_with(|| a.approval_id.cmp(&b.approval_id)));
    Ok(out)
}

/// Remove one pending card by approval_id. Missing file is NOT an error (the
/// card may already be gone — this runs on every escalator return path and must
/// be idempotent). Rejects path-separator ids defensively.
pub fn remove_pending(home: &Path, approval_id: &str) -> anyhow::Result<()> {
    if approval_id.contains('/') || approval_id.contains('\\') || approval_id.contains("..") {
        anyhow::bail!("invalid approval_id: {approval_id}");
    }
    let path = pending_dir(home).join(format!("{approval_id}.json"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, created_ms: u64) -> PendingCard {
        PendingCard {
            approval_id: id.to_string(),
            task_id: "task-1".to_string(),
            tool: "Bash".to_string(),
            risk: "execute_high".to_string(),
            reason: "pre-action approval".to_string(),
            created_ms,
        }
    }

    #[test]
    fn write_then_list_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), &card("contract-xyz", 100)).unwrap();
        let cards = list_pending(tmp.path()).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].approval_id, "contract-xyz");
        assert_eq!(cards[0].task_id, "task-1");
        assert_eq!(cards[0].tool, "Bash");
        assert_eq!(cards[0].risk, "execute_high");
        assert_eq!(cards[0].reason, "pre-action approval");
        assert_eq!(cards[0].created_ms, 100);
    }

    #[test]
    fn list_orders_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), &card("b", 200)).unwrap();
        write_pending(tmp.path(), &card("a", 100)).unwrap();
        write_pending(tmp.path(), &card("c", 300)).unwrap();
        let cards = list_pending(tmp.path()).unwrap();
        assert_eq!(
            cards.iter().map(|c| c.approval_id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn remove_clears_card_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), &card("gone", 1)).unwrap();
        assert_eq!(list_pending(tmp.path()).unwrap().len(), 1);
        remove_pending(tmp.path(), "gone").unwrap();
        assert!(list_pending(tmp.path()).unwrap().is_empty());
        // removing again is a no-op, not an error (every escalator return path
        // calls remove; double-remove must never blow up)
        remove_pending(tmp.path(), "gone").unwrap();
        // removing one that never existed is also fine
        remove_pending(tmp.path(), "never-was").unwrap();
    }

    #[test]
    fn list_on_missing_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list_pending(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn half_written_and_foreign_files_are_invisible_to_list() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = pending_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".partial.json.tmp"), b"{not json").unwrap();
        std::fs::write(dir.join("garbage.json"), b"{not json").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignore me").unwrap();
        assert!(list_pending(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn write_rejects_path_traversal_ids() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(write_pending(tmp.path(), &card("../evil", 1)).is_err());
        assert!(write_pending(tmp.path(), &card("a/b", 1)).is_err());
        assert!(write_pending(tmp.path(), &card("a\\b", 1)).is_err());
        assert!(write_pending(tmp.path(), &card("", 1)).is_err());
    }
}
