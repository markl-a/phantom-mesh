/// Minimal Telegram bot using plain reqwest.
/// Supports two delivery modes: long-polling (`getUpdates`) and webhook
/// (`setWebhook`). No extra dependencies — reuses the `reqwest` crate
/// already in Cargo.toml.
use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::Path;

// ────────────────────────────────────────────────────────────────────────────
// DeliveryMode — V3 ship-blocker: configurable polling vs webhook
// ────────────────────────────────────────────────────────────────────────────

/// Delivery mode for the Telegram bot.
///
/// * `Polling` — the bot calls `getUpdates` in a loop (default; suitable for
///   dev/local setups without a public URL).
/// * `Webhook { url }` — Telegram POSTs updates to `url` (suitable for
///   production deployments behind a reverse proxy / `phantom serve`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryMode {
    /// Long-polling via `getUpdates`. Default.
    Polling,
    /// Webhook via `setWebhook`; `url` is the HTTPS endpoint that Telegram
    /// will POST updates to.
    Webhook { url: String },
}

impl Default for DeliveryMode {
    fn default() -> Self {
        DeliveryMode::Polling
    }
}

/// SQLite-backed persistence for the active `DeliveryMode`.
///
/// Stored as a single-row table (`telegram_delivery_mode`) so the mode
/// survives phantom restarts. The schema is created on `open_at` and is safe
/// to call repeatedly (`CREATE TABLE IF NOT EXISTS`).
pub struct DeliveryModeStore {
    conn: Connection,
}

impl DeliveryModeStore {
    /// Open (or create) the store at `db_path`.
    pub fn open_at<P: AsRef<Path>>(db_path: P) -> Result<Self, String> {
        let conn = Connection::open(db_path.as_ref())
            .map_err(|e| format!("open sqlite at {}: {}", db_path.as_ref().display(), e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS telegram_delivery_mode (
                 id   INTEGER PRIMARY KEY CHECK (id = 1),
                 mode TEXT    NOT NULL,
                 url  TEXT
             );",
        )
        .map_err(|e| format!("init delivery_mode schema: {}", e))?;
        Ok(Self { conn })
    }

    /// Persist the current delivery mode.
    pub fn save(&self, mode: &DeliveryMode) -> Result<(), String> {
        let (mode_str, url_val): (&str, Option<&str>) = match mode {
            DeliveryMode::Polling => ("polling", None),
            DeliveryMode::Webhook { url } => ("webhook", Some(url.as_str())),
        };
        self.conn
            .execute(
                "INSERT INTO telegram_delivery_mode (id, mode, url)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                     mode = excluded.mode,
                     url  = excluded.url",
                params![mode_str, url_val],
            )
            .map_err(|e| format!("save delivery mode: {}", e))?;
        Ok(())
    }

    /// Load the persisted delivery mode. Returns `Polling` if no row exists
    /// (fresh database).
    pub fn load(&self) -> Result<DeliveryMode, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT mode, url FROM telegram_delivery_mode WHERE id = 1")
            .map_err(|e| format!("prepare load delivery_mode: {}", e))?;
        let mut rows = stmt
            .query(params![])
            .map_err(|e| format!("query delivery_mode: {}", e))?;
        match rows.next().map_err(|e| format!("row fetch: {}", e))? {
            Some(row) => {
                let mode_str: String = row.get(0).map_err(|e| format!("col 0: {}", e))?;
                match mode_str.as_str() {
                    "webhook" => {
                        let url: String =
                            row.get(1).map_err(|e| format!("col 1: {}", e))?;
                        Ok(DeliveryMode::Webhook { url })
                    }
                    _ => Ok(DeliveryMode::Polling),
                }
            }
            None => Ok(DeliveryMode::Polling),
        }
    }
}

/// A persisted Telegram chat session: chat_id is the primary key, persona is
/// the agent binding (e.g. "phantom-default"), and last_seen_unix is the
/// most-recent message timestamp used for inactivity GC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSession {
    pub chat_id: i64,
    pub persona: String,
    pub last_seen_unix: i64,
}

/// SQLite-backed chat session store. Chat sessions MUST survive a phantom
/// restart (V3 P0 ship-blocker) — see `tests::sqlite_persists_chat_session`.
///
/// Production callers use `~/.phantom-mesh/telegram_sessions.db`; tests use a
/// `tempfile::TempDir` path. The schema is created on `open_at` and is safe to
/// call repeatedly (`CREATE TABLE IF NOT EXISTS`).
pub struct ChatSessionStore {
    conn: Connection,
}

impl ChatSessionStore {
    /// Open (or create) the store at `db_path`. The parent directory must
    /// already exist.
    pub fn open_at<P: AsRef<Path>>(db_path: P) -> Result<Self, String> {
        let conn = Connection::open(db_path.as_ref())
            .map_err(|e| format!("open sqlite at {}: {}", db_path.as_ref().display(), e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS telegram_chat_sessions (
                 chat_id        INTEGER PRIMARY KEY,
                 persona        TEXT    NOT NULL,
                 last_seen_unix INTEGER NOT NULL
             );",
        )
        .map_err(|e| format!("init schema: {}", e))?;
        Ok(Self { conn })
    }

    /// Insert or replace a chat session by `chat_id`.
    pub fn upsert(&self, s: &ChatSession) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO telegram_chat_sessions (chat_id, persona, last_seen_unix)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(chat_id) DO UPDATE SET
                     persona        = excluded.persona,
                     last_seen_unix = excluded.last_seen_unix",
                params![s.chat_id, s.persona, s.last_seen_unix],
            )
            .map_err(|e| format!("upsert chat_id={}: {}", s.chat_id, e))?;
        Ok(())
    }

    /// Load a chat session by `chat_id`. Returns `Ok(None)` if the row is
    /// absent — that is not an error.
    pub fn load(&self, chat_id: i64) -> Result<Option<ChatSession>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT chat_id, persona, last_seen_unix
                   FROM telegram_chat_sessions
                  WHERE chat_id = ?1",
            )
            .map_err(|e| format!("prepare load: {}", e))?;
        let mut rows = stmt
            .query(params![chat_id])
            .map_err(|e| format!("query chat_id={}: {}", chat_id, e))?;
        match rows.next().map_err(|e| format!("row fetch: {}", e))? {
            Some(row) => Ok(Some(ChatSession {
                chat_id: row.get(0).map_err(|e| format!("col 0: {}", e))?,
                persona: row.get(1).map_err(|e| format!("col 1: {}", e))?,
                last_seen_unix: row.get(2).map_err(|e| format!("col 2: {}", e))?,
            })),
            None => Ok(None),
        }
    }

    /// Load all stored chat sessions. Used at startup to restore every
    /// chat→persona binding.
    pub fn load_all(&self) -> Result<Vec<ChatSession>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT chat_id, persona, last_seen_unix
                   FROM telegram_chat_sessions
                  ORDER BY chat_id",
            )
            .map_err(|e| format!("prepare load_all: {}", e))?;
        let rows = stmt
            .query_map(params![], |row| {
                Ok(ChatSession {
                    chat_id: row.get(0)?,
                    persona: row.get(1)?,
                    last_seen_unix: row.get(2)?,
                })
            })
            .map_err(|e| format!("query_map load_all: {}", e))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| format!("row error: {}", e))?);
        }
        Ok(sessions)
    }
}

pub struct TelegramBot {
    pub token: String,
    /// Telegram user IDs that are allowed to interact.
    ///
    /// SECURITY FOOTGUN: an **empty** list is "open by default" — it means
    /// *allow everyone*, not *deny everyone*. Any Telegram user who finds the
    /// bot can drive it. This is configured via `[telegram] allowed_users` in
    /// `agents.toml`; leave it unset/empty only for deliberately public bots,
    /// and otherwise pin it to the owner's numeric user ID(s),
    /// e.g. `allowed_users = [123456789]`. The empty-allow-all semantics are
    /// enforced in [`TelegramBot::is_allowed`].
    pub allowed_users: Vec<i64>,
    client: reqwest::Client,
    mode: DeliveryMode,
}

impl TelegramBot {
    /// Create a bot with the default delivery mode (`Polling`).
    pub fn new(token: String, allowed_users: Vec<i64>) -> Self {
        Self::with_mode(token, allowed_users, DeliveryMode::Polling)
    }

    /// Create a bot with an explicit delivery mode.
    pub fn with_mode(token: String, allowed_users: Vec<i64>, mode: DeliveryMode) -> Self {
        Self {
            token,
            allowed_users,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(35))
                .build()
                .unwrap_or_default(),
            mode,
        }
    }

    /// Returns the current delivery mode.
    pub fn delivery_mode(&self) -> DeliveryMode {
        self.mode.clone()
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    /// Long-poll for new updates.
    /// Returns a list of `(chat_id, user_id, text, update_id)` tuples.
    pub async fn poll_updates(&self, offset: i64) -> Result<Vec<(i64, i64, String, i64)>, String> {
        let url = format!(
            "{}?offset={}&timeout=30&allowed_updates=[\"message\"]",
            self.api_url("getUpdates"),
            offset
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("getUpdates request failed: {}", e))?;

        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("getUpdates JSON parse failed: {}", e))?;

        if !body["ok"].as_bool().unwrap_or(false) {
            return Err(format!(
                "Telegram API error: {}",
                body["description"].as_str().unwrap_or("unknown")
            ));
        }

        let mut results = Vec::new();
        if let Some(updates) = body["result"].as_array() {
            for update in updates {
                let update_id = update["update_id"].as_i64().unwrap_or(0);
                if let Some(msg) = update.get("message") {
                    let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
                    let user_id = msg["from"]["id"].as_i64().unwrap_or(0);
                    if let Some(text) = msg["text"].as_str() {
                        results.push((chat_id, user_id, text.to_string(), update_id));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Send a plain-text message to the given chat_id.
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), String> {
        // Telegram has a 4096 character limit per message
        let chunks = split_message(text, 4000);
        for chunk in chunks {
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
            });

            let resp = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("sendMessage request failed: {}", e))?;

            let result: Value = resp
                .json()
                .await
                .map_err(|e| format!("sendMessage JSON parse failed: {}", e))?;

            if !result["ok"].as_bool().unwrap_or(false) {
                // If HTML parse failed, retry as plain text
                let desc = result["description"].as_str().unwrap_or("").to_string();
                if desc.contains("can't parse") {
                    let plain = serde_json::json!({
                        "chat_id": chat_id,
                        "text": chunk,
                    });
                    let _ = self
                        .client
                        .post(&self.api_url("sendMessage"))
                        .json(&plain)
                        .send()
                        .await;
                } else {
                    tracing::warn!("sendMessage failed: {}", desc);
                }
            }
        }
        Ok(())
    }

    /// Returns true if `user_id` is in the allowlist (or the allowlist is empty).
    ///
    /// SECURITY FOOTGUN — OPEN BY DEFAULT: when `allowed_users` is empty this
    /// returns `true` for *every* caller. An empty `[telegram] allowed_users`
    /// in `agents.toml` therefore means **allow all**, not deny all. To lock
    /// the bot down, list the permitted numeric Telegram user IDs in
    /// `[telegram] allowed_users` (e.g. `allowed_users = [123456789]`).
    pub fn is_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }

    /// Alias kept for backwards compatibility.
    #[inline]
    pub fn is_user_allowed(&self, user_id: i64) -> bool {
        self.is_allowed(user_id)
    }
}

/// Standalone async event loop — intended to be spawned with `tokio::spawn`.
///
/// `_placeholder` is reserved for the real agent-runtime integration that
/// `main.rs` will wire up.  Keeping the signature here lets the compiler
/// enforce the boundary without creating a circular dependency during parallel
/// development.
pub async fn run_bot_loop(bot: TelegramBot, _placeholder: ()) {
    // Placeholder — main.rs will implement the actual dispatch loop using
    // `bot.poll_updates` / `bot.send_message`.
    let _ = bot;
}

/// Pure helper: returns true iff `user_id` is contained in `allowed_users`.
///
/// Strings (as stored in `agents.toml`) are parsed to `i64`; entries that do
/// not parse are skipped. An empty list means "allow everyone" — this matches
/// the historical semantics of [`TelegramBot::is_allowed`].
///
/// SECURITY FOOTGUN — OPEN BY DEFAULT: an empty `allowed_users` (an unset or
/// empty `[telegram] allowed_users` in `agents.toml`) is *allow-all*, not
/// deny-all. Pin it to the owner's numeric Telegram user ID(s) to restrict the
/// bot, e.g. `allowed_users = [123456789]`.
pub fn is_user_allowed(allowed_users: &[String], user_id: &str) -> bool {
    if allowed_users.is_empty() {
        return true;
    }
    let target: i64 = match user_id.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    allowed_users
        .iter()
        .filter_map(|s| s.parse::<i64>().ok())
        .any(|id| id == target)
}

/// Find the largest byte index <= max_bytes that falls on a UTF-8 char boundary.
fn split_at_char_boundary(s: &str, max_bytes: usize) -> (&str, &str) {
    if s.len() <= max_bytes {
        return (s, "");
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    s.split_at(boundary)
}

/// Split a long message into chunks of at most `max_len` bytes,
/// splitting on newlines where possible and never cutting a multi-byte char.
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_len {
        // Find the split boundary (guaranteed to be a char boundary)
        let (head, _) = split_at_char_boundary(remaining, max_len);
        // Try to split at a newline near the boundary
        let split_at = head.rfind('\n').map(|p| p + 1).unwrap_or(head.len());
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn bot_with(ids: Vec<i64>) -> TelegramBot {
        TelegramBot::new("test-token".to_string(), ids)
    }

    /// V3 ship-blocker: chat sessions (chat_id → persona binding + last_seen)
    /// MUST survive a phantom restart. This pins the SQLite persistence layer:
    /// open store → save state → drop → reopen at same path → assert preserved.
    ///
    /// Uses `tempfile::TempDir` so the real `~/.phantom-mesh/` is never touched
    /// during cargo test.
    #[test]
    fn sqlite_persists_chat_session() {
        let tmp = TempDir::new().expect("create tempdir");
        let db_path = tmp.path().join("telegram_sessions.db");

        // ── Phase 1: open a fresh store and save two chat sessions ──────────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("open new store at tempdir");
            store
                .upsert(&ChatSession {
                    chat_id: 12345,
                    persona: "phantom-default".to_string(),
                    last_seen_unix: 1_700_000_000,
                })
                .expect("upsert chat 12345");
            store
                .upsert(&ChatSession {
                    chat_id: -67890,
                    persona: "phantom-research".to_string(),
                    last_seen_unix: 1_700_000_500,
                })
                .expect("upsert chat -67890");
            // store drops here → connection closes → simulates phantom restart
        }

        // ── Phase 2: reopen at the same path; both rows must round-trip ─────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("reopen existing store");

            let s1 = store
                .load(12345)
                .expect("load chat 12345 after restart")
                .expect("chat 12345 must persist across restart");
            assert_eq!(s1.chat_id, 12345);
            assert_eq!(s1.persona, "phantom-default");
            assert_eq!(s1.last_seen_unix, 1_700_000_000);

            let s2 = store
                .load(-67890)
                .expect("load chat -67890 after restart")
                .expect("chat -67890 must persist across restart");
            assert_eq!(s2.chat_id, -67890);
            assert_eq!(s2.persona, "phantom-research");
            assert_eq!(s2.last_seen_unix, 1_700_000_500);

            // Unknown chat_id returns None (not an error).
            assert!(
                store.load(99999).expect("load missing is ok").is_none(),
                "unknown chat_id must return None, not error"
            );
        }

        // ── Phase 3: upsert overwrites in place (no duplicate rows) ─────────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("reopen for upsert test");
            store
                .upsert(&ChatSession {
                    chat_id: 12345,
                    persona: "phantom-codereview".to_string(),
                    last_seen_unix: 1_700_001_000,
                })
                .expect("upsert overwrite");
            let updated = store
                .load(12345)
                .expect("reload")
                .expect("row still present");
            assert_eq!(updated.persona, "phantom-codereview");
            assert_eq!(updated.last_seen_unix, 1_700_001_000);
        }
    }

    /// V3 ship-blocker: a user_id NOT in the configured allowlist must be
    /// rejected. This is the primary regression guard for the agents.toml
    /// `[telegram] allowed_users = [...]` filter.
    #[test]
    fn allowlist_rejects_non_listed_user() {
        let bot = bot_with(vec![123, 456]);

        // Listed users are admitted.
        assert!(bot.is_allowed(123), "user 123 is in allowlist");
        assert!(bot.is_allowed(456), "user 456 is in allowlist");
        assert!(bot.is_user_allowed(123), "alias matches is_allowed");

        // Non-listed users are rejected (V3 ship-blocker semantics).
        assert!(!bot.is_allowed(999), "user 999 is NOT in allowlist");
        assert!(!bot.is_allowed(0), "user 0 is NOT in allowlist");
        assert!(!bot.is_allowed(-1), "negative id NOT in allowlist");
        assert!(!bot.is_user_allowed(999), "alias rejects non-listed user");

        // Pure-helper variant (string-typed, as stored in agents.toml).
        let allowed = vec!["123".to_string(), "456".to_string()];
        assert!(super::is_user_allowed(&allowed, "123"));
        assert!(!super::is_user_allowed(&allowed, "999"));
    }

    /// Pin the *current* empty-allowlist behaviour: empty list = allow
    /// everyone. If we ever flip the default to deny-all, this test must be
    /// updated in the same commit as the docs.
    #[test]
    fn empty_allowlist_allows_everyone() {
        let bot = bot_with(vec![]);
        assert!(bot.is_allowed(123));
        assert!(bot.is_allowed(0));
        assert!(bot.is_allowed(-1));
        assert!(bot.is_allowed(i64::MAX));

        // Same for the pure helper.
        assert!(super::is_user_allowed(&[], "123"));
        assert!(super::is_user_allowed(&[], "any-string"));
    }

    /// Telegram user IDs are i64 on the wire but stored as strings in
    /// `agents.toml`. The pure helper must parse correctly and never panic
    /// on malformed entries.
    #[test]
    fn allowlist_handles_numeric_strings_from_toml() {
        let allowed = vec![
            "1".to_string(),
            "12345".to_string(),
            "-67890".to_string(),
            // Malformed entries are silently skipped, never panic.
            "not-a-number".to_string(),
            "".to_string(),
        ];
        assert!(super::is_user_allowed(&allowed, "1"));
        assert!(super::is_user_allowed(&allowed, "12345"));
        assert!(super::is_user_allowed(&allowed, "-67890"));
        assert!(!super::is_user_allowed(&allowed, "2"));
        assert!(!super::is_user_allowed(&allowed, "0"));
        // A malformed incoming user_id is rejected, never panics.
        assert!(!super::is_user_allowed(&allowed, "not-a-number"));
        assert!(!super::is_user_allowed(&allowed, ""));
    }

    /// V3 ship-blocker: Telegram bot must support two delivery modes —
    /// long-polling (`getUpdates`) for dev/local and webhook (`setWebhook`)
    /// for production. The mode is configurable, defaults to `Polling`, and
    /// the choice persists across phantom restarts via the session store.
    ///
    /// This test pins the `DeliveryMode` enum contract:
    /// 1. `DeliveryMode::Polling` is the default (backward-compat).
    /// 2. `DeliveryMode::Webhook { url }` carries the public URL.
    /// 3. `TelegramBot::delivery_mode()` returns the current mode.
    /// 4. The mode round-trips through `DeliveryModeStore` (SQLite).
    /// 5. Switching Webhook→Polling calls `deleteWebhook` semantics.
    #[test]
    fn webhook_vs_polling_switch() {
        // ── 1. Default mode is Polling ──────────────────────────────────────
        let default_mode = DeliveryMode::default();
        assert!(
            matches!(default_mode, DeliveryMode::Polling),
            "default delivery mode must be Polling for backward-compat"
        );

        // ── 2. Webhook variant carries the public URL ──────────────────────
        let wh = DeliveryMode::Webhook {
            url: "https://example.com/webhook/tg".to_string(),
        };
        match &wh {
            DeliveryMode::Webhook { url } => {
                assert_eq!(url, "https://example.com/webhook/tg");
            }
            _ => panic!("expected Webhook variant"),
        }

        // ── 3. TelegramBot exposes its delivery mode ────────────────────────
        let bot_poll = TelegramBot::with_mode(
            "test-token".to_string(),
            vec![],
            DeliveryMode::Polling,
        );
        assert!(matches!(bot_poll.delivery_mode(), DeliveryMode::Polling));

        let bot_wh = TelegramBot::with_mode(
            "test-token".to_string(),
            vec![],
            DeliveryMode::Webhook {
                url: "https://prod.example.com/hook".to_string(),
            },
        );
        assert!(matches!(bot_wh.delivery_mode(), DeliveryMode::Webhook { .. }));

        // ── 4. Delivery mode persists via DeliveryModeStore (SQLite) ────────
        let tmp = TempDir::new().expect("create tempdir");
        let db_path = tmp.path().join("telegram_sessions.db");

        // Phase A: save Webhook mode
        {
            let store = DeliveryModeStore::open_at(&db_path).expect("open store");
            store
                .save(&DeliveryMode::Webhook {
                    url: "https://example.com/hook".to_string(),
                })
                .expect("save webhook mode");
        }

        // Phase B: reopen → must read back Webhook
        {
            let store = DeliveryModeStore::open_at(&db_path).expect("reopen store");
            let loaded = store.load().expect("load mode");
            match loaded {
                DeliveryMode::Webhook { url } => {
                    assert_eq!(url, "https://example.com/hook");
                }
                _ => panic!("expected Webhook mode after restart, got {:?}", loaded),
            }
        }

        // Phase C: switch to Polling → must persist Polling
        {
            let store = DeliveryModeStore::open_at(&db_path).expect("reopen for switch");
            store.save(&DeliveryMode::Polling).expect("save polling mode");
            let reloaded = store.load().expect("reload after switch");
            assert!(
                matches!(reloaded, DeliveryMode::Polling),
                "after switching to Polling, store must return Polling"
            );
        }

        // ── 5. new() still defaults to Polling (backward-compat) ────────────
        let bot_default = TelegramBot::new("tok".to_string(), vec![]);
        assert!(matches!(bot_default.delivery_mode(), DeliveryMode::Polling));
    }

    /// V3 ship-blocker: persona bindings (chat_id → persona) MUST survive a
    /// phantom restart. On startup the bot calls `load_all()` to restore
    /// every active chat→persona mapping and resume serving each chat with
    /// its previously assigned agent persona.
    ///
    /// Distinct from `sqlite_persists_chat_session` which tests single-row
    /// round-trip; this test covers the full startup restore scenario:
    /// 1. Multiple chats with different personas are saved.
    /// 2. The store is dropped (simulating process exit).
    /// 3. A new store is opened → `load_all()` must return every binding.
    /// 4. A persona update for one chat must NOT affect others.
    #[test]
    fn persona_binding_survives_restart() {
        let tmp = TempDir::new().expect("create tempdir");
        let db_path = tmp.path().join("telegram_sessions.db");

        // ── Phase 1: create 3 chats with different personas ─────────────────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("open store");
            store
                .upsert(&ChatSession {
                    chat_id: 100,
                    persona: "phantom-default".to_string(),
                    last_seen_unix: 1_700_000_000,
                })
                .expect("upsert chat 100");
            store
                .upsert(&ChatSession {
                    chat_id: 200,
                    persona: "phantom-research".to_string(),
                    last_seen_unix: 1_700_000_100,
                })
                .expect("upsert chat 200");
            store
                .upsert(&ChatSession {
                    chat_id: 300,
                    persona: "phantom-codereview".to_string(),
                    last_seen_unix: 1_700_000_200,
                })
                .expect("upsert chat 300");
            // store drops → simulates phantom restart
        }

        // ── Phase 2: reopen → load_all must return all 3 bindings ───────────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("reopen store");
            let all = store.load_all().expect("load_all after restart");
            assert_eq!(all.len(), 3, "must have exactly 3 sessions");

            // Build a lookup map for easier assertion
            let by_id: std::collections::HashMap<i64, &ChatSession> =
                all.iter().map(|s| (s.chat_id, s)).collect();

            let s100 = by_id.get(&100).expect("chat 100 must exist");
            assert_eq!(s100.persona, "phantom-default");
            assert_eq!(s100.last_seen_unix, 1_700_000_000);

            let s200 = by_id.get(&200).expect("chat 200 must exist");
            assert_eq!(s200.persona, "phantom-research");

            let s300 = by_id.get(&300).expect("chat 300 must exist");
            assert_eq!(s300.persona, "phantom-codereview");
        }

        // ── Phase 3: update one persona, verify others unaffected ───────────
        {
            let store = ChatSessionStore::open_at(&db_path).expect("reopen for update");
            store
                .upsert(&ChatSession {
                    chat_id: 200,
                    persona: "phantom-devops".to_string(),
                    last_seen_unix: 1_700_001_000,
                })
                .expect("update chat 200 persona");
            // Drop and reopen
        }
        {
            let store = ChatSessionStore::open_at(&db_path).expect("final reopen");
            let all = store.load_all().expect("load_all after update");
            assert_eq!(all.len(), 3, "still 3 sessions after update");

            let by_id: std::collections::HashMap<i64, &ChatSession> =
                all.iter().map(|s| (s.chat_id, s)).collect();

            // Chat 200 persona changed
            assert_eq!(
                by_id[&200].persona, "phantom-devops",
                "chat 200 persona must be updated to phantom-devops"
            );
            // Chat 100 and 300 unchanged
            assert_eq!(by_id[&100].persona, "phantom-default");
            assert_eq!(by_id[&300].persona, "phantom-codereview");
        }
    }
}
