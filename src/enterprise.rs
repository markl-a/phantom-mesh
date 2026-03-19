//! Enterprise Features (Phase I) — Team Management, Usage Reporting,
//! API Key Management, and Webhook System.
//!
//! All state is held in-memory (no SQLite dependency) so callers can
//! embed these managers inside an `Arc` and share across async tasks.

use anyhow::{bail, Result};
use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

// ════════════════════════════════════════════════════════════════════════════
//  1. Team Management
// ════════════════════════════════════════════════════════════════════════════

/// A member within a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub user: String,
    pub role: String, // "admin", "member", "viewer"
    pub joined_at: String,
}

/// Per-team resource quotas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamQuotas {
    pub max_tasks_per_day: u32,
    pub max_cost_usd_per_day: f64,
    pub max_members: u32,
}

impl Default for TeamQuotas {
    fn default() -> Self {
        Self {
            max_tasks_per_day: 1000,
            max_cost_usd_per_day: 50.0,
            max_members: 20,
        }
    }
}

/// A team with members, quotas, and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub name: String,
    pub members: Vec<TeamMember>,
    pub quotas: TeamQuotas,
    pub created_at: String,
}

/// Aggregated usage statistics for a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamUsage {
    pub team: String,
    pub total_tasks: u64,
    pub total_cost_usd: f64,
    pub active_members: u32,
}

/// In-memory team manager.
pub struct TeamManager {
    teams: Mutex<HashMap<String, Team>>,
    /// Per-team usage counters: team_name -> (tasks, cost_usd)
    usage: Mutex<HashMap<String, (u64, f64)>>,
}

impl TeamManager {
    pub fn new() -> Self {
        Self {
            teams: Mutex::new(HashMap::new()),
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new team. The creator is added as the initial admin member.
    pub fn create_team(&self, name: &str, admin: &str) -> Result<Team> {
        let mut teams = self.teams.lock().unwrap();
        if teams.contains_key(name) {
            bail!("Team '{}' already exists", name);
        }

        let team = Team {
            name: name.to_string(),
            members: vec![TeamMember {
                user: admin.to_string(),
                role: "admin".to_string(),
                joined_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            }],
            quotas: TeamQuotas::default(),
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        };
        info!("Created team '{}' with admin '{}'", name, admin);
        teams.insert(name.to_string(), team.clone());

        // Initialise usage counters
        self.usage
            .lock()
            .unwrap()
            .insert(name.to_string(), (0, 0.0));

        Ok(team)
    }

    /// Add a member to an existing team.
    pub fn add_member(&self, team: &str, user: &str, role: &str) -> Result<()> {
        let mut teams = self.teams.lock().unwrap();
        let t = teams.get_mut(team).ok_or_else(|| anyhow::anyhow!("Team '{}' not found", team))?;

        // Check quota
        if t.members.len() as u32 >= t.quotas.max_members {
            bail!(
                "Team '{}' has reached max members ({})",
                team,
                t.quotas.max_members
            );
        }

        // Prevent duplicate
        if t.members.iter().any(|m| m.user == user) {
            bail!("User '{}' is already a member of team '{}'", user, team);
        }

        t.members.push(TeamMember {
            user: user.to_string(),
            role: role.to_string(),
            joined_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        });
        debug!("Added '{}' (role={}) to team '{}'", user, role, team);
        Ok(())
    }

    /// Remove a member from a team.
    pub fn remove_member(&self, team: &str, user: &str) -> Result<()> {
        let mut teams = self.teams.lock().unwrap();
        let t = teams.get_mut(team).ok_or_else(|| anyhow::anyhow!("Team '{}' not found", team))?;
        let before = t.members.len();
        t.members.retain(|m| m.user != user);
        if t.members.len() == before {
            bail!("User '{}' is not a member of team '{}'", user, team);
        }
        debug!("Removed '{}' from team '{}'", user, team);
        Ok(())
    }

    /// Record usage for a team (called externally, e.g. after task execution).
    pub fn record_usage(&self, team: &str, tasks: u64, cost_usd: f64) {
        let mut usage = self.usage.lock().unwrap();
        let entry = usage.entry(team.to_string()).or_insert((0, 0.0));
        entry.0 += tasks;
        entry.1 += cost_usd;
    }

    /// Get aggregated usage for a team.
    pub fn get_team_usage(&self, team: &str) -> Result<TeamUsage> {
        let teams = self.teams.lock().unwrap();
        let t = teams.get(team).ok_or_else(|| anyhow::anyhow!("Team '{}' not found", team))?;
        let usage = self.usage.lock().unwrap();
        let (tasks, cost) = usage.get(team).copied().unwrap_or((0, 0.0));
        Ok(TeamUsage {
            team: team.to_string(),
            total_tasks: tasks,
            total_cost_usd: cost,
            active_members: t.members.len() as u32,
        })
    }

    /// Get a team by name.
    pub fn get_team(&self, name: &str) -> Option<Team> {
        self.teams.lock().unwrap().get(name).cloned()
    }

    /// List all team names.
    pub fn list_teams(&self) -> Vec<String> {
        self.teams.lock().unwrap().keys().cloned().collect()
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  2. Usage Reporting
// ════════════════════════════════════════════════════════════════════════════

/// Summary for a single day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReport {
    pub date: String,
    pub tasks_run: u64,
    pub tools_used: HashMap<String, u64>,
    pub total_cost_usd: f64,
    pub top_hands: Vec<(String, u64)>,
}

/// Weekly trends report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyReport {
    pub week_start: String,
    pub week_end: String,
    pub daily_reports: Vec<DailyReport>,
    pub total_tasks: u64,
    pub total_cost_usd: f64,
    pub avg_tasks_per_day: f64,
    pub avg_cost_per_day: f64,
}

/// In-memory usage reporter. Stores daily data keyed by date string (YYYY-MM-DD).
pub struct UsageReporter {
    /// date -> (tasks, tools map, cost, hands map)
    data: Mutex<HashMap<String, (u64, HashMap<String, u64>, f64, HashMap<String, u64>)>>,
}

impl UsageReporter {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Record a task execution for reporting.
    pub fn record(
        &self,
        date: &str,
        tool: &str,
        hand: Option<&str>,
        cost_usd: f64,
    ) {
        let mut data = self.data.lock().unwrap();
        let entry = data.entry(date.to_string()).or_insert_with(|| {
            (0, HashMap::new(), 0.0, HashMap::new())
        });
        entry.0 += 1;
        *entry.1.entry(tool.to_string()).or_insert(0) += 1;
        entry.2 += cost_usd;
        if let Some(h) = hand {
            *entry.3.entry(h.to_string()).or_insert(0) += 1;
        }
    }

    /// Generate a report for a specific date.
    pub fn daily_report(&self, date: &str) -> DailyReport {
        let data = self.data.lock().unwrap();
        if let Some((tasks, tools, cost, hands)) = data.get(date) {
            let mut top_hands: Vec<(String, u64)> =
                hands.iter().map(|(k, v)| (k.clone(), *v)).collect();
            top_hands.sort_by(|a, b| b.1.cmp(&a.1));
            top_hands.truncate(10);
            DailyReport {
                date: date.to_string(),
                tasks_run: *tasks,
                tools_used: tools.clone(),
                total_cost_usd: *cost,
                top_hands,
            }
        } else {
            DailyReport {
                date: date.to_string(),
                tasks_run: 0,
                tools_used: HashMap::new(),
                total_cost_usd: 0.0,
                top_hands: Vec::new(),
            }
        }
    }

    /// Generate a weekly report covering the last 7 days (or from available data).
    pub fn weekly_report(&self, dates: &[&str]) -> WeeklyReport {
        let daily: Vec<DailyReport> = dates.iter().map(|d| self.daily_report(d)).collect();
        let total_tasks: u64 = daily.iter().map(|d| d.tasks_run).sum();
        let total_cost: f64 = daily.iter().map(|d| d.total_cost_usd).sum();
        let count = dates.len().max(1) as f64;

        WeeklyReport {
            week_start: dates.first().unwrap_or(&"").to_string(),
            week_end: dates.last().unwrap_or(&"").to_string(),
            daily_reports: daily,
            total_tasks,
            total_cost_usd: total_cost,
            avg_tasks_per_day: total_tasks as f64 / count,
            avg_cost_per_day: total_cost / count,
        }
    }

    /// Export a daily report as CSV.
    pub fn export_csv(&self, report: &DailyReport) -> String {
        let mut csv = String::from("metric,value\n");
        csv.push_str(&format!("date,{}\n", report.date));
        csv.push_str(&format!("tasks_run,{}\n", report.tasks_run));
        csv.push_str(&format!("total_cost_usd,{:.4}\n", report.total_cost_usd));
        csv.push_str(&format!("tools_count,{}\n", report.tools_used.len()));
        csv.push_str(&format!("top_hands_count,{}\n", report.top_hands.len()));

        // Tool breakdown
        csv.push_str("\ntool,count\n");
        let mut tools: Vec<_> = report.tools_used.iter().collect();
        tools.sort_by(|a, b| b.1.cmp(a.1));
        for (tool, count) in tools {
            csv.push_str(&format!("{},{}\n", tool, count));
        }

        // Hand breakdown
        csv.push_str("\nhand,count\n");
        for (hand, count) in &report.top_hands {
            csv.push_str(&format!("{},{}\n", hand, count));
        }

        csv
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  3. API Key Management
// ════════════════════════════════════════════════════════════════════════════

/// Prefix stored on hashed keys to distinguish from legacy plaintext entries.
const HASH_PREFIX: &str = "sha256:";

/// An API key with metadata.
///
/// The `key` field stores **either**:
///   - A SHA-256 hash prefixed with `"sha256:"` (new keys), **or**
///   - The raw plaintext key (legacy / pre-hash keys).
///
/// Callers receive the plaintext key exactly once from [`ApiKeyManager::create_key`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<i64>, // Unix timestamp, None = never expires
    pub revoked: bool,
}

/// Info returned after successful key validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub name: String,
    pub permissions: Vec<String>,
    pub rate_limit_remaining: u32,
}

/// Generate a secure random API key with the given prefix.
fn generate_enterprise_key(prefix: &str) -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 24] = rng.gen();
    format!("{}_{}", prefix, hex::encode(bytes))
}

/// Generate a random API key with the `clawtex_` prefix and 32 hex characters.
///
/// Format: `clawtex_<32 hex chars>` (16 random bytes = 128 bits of entropy).
pub fn generate_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    format!("clawtex_{}", hex::encode(bytes))
}

/// Compute the SHA-256 hash of `key` and return it as a lowercase hex string
/// **without** the `"sha256:"` prefix.
pub fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Constant-time comparison of a provided plaintext key against a stored hash.
///
/// `stored_hash` may be:
///   - Prefixed with `"sha256:"` (the hash is the remainder), or
///   - A bare hex hash (64 hex chars).
///
/// Returns `true` if `hash_key(provided)` equals the stored hash.
pub fn verify_key(provided: &str, stored_hash: &str) -> bool {
    let expected = stored_hash.strip_prefix(HASH_PREFIX).unwrap_or(stored_hash);
    let computed = hash_key(provided);

    // Constant-time comparison to prevent timing attacks.
    // Both strings are lowercase hex of SHA-256 (64 chars).
    if computed.len() != expected.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in computed.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Rate-limit state for a single key.
struct KeyRateState {
    /// Remaining requests in the current window.
    remaining: u32,
    /// When the current window started (epoch secs).
    window_start: i64,
}

/// In-memory API key manager with per-key rate limiting.
///
/// New keys are stored as SHA-256 hashes (prefixed `"sha256:"`).
/// Legacy plaintext keys are still accepted for backward compatibility:
/// [`validate_key`](ApiKeyManager::validate_key) first tries a hash lookup,
/// then falls back to a plaintext lookup so pre-existing keys continue to work.
pub struct ApiKeyManager {
    /// Map from **lookup key** to `ApiKey`.
    ///
    /// For hashed keys the lookup key is `"sha256:<hex>"`.
    /// For legacy plaintext keys the lookup key is the raw key string.
    keys: Mutex<HashMap<String, ApiKey>>,
    /// Per-key rate limiting: lookup_key -> state
    rate_state: Mutex<HashMap<String, KeyRateState>>,
    /// Max requests per window
    rate_limit_per_window: u32,
    /// Window size in seconds
    rate_window_secs: i64,
}

impl ApiKeyManager {
    /// Create a new ApiKeyManager.
    /// `rate_limit_per_window` = max requests allowed per `rate_window_secs`.
    pub fn new(rate_limit_per_window: u32, rate_window_secs: i64) -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
            rate_state: Mutex::new(HashMap::new()),
            rate_limit_per_window,
            rate_window_secs,
        }
    }

    /// Create a new API key.
    ///
    /// Returns an [`ApiKey`] whose `.key` field contains the **plaintext** key
    /// (this is the only time the plaintext is available).  Internally, only
    /// the SHA-256 hash is stored.
    pub fn create_key(
        &self,
        name: &str,
        permissions: Vec<String>,
        expires_at: Option<i64>,
    ) -> ApiKey {
        let key_str = generate_enterprise_key("ent");
        let hashed = format!("{}{}", HASH_PREFIX, hash_key(&key_str));

        let stored_key = ApiKey {
            key: hashed.clone(),
            name: name.to_string(),
            permissions: permissions.clone(),
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            expires_at,
            revoked: false,
        };
        info!("Created enterprise API key '{}' (name={})", &key_str[..12], name);
        self.keys.lock().unwrap().insert(hashed.clone(), stored_key);

        // Init rate limiter state (keyed by hash)
        self.rate_state.lock().unwrap().insert(
            hashed,
            KeyRateState {
                remaining: self.rate_limit_per_window,
                window_start: Utc::now().timestamp(),
            },
        );

        // Return plaintext key to the caller (one-time reveal).
        ApiKey {
            key: key_str,
            name: name.to_string(),
            permissions,
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            expires_at,
            revoked: false,
        }
    }

    /// Resolve the internal lookup key for a caller-provided plaintext key.
    ///
    /// 1. Compute the SHA-256 hash and check if `"sha256:<hash>"` exists.
    /// 2. Fall back to the raw `key` string (legacy plaintext entry).
    /// 3. Return `None` if neither exists.
    fn resolve_lookup_key(&self, key: &str, keys: &HashMap<String, ApiKey>) -> Option<String> {
        // Try hashed lookup first
        let hashed = format!("{}{}", HASH_PREFIX, hash_key(key));
        if keys.contains_key(&hashed) {
            return Some(hashed);
        }
        // Backward-compat: try plaintext lookup
        if keys.contains_key(key) {
            return Some(key.to_string());
        }
        None
    }

    /// Validate an API key. Returns `Some(ApiKeyInfo)` if valid, `None` otherwise.
    /// Also decrements the rate-limit counter.
    ///
    /// Supports both hashed (new) and plaintext (legacy) stored keys.
    pub fn validate_key(&self, key: &str) -> Option<ApiKeyInfo> {
        let keys = self.keys.lock().unwrap();
        let lookup = self.resolve_lookup_key(key, &keys)?;
        let api_key = keys.get(&lookup)?;

        // Check revocation
        if api_key.revoked {
            debug!("Key '{}...' is revoked", &key[..12.min(key.len())]);
            return None;
        }

        // Check expiry
        if let Some(exp) = api_key.expires_at {
            if Utc::now().timestamp() > exp {
                debug!("Key '{}...' has expired", &key[..12.min(key.len())]);
                return None;
            }
        }

        // Rate limiting (keyed by lookup key)
        let mut rate = self.rate_state.lock().unwrap();
        let now = Utc::now().timestamp();
        let state = rate.entry(lookup.clone()).or_insert(KeyRateState {
            remaining: self.rate_limit_per_window,
            window_start: now,
        });

        // Reset window if expired
        if now - state.window_start >= self.rate_window_secs {
            state.remaining = self.rate_limit_per_window;
            state.window_start = now;
        }

        if state.remaining == 0 {
            debug!("Key '{}...' rate limited", &key[..12.min(key.len())]);
            return None;
        }

        state.remaining -= 1;

        Some(ApiKeyInfo {
            name: api_key.name.clone(),
            permissions: api_key.permissions.clone(),
            rate_limit_remaining: state.remaining,
        })
    }

    /// Revoke an API key. Returns true if the key existed and was revoked.
    ///
    /// Accepts the **plaintext** key; resolves via hash or plaintext lookup.
    pub fn revoke_key(&self, key: &str) -> bool {
        let mut keys = self.keys.lock().unwrap();
        let lookup = match self.resolve_lookup_key(key, &keys) {
            Some(l) => l,
            None => return false,
        };
        if let Some(api_key) = keys.get_mut(&lookup) {
            if api_key.revoked {
                return false; // Already revoked
            }
            api_key.revoked = true;
            info!("Revoked enterprise API key '{}...'", &key[..12.min(key.len())]);
            true
        } else {
            false
        }
    }

    /// List all keys (including revoked).
    ///
    /// Note: The `key` field in the returned entries contains the **hash**
    /// (for new keys) or the plaintext (for legacy keys).
    pub fn list_keys(&self) -> Vec<ApiKey> {
        self.keys.lock().unwrap().values().cloned().collect()
    }

    /// Get the remaining rate-limit count for a key without consuming a request.
    ///
    /// Accepts a **plaintext** key; resolves the internal lookup key.
    pub fn rate_limit_remaining(&self, key: &str) -> Option<u32> {
        let keys = self.keys.lock().unwrap();
        let lookup = self.resolve_lookup_key(key, &keys)?;
        drop(keys);

        let rate = self.rate_state.lock().unwrap();
        let now = Utc::now().timestamp();
        rate.get(&lookup).map(|s| {
            if now - s.window_start >= self.rate_window_secs {
                self.rate_limit_per_window // window would reset
            } else {
                s.remaining
            }
        })
    }

    /// Insert a **legacy plaintext** key directly into the store.
    ///
    /// This is provided for migration/backward-compatibility scenarios where
    /// pre-existing keys need to be loaded without re-hashing.
    #[allow(dead_code)]
    pub fn insert_legacy_plaintext_key(&self, api_key: ApiKey) {
        let lookup = api_key.key.clone();
        self.keys.lock().unwrap().insert(lookup.clone(), api_key);
        self.rate_state.lock().unwrap().insert(
            lookup,
            KeyRateState {
                remaining: self.rate_limit_per_window,
                window_start: Utc::now().timestamp(),
            },
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  4. Webhook System
// ════════════════════════════════════════════════════════════════════════════

/// Configuration for a registered webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub created_at: String,
    pub active: bool,
    pub secret: String, // shared secret for HMAC signing
}

/// Record of a single webhook delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: String,
    pub webhook_id: String,
    pub event: String,
    pub payload: Value,
    pub status_code: Option<u16>,
    pub success: bool,
    pub attempt: u32,
    pub delivered_at: String,
    pub error: Option<String>,
}

/// In-memory webhook manager with delivery tracking and retry support.
pub struct WebhookManager {
    webhooks: Mutex<HashMap<String, WebhookConfig>>,
    deliveries: Mutex<Vec<WebhookDelivery>>,
    max_retries: u32,
}

impl WebhookManager {
    pub fn new(max_retries: u32) -> Self {
        Self {
            webhooks: Mutex::new(HashMap::new()),
            deliveries: Mutex::new(Vec::new()),
            max_retries,
        }
    }

    /// Register a new webhook for the given events.
    pub fn register_webhook(&self, url: &str, events: Vec<String>) -> WebhookConfig {
        let id = uuid::Uuid::new_v4().to_string();
        let secret = generate_enterprise_key("whsec");
        let config = WebhookConfig {
            id: id.clone(),
            url: url.to_string(),
            events,
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            active: true,
            secret,
        };
        info!("Registered webhook '{}' for url '{}'", id, url);
        self.webhooks.lock().unwrap().insert(id, config.clone());
        config
    }

    /// Unregister (deactivate) a webhook by ID.
    pub fn unregister_webhook(&self, id: &str) -> bool {
        let mut hooks = self.webhooks.lock().unwrap();
        if let Some(wh) = hooks.get_mut(id) {
            wh.active = false;
            info!("Unregistered webhook '{}'", id);
            true
        } else {
            false
        }
    }

    /// Get all active webhooks subscribed to a specific event.
    pub fn webhooks_for_event(&self, event: &str) -> Vec<WebhookConfig> {
        self.webhooks
            .lock()
            .unwrap()
            .values()
            .filter(|wh| wh.active && wh.events.iter().any(|e| e == event || e == "*"))
            .cloned()
            .collect()
    }

    /// Fire a webhook event. In production this would do async HTTP POSTs;
    /// here we record deliveries for tracking and testability.
    ///
    /// Returns the list of delivery records created.
    pub fn fire_webhook(&self, event: &str, payload: Value) -> Vec<WebhookDelivery> {
        let targets = self.webhooks_for_event(event);
        let mut results = Vec::new();

        for wh in targets {
            let delivery = WebhookDelivery {
                id: uuid::Uuid::new_v4().to_string(),
                webhook_id: wh.id.clone(),
                event: event.to_string(),
                payload: payload.clone(),
                status_code: Some(200), // simulated success
                success: true,
                attempt: 1,
                delivered_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                error: None,
            };
            debug!(
                "Fired webhook '{}' for event '{}' -> {}",
                wh.id, event, wh.url
            );
            results.push(delivery.clone());
            self.deliveries.lock().unwrap().push(delivery);
        }

        results
    }

    /// Record a failed delivery attempt (for retry tracking).
    pub fn record_failure(
        &self,
        webhook_id: &str,
        event: &str,
        payload: Value,
        attempt: u32,
        error: &str,
    ) -> Option<WebhookDelivery> {
        let delivery = WebhookDelivery {
            id: uuid::Uuid::new_v4().to_string(),
            webhook_id: webhook_id.to_string(),
            event: event.to_string(),
            payload,
            status_code: None,
            success: false,
            attempt,
            delivered_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            error: Some(error.to_string()),
        };
        self.deliveries.lock().unwrap().push(delivery.clone());

        if attempt < self.max_retries {
            warn!(
                "Webhook '{}' delivery failed (attempt {}/{}): {}",
                webhook_id, attempt, self.max_retries, error
            );
            Some(delivery)
        } else {
            warn!(
                "Webhook '{}' delivery exhausted retries ({}/{}): {}",
                webhook_id, attempt, self.max_retries, error
            );
            None
        }
    }

    /// Get all delivery records for a webhook.
    pub fn deliveries_for_webhook(&self, webhook_id: &str) -> Vec<WebhookDelivery> {
        self.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|d| d.webhook_id == webhook_id)
            .cloned()
            .collect()
    }

    /// Check if there are pending retries below max for a given webhook+event.
    pub fn should_retry(&self, webhook_id: &str, event: &str) -> bool {
        let deliveries = self.deliveries.lock().unwrap();
        let last_attempt = deliveries
            .iter()
            .filter(|d| d.webhook_id == webhook_id && d.event == event && !d.success)
            .map(|d| d.attempt)
            .max()
            .unwrap_or(0);
        last_attempt < self.max_retries
    }

    /// List all registered webhooks (active and inactive).
    pub fn list_webhooks(&self) -> Vec<WebhookConfig> {
        self.webhooks.lock().unwrap().values().cloned().collect()
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Team Management Tests ──────────────────────────────────────────────

    #[test]
    fn test_create_team() {
        let mgr = TeamManager::new();
        let team = mgr.create_team("engineering", "alice").unwrap();
        assert_eq!(team.name, "engineering");
        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].user, "alice");
        assert_eq!(team.members[0].role, "admin");
        assert!(!team.created_at.is_empty());
    }

    #[test]
    fn test_create_duplicate_team() {
        let mgr = TeamManager::new();
        mgr.create_team("devops", "bob").unwrap();
        let result = mgr.create_team("devops", "carol");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_add_member() {
        let mgr = TeamManager::new();
        mgr.create_team("backend", "alice").unwrap();
        mgr.add_member("backend", "bob", "member").unwrap();
        let team = mgr.get_team("backend").unwrap();
        assert_eq!(team.members.len(), 2);
        assert_eq!(team.members[1].user, "bob");
        assert_eq!(team.members[1].role, "member");
    }

    #[test]
    fn test_add_duplicate_member() {
        let mgr = TeamManager::new();
        mgr.create_team("frontend", "alice").unwrap();
        let result = mgr.add_member("frontend", "alice", "member");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already a member"));
    }

    #[test]
    fn test_add_member_team_not_found() {
        let mgr = TeamManager::new();
        let result = mgr.add_member("nonexistent", "alice", "member");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_remove_member() {
        let mgr = TeamManager::new();
        mgr.create_team("data", "alice").unwrap();
        mgr.add_member("data", "bob", "member").unwrap();
        mgr.remove_member("data", "bob").unwrap();
        let team = mgr.get_team("data").unwrap();
        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].user, "alice");
    }

    #[test]
    fn test_remove_member_not_found() {
        let mgr = TeamManager::new();
        mgr.create_team("infra", "alice").unwrap();
        let result = mgr.remove_member("infra", "ghost");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a member"));
    }

    #[test]
    fn test_team_usage_tracking() {
        let mgr = TeamManager::new();
        mgr.create_team("ml", "alice").unwrap();
        mgr.record_usage("ml", 10, 1.5);
        mgr.record_usage("ml", 5, 0.75);
        let usage = mgr.get_team_usage("ml").unwrap();
        assert_eq!(usage.team, "ml");
        assert_eq!(usage.total_tasks, 15);
        assert!((usage.total_cost_usd - 2.25).abs() < 0.001);
        assert_eq!(usage.active_members, 1);
    }

    #[test]
    fn test_team_usage_not_found() {
        let mgr = TeamManager::new();
        let result = mgr.get_team_usage("nope");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_teams() {
        let mgr = TeamManager::new();
        mgr.create_team("alpha", "a").unwrap();
        mgr.create_team("beta", "b").unwrap();
        let teams = mgr.list_teams();
        assert_eq!(teams.len(), 2);
        assert!(teams.contains(&"alpha".to_string()));
        assert!(teams.contains(&"beta".to_string()));
    }

    #[test]
    fn test_team_quotas_default() {
        let q = TeamQuotas::default();
        assert_eq!(q.max_tasks_per_day, 1000);
        assert!((q.max_cost_usd_per_day - 50.0).abs() < 0.01);
        assert_eq!(q.max_members, 20);
    }

    // ── Usage Reporting Tests ──────────────────────────────────────────────

    #[test]
    fn test_daily_report_empty() {
        let reporter = UsageReporter::new();
        let report = reporter.daily_report("2026-03-18");
        assert_eq!(report.date, "2026-03-18");
        assert_eq!(report.tasks_run, 0);
        assert!(report.tools_used.is_empty());
        assert_eq!(report.total_cost_usd, 0.0);
        assert!(report.top_hands.is_empty());
    }

    #[test]
    fn test_daily_report_with_data() {
        let reporter = UsageReporter::new();
        reporter.record("2026-03-18", "web_search", Some("content"), 0.01);
        reporter.record("2026-03-18", "web_search", Some("content"), 0.02);
        reporter.record("2026-03-18", "shell", Some("seo_content"), 0.05);
        let report = reporter.daily_report("2026-03-18");
        assert_eq!(report.tasks_run, 3);
        assert_eq!(report.tools_used["web_search"], 2);
        assert_eq!(report.tools_used["shell"], 1);
        assert!((report.total_cost_usd - 0.08).abs() < 0.001);
        assert!(!report.top_hands.is_empty());
    }

    #[test]
    fn test_weekly_report() {
        let reporter = UsageReporter::new();
        reporter.record("2026-03-11", "web_search", None, 0.1);
        reporter.record("2026-03-12", "shell", None, 0.2);
        reporter.record("2026-03-13", "web_search", None, 0.05);
        let dates = vec!["2026-03-11", "2026-03-12", "2026-03-13"];
        let weekly = reporter.weekly_report(&dates);
        assert_eq!(weekly.week_start, "2026-03-11");
        assert_eq!(weekly.week_end, "2026-03-13");
        assert_eq!(weekly.total_tasks, 3);
        assert!((weekly.total_cost_usd - 0.35).abs() < 0.001);
        assert!((weekly.avg_tasks_per_day - 1.0).abs() < 0.001);
        assert_eq!(weekly.daily_reports.len(), 3);
    }

    #[test]
    fn test_export_csv() {
        let reporter = UsageReporter::new();
        reporter.record("2026-03-18", "web_search", Some("content"), 0.03);
        reporter.record("2026-03-18", "shell", None, 0.01);
        let report = reporter.daily_report("2026-03-18");
        let csv = reporter.export_csv(&report);
        assert!(csv.contains("metric,value"));
        assert!(csv.contains("date,2026-03-18"));
        assert!(csv.contains("tasks_run,2"));
        assert!(csv.contains("tool,count"));
        assert!(csv.contains("web_search,1"));
        assert!(csv.contains("shell,1"));
    }

    #[test]
    fn test_top_hands_sorted() {
        let reporter = UsageReporter::new();
        for _ in 0..5 {
            reporter.record("2026-03-18", "web_search", Some("content"), 0.01);
        }
        for _ in 0..3 {
            reporter.record("2026-03-18", "shell", Some("seo"), 0.01);
        }
        for _ in 0..8 {
            reporter.record("2026-03-18", "http_request", Some("lead"), 0.01);
        }
        let report = reporter.daily_report("2026-03-18");
        assert_eq!(report.top_hands[0].0, "lead");
        assert_eq!(report.top_hands[0].1, 8);
        assert_eq!(report.top_hands[1].0, "content");
        assert_eq!(report.top_hands[1].1, 5);
    }

    // ── API Key Management Tests ───────────────────────────────────────────

    #[test]
    fn test_create_api_key() {
        let mgr = ApiKeyManager::new(100, 3600);
        let key = mgr.create_key("test-service", vec!["read".into(), "write".into()], None);
        // Returned key is plaintext (starts with ent_)
        assert!(key.key.starts_with("ent_"));
        assert_eq!(key.name, "test-service");
        assert_eq!(key.permissions, vec!["read", "write"]);
        assert!(!key.revoked);
        assert!(key.expires_at.is_none());
    }

    #[test]
    fn test_create_key_stores_hash_not_plaintext() {
        let mgr = ApiKeyManager::new(100, 3600);
        let key = mgr.create_key("hashed-svc", vec![], None);
        let plaintext = key.key.clone();
        // The stored entries must NOT contain the plaintext key
        let stored = mgr.list_keys();
        assert_eq!(stored.len(), 1);
        assert_ne!(stored[0].key, plaintext);
        assert!(stored[0].key.starts_with("sha256:"));
    }

    #[test]
    fn test_validate_key_success() {
        let mgr = ApiKeyManager::new(100, 3600);
        let key = mgr.create_key("svc", vec!["read".into()], None);
        let info = mgr.validate_key(&key.key).unwrap();
        assert_eq!(info.name, "svc");
        assert_eq!(info.permissions, vec!["read"]);
        assert_eq!(info.rate_limit_remaining, 99); // one consumed
    }

    #[test]
    fn test_validate_key_revoked() {
        let mgr = ApiKeyManager::new(100, 3600);
        let key = mgr.create_key("revoke-test", vec![], None);
        mgr.revoke_key(&key.key);
        assert!(mgr.validate_key(&key.key).is_none());
    }

    #[test]
    fn test_validate_key_expired() {
        let mgr = ApiKeyManager::new(100, 3600);
        // Expired 1 hour ago
        let expired_ts = Utc::now().timestamp() - 3600;
        let key = mgr.create_key("expired-svc", vec![], Some(expired_ts));
        assert!(mgr.validate_key(&key.key).is_none());
    }

    #[test]
    fn test_validate_key_nonexistent() {
        let mgr = ApiKeyManager::new(100, 3600);
        assert!(mgr.validate_key("ent_nonexistent_key_12345678901234567890").is_none());
    }

    #[test]
    fn test_rate_limiting() {
        let mgr = ApiKeyManager::new(3, 3600); // Only 3 per window
        let key = mgr.create_key("limited", vec![], None);
        assert!(mgr.validate_key(&key.key).is_some()); // 1
        assert!(mgr.validate_key(&key.key).is_some()); // 2
        assert!(mgr.validate_key(&key.key).is_some()); // 3
        assert!(mgr.validate_key(&key.key).is_none()); // rate limited
    }

    #[test]
    fn test_revoke_key() {
        let mgr = ApiKeyManager::new(100, 3600);
        let key = mgr.create_key("rev", vec![], None);
        assert!(mgr.revoke_key(&key.key));
        // Double revoke returns false
        assert!(!mgr.revoke_key(&key.key));
    }

    #[test]
    fn test_revoke_nonexistent_key() {
        let mgr = ApiKeyManager::new(100, 3600);
        assert!(!mgr.revoke_key("ent_does_not_exist_123456789012345678901234"));
    }

    #[test]
    fn test_list_keys() {
        let mgr = ApiKeyManager::new(100, 3600);
        mgr.create_key("a", vec![], None);
        mgr.create_key("b", vec![], None);
        mgr.create_key("c", vec![], None);
        let keys = mgr.list_keys();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn test_key_uniqueness() {
        let k1 = generate_enterprise_key("ent");
        let k2 = generate_enterprise_key("ent");
        assert_ne!(k1, k2);
        assert!(k1.starts_with("ent_"));
        assert!(k2.starts_with("ent_"));
    }

    #[test]
    fn test_rate_limit_remaining_check() {
        let mgr = ApiKeyManager::new(10, 3600);
        let key = mgr.create_key("peek", vec![], None);
        // Before any validation
        assert_eq!(mgr.rate_limit_remaining(&key.key), Some(10));
        mgr.validate_key(&key.key);
        assert_eq!(mgr.rate_limit_remaining(&key.key), Some(9));
    }

    // ── API Key Security Tests (hash_key, verify_key, generate_key) ──────

    #[test]
    fn test_hash_key_deterministic() {
        let h1 = hash_key("clawtex_abcdef1234567890abcdef12");
        let h2 = hash_key("clawtex_abcdef1234567890abcdef12");
        assert_eq!(h1, h2);
        // SHA-256 produces 64 hex chars
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn test_hash_key_different_inputs() {
        let h1 = hash_key("key_aaa");
        let h2 = hash_key("key_bbb");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_verify_key_correct() {
        let plaintext = "clawtex_test1234567890abcdef1234";
        let stored = format!("sha256:{}", hash_key(plaintext));
        assert!(verify_key(plaintext, &stored));
    }

    #[test]
    fn test_verify_key_incorrect() {
        let stored = format!("sha256:{}", hash_key("correct_key"));
        assert!(!verify_key("wrong_key", &stored));
    }

    #[test]
    fn test_verify_key_bare_hash() {
        // verify_key should also work with a bare hex hash (no "sha256:" prefix)
        let plaintext = "some_api_key_value";
        let bare_hash = hash_key(plaintext);
        assert!(verify_key(plaintext, &bare_hash));
        assert!(!verify_key("different_key", &bare_hash));
    }

    #[test]
    fn test_generate_key_format() {
        let key = generate_key();
        assert!(key.starts_with("clawtex_"), "key should start with clawtex_");
        // "clawtex_" (8 chars) + 32 hex chars = 40 total
        assert_eq!(key.len(), 40, "key should be 40 chars total");
        // The hex part should be valid hex
        let hex_part = &key[8..];
        assert_eq!(hex_part.len(), 32);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_key_uniqueness() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert_ne!(k1, k2, "generated keys should be unique");
        assert!(k1.starts_with("clawtex_"));
        assert!(k2.starts_with("clawtex_"));
    }

    #[test]
    fn test_backward_compat_plaintext_key() {
        // Simulate a legacy key stored as plaintext (pre-hash migration).
        let mgr = ApiKeyManager::new(100, 3600);
        let legacy_plaintext = "ent_legacy_plaintext_key_abcdef0123456789";
        let legacy = ApiKey {
            key: legacy_plaintext.to_string(),
            name: "legacy-svc".to_string(),
            permissions: vec!["read".into()],
            created_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: None,
            revoked: false,
        };
        mgr.insert_legacy_plaintext_key(legacy);

        // Validate with the plaintext key should still work
        let info = mgr.validate_key(legacy_plaintext).unwrap();
        assert_eq!(info.name, "legacy-svc");
        assert_eq!(info.permissions, vec!["read"]);
    }

    #[test]
    fn test_verify_key_constant_time_length_mismatch() {
        // If stored hash has wrong length, verify_key should return false
        // without panicking
        assert!(!verify_key("some_key", "tooshort"));
        assert!(!verify_key("some_key", ""));
    }

    #[test]
    fn test_hash_key_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let empty_hash = hash_key("");
        assert_eq!(
            empty_hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // ── Webhook System Tests ───────────────────────────────────────────────

    #[test]
    fn test_register_webhook() {
        let mgr = WebhookManager::new(3);
        let wh = mgr.register_webhook(
            "https://example.com/hook",
            vec!["task.completed".into(), "hand.finished".into()],
        );
        assert!(!wh.id.is_empty());
        assert_eq!(wh.url, "https://example.com/hook");
        assert_eq!(wh.events.len(), 2);
        assert!(wh.active);
        assert!(wh.secret.starts_with("whsec_"));
    }

    #[test]
    fn test_fire_webhook() {
        let mgr = WebhookManager::new(3);
        mgr.register_webhook("https://a.com/hook", vec!["task.done".into()]);
        let deliveries = mgr.fire_webhook("task.done", serde_json::json!({"task_id": "123"}));
        assert_eq!(deliveries.len(), 1);
        assert!(deliveries[0].success);
        assert_eq!(deliveries[0].event, "task.done");
        assert_eq!(deliveries[0].status_code, Some(200));
    }

    #[test]
    fn test_fire_webhook_no_subscribers() {
        let mgr = WebhookManager::new(3);
        mgr.register_webhook("https://a.com/hook", vec!["task.done".into()]);
        // Fire an event nobody subscribed to
        let deliveries = mgr.fire_webhook("hand.failed", serde_json::json!({}));
        assert!(deliveries.is_empty());
    }

    #[test]
    fn test_fire_webhook_wildcard() {
        let mgr = WebhookManager::new(3);
        mgr.register_webhook("https://all.com/hook", vec!["*".into()]);
        let d1 = mgr.fire_webhook("task.done", serde_json::json!({}));
        let d2 = mgr.fire_webhook("hand.failed", serde_json::json!({}));
        assert_eq!(d1.len(), 1);
        assert_eq!(d2.len(), 1);
    }

    #[test]
    fn test_unregister_webhook() {
        let mgr = WebhookManager::new(3);
        let wh = mgr.register_webhook("https://a.com/hook", vec!["task.done".into()]);
        assert!(mgr.unregister_webhook(&wh.id));
        // Should no longer fire
        let deliveries = mgr.fire_webhook("task.done", serde_json::json!({}));
        assert!(deliveries.is_empty());
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mgr = WebhookManager::new(3);
        assert!(!mgr.unregister_webhook("nonexistent-id"));
    }

    #[test]
    fn test_delivery_tracking() {
        let mgr = WebhookManager::new(3);
        let wh = mgr.register_webhook("https://a.com/hook", vec!["ev".into()]);
        mgr.fire_webhook("ev", serde_json::json!({"n": 1}));
        mgr.fire_webhook("ev", serde_json::json!({"n": 2}));
        let deliveries = mgr.deliveries_for_webhook(&wh.id);
        assert_eq!(deliveries.len(), 2);
    }

    #[test]
    fn test_record_failure_and_retry() {
        let mgr = WebhookManager::new(3);
        let wh = mgr.register_webhook("https://fail.com/hook", vec!["err".into()]);

        // First failure
        let d = mgr.record_failure(&wh.id, "err", serde_json::json!({}), 1, "timeout");
        assert!(d.is_some()); // can still retry
        assert!(mgr.should_retry(&wh.id, "err"));

        // Second failure
        mgr.record_failure(&wh.id, "err", serde_json::json!({}), 2, "timeout");
        assert!(mgr.should_retry(&wh.id, "err"));

        // Third failure (max retries reached)
        let d = mgr.record_failure(&wh.id, "err", serde_json::json!({}), 3, "timeout");
        assert!(d.is_none()); // retries exhausted
        assert!(!mgr.should_retry(&wh.id, "err"));
    }

    #[test]
    fn test_multiple_webhooks_same_event() {
        let mgr = WebhookManager::new(3);
        mgr.register_webhook("https://a.com/hook", vec!["task.done".into()]);
        mgr.register_webhook("https://b.com/hook", vec!["task.done".into()]);
        mgr.register_webhook("https://c.com/hook", vec!["other.event".into()]);
        let deliveries = mgr.fire_webhook("task.done", serde_json::json!({}));
        assert_eq!(deliveries.len(), 2); // a and b, not c
    }

    #[test]
    fn test_list_webhooks() {
        let mgr = WebhookManager::new(3);
        mgr.register_webhook("https://a.com", vec!["a".into()]);
        mgr.register_webhook("https://b.com", vec!["b".into()]);
        let hooks = mgr.list_webhooks();
        assert_eq!(hooks.len(), 2);
    }

    #[test]
    fn test_add_member_exceeds_quota() {
        let mgr = TeamManager::new();
        mgr.create_team("tiny", "admin").unwrap();

        // Override quota to max 2 members
        {
            let mut teams = mgr.teams.lock().unwrap();
            teams.get_mut("tiny").unwrap().quotas.max_members = 2;
        }

        mgr.add_member("tiny", "user2", "member").unwrap(); // 2nd member, ok
        let result = mgr.add_member("tiny", "user3", "member"); // 3rd, over quota
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("max members"));
    }
}
