//! Conversation / session model — per-chat message history with on-disk persistence.
//!
//! Each session is identified by a `chat_id` and stored as a newline-delimited
//! JSON ([JSONL](https://jsonlines.org/)) file under
//! `~/.phantom-mesh/conversations/<chat_id>.jsonl`, one [`ChatMessage`] per line.
//! An in-memory LRU-style cache fronts the disk so repeated reads avoid I/O,
//! while every mutation is also flushed to disk so history survives restarts.
//!
//! (Note: the project README refers to this as "SQLite-backed" history; the
//! current implementation uses the simpler append-only JSONL layout described
//! above. Same conceptual model — durable per-session conversation history —
//! with no extra dependency.)
//!
//! # What this module provides
//!
//! - [`ConversationStore`] — the central handle. Clone-cheap (`Arc`-wrapped
//!   state), safe to share across async tasks. Supports append, read, search,
//!   fork, rename, delete, export-to-Markdown, and titling.
//! - [`SessionInfo`] — lightweight metadata (id, message count, byte size,
//!   last-modified timestamp) for listing sessions.
//! - [`compact_via_llm`] — free-standing helper that summarizes the older part
//!   of a session via an LLM call and rewrites the on-disk history.
//!
//! # Compaction
//!
//! Two mechanisms keep history bounded:
//!
//! 1. **Hard cap** — once a cached history exceeds [`MAX_HISTORY`] messages, the
//!    oldest [`COMPACTION_DROP`] are dropped (see [`ConversationStore::maybe_compact`]).
//! 2. **LLM summary** — [`compact_via_llm`] / [`ConversationStore::replace_with_summary`]
//!    collapse the older portion into a single summary message, keeping the most
//!    recent N verbatim. The on-disk JSONL is rewritten atomically (write `.tmp`
//!    then rename) so a crash mid-rewrite never corrupts the file.
//!
//! # Concurrency
//!
//! Disk writes for a given `chat_id` are serialized through a per-id mutex so
//! two concurrent appends to the same session cannot interleave lines.

use std::collections::HashMap;
use std::sync::Arc;

use crate::providers::traits::ChatMessage;
use crate::vault::conversation_seal;

/// Maximum number of messages kept in a cached history before the hard-cap
/// compaction in [`ConversationStore::maybe_compact`] trims the oldest ones.
const MAX_HISTORY: usize = 200;
/// Number of oldest messages dropped when [`MAX_HISTORY`] is exceeded.
const COMPACTION_DROP: usize = 50;

/// Lightweight metadata about a single stored session, returned by
/// [`ConversationStore::session_info`] and [`ConversationStore::list_with_info`].
pub struct SessionInfo {
    /// The session's `chat_id`.
    pub id: String,
    /// Number of messages currently in the session.
    pub message_count: usize,
    /// Size of the on-disk JSONL file in bytes (0 if not yet written).
    pub size_bytes: u64,
    /// Last-modified timestamp of the JSONL file, formatted as
    /// `YYYY-MM-DD HH:MM:SS UTC` (or `"unknown"` if unavailable).
    pub last_modified: String,
}

/// Durable, cache-fronted store for per-session conversation history.
///
/// Cheap to clone (all state is behind `Arc`); share freely across async tasks.
/// See the [module docs](self) for the storage layout and compaction rules.
#[derive(Clone)]
pub struct ConversationStore {
    cache: Arc<tokio::sync::Mutex<HashMap<String, Vec<ChatMessage>>>>,
    base_dir: std::path::PathBuf,
    write_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Default for ConversationStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationStore {
    /// Create a store rooted at `~/.phantom-mesh/conversations` (falling back to
    /// `./conversations` if `$HOME` is unset). Creates the directory if missing.
    pub fn new() -> Self {
        let base_dir = crate::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join(".phantom-mesh"))
            .join("conversations");
        std::fs::create_dir_all(&base_dir).ok();
        Self {
            cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            base_dir,
            write_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Create a store rooted at an explicit directory. Useful for tests and for
    /// callers that want a non-default location. Creates the directory if missing.
    pub fn new_with_dir(base_dir: std::path::PathBuf) -> Self {
        std::fs::create_dir_all(&base_dir).ok();
        Self {
            cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            base_dir,
            write_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    fn chat_file(&self, chat_id: &str) -> std::path::PathBuf {
        let safe_id: String = chat_id
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ':' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.base_dir.join(format!("{}.jsonl", safe_id))
    }

    fn load_from_disk(&self, chat_id: &str) -> Vec<ChatMessage> {
        let path = self.chat_file(chat_id);
        if !path.exists() {
            return Vec::new();
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match conversation_seal::open_line(l) {
                Ok(json) => serde_json::from_str(&json).ok(),
                Err(_) => {
                    tracing::error!(
                        chat_id = chat_id,
                        "conversation line undecryptable, skipped (fail-closed)"
                    );
                    None
                }
            })
            .collect()
    }

    fn write_to_file(&self, chat_id: &str, user_msg: &ChatMessage, asst_msg: &ChatMessage) {
        use std::io::Write;
        let path = self.chat_file(chat_id);
        let enabled = conversation_seal::conversations_e2ee_enabled();

        let Ok(user_json) = serde_json::to_string(user_msg) else {
            return;
        };
        let Ok(asst_json) = serde_json::to_string(asst_msg) else {
            return;
        };

        let user_line = if enabled {
            match conversation_seal::seal_line(&user_json) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(
                        chat_id = chat_id,
                        "conversation seal failed, refusing to write plaintext: {e}"
                    );
                    return;
                }
            }
        } else {
            user_json
        };
        let asst_line = if enabled {
            match conversation_seal::seal_line(&asst_json) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(
                        chat_id = chat_id,
                        "conversation seal failed, refusing to write plaintext: {e}"
                    );
                    return;
                }
            }
        } else {
            asst_json
        };

        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{}", user_line);
            let _ = writeln!(f, "{}", asst_line);
        }
    }

    async fn append_to_disk_safe(
        &self,
        chat_id: &str,
        user_msg: &ChatMessage,
        asst_msg: &ChatMessage,
    ) {
        let lock = {
            let mut locks = self.write_locks.lock().await;
            locks
                .entry(chat_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        self.write_to_file(chat_id, user_msg, asst_msg);
    }

    /// Compact a history vec in-place: if it exceeds MAX_HISTORY, drop the oldest COMPACTION_DROP messages.
    fn maybe_compact(history: &mut Vec<ChatMessage>) {
        if history.len() > MAX_HISTORY {
            history.drain(0..COMPACTION_DROP);
        }
    }

    /// Return the full message history for `chat_id`, loading from disk into the
    /// cache on first access. Returns an empty vec for an unknown session.
    pub async fn get_history(&self, chat_id: &str) -> Vec<ChatMessage> {
        let mut cache = self.cache.lock().await;
        if !cache.contains_key(chat_id) {
            let msgs = self.load_from_disk(chat_id);
            cache.insert(chat_id.to_string(), msgs);
        }
        cache.get(chat_id).cloned().unwrap_or_default()
    }

    /// Append a user/assistant message pair to `chat_id`, persisting to disk
    /// (serialized per id) and updating the cache. Applies hard-cap compaction.
    pub async fn append(&self, chat_id: &str, user_msg: ChatMessage, asst_msg: ChatMessage) {
        self.append_to_disk_safe(chat_id, &user_msg, &asst_msg)
            .await;
        let mut cache = self.cache.lock().await;
        let entry = cache.entry(chat_id.to_string()).or_default();
        entry.push(user_msg);
        entry.push(asst_msg);
        Self::maybe_compact(entry);
    }

    /// Replace the older portion of a session's history with a single
    /// summary message, keeping the most recent `keep_recent` messages
    /// verbatim. The on-disk JSONL is rewritten atomically (.tmp + rename).
    /// Returns the number of messages that were collapsed.
    ///
    /// Callers (REPL `/compact`, auto-compact path) are responsible for
    /// generating the summary text via an LLM call before invoking this.
    pub async fn replace_with_summary(
        &self,
        chat_id: &str,
        summary: &str,
        keep_recent: usize,
    ) -> std::io::Result<usize> {
        let lock = {
            let mut locks = self.write_locks.lock().await;
            locks
                .entry(chat_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Pull the freshest copy from disk to avoid racing with a stale cache.
        let mut history = self.load_from_disk(chat_id);
        let total = history.len();
        if total <= keep_recent {
            return Ok(0);
        }
        let dropped = total - keep_recent;
        let recent: Vec<ChatMessage> = history.split_off(dropped);

        let mut new_history = Vec::with_capacity(recent.len() + 1);
        new_history.push(ChatMessage {
            role: "user".into(),
            content: format!(
                "[Conversation summary — {} earlier messages compacted]\n{}",
                dropped,
                summary.trim()
            ),
            tool_calls: None,
        });
        new_history.extend(recent);

        // Atomically rewrite the JSONL file.
        use std::io::Write;
        let path = self.chat_file(chat_id);
        let tmp = path.with_extension("jsonl.tmp");
        let enabled = conversation_seal::conversations_e2ee_enabled();
        {
            let mut f = std::fs::File::create(&tmp)?;
            for m in &new_history {
                let line = serde_json::to_string(m)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let line = if enabled {
                    conversation_seal::seal_line(&line)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
                } else {
                    line
                };
                writeln!(f, "{}", line)?;
            }
        }
        std::fs::rename(&tmp, &path)?;

        // Refresh cache to match disk.
        let mut cache = self.cache.lock().await;
        cache.insert(chat_id.to_string(), new_history);
        Ok(dropped)
    }

    /// Rough character count across all messages in a session — used by the
    /// REPL to decide when to auto-trigger LLM-summarized compaction. The
    /// LLM token cost is roughly chars / 4 across model families.
    pub async fn total_chars(&self, chat_id: &str) -> usize {
        self.get_history(chat_id)
            .await
            .iter()
            .map(|m| m.content.len())
            .sum()
    }

    /// Number of sessions currently warm in the cache (clamped to at least 1).
    pub async fn active_count(&self) -> usize {
        self.cache.lock().await.len().max(1)
    }

    /// Drop `chat_id` from the in-memory cache. The on-disk file is left intact;
    /// a later read will reload it.
    pub async fn evict(&self, chat_id: &str) {
        self.cache.lock().await.remove(chat_id);
    }

    /// Copy `src_id`'s on-disk JSONL into `new_id`, then warm the cache for
    /// the new id with the source's messages. Both ids end up with the same
    /// message history; future `append`s to one won't affect the other.
    /// Returns Ok(num_messages_copied) or Err on I/O failure / no source.
    pub async fn fork(&self, src_id: &str, new_id: &str) -> std::io::Result<usize> {
        let src_path = self.base_dir.join(format!("{}.jsonl", src_id));
        let dst_path = self.base_dir.join(format!("{}.jsonl", new_id));
        if !src_path.exists() {
            // Allow "fork from empty" by creating an empty file
            std::fs::create_dir_all(&self.base_dir).ok();
            std::fs::File::create(&dst_path)?;
        } else {
            std::fs::create_dir_all(&self.base_dir).ok();
            std::fs::copy(&src_path, &dst_path)?;
        }
        // Warm cache for the new id
        let msgs = self.load_from_disk(new_id);
        let count = msgs.len();
        let mut cache = self.cache.lock().await;
        cache.insert(new_id.to_string(), msgs);
        Ok(count)
    }

    /// Return all known session ids (union of on-disk `.jsonl` files and cached
    /// sessions), sorted lexicographically.
    pub async fn list(&self) -> Vec<String> {
        let mut ids: std::collections::HashSet<String> = std::fs::read_dir(&self.base_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".jsonl").map(|s| s.to_string())
            })
            .collect();
        let cache = self.cache.lock().await;
        ids.extend(cache.keys().cloned());
        let mut result: Vec<String> = ids.into_iter().collect();
        result.sort();
        result
    }

    /// Gather [`SessionInfo`] metadata for a single session.
    pub async fn session_info(&self, chat_id: &str) -> SessionInfo {
        let history = self.get_history(chat_id).await;
        let path = self.chat_file(chat_id);
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let last_modified = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|t| {
                let secs = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                // Format as ISO-8601-ish: YYYY-MM-DD HH:MM:SS UTC
                let s = secs;
                let days_since_epoch = s / 86400;
                let time_of_day = s % 86400;
                // Simple Gregorian calendar calculation
                let year = days_to_year(days_since_epoch);
                let day_of_year = days_since_epoch - years_to_days(year);
                let (month, day) = day_of_year_to_month_day(year, day_of_year);
                let hh = time_of_day / 3600;
                let mm = (time_of_day % 3600) / 60;
                let ss = time_of_day % 60;
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                    year, month, day, hh, mm, ss
                )
            })
            .unwrap_or_else(|_| "unknown".to_string());
        SessionInfo {
            id: chat_id.to_string(),
            message_count: history.len(),
            size_bytes,
            last_modified,
        }
    }

    /// Returns richer info for every known session (disk + cache).
    pub async fn list_with_info(&self) -> Vec<SessionInfo> {
        let ids = self.list().await;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            result.push(self.session_info(&id).await);
        }
        result
    }

    /// Derive a short title for the session from the first 60 chars of `message`
    /// and persist it to a sibling `.title` file. Failures are logged, not fatal.
    pub async fn auto_title(&self, chat_id: &str, message: &str) {
        let title: String = message.chars().take(60).collect();
        let title_path = self.base_dir.join(format!("{}.title", chat_id));
        if let Err(e) = std::fs::write(&title_path, title.trim()) {
            tracing::warn!(path = %title_path.display(), chat_id = chat_id, "session auto_title write failed: {}", e);
        }
    }

    /// Read the persisted title for the session, if any (None when absent/empty).
    pub async fn get_title(&self, chat_id: &str) -> Option<String> {
        let title_path = self.base_dir.join(format!("{}.title", chat_id));
        std::fs::read_to_string(title_path)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }

    /// Delete a session's `.jsonl` (and `.title`) files and evict it from cache.
    /// Returns `true` if the chat file existed before deletion.
    pub async fn delete(&self, chat_id: &str) -> bool {
        let file = self.chat_file(chat_id);
        let existed = file.exists();
        if existed {
            if let Err(e) = std::fs::remove_file(&file) {
                tracing::warn!(path = %file.display(), chat_id = chat_id, "session delete: chat file remove failed (disk/memory drift): {}", e);
            }
        }
        let title_path = self.base_dir.join(format!("{}.title", chat_id));
        if title_path.exists() {
            if let Err(e) = std::fs::remove_file(&title_path) {
                tracing::warn!(path = %title_path.display(), chat_id = chat_id, "session delete: title file remove failed: {}", e);
            }
        }
        self.cache.lock().await.remove(chat_id);
        existed
    }

    // -------------------------------------------------------------------------
    // Search
    // -------------------------------------------------------------------------

    /// Search across ALL sessions; returns session IDs whose messages contain `query`.
    pub async fn search(&self, query: &str) -> Vec<String> {
        let ids = self.list().await;
        let query_lower = query.to_lowercase();
        let mut matching = Vec::new();
        for id in ids {
            let history = self.get_history(&id).await;
            if history
                .iter()
                .any(|m| m.content.to_lowercase().contains(&query_lower))
            {
                matching.push(id);
            }
        }
        matching
    }

    /// Search within a single session; returns the content of matching messages.
    pub async fn search_in_session(&self, session_id: &str, query: &str) -> Vec<String> {
        let history = self.get_history(session_id).await;
        let query_lower = query.to_lowercase();
        history
            .into_iter()
            .filter(|m| m.content.to_lowercase().contains(&query_lower))
            .map(|m| m.content)
            .collect()
    }

    // -------------------------------------------------------------------------
    // Export to Markdown
    // -------------------------------------------------------------------------

    /// Export a session's conversation as formatted Markdown.
    pub async fn export_markdown(&self, session_id: &str) -> String {
        let history = self.get_history(session_id).await;
        let mut out = format!("# Session: {}\n", session_id);

        // Group messages into (user, assistant) pairs called "turns".
        let mut turn = 0usize;
        let mut i = 0;
        while i < history.len() {
            let msg = &history[i];
            if msg.role == "user" {
                turn += 1;
                out.push_str(&format!("\n## Turn {}\n", turn));
                out.push_str(&format!("\n**User:** {}\n", msg.content.trim()));

                // Look ahead for the assistant reply
                if let Some(asst) = history.get(i + 1).filter(|m| m.role == "assistant") {
                    out.push_str(&format!("\n**Assistant:** {}\n", asst.content.trim()));

                    // List tool calls if present
                    if let Some(tools) = &asst.tool_calls {
                        let names: Vec<String> = extract_tool_names(tools);
                        if !names.is_empty() {
                            out.push_str(&format!("\n*Tools used: {}*\n", names.join(", ")));
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            } else {
                // Stand-alone assistant message (e.g. after compaction)
                turn += 1;
                out.push_str(&format!("\n## Turn {}\n", turn));
                out.push_str(&format!("\n**Assistant:** {}\n", msg.content.trim()));
                if let Some(tools) = &msg.tool_calls {
                    let names = extract_tool_names(tools);
                    if !names.is_empty() {
                        out.push_str(&format!("\n*Tools used: {}*\n", names.join(", ")));
                    }
                }
                i += 1;
            }
        }

        out
    }

    // -------------------------------------------------------------------------
    // Rename / move
    // -------------------------------------------------------------------------

    /// Rename a session by moving its `.jsonl` (and `.title`) files on disk.
    pub async fn rename(&self, old_id: &str, new_id: &str) -> anyhow::Result<()> {
        let old_file = self.chat_file(old_id);
        let new_file = self.chat_file(new_id);

        if new_file.exists() {
            anyhow::bail!("Session '{}' already exists", new_id);
        }
        if !old_file.exists() {
            anyhow::bail!("Session '{}' not found", old_id);
        }

        std::fs::rename(&old_file, &new_file)?;

        // Move title file if it exists
        let old_title = self.base_dir.join(format!("{}.title", old_id));
        let new_title = self.base_dir.join(format!("{}.title", new_id));
        if old_title.exists() {
            std::fs::rename(&old_title, &new_title)?;
        }

        // Update cache: move the loaded messages to the new key
        let mut cache = self.cache.lock().await;
        if let Some(msgs) = cache.remove(old_id) {
            cache.insert(new_id.to_string(), msgs);
        }

        Ok(())
    }
}

/// Summarize the older portion of a session's history via an LLM call,
/// then atomically replace the on-disk JSONL with `[summary, ...last N]`.
///
/// Used by `/compact` (REPL + TUI) and the REPL's auto-compact threshold
/// path. Free-standing because it composes ConversationStore with an
/// AgentRuntime — keeping it outside `impl ConversationStore` avoids
/// dragging a runtime dep into the store.
///
/// Returns (dropped_count, summary_char_count).
pub async fn compact_via_llm(
    runtime: &crate::agent::AgentRuntime,
    agent_name: &str,
    cost_tracker: &crate::cost::CostTracker,
    conversations: &ConversationStore,
    chat_id: &str,
    history: &[ChatMessage],
    keep_recent: usize,
) -> anyhow::Result<(usize, usize)> {
    if history.len() <= keep_recent {
        return Ok((0, 0));
    }
    let head = &history[..history.len() - keep_recent];
    let convo_text: String = head
        .iter()
        .map(|m| {
            let body = if m.content.len() > 4000 {
                let prefix: String = m.content.chars().take(4000).collect();
                format!("{}…[truncated]", prefix)
            } else {
                m.content.clone()
            };
            format!("[{}]\n{}", m.role, body)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let prompt = format!(
        "Summarize the conversation below into a single tight paragraph (≤500 words). \
         Cover: stated goals, files touched, decisions made, results, and unresolved \
         threads. Reply with ONLY the summary — no preamble, no headings.\n\n\
         === CONVERSATION ===\n{}",
        convo_text,
    );

    let result = runtime
        .run_with_callbacks(
            agent_name,
            &prompt,
            &[],
            Some("You are a precise conversation summarizer. Output only the requested summary."),
            cost_tracker,
            |_ev| {},
        )
        .await?;

    let summary = result.output.trim().to_string();
    let summary_chars = summary.chars().count();
    let dropped = conversations
        .replace_with_summary(chat_id, &summary, keep_recent)
        .await?;
    Ok((dropped, summary_chars))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extract tool names from a tool_calls JSON value.
fn extract_tool_names(value: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                names.push(name.to_string());
            } else if let Some(func) = item.get("function") {
                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Minimal calendar helpers (no external dependencies)
// ---------------------------------------------------------------------------

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_to_year(days: u64) -> u64 {
    // Approximate, then correct
    let mut year = 1970 + days / 366;
    while years_to_days(year + 1) <= days {
        year += 1;
    }
    while years_to_days(year) > days {
        year -= 1;
    }
    year
}

fn years_to_days(year: u64) -> u64 {
    let y = year - 1;
    let leap_years = y / 4 - y / 100 + y / 400;
    let _non_leap = y - 1970 + (1970 / 4 - 1970 / 100 + 1970 / 400); // relative to 1970 (unused, kept for readability)
                                                                     // Days from Unix epoch (1970-01-01) to start of `year`
    (year - 1970) * 365 + (leap_years - (1969 / 4 - 1969 / 100 + 1969 / 400))
}

fn day_of_year_to_month_day(year: u64, day_of_year: u64) -> (u64, u64) {
    let months = [
        31u64,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut remaining = day_of_year;
    for (i, &days) in months.iter().enumerate() {
        if remaining < days {
            return (i as u64 + 1, remaining + 1);
        }
        remaining -= days;
    }
    (12, 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, body: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: body.into(),
            tool_calls: None,
        }
    }

    fn temp_store() -> (ConversationStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("tmpdir");
        let store = ConversationStore::new_with_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[tokio::test]
    async fn replace_with_summary_collapses_old_keeps_recent() {
        let (store, _dir) = temp_store();
        let chat = "compact-test";
        // Seed 10 turns (20 messages alternating user/assistant).
        for i in 0..10 {
            store
                .append(
                    chat,
                    msg("user", &format!("user turn {}", i)),
                    msg("assistant", &format!("assistant turn {}", i)),
                )
                .await;
        }
        assert_eq!(store.get_history(chat).await.len(), 20);

        let dropped = store
            .replace_with_summary(chat, "the gist of turns 0..7", 6)
            .await
            .expect("replace ok");
        assert_eq!(dropped, 14);

        let after = store.get_history(chat).await;
        assert_eq!(after.len(), 7, "1 summary + 6 recent");
        assert!(
            after[0].content.contains("Conversation summary"),
            "first message is the synthesized summary, got: {:?}",
            after[0].content
        );
        assert!(after[0].content.contains("the gist of turns 0..7"));
        // Last 6 messages preserved verbatim — they are turns 7,7 / 8,8 / 9,9.
        assert_eq!(after[1].content, "user turn 7");
        assert_eq!(after[6].content, "assistant turn 9");
    }

    #[tokio::test]
    async fn replace_with_summary_noop_when_below_keep_threshold() {
        let (store, _dir) = temp_store();
        let chat = "short";
        for i in 0..2 {
            store
                .append(
                    chat,
                    msg("user", &format!("u{}", i)),
                    msg("assistant", &format!("a{}", i)),
                )
                .await;
        }
        let dropped = store
            .replace_with_summary(chat, "summary", 10)
            .await
            .unwrap();
        assert_eq!(dropped, 0);
        assert_eq!(store.get_history(chat).await.len(), 4);
    }

    #[tokio::test]
    async fn replace_with_summary_persists_across_reload() {
        let (store, dir) = temp_store();
        let chat = "persist";
        for i in 0..8 {
            store
                .append(
                    chat,
                    msg("user", &format!("u{}", i)),
                    msg("assistant", &format!("a{}", i)),
                )
                .await;
        }
        store
            .replace_with_summary(chat, "synthesized summary text", 4)
            .await
            .unwrap();

        // Build a fresh store at the same dir; loading from disk should
        // see the rewritten history, not the pre-compact 16 messages.
        let store2 = ConversationStore::new_with_dir(dir.path().to_path_buf());
        let after = store2.get_history(chat).await;
        assert_eq!(after.len(), 5);
        assert!(after[0].content.contains("synthesized summary text"));
    }

    #[tokio::test]
    async fn total_chars_sums_message_bodies() {
        let (store, _dir) = temp_store();
        let chat = "chars";
        store
            .append(chat, msg("user", "hello"), msg("assistant", "world!"))
            .await;
        assert_eq!(
            store.total_chars(chat).await,
            "hello".len() + "world!".len()
        );
    }
}
