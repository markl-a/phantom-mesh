//! Smoke Test Suite -- built-in deployment validation.
//!
//! Provides a `SmokeTestSuite` that runs a series of quick checks to verify
//! that the core subsystems (config, database, tools, hands, providers,
//! encryption, memory, rate-limiter, networking, cluster, cost tracker, i18n)
//! are functioning correctly after deployment.
//!
//! # CLI integration hint
//!
//! ```ignore
//! // In main.rs, add a subcommand:
//! // clawtex-core smoke-test
//! //
//! // let report = clawtex_core::smoke_test::SmokeTestSuite::default_suite()
//! //     .run_all().await;
//! // println!("{}", report.to_text());
//! // std::process::exit(if report.failed == 0 { 0 } else { 1 });
//! ```

use std::future::Future;
use std::net::TcpListener;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Outcome of a single smoke test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmokeTestResult {
    Passed,
    Failed(String),
    Skipped(String),
}

impl SmokeTestResult {
    pub fn is_passed(&self) -> bool {
        matches!(self, SmokeTestResult::Passed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, SmokeTestResult::Failed(_))
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, SmokeTestResult::Skipped(_))
    }
}

impl std::fmt::Display for SmokeTestResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmokeTestResult::Passed => write!(f, "PASS"),
            SmokeTestResult::Failed(reason) => write!(f, "FAIL: {}", reason),
            SmokeTestResult::Skipped(reason) => write!(f, "SKIP: {}", reason),
        }
    }
}

// ---------------------------------------------------------------------------
// Single smoke test
// ---------------------------------------------------------------------------

/// A named smoke test backed by an async function.
pub struct SmokeTest {
    pub name: String,
    pub description: String,
    test_fn: Box<dyn Fn() -> Pin<Box<dyn Future<Output = SmokeTestResult> + Send>> + Send + Sync>,
}

impl SmokeTest {
    /// Create a new smoke test from a name, description, and async closure.
    pub fn new<F, Fut>(name: &str, description: &str, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SmokeTestResult> + Send + 'static,
    {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            test_fn: Box::new(move || Box::pin(f())),
        }
    }

    /// Run the test and return its result.
    pub async fn run(&self) -> SmokeTestResult {
        (self.test_fn)().await
    }
}

// ---------------------------------------------------------------------------
// Detail entry (result + duration per test)
// ---------------------------------------------------------------------------

/// Detail for a single test execution inside a report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmokeTestDetail {
    pub name: String,
    pub description: String,
    pub result: SmokeTestResult,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// Aggregate report produced by `SmokeTestSuite::run_all`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmokeTestReport {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total: usize,
    pub total_duration_ms: u64,
    pub details: Vec<SmokeTestDetail>,
}

impl SmokeTestReport {
    /// Serialize the report as a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Render the report as a human-readable text block.
    pub fn to_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("=== Clawtex Smoke Test Report ===".to_string());
        lines.push(String::new());

        for detail in &self.details {
            let status = match &detail.result {
                SmokeTestResult::Passed => "[PASS]".to_string(),
                SmokeTestResult::Failed(r) => format!("[FAIL] {}", r),
                SmokeTestResult::Skipped(r) => format!("[SKIP] {}", r),
            };
            lines.push(format!(
                "  {} {} ({}ms) -- {}",
                status, detail.name, detail.duration_ms, detail.description,
            ));
        }

        lines.push(String::new());
        lines.push(format!(
            "Total: {} | Passed: {} | Failed: {} | Skipped: {} | Duration: {}ms",
            self.total, self.passed, self.failed, self.skipped, self.total_duration_ms,
        ));
        lines.join("\n")
    }

    /// Return true when every test passed (none failed).
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

/// Ordered collection of smoke tests.
pub struct SmokeTestSuite {
    tests: Vec<SmokeTest>,
}

impl SmokeTestSuite {
    /// Create an empty suite.
    pub fn new() -> Self {
        Self { tests: Vec::new() }
    }

    /// Add a test to the suite.
    pub fn add(&mut self, test: SmokeTest) {
        self.tests.push(test);
    }

    /// Number of tests in the suite.
    pub fn len(&self) -> usize {
        self.tests.len()
    }

    /// Whether the suite is empty.
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    /// Run all tests sequentially and produce a report.
    pub async fn run_all(&self) -> SmokeTestReport {
        let suite_start = Instant::now();
        let mut details = Vec::with_capacity(self.tests.len());
        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;

        for test in &self.tests {
            let test_start = Instant::now();
            let result = test.run().await;
            let duration_ms = test_start.elapsed().as_millis() as u64;

            match &result {
                SmokeTestResult::Passed => {
                    info!("[smoke] PASS: {}", test.name);
                    passed += 1;
                }
                SmokeTestResult::Failed(reason) => {
                    warn!("[smoke] FAIL: {} -- {}", test.name, reason);
                    failed += 1;
                }
                SmokeTestResult::Skipped(reason) => {
                    debug!("[smoke] SKIP: {} -- {}", test.name, reason);
                    skipped += 1;
                }
            }

            details.push(SmokeTestDetail {
                name: test.name.clone(),
                description: test.description.clone(),
                result,
                duration_ms,
            });
        }

        let total_duration_ms = suite_start.elapsed().as_millis() as u64;

        SmokeTestReport {
            passed,
            failed,
            skipped,
            total: self.tests.len(),
            total_duration_ms,
            details,
        }
    }

    /// Build the default suite containing all built-in smoke tests.
    ///
    /// Each test is self-contained and uses temporary resources where possible
    /// so that the suite can run safely in any environment.
    pub fn default_suite() -> Self {
        let mut suite = Self::new();

        suite.add(SmokeTest::new(
            "config_loadable",
            "agents.toml can be parsed as valid TOML",
            test_config_loadable,
        ));
        suite.add(SmokeTest::new(
            "workspace_writable",
            "~/.clawtex/workspace/ is writable",
            test_workspace_writable,
        ));
        suite.add(SmokeTest::new(
            "database_accessible",
            "SQLite databases can be opened and created",
            test_database_accessible,
        ));
        suite.add(SmokeTest::new(
            "tools_registered",
            "ToolRegistry loads all tools without panic",
            test_tools_registered,
        ));
        suite.add(SmokeTest::new(
            "hands_loadable",
            "All hand.toml files parse successfully",
            test_hands_loadable,
        ));
        suite.add(SmokeTest::new(
            "providers_configured",
            "At least one LLM provider is configured",
            test_providers_configured,
        ));
        suite.add(SmokeTest::new(
            "encryption_roundtrip",
            "ChaCha20-Poly1305 encrypt/decrypt roundtrip works",
            test_encryption_roundtrip,
        ));
        suite.add(SmokeTest::new(
            "memory_store_recall",
            "Memory DB store and keyword recall works",
            test_memory_store_recall,
        ));
        suite.add(SmokeTest::new(
            "rate_limiter_works",
            "ActionTracker records and counts correctly",
            test_rate_limiter_works,
        ));
        suite.add(SmokeTest::new(
            "http_server_binds",
            "TCP listener can bind to configured port and release",
            test_http_server_binds,
        ));
        suite.add(SmokeTest::new(
            "cluster_registry_works",
            "ClusterRegistry can be created and registers local node",
            test_cluster_registry_works,
        ));
        suite.add(SmokeTest::new(
            "cost_tracker_works",
            "CostTracker DB read/write functions correctly",
            test_cost_tracker_works,
        ));
        suite.add(SmokeTest::new(
            "i18n_loaded",
            "Translations available for all built-in locales",
            test_i18n_loaded,
        ));

        suite
    }
}

// ---------------------------------------------------------------------------
// Helper: resolve ~/.clawtex directory
// ---------------------------------------------------------------------------

fn clawtex_home() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".clawtex")
}

// ---------------------------------------------------------------------------
// Built-in smoke tests
// ---------------------------------------------------------------------------

/// Test that `~/.clawtex/agents.toml` exists and is valid TOML.
async fn test_config_loadable() -> SmokeTestResult {
    let config_path = clawtex_home().join("agents.toml");
    if !config_path.exists() {
        return SmokeTestResult::Skipped(format!(
            "agents.toml not found at {}",
            config_path.display()
        ));
    }
    match std::fs::read_to_string(&config_path) {
        Ok(content) => match content.parse::<toml::Table>() {
            Ok(_) => SmokeTestResult::Passed,
            Err(e) => SmokeTestResult::Failed(format!("TOML parse error: {}", e)),
        },
        Err(e) => SmokeTestResult::Failed(format!("Cannot read agents.toml: {}", e)),
    }
}

/// Test that the workspace directory is writable.
async fn test_workspace_writable() -> SmokeTestResult {
    let ws = clawtex_home().join("workspace");
    if let Err(e) = std::fs::create_dir_all(&ws) {
        return SmokeTestResult::Failed(format!("Cannot create workspace dir: {}", e));
    }
    let probe = ws.join(".smoke_test_probe");
    match std::fs::write(&probe, b"smoke") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            SmokeTestResult::Passed
        }
        Err(e) => SmokeTestResult::Failed(format!("Cannot write to workspace: {}", e)),
    }
}

/// Test that SQLite databases can be opened/created.
async fn test_database_accessible() -> SmokeTestResult {
    let tmp = std::env::temp_dir().join("clawtex_smoke_test.db");
    let tmp_str = tmp.to_string_lossy().to_string();
    match rusqlite::Connection::open(&tmp_str) {
        Ok(conn) => {
            let res = conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS smoke_check (id INTEGER PRIMARY KEY); DROP TABLE smoke_check;",
            );
            let _ = std::fs::remove_file(&tmp);
            match res {
                Ok(()) => SmokeTestResult::Passed,
                Err(e) => SmokeTestResult::Failed(format!("SQLite execute error: {}", e)),
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            SmokeTestResult::Failed(format!("Cannot open SQLite DB: {}", e))
        }
    }
}

/// Test that the ToolRegistry can be created and all tools load without panic.
async fn test_tools_registered() -> SmokeTestResult {
    use crate::tools::{SecurityConfig, ToolRegistry};

    let security = SecurityConfig {
        workspace_dir: std::env::temp_dir()
            .join("clawtex_smoke_ws")
            .to_string_lossy()
            .to_string(),
        workspace_only: true,
        allowed_commands: vec!["echo".to_string()],
        rate_limit: Default::default(),
        allowed_paths: vec![],
    };

    // Catch panics from tool registration.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ToolRegistry::new(security)
    }));

    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("clawtex_smoke_ws"));

    match result {
        Ok(registry) => {
            let names = registry.names();
            if names.is_empty() {
                SmokeTestResult::Failed("ToolRegistry loaded 0 tools".to_string())
            } else {
                SmokeTestResult::Passed
            }
        }
        Err(_) => SmokeTestResult::Failed("ToolRegistry panicked during construction".to_string()),
    }
}

/// Test that hand.toml files in `~/.clawtex/hands/` parse correctly.
async fn test_hands_loadable() -> SmokeTestResult {
    use crate::hands::HandRegistry;

    let hands_dir = clawtex_home().join("hands");
    let hands_str = hands_dir.to_string_lossy().to_string();

    if !hands_dir.exists() {
        return SmokeTestResult::Skipped("Hands directory does not exist".to_string());
    }

    match HandRegistry::load(&hands_str) {
        Ok(registry) => {
            let names = registry.names();
            if names.is_empty() {
                SmokeTestResult::Skipped("No hands found in hands directory".to_string())
            } else {
                SmokeTestResult::Passed
            }
        }
        Err(e) => SmokeTestResult::Failed(format!("HandRegistry load failed: {}", e)),
    }
}

/// Test that at least one provider is configured in agents.toml.
async fn test_providers_configured() -> SmokeTestResult {
    let config_path = clawtex_home().join("agents.toml");
    if !config_path.exists() {
        return SmokeTestResult::Skipped("agents.toml not found".to_string());
    }
    match std::fs::read_to_string(&config_path) {
        Ok(content) => match content.parse::<toml::Table>() {
            Ok(table) => {
                if let Some(providers) = table.get("providers") {
                    if let Some(map) = providers.as_table() {
                        if map.is_empty() {
                            SmokeTestResult::Failed(
                                "No providers configured in [providers] section".to_string(),
                            )
                        } else {
                            SmokeTestResult::Passed
                        }
                    } else {
                        SmokeTestResult::Failed("[providers] is not a table".to_string())
                    }
                } else {
                    SmokeTestResult::Failed(
                        "No [providers] section found in agents.toml".to_string(),
                    )
                }
            }
            Err(e) => SmokeTestResult::Failed(format!("TOML parse error: {}", e)),
        },
        Err(e) => SmokeTestResult::Failed(format!("Cannot read agents.toml: {}", e)),
    }
}

/// Test that ChaCha20-Poly1305 encryption round-trips correctly.
async fn test_encryption_roundtrip() -> SmokeTestResult {
    use crate::security::SecretManager;

    let key = [42u8; 32];
    let sm = SecretManager::with_key(&key);

    let plaintext = "smoke-test-secret-value-2026";
    match sm.encrypt(plaintext) {
        Ok(encrypted) => {
            if !encrypted.starts_with("enc2:") {
                return SmokeTestResult::Failed(format!(
                    "Encrypted value missing enc2: prefix, got: {}",
                    &encrypted[..encrypted.len().min(20)]
                ));
            }
            match sm.decrypt(&encrypted) {
                Ok(decrypted) => {
                    if decrypted == plaintext {
                        SmokeTestResult::Passed
                    } else {
                        SmokeTestResult::Failed(format!(
                            "Decrypted value mismatch: expected '{}', got '{}'",
                            plaintext, decrypted
                        ))
                    }
                }
                Err(e) => SmokeTestResult::Failed(format!("Decrypt failed: {}", e)),
            }
        }
        Err(e) => SmokeTestResult::Failed(format!("Encrypt failed: {}", e)),
    }
}

/// Test that the memory SQLite backend can store and keyword-recall entries.
async fn test_memory_store_recall() -> SmokeTestResult {
    use crate::memory::{MemoryConfig, MemoryStore, MemoryCategory};

    let tmp = std::env::temp_dir().join("clawtex_smoke_memory.db");
    let tmp_str = tmp.to_string_lossy().to_string();

    let config = MemoryConfig {
        embeddings_enabled: false,
        ..Default::default()
    };

    let store = match MemoryStore::sqlite(&tmp_str, config) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return SmokeTestResult::Failed(format!("Cannot create MemoryStore: {}", e));
        }
    };

    // Store
    if let Err(e) = store
        .store(
            "smoke_test_key",
            "The quick brown fox",
            MemoryCategory::Core,
            None,
        )
        .await
    {
        let _ = std::fs::remove_file(&tmp);
        return SmokeTestResult::Failed(format!("Memory store failed: {}", e));
    }

    // Recall
    match store.recall("fox", 5, None).await {
        Ok(results) => {
            let _ = std::fs::remove_file(&tmp);
            if results.is_empty() {
                SmokeTestResult::Failed("Recall returned 0 results for stored entry".to_string())
            } else {
                SmokeTestResult::Passed
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            SmokeTestResult::Failed(format!("Memory recall failed: {}", e))
        }
    }
}

/// Test that the ActionTracker records and counts actions correctly.
async fn test_rate_limiter_works() -> SmokeTestResult {
    use crate::tools::ActionTracker;

    let tracker = ActionTracker::new(Duration::from_secs(3600));

    // Initially empty
    if tracker.count() != 0 {
        return SmokeTestResult::Failed(format!(
            "Expected 0 initial count, got {}",
            tracker.count()
        ));
    }

    // Record a few actions
    let c1 = tracker.record();
    let c2 = tracker.record();
    let c3 = tracker.record();

    if c1 != 1 || c2 != 2 || c3 != 3 {
        return SmokeTestResult::Failed(format!(
            "Expected sequential counts 1,2,3 but got {},{},{}",
            c1, c2, c3
        ));
    }

    if tracker.count() != 3 {
        return SmokeTestResult::Failed(format!("Expected count 3, got {}", tracker.count()));
    }

    // Reset and verify
    tracker.reset();
    if tracker.count() != 0 {
        return SmokeTestResult::Failed(format!(
            "Expected 0 after reset, got {}",
            tracker.count()
        ));
    }

    SmokeTestResult::Passed
}

/// Test that a TCP listener can bind to port 0 (OS-assigned), proving networking works.
/// Uses port 0 to avoid conflicts with a running daemon.
async fn test_http_server_binds() -> SmokeTestResult {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            let addr = listener.local_addr();
            drop(listener); // Release immediately.
            match addr {
                Ok(a) => {
                    if a.port() == 0 {
                        SmokeTestResult::Failed(
                            "Bound to port 0 but local_addr reports port 0".to_string(),
                        )
                    } else {
                        SmokeTestResult::Passed
                    }
                }
                Err(e) => {
                    SmokeTestResult::Failed(format!("local_addr failed: {}", e))
                }
            }
        }
        Err(e) => SmokeTestResult::Failed(format!("TCP bind failed: {}", e)),
    }
}

/// Test that ClusterRegistry can be created and the local node is auto-registered.
async fn test_cluster_registry_works() -> SmokeTestResult {
    use crate::cluster::ClusterRegistry;

    let tmp = std::env::temp_dir().join("clawtex_smoke_cluster.db");
    let tmp_str = tmp.to_string_lossy().to_string();

    match ClusterRegistry::new(&tmp_str).await {
        Ok(registry) => {
            let nodes = registry.status().await;
            let _ = std::fs::remove_file(&tmp);
            if nodes.is_empty() {
                SmokeTestResult::Failed(
                    "ClusterRegistry created but no local node registered".to_string(),
                )
            } else {
                // Verify the local node exists.
                let has_local = nodes.iter().any(|n| n.name == "local");
                if has_local {
                    SmokeTestResult::Passed
                } else {
                    SmokeTestResult::Failed(
                        "ClusterRegistry has nodes but none named 'local'".to_string(),
                    )
                }
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            SmokeTestResult::Failed(format!("ClusterRegistry::new failed: {}", e))
        }
    }
}

/// Test that CostTracker can create its DB, write, and read back a record.
async fn test_cost_tracker_works() -> SmokeTestResult {
    use crate::cost_tracker::{CostRecord, CostTracker};
    use chrono::Utc;

    let tmp = std::env::temp_dir().join("clawtex_smoke_costs.db");
    let tmp_str = tmp.to_string_lossy().to_string();

    let tracker = match CostTracker::new(&tmp_str) {
        Ok(t) => t,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return SmokeTestResult::Failed(format!("CostTracker::new failed: {}", e));
        }
    };

    let record = CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        agent: "smoke_agent".to_string(),
        provider: "smoke_provider".to_string(),
        model: "smoke_model".to_string(),
        tokens_in: 100,
        tokens_out: 50,
        total_tokens: 150,
        estimated_cost_usd: 0.001,
        duration_secs: 1.5,
        context: Some("smoke_test".to_string()),
    };

    if let Err(e) = tracker.record(&record) {
        let _ = std::fs::remove_file(&tmp);
        return SmokeTestResult::Failed(format!("CostTracker record failed: {}", e));
    }

    // Verify budget check succeeds (spent $0.001 vs $10 limit).
    match tracker.check_budget("smoke_agent", 10.0) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            SmokeTestResult::Passed
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            SmokeTestResult::Failed(format!("Budget check unexpectedly failed: {}", e))
        }
    }
}

/// Test that built-in i18n translations are loaded for all expected locales.
async fn test_i18n_loaded() -> SmokeTestResult {
    use crate::i18n::I18n;

    let i18n = I18n::new("en");

    let required_locales = ["en", "zh-TW", "ja", "zh-CN", "ko"];

    for locale in &required_locales {
        if !i18n.has_locale(locale) {
            return SmokeTestResult::Failed(format!("Missing locale: {}", locale));
        }
    }

    // Verify a known key resolves for the default locale.
    let welcome = i18n.t("welcome");
    if welcome.is_empty() || welcome == "welcome" {
        return SmokeTestResult::Failed(
            "t(\"welcome\") returned empty or raw key for default locale".to_string(),
        );
    }

    // Verify cross-locale lookup works.
    let ja_welcome = i18n.t_for_locale("ja", "welcome");
    if ja_welcome.is_empty() || ja_welcome == "welcome" {
        return SmokeTestResult::Failed(
            "t_for_locale(\"ja\", \"welcome\") returned empty or raw key".to_string(),
        );
    }

    SmokeTestResult::Passed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SmokeTestResult tests -----------------------------------------------

    #[test]
    fn test_result_passed_is_passed() {
        assert!(SmokeTestResult::Passed.is_passed());
        assert!(!SmokeTestResult::Passed.is_failed());
        assert!(!SmokeTestResult::Passed.is_skipped());
    }

    #[test]
    fn test_result_failed_is_failed() {
        let r = SmokeTestResult::Failed("bad".to_string());
        assert!(r.is_failed());
        assert!(!r.is_passed());
        assert!(!r.is_skipped());
    }

    #[test]
    fn test_result_skipped_is_skipped() {
        let r = SmokeTestResult::Skipped("n/a".to_string());
        assert!(r.is_skipped());
        assert!(!r.is_passed());
        assert!(!r.is_failed());
    }

    #[test]
    fn test_result_display_passed() {
        assert_eq!(SmokeTestResult::Passed.to_string(), "PASS");
    }

    #[test]
    fn test_result_display_failed() {
        let r = SmokeTestResult::Failed("boom".to_string());
        assert_eq!(r.to_string(), "FAIL: boom");
    }

    #[test]
    fn test_result_display_skipped() {
        let r = SmokeTestResult::Skipped("no config".to_string());
        assert_eq!(r.to_string(), "SKIP: no config");
    }

    // -- SmokeTestSuite tests ------------------------------------------------

    #[test]
    fn test_suite_new_is_empty() {
        let suite = SmokeTestSuite::new();
        assert!(suite.is_empty());
        assert_eq!(suite.len(), 0);
    }

    #[test]
    fn test_suite_add_increments_len() {
        let mut suite = SmokeTestSuite::new();
        suite.add(SmokeTest::new("a", "test a", || async {
            SmokeTestResult::Passed
        }));
        assert_eq!(suite.len(), 1);
        assert!(!suite.is_empty());
    }

    #[tokio::test]
    async fn test_suite_run_all_single_pass() {
        let mut suite = SmokeTestSuite::new();
        suite.add(SmokeTest::new("ok", "always passes", || async {
            SmokeTestResult::Passed
        }));
        let report = suite.run_all().await;
        assert_eq!(report.total, 1);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.skipped, 0);
        assert!(report.all_passed());
    }

    #[tokio::test]
    async fn test_suite_run_all_mixed() {
        let mut suite = SmokeTestSuite::new();
        suite.add(SmokeTest::new("p", "pass", || async {
            SmokeTestResult::Passed
        }));
        suite.add(SmokeTest::new("f", "fail", || async {
            SmokeTestResult::Failed("intentional".to_string())
        }));
        suite.add(SmokeTest::new("s", "skip", || async {
            SmokeTestResult::Skipped("no env".to_string())
        }));
        let report = suite.run_all().await;
        assert_eq!(report.total, 3);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.skipped, 1);
        assert!(!report.all_passed());
    }

    // -- SmokeTestReport tests -----------------------------------------------

    #[test]
    fn test_report_to_json_valid() {
        let report = SmokeTestReport {
            passed: 2,
            failed: 1,
            skipped: 0,
            total: 3,
            total_duration_ms: 100,
            details: vec![
                SmokeTestDetail {
                    name: "a".to_string(),
                    description: "test a".to_string(),
                    result: SmokeTestResult::Passed,
                    duration_ms: 10,
                },
                SmokeTestDetail {
                    name: "b".to_string(),
                    description: "test b".to_string(),
                    result: SmokeTestResult::Passed,
                    duration_ms: 20,
                },
                SmokeTestDetail {
                    name: "c".to_string(),
                    description: "test c".to_string(),
                    result: SmokeTestResult::Failed("oops".to_string()),
                    duration_ms: 70,
                },
            ],
        };
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["passed"], 2);
        assert_eq!(parsed["failed"], 1);
        assert_eq!(parsed["total"], 3);
    }

    #[test]
    fn test_report_to_text_contains_header() {
        let report = SmokeTestReport {
            passed: 1,
            failed: 0,
            skipped: 0,
            total: 1,
            total_duration_ms: 5,
            details: vec![SmokeTestDetail {
                name: "x".to_string(),
                description: "desc x".to_string(),
                result: SmokeTestResult::Passed,
                duration_ms: 5,
            }],
        };
        let text = report.to_text();
        assert!(text.contains("Clawtex Smoke Test Report"));
        assert!(text.contains("[PASS]"));
        assert!(text.contains("Passed: 1"));
    }

    #[test]
    fn test_report_all_passed_true() {
        let report = SmokeTestReport {
            passed: 3,
            failed: 0,
            skipped: 1,
            total: 4,
            total_duration_ms: 50,
            details: vec![],
        };
        assert!(report.all_passed());
    }

    #[test]
    fn test_report_all_passed_false() {
        let report = SmokeTestReport {
            passed: 2,
            failed: 1,
            skipped: 0,
            total: 3,
            total_duration_ms: 50,
            details: vec![],
        };
        assert!(!report.all_passed());
    }

    // -- Default suite tests -------------------------------------------------

    #[test]
    fn test_default_suite_has_13_tests() {
        let suite = SmokeTestSuite::default_suite();
        assert_eq!(suite.len(), 13);
    }

    #[test]
    fn test_default_suite_test_names() {
        let suite = SmokeTestSuite::default_suite();
        let names: Vec<&str> = suite.tests.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"config_loadable"));
        assert!(names.contains(&"workspace_writable"));
        assert!(names.contains(&"database_accessible"));
        assert!(names.contains(&"tools_registered"));
        assert!(names.contains(&"hands_loadable"));
        assert!(names.contains(&"providers_configured"));
        assert!(names.contains(&"encryption_roundtrip"));
        assert!(names.contains(&"memory_store_recall"));
        assert!(names.contains(&"rate_limiter_works"));
        assert!(names.contains(&"http_server_binds"));
        assert!(names.contains(&"cluster_registry_works"));
        assert!(names.contains(&"cost_tracker_works"));
        assert!(names.contains(&"i18n_loaded"));
    }

    // -- Individual smoke test function tests --------------------------------

    #[tokio::test]
    async fn test_smoke_encryption_roundtrip() {
        let result = test_encryption_roundtrip().await;
        assert!(
            result.is_passed(),
            "encryption roundtrip should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_rate_limiter_works() {
        let result = test_rate_limiter_works().await;
        assert!(
            result.is_passed(),
            "rate limiter should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_http_server_binds() {
        let result = test_http_server_binds().await;
        assert!(
            result.is_passed(),
            "http server bind should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_database_accessible() {
        let result = test_database_accessible().await;
        assert!(
            result.is_passed(),
            "database accessible should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_memory_store_recall() {
        let result = test_memory_store_recall().await;
        assert!(
            result.is_passed(),
            "memory store/recall should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_cluster_registry_works() {
        let result = test_cluster_registry_works().await;
        assert!(
            result.is_passed(),
            "cluster registry should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_cost_tracker_works() {
        let result = test_cost_tracker_works().await;
        assert!(
            result.is_passed(),
            "cost tracker should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_tools_registered() {
        let result = test_tools_registered().await;
        assert!(
            result.is_passed(),
            "tools registered should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_i18n_loaded() {
        let result = test_i18n_loaded().await;
        assert!(
            result.is_passed(),
            "i18n loaded should pass: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smoke_workspace_writable() {
        let result = test_workspace_writable().await;
        assert!(
            result.is_passed(),
            "workspace writable should pass: {:?}",
            result
        );
    }

    // -- Report serialization round-trip -------------------------------------

    #[test]
    fn test_report_json_roundtrip() {
        let report = SmokeTestReport {
            passed: 5,
            failed: 2,
            skipped: 1,
            total: 8,
            total_duration_ms: 250,
            details: vec![
                SmokeTestDetail {
                    name: "alpha".to_string(),
                    description: "first".to_string(),
                    result: SmokeTestResult::Passed,
                    duration_ms: 10,
                },
                SmokeTestDetail {
                    name: "beta".to_string(),
                    description: "second".to_string(),
                    result: SmokeTestResult::Failed("reason".to_string()),
                    duration_ms: 20,
                },
                SmokeTestDetail {
                    name: "gamma".to_string(),
                    description: "third".to_string(),
                    result: SmokeTestResult::Skipped("not applicable".to_string()),
                    duration_ms: 0,
                },
            ],
        };
        let json = report.to_json();
        let deserialized: SmokeTestReport =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.passed, 5);
        assert_eq!(deserialized.failed, 2);
        assert_eq!(deserialized.skipped, 1);
        assert_eq!(deserialized.total, 8);
        assert_eq!(deserialized.details.len(), 3);
        assert_eq!(deserialized.details[0].result, SmokeTestResult::Passed);
        assert_eq!(
            deserialized.details[1].result,
            SmokeTestResult::Failed("reason".to_string())
        );
        assert_eq!(
            deserialized.details[2].result,
            SmokeTestResult::Skipped("not applicable".to_string())
        );
    }

    // -- SmokeTest individual test -------------------------------------------

    #[tokio::test]
    async fn test_smoke_test_struct_run() {
        let t = SmokeTest::new("my_test", "my description", || async {
            SmokeTestResult::Passed
        });
        assert_eq!(t.name, "my_test");
        assert_eq!(t.description, "my description");
        let result = t.run().await;
        assert!(result.is_passed());
    }

    // -- Duration tracking ---------------------------------------------------

    #[tokio::test]
    async fn test_suite_duration_tracking() {
        let mut suite = SmokeTestSuite::new();
        suite.add(SmokeTest::new("d", "delay test", || async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            SmokeTestResult::Passed
        }));
        let report = suite.run_all().await;
        assert!(report.total_duration_ms >= 10, "Duration should be >= 10ms");
        assert!(
            report.details[0].duration_ms >= 10,
            "Detail duration should be >= 10ms"
        );
    }

    // -- Text report formatting detail ---------------------------------------

    #[test]
    fn test_report_text_contains_fail_details() {
        let report = SmokeTestReport {
            passed: 0,
            failed: 1,
            skipped: 0,
            total: 1,
            total_duration_ms: 50,
            details: vec![SmokeTestDetail {
                name: "bad_test".to_string(),
                description: "should fail".to_string(),
                result: SmokeTestResult::Failed("explosion".to_string()),
                duration_ms: 50,
            }],
        };
        let text = report.to_text();
        assert!(text.contains("[FAIL]"));
        assert!(text.contains("explosion"));
        assert!(text.contains("bad_test"));
        assert!(text.contains("Failed: 1"));
    }

    #[test]
    fn test_report_text_contains_skip_details() {
        let report = SmokeTestReport {
            passed: 0,
            failed: 0,
            skipped: 1,
            total: 1,
            total_duration_ms: 1,
            details: vec![SmokeTestDetail {
                name: "skip_test".to_string(),
                description: "conditionally skipped".to_string(),
                result: SmokeTestResult::Skipped("no config".to_string()),
                duration_ms: 1,
            }],
        };
        let text = report.to_text();
        assert!(text.contains("[SKIP]"));
        assert!(text.contains("no config"));
        assert!(text.contains("Skipped: 1"));
    }
}
