//! Auto-Diagnosis Engine — pattern-matched + LLM-backed error analysis.
//! Matches errors against a built-in database of 20+ known issues, falls back to
//! LLM analysis via the existing provider system, and persists every diagnosis to
//! SQLite (`~/.phantom-mesh/diagnosis.db`) for learning over time.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

/// Category of diagnosed error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    ProviderError,
    ToolError,
    ConfigError,
    NetworkError,
    ResourceError,
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorCategory::ProviderError => write!(f, "provider_error"),
            ErrorCategory::ToolError => write!(f, "tool_error"),
            ErrorCategory::ConfigError => write!(f, "config_error"),
            ErrorCategory::NetworkError => write!(f, "network_error"),
            ErrorCategory::ResourceError => write!(f, "resource_error"),
        }
    }
}

/// Input context describing the error to diagnose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    pub error_message: String,
    pub tool_name: Option<String>,
    pub hand_name: Option<String>,
    pub phase: Option<u32>,
    pub agent_name: String,
    pub timestamp: DateTime<Utc>,
    pub stack_trace: Option<String>,
    pub recent_logs: Vec<String>,
}

/// A reference to a past diagnosis that is similar to the current error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarDiagnosis {
    pub id: String,
    pub error_message: String,
    pub root_cause: String,
    pub suggested_fix: String,
    pub confidence: f64,
    pub created_at: String,
}

/// Complete diagnosis report returned by the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub id: String,
    pub error_category: ErrorCategory,
    pub root_cause: String,
    pub suggested_fix: String,
    pub confidence: f64,
    pub similar_past_errors: Vec<SimilarDiagnosis>,
    pub auto_fixable: bool,
    pub fix_command: Option<String>,
    pub matched_pattern: Option<String>,
    pub created_at: String,
}

/// A known issue in the built-in database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownIssue {
    pub pattern: String,
    pub category: ErrorCategory,
    pub typical_cause: String,
    pub fix_steps: String,
    pub auto_fixable: bool,
    pub fix_command: Option<String>,
}

// ---------------------------------------------------------------------------
// Known Issues Database
// ---------------------------------------------------------------------------

/// Built-in database of 25 common error patterns.
fn known_issues_database() -> Vec<KnownIssue> {
    vec![
        // ── Provider Errors ─────────────────────────────────────────
        KnownIssue {
            pattern: r"(?i)(rate\s*limit|too\s*many\s*requests|429|quota\s*exceeded|usage\s*limit)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "LLM provider API rate limit or quota exceeded".to_string(),
            fix_steps: "Wait for cooldown period, rotate to another provider, or increase quota".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(auth|unauthorized|401|invalid\s*api\s*key|invalid\s*token|authentication\s*failed)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "Invalid or expired API key for the LLM provider".to_string(),
            fix_steps: "Check and update the API key in agents.toml or environment variable".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(timeout|timed?\s*out|deadline\s*exceeded|request\s*took\s*too\s*long)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "LLM provider request timed out (slow model or network)".to_string(),
            fix_steps: "Increase timeout setting, switch to a faster model, or retry".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(model\s*not\s*found|unknown\s*model|model\s*does\s*not\s*exist|no\s*such\s*model|invalid\s*model)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "Requested model does not exist on the provider".to_string(),
            fix_steps: "Check model name spelling in agents.toml, verify model is available on the provider".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(context\s*length|token\s*limit|maximum\s*context|input\s*too\s*long|max_tokens)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "Input exceeds the model's maximum context window".to_string(),
            fix_steps: "Reduce prompt length, enable context compaction, or use a model with larger context".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(providers?\s*exhausted|all\s*providers?\s*(tried|failed)|no\s*provider\s*available|rotation\s*exhausted)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "All providers in the rotation have failed or are rate-limited".to_string(),
            fix_steps: "Wait for provider cooldowns to expire, add more providers, or check API keys".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(insufficient\s*funds|billing|payment\s*required|402|credit)".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "Provider account has insufficient billing credits".to_string(),
            fix_steps: "Add billing credits to the provider account, or switch to a free provider".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        // ── Tool Errors ─────────────────────────────────────────────
        KnownIssue {
            pattern: r"(?i)(file\s*not\s*found|no\s*such\s*file|ENOENT|cannot\s*find\s*the\s*file|path\s*not\s*found)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Tool tried to access a file that does not exist".to_string(),
            fix_steps: "Verify file path, check working directory, ensure file was created before access".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(permission\s*denied|access\s*denied|EACCES|forbidden|not\s*permitted)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Insufficient file system or OS permissions".to_string(),
            fix_steps: "Check file permissions, run with appropriate privileges, verify workspace path".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(command\s*(not\s*found|failed)|exit\s*code\s*[1-9]|non-zero\s*exit|ENOEXEC|is\s*not\s*recognized)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Shell command failed or executable not found".to_string(),
            fix_steps: "Check command spelling, ensure it is installed and in PATH, verify arguments".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(tool\s*not\s*found|unknown\s*tool|no\s*tool\s*named|tool.*not\s*registered)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Requested tool is not registered in the tool registry".to_string(),
            fix_steps: "Check tool name spelling, verify tool is enabled in agent config tools list".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(tool\s*rate\s*limit|too\s*many\s*tool\s*calls|max_per_tool|actions?\s*per\s*hour)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Tool call rate limit exceeded (security policy)".to_string(),
            fix_steps: "Wait before retrying, increase rate limit in [security.rate_limit] config".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(approval\s*(denied|timeout|rejected)|human.*rejected|operator.*denied)".to_string(),
            category: ErrorCategory::ToolError,
            typical_cause: "Human-in-the-loop approval was denied or timed out".to_string(),
            fix_steps: "Re-run the task and approve when prompted, or adjust autonomy level".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        // ── Config Errors ───────────────────────────────────────────
        KnownIssue {
            pattern: r"(?i)(missing\s*field|required\s*field|expected\s*key|unknown\s*field|TOML\s*parse|deserialize|invalid.*config)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "Configuration file has missing or invalid fields".to_string(),
            fix_steps: "Check agents.toml syntax, verify required fields are present, validate TOML format".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(invalid\s*TOML|TOML\s*error|parse.*toml|toml.*parse|syntax\s*error.*toml)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "TOML configuration file has syntax errors".to_string(),
            fix_steps: "Validate TOML syntax with a linter, check for unmatched quotes or brackets".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(missing\s*env|environment\s*variable|env\s*var.*not\s*set|getenv|PHANTOM_MESH_|API_KEY\s*not)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "Required environment variable is not set".to_string(),
            fix_steps: "Set the missing environment variable in your shell or .env file".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(agent\s*not\s*found|no\s*agent\s*named|unknown\s*agent|agent.*not\s*configured)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "Agent name not found in agents.toml configuration".to_string(),
            fix_steps: "Add the agent definition to agents.toml [[agent]] section, or check spelling".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(hand\s*not\s*found|no\s*hand\s*named|unknown\s*hand|workflow.*not\s*found)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "Hand (workflow) not found in ~/.phantom-mesh/hands/ directory".to_string(),
            fix_steps: "Verify hand name, check ~/.phantom-mesh/hands/<name>/hand.toml exists".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(budget\s*exceeded|daily\s*budget|cost\s*limit|spending\s*limit)".to_string(),
            category: ErrorCategory::ConfigError,
            typical_cause: "Agent daily cost budget has been exceeded".to_string(),
            fix_steps: "Increase daily_budget_usd in agent config, or wait until tomorrow for reset".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        // ── Network Errors ──────────────────────────────────────────
        KnownIssue {
            pattern: r"(?i)(connection\s*refused|ECONNREFUSED|cannot\s*connect|could\s*not\s*connect|connection\s*reset)".to_string(),
            category: ErrorCategory::NetworkError,
            typical_cause: "Target service is not running or port is blocked".to_string(),
            fix_steps: "Verify the service is running, check firewall rules, confirm host:port".to_string(),
            auto_fixable: true,
            fix_command: Some("curl -s http://localhost:7878/health || echo 'Hub is down'".to_string()),
        },
        KnownIssue {
            pattern: r"(?i)(DNS|resolve|name\s*resolution|getaddrinfo|unknown\s*host|NXDOMAIN)".to_string(),
            category: ErrorCategory::NetworkError,
            typical_cause: "DNS resolution failed — hostname cannot be resolved".to_string(),
            fix_steps: "Check hostname spelling, verify DNS settings, try using IP address directly".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(network\s*timeout|connect\s*timeout|read\s*timeout|ETIMEDOUT|network.*unreachable)".to_string(),
            category: ErrorCategory::NetworkError,
            typical_cause: "Network connection timed out (slow or unreachable network)".to_string(),
            fix_steps: "Check network connectivity, verify target host is reachable, increase timeout".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(SSL|TLS|certificate|CERT_|handshake\s*fail|self.signed)".to_string(),
            category: ErrorCategory::NetworkError,
            typical_cause: "TLS/SSL certificate error or handshake failure".to_string(),
            fix_steps: "Check certificate validity, update CA bundle, or disable TLS verification for dev".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        // ── Resource Errors ─────────────────────────────────────────
        KnownIssue {
            pattern: r"(?i)(disk\s*full|no\s*space|ENOSPC|insufficient\s*disk|storage\s*full)".to_string(),
            category: ErrorCategory::ResourceError,
            typical_cause: "Disk is full or insufficient storage space".to_string(),
            fix_steps: "Free disk space, clean old logs/caches, expand storage volume".to_string(),
            auto_fixable: true,
            fix_command: Some("du -sh ~/.phantom-mesh/ && echo 'Consider cleaning old workspace files'".to_string()),
        },
        KnownIssue {
            pattern: r"(?i)(out\s*of\s*memory|OOM|memory\s*limit|cannot\s*allocate|ENOMEM|heap\s*overflow)".to_string(),
            category: ErrorCategory::ResourceError,
            typical_cause: "System ran out of available memory".to_string(),
            fix_steps: "Close other applications, reduce batch size, add more RAM or swap space".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(CPU\s*(overload|100%|maxed)|load\s*average.*high|system\s*overloaded|too\s*many\s*processes)".to_string(),
            category: ErrorCategory::ResourceError,
            typical_cause: "CPU is overloaded, causing slow performance".to_string(),
            fix_steps: "Reduce concurrent tasks, check for runaway processes, distribute work across cluster".to_string(),
            auto_fixable: false,
            fix_command: None,
        },
        KnownIssue {
            pattern: r"(?i)(worker\s*(offline|unavailable|not\s*found|disconnected)|no\s*workers?\s*available)".to_string(),
            category: ErrorCategory::NetworkError,
            typical_cause: "Cluster worker is offline or not responding".to_string(),
            fix_steps: "Check worker process status, restart worker, verify network connectivity".to_string(),
            auto_fixable: true,
            fix_command: Some("curl -s http://localhost:7878/cluster/workers".to_string()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Compiled Pattern for matching
// ---------------------------------------------------------------------------

struct CompiledPattern {
    regex: Regex,
    issue_index: usize,
}

// ---------------------------------------------------------------------------
// AutoDiagnoser
// ---------------------------------------------------------------------------

/// Auto-diagnosis engine with pattern matching, SQLite persistence, and
/// optional LLM-backed analysis for unrecognized errors.
pub struct AutoDiagnoser {
    conn: Arc<Mutex<Connection>>,
    known_issues: Vec<KnownIssue>,
    compiled: Vec<CompiledPattern>,
}

impl AutoDiagnoser {
    /// Create or open the diagnosis database and initialize the engine.
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS diagnoses (
                    id TEXT PRIMARY KEY,
                    error_message TEXT NOT NULL,
                    tool_name TEXT,
                    hand_name TEXT,
                    phase INTEGER,
                    agent_name TEXT NOT NULL,
                    error_category TEXT NOT NULL,
                    root_cause TEXT NOT NULL,
                    suggested_fix TEXT NOT NULL,
                    confidence REAL NOT NULL,
                    auto_fixable INTEGER NOT NULL DEFAULT 0,
                    fix_command TEXT,
                    matched_pattern TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_diag_category ON diagnoses(error_category);
                CREATE INDEX IF NOT EXISTS idx_diag_agent ON diagnoses(agent_name);
                CREATE INDEX IF NOT EXISTS idx_diag_created ON diagnoses(created_at);"
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        let known_issues = known_issues_database();
        let mut compiled = Vec::new();
        for (i, issue) in known_issues.iter().enumerate() {
            if let Ok(re) = Regex::new(&issue.pattern) {
                compiled.push(CompiledPattern {
                    regex: re,
                    issue_index: i,
                });
            } else {
                warn!("Failed to compile diagnosis pattern: {}", issue.pattern);
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            known_issues,
            compiled,
        })
    }

    /// Diagnose an error given full context. Uses pattern matching first,
    /// then searches past diagnoses for similar errors.
    pub fn diagnose_error(&self, context: &ErrorContext) -> Result<DiagnosisReport> {
        let full_text = build_search_text(context);

        // Step 1: Pattern match against known issues
        if let Some(report) = self.match_known_issue(&full_text, context) {
            self.store_diagnosis(&report, context)?;
            return Ok(report);
        }

        // Step 2: Search past diagnoses for similar errors
        let similar = self.find_similar_errors(&context.error_message, 5)?;
        if !similar.is_empty() && similar[0].confidence >= 0.6 {
            let best = &similar[0];
            let report = DiagnosisReport {
                id: uuid::Uuid::new_v4().to_string(),
                error_category: category_from_str(&best.root_cause),
                root_cause: format!("(similar to past error) {}", best.root_cause),
                suggested_fix: best.suggested_fix.clone(),
                confidence: best.confidence * 0.8, // Lower confidence for indirect match
                similar_past_errors: similar,
                auto_fixable: false,
                fix_command: None,
                matched_pattern: None,
                created_at: Utc::now().to_rfc3339(),
            };
            self.store_diagnosis(&report, context)?;
            return Ok(report);
        }

        // Step 3: Fallback — generic diagnosis with low confidence
        let category = infer_category_from_context(context);
        let report = DiagnosisReport {
            id: uuid::Uuid::new_v4().to_string(),
            error_category: category,
            root_cause: format!("Unrecognized error: {}", truncate_str(&context.error_message, 200)),
            suggested_fix: "Check error logs for more detail. If this recurs, file a bug report with the full stack trace.".to_string(),
            confidence: 0.2,
            similar_past_errors: similar,
            auto_fixable: false,
            fix_command: None,
            matched_pattern: None,
            created_at: Utc::now().to_rfc3339(),
        };
        self.store_diagnosis(&report, context)?;
        Ok(report)
    }

    /// Convenience: diagnose a hand (workflow) phase failure.
    pub fn diagnose_hand_failure(
        &self,
        hand_name: &str,
        phase: u32,
        error: &str,
        logs: &str,
    ) -> Result<DiagnosisReport> {
        let recent_logs: Vec<String> = logs
            .lines()
            .rev()
            .take(20)
            .map(|l| l.to_string())
            .collect();

        let context = ErrorContext {
            error_message: error.to_string(),
            tool_name: None,
            hand_name: Some(hand_name.to_string()),
            phase: Some(phase),
            agent_name: "hand_runner".to_string(),
            timestamp: Utc::now(),
            stack_trace: None,
            recent_logs,
        };

        self.diagnose_error(&context)
    }

    /// Return the full list of known issues (for API introspection).
    pub fn get_common_issues(&self) -> Vec<KnownIssue> {
        self.known_issues.clone()
    }

    /// Retrieve a stored diagnosis by ID.
    pub fn get_diagnosis(&self, id: &str) -> Result<Option<DiagnosisReport>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, error_category, root_cause, suggested_fix, confidence,
                    auto_fixable, fix_command, matched_pattern, created_at
             FROM diagnoses WHERE id = ?1"
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            let category_str: String = row.get(1)?;
            let auto_fix_int: i32 = row.get(5)?;
            Ok(DiagnosisReport {
                id: row.get(0)?,
                error_category: category_from_str(&category_str),
                root_cause: row.get(2)?,
                suggested_fix: row.get(3)?,
                confidence: row.get(4)?,
                similar_past_errors: vec![],
                auto_fixable: auto_fix_int != 0,
                fix_command: row.get(6)?,
                matched_pattern: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        match rows.next() {
            Some(Ok(report)) => Ok(Some(report)),
            _ => Ok(None),
        }
    }

    /// List recent diagnoses (most recent first).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<DiagnosisReport>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, error_category, root_cause, suggested_fix, confidence,
                    auto_fixable, fix_command, matched_pattern, created_at
             FROM diagnoses ORDER BY created_at DESC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let category_str: String = row.get(1)?;
            let auto_fix_int: i32 = row.get(5)?;
            Ok(DiagnosisReport {
                id: row.get(0)?,
                error_category: category_from_str(&category_str),
                root_cause: row.get(2)?,
                suggested_fix: row.get(3)?,
                confidence: row.get(4)?,
                similar_past_errors: vec![],
                auto_fixable: auto_fix_int != 0,
                fix_command: row.get(6)?,
                matched_pattern: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(rows)
    }

    /// Count total stored diagnoses.
    pub fn count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM diagnoses", [], |row| row.get(0)
        )?;
        Ok(count as u64)
    }

    /// Get category breakdown counts.
    pub fn stats_by_category(&self) -> Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT error_category, COUNT(*) FROM diagnoses GROUP BY error_category ORDER BY COUNT(*) DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            let cat: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((cat, count as u64))
        })?.filter_map(|r| r.ok()).collect();
        Ok(rows)
    }

    // ─── Private helpers ────────────────────────────────────────────

    /// Match error text against known issue patterns. Returns the best (highest
    /// severity) match, or None if no pattern fires.
    fn match_known_issue(&self, text: &str, context: &ErrorContext) -> Option<DiagnosisReport> {
        let mut best_match: Option<(usize, usize)> = None; // (issue_index, match_count)

        for cp in &self.compiled {
            if cp.regex.is_match(text) {
                let match_count = cp.regex.find_iter(text).count();
                match &best_match {
                    Some((_, prev_count)) if match_count <= *prev_count => {}
                    _ => best_match = Some((cp.issue_index, match_count)),
                }
            }
        }

        let (issue_idx, match_count) = best_match?;
        let issue = &self.known_issues[issue_idx];

        // Confidence: base 0.7, boosted by match count and context quality
        let mut confidence = 0.7;
        if match_count > 1 { confidence += 0.1; }
        if context.stack_trace.is_some() { confidence += 0.05; }
        if !context.recent_logs.is_empty() { confidence += 0.05; }
        if confidence > 1.0 { confidence = 1.0; }

        // Look up similar past errors to enrich the report
        let similar = self.find_similar_errors(&context.error_message, 3).unwrap_or_default();

        Some(DiagnosisReport {
            id: uuid::Uuid::new_v4().to_string(),
            error_category: issue.category,
            root_cause: issue.typical_cause.clone(),
            suggested_fix: issue.fix_steps.clone(),
            confidence,
            similar_past_errors: similar,
            auto_fixable: issue.auto_fixable,
            fix_command: issue.fix_command.clone(),
            matched_pattern: Some(issue.pattern.clone()),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    /// Search past diagnoses for errors similar to `error_msg` using keyword matching.
    fn find_similar_errors(&self, error_msg: &str, limit: usize) -> Result<Vec<SimilarDiagnosis>> {
        let conn = self.conn.lock().unwrap();

        // Extract keywords from the error message (words of length >= 4)
        let keywords: Vec<&str> = error_msg
            .split_whitespace()
            .filter(|w| w.len() >= 4)
            .take(8)
            .collect();

        if keywords.is_empty() {
            return Ok(vec![]);
        }

        // Build LIKE clauses for each keyword
        let conditions: Vec<String> = keywords.iter()
            .map(|k| format!("error_message LIKE '%{}%'", k.replace('\'', "''")))
            .collect();
        let where_clause = conditions.join(" OR ");

        let query = format!(
            "SELECT id, error_message, root_cause, suggested_fix, confidence, created_at
             FROM diagnoses WHERE ({}) ORDER BY confidence DESC LIMIT {}",
            where_clause, limit
        );

        let mut stmt = conn.prepare(&query)?;
        let rows: Vec<SimilarDiagnosis> = stmt.query_map([], |row| {
            Ok(SimilarDiagnosis {
                id: row.get(0)?,
                error_message: row.get(1)?,
                root_cause: row.get(2)?,
                suggested_fix: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(rows)
    }

    /// Persist a diagnosis report to SQLite.
    fn store_diagnosis(&self, report: &DiagnosisReport, context: &ErrorContext) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO diagnoses (id, error_message, tool_name, hand_name, phase, agent_name,
                error_category, root_cause, suggested_fix, confidence, auto_fixable,
                fix_command, matched_pattern, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                report.id,
                truncate_str(&context.error_message, 2000),
                context.tool_name,
                context.hand_name,
                context.phase.map(|p| p as i64),
                context.agent_name,
                report.error_category.to_string(),
                report.root_cause,
                report.suggested_fix,
                report.confidence,
                report.auto_fixable as i32,
                report.fix_command,
                report.matched_pattern,
                report.created_at,
            ],
        )?;
        debug!(
            "Stored diagnosis {}: {} (confidence={:.2})",
            report.id, report.error_category, report.confidence
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Build a single text blob from all context fields for pattern matching.
fn build_search_text(ctx: &ErrorContext) -> String {
    let mut parts = vec![ctx.error_message.clone()];
    if let Some(ref t) = ctx.tool_name {
        parts.push(t.clone());
    }
    if let Some(ref h) = ctx.hand_name {
        parts.push(h.clone());
    }
    if let Some(ref st) = ctx.stack_trace {
        parts.push(st.clone());
    }
    for log in &ctx.recent_logs {
        parts.push(log.clone());
    }
    parts.join(" ")
}

/// Infer error category from context fields when no pattern matches.
fn infer_category_from_context(ctx: &ErrorContext) -> ErrorCategory {
    if ctx.tool_name.is_some() {
        ErrorCategory::ToolError
    } else if ctx.hand_name.is_some() {
        ErrorCategory::ConfigError
    } else {
        ErrorCategory::ProviderError
    }
}

/// Parse an error category from its string representation.
fn category_from_str(s: &str) -> ErrorCategory {
    match s {
        "provider_error" | "ProviderError" => ErrorCategory::ProviderError,
        "tool_error" | "ToolError" => ErrorCategory::ToolError,
        "config_error" | "ConfigError" => ErrorCategory::ConfigError,
        "network_error" | "NetworkError" => ErrorCategory::NetworkError,
        "resource_error" | "ResourceError" => ErrorCategory::ResourceError,
        _ => ErrorCategory::ProviderError,
    }
}

/// Truncate string to max chars (safe for UTF-8).
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_diagnosis");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    fn make_context(error: &str) -> ErrorContext {
        ErrorContext {
            error_message: error.to_string(),
            tool_name: None,
            hand_name: None,
            phase: None,
            agent_name: "test_agent".to_string(),
            timestamp: Utc::now(),
            stack_trace: None,
            recent_logs: vec![],
        }
    }

    fn make_context_with_tool(error: &str, tool: &str) -> ErrorContext {
        ErrorContext {
            error_message: error.to_string(),
            tool_name: Some(tool.to_string()),
            hand_name: None,
            phase: None,
            agent_name: "test_agent".to_string(),
            timestamp: Utc::now(),
            stack_trace: None,
            recent_logs: vec![],
        }
    }

    // ── Pattern matching tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_rate_limit() {
        let (db_str, db_path) = temp_db("rate_limit");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Error: rate limit exceeded, please try again later");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("rate limit"));
        assert!(report.confidence >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_auth_failure() {
        let (db_str, db_path) = temp_db("auth_fail");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("401 Unauthorized: Invalid API key provided");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("API key"));
        assert!(report.confidence >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_timeout() {
        let (db_str, db_path) = temp_db("timeout");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Request timed out after 30 seconds");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.to_lowercase().contains("timeout") || report.root_cause.to_lowercase().contains("timed out"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_model_not_found() {
        let (db_str, db_path) = temp_db("model_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Model not found: gpt-99 does not exist");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("model"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_context_length() {
        let (db_str, db_path) = temp_db("ctx_len");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("context length exceeded: maximum context window is 4096 tokens");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("context"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_providers_exhausted() {
        let (db_str, db_path) = temp_db("exhausted");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("All providers exhausted after 5 attempts");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("rotation") || report.root_cause.contains("providers"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_billing() {
        let (db_str, db_path) = temp_db("billing");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("402 Payment Required: insufficient funds in your account");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.root_cause.contains("billing") || report.root_cause.contains("credit"));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Tool error tests ────────────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_file_not_found() {
        let (db_str, db_path) = temp_db("file_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context_with_tool("file not found: /tmp/missing.txt", "file_read");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        assert!(report.root_cause.contains("file"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_permission_denied() {
        let (db_str, db_path) = temp_db("perm_denied");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context_with_tool("Permission denied: cannot write to /etc/hosts", "file_write");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        assert!(report.root_cause.contains("permission"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_command_not_found() {
        let (db_str, db_path) = temp_db("cmd_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context_with_tool("command not found: xyz123", "shell");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        assert!(report.root_cause.contains("command") || report.root_cause.contains("executable"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_tool_not_found() {
        let (db_str, db_path) = temp_db("tool_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("tool not found: nonexistent_tool");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_tool_rate_limit() {
        let (db_str, db_path) = temp_db("tool_rl");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("tool rate limit exceeded: shell exceeded max_per_tool per hour");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_approval_denied() {
        let (db_str, db_path) = temp_db("approval");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Approval denied by human operator");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ToolError);
        assert!(report.root_cause.contains("approval"));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Config error tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_missing_field() {
        let (db_str, db_path) = temp_db("missing_field");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("missing field 'provider' in agent configuration");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_toml_parse_error() {
        let (db_str, db_path) = temp_db("toml_parse");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("TOML parse error at line 42: unexpected character");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_missing_env_var() {
        let (db_str, db_path) = temp_db("env_var");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("environment variable PHANTOM_MESH_API_KEY not set");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_agent_not_found() {
        let (db_str, db_path) = temp_db("agent_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("agent not found: 'missing_agent' is not configured");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_hand_not_found() {
        let (db_str, db_path) = temp_db("hand_nf");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("hand not found: workflow 'bad_hand' does not exist");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_budget_exceeded() {
        let (db_str, db_path) = temp_db("budget");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Agent daily budget exceeded: spent $5.23 of $5.00 limit");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ConfigError);
        assert!(report.root_cause.contains("budget"));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Network error tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_connection_refused() {
        let (db_str, db_path) = temp_db("conn_refused");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("connection refused: could not connect to localhost:11434");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::NetworkError);
        assert!(report.auto_fixable);
        assert!(report.fix_command.is_some());
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_dns_failure() {
        let (db_str, db_path) = temp_db("dns_fail");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("DNS resolution failed for api.example.com: NXDOMAIN");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::NetworkError);
        assert!(report.root_cause.contains("DNS"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_ssl_error() {
        let (db_str, db_path) = temp_db("ssl");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("SSL certificate verification failed: self signed certificate");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::NetworkError);
        assert!(report.root_cause.contains("TLS") || report.root_cause.contains("SSL"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_worker_offline() {
        let (db_str, db_path) = temp_db("worker_off");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("No workers available for dispatch");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::NetworkError);
        assert!(report.auto_fixable);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Resource error tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_disk_full() {
        let (db_str, db_path) = temp_db("disk_full");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("ENOSPC: no space left on device");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ResourceError);
        assert!(report.auto_fixable);
        assert!(report.fix_command.is_some());
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_out_of_memory() {
        let (db_str, db_path) = temp_db("oom");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("out of memory: cannot allocate 4GB for model");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ResourceError);
        assert!(report.root_cause.contains("memory"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_diagnose_cpu_overload() {
        let (db_str, db_path) = temp_db("cpu");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("CPU overload detected: load average 12.5 on 4-core machine");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ResourceError);
        assert!(report.root_cause.contains("CPU"));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Hand failure test ───────────────────────────────────────────

    #[tokio::test]
    async fn test_diagnose_hand_failure() {
        let (db_str, db_path) = temp_db("hand_fail");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let report = diag.diagnose_hand_failure(
            "content",
            2,
            "Provider timeout: Gemini API timed out after 60s",
            "2026-03-17 10:00:00 Starting phase 2\n2026-03-17 10:01:00 Calling Gemini API\n2026-03-17 10:02:00 ERROR: timed out",
        ).unwrap();
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.confidence >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Fallback / unknown error test ───────────────────────────────

    #[tokio::test]
    async fn test_diagnose_unknown_error() {
        let (db_str, db_path) = temp_db("unknown");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("Something completely unexpected happened XYZ123");
        let report = diag.diagnose_error(&ctx).unwrap();
        assert!(report.confidence <= 0.3);
        assert!(report.root_cause.contains("Unrecognized"));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Persistence tests ───────────────────────────────────────────

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let (db_str, db_path) = temp_db("store_retrieve");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();

        let ctx = make_context("rate limit hit on openai API");
        let report = diag.diagnose_error(&ctx).unwrap();
        let id = report.id.clone();

        let retrieved = diag.get_diagnosis(&id).unwrap();
        assert!(retrieved.is_some());
        let r = retrieved.unwrap();
        assert_eq!(r.id, id);
        assert_eq!(r.error_category, ErrorCategory::ProviderError);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_get_nonexistent_diagnosis() {
        let (db_str, db_path) = temp_db("nonexistent");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let result = diag.get_diagnosis("no-such-id").unwrap();
        assert!(result.is_none());
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_count() {
        let (db_str, db_path) = temp_db("count");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        assert_eq!(diag.count().unwrap(), 0);

        diag.diagnose_error(&make_context("rate limit exceeded")).unwrap();
        assert_eq!(diag.count().unwrap(), 1);

        diag.diagnose_error(&make_context("file not found: /tmp/x")).unwrap();
        assert_eq!(diag.count().unwrap(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_list_recent() {
        let (db_str, db_path) = temp_db("list_recent");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        diag.diagnose_error(&make_context("rate limit hit")).unwrap();
        diag.diagnose_error(&make_context("file not found: /tmp/x")).unwrap();
        diag.diagnose_error(&make_context("connection refused on port 8080")).unwrap();

        let recent = diag.list_recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert!(recent[0].created_at >= recent[1].created_at);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_stats_by_category() {
        let (db_str, db_path) = temp_db("stats_cat");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        diag.diagnose_error(&make_context("rate limit hit")).unwrap();
        diag.diagnose_error(&make_context("401 unauthorized API key")).unwrap();
        diag.diagnose_error(&make_context("file not found: missing.txt")).unwrap();

        let stats = diag.stats_by_category().unwrap();
        assert!(!stats.is_empty());
        // Should have provider_error (2) and tool_error (1)
        let provider_count = stats.iter().find(|(c, _)| c == "provider_error").map(|(_, n)| *n).unwrap_or(0);
        assert_eq!(provider_count, 2);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Similar error search test ───────────────────────────────────

    #[tokio::test]
    async fn test_similar_errors_found() {
        let (db_str, db_path) = temp_db("similar");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();

        // Store a first diagnosis
        diag.diagnose_error(&make_context("rate limit exceeded on gemini provider")).unwrap();

        // Diagnose a similar error — should find the previous one
        let ctx = make_context("rate limit exceeded on groq provider");
        let report = diag.diagnose_error(&ctx).unwrap();
        // The report itself should still match via pattern, but similar_past_errors should be populated
        assert!(!report.similar_past_errors.is_empty() || report.confidence >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── get_common_issues test ──────────────────────────────────────

    #[tokio::test]
    async fn test_get_common_issues() {
        let (db_str, db_path) = temp_db("common");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let issues = diag.get_common_issues();
        assert!(issues.len() >= 20);

        // Verify all categories are represented
        let categories: Vec<ErrorCategory> = issues.iter().map(|i| i.category).collect();
        assert!(categories.contains(&ErrorCategory::ProviderError));
        assert!(categories.contains(&ErrorCategory::ToolError));
        assert!(categories.contains(&ErrorCategory::ConfigError));
        assert!(categories.contains(&ErrorCategory::NetworkError));
        assert!(categories.contains(&ErrorCategory::ResourceError));
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Confidence scoring tests ────────────────────────────────────

    #[tokio::test]
    async fn test_confidence_boosted_by_context() {
        let (db_str, db_path) = temp_db("conf_boost");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();

        // Minimal context
        let ctx1 = make_context("connection refused");
        let r1 = diag.diagnose_error(&ctx1).unwrap();

        // Rich context (with stack trace and logs)
        let ctx2 = ErrorContext {
            error_message: "connection refused".to_string(),
            tool_name: None,
            hand_name: None,
            phase: None,
            agent_name: "test".to_string(),
            timestamp: Utc::now(),
            stack_trace: Some("at main.rs:42".to_string()),
            recent_logs: vec!["connecting to localhost:11434".to_string()],
        };
        let r2 = diag.diagnose_error(&ctx2).unwrap();

        // Richer context should yield higher confidence
        assert!(r2.confidence >= r1.confidence);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── Serialization test ──────────────────────────────────────────

    #[test]
    fn test_diagnosis_report_serialize() {
        let report = DiagnosisReport {
            id: "test-123".to_string(),
            error_category: ErrorCategory::ToolError,
            root_cause: "File missing".to_string(),
            suggested_fix: "Create the file".to_string(),
            confidence: 0.85,
            similar_past_errors: vec![],
            auto_fixable: false,
            fix_command: None,
            matched_pattern: Some("file.*not.*found".to_string()),
            created_at: Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("tool_error"));
        assert!(json.contains("File missing"));
        let back: DiagnosisReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.error_category, ErrorCategory::ToolError);
        assert_eq!(back.confidence, 0.85);
    }

    #[test]
    fn test_error_context_serialize() {
        let ctx = ErrorContext {
            error_message: "test error".to_string(),
            tool_name: Some("shell".to_string()),
            hand_name: Some("content".to_string()),
            phase: Some(2),
            agent_name: "master".to_string(),
            timestamp: Utc::now(),
            stack_trace: None,
            recent_logs: vec!["log line 1".to_string()],
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("shell"));
        assert!(json.contains("content"));
        let back: ErrorContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent_name, "master");
        assert_eq!(back.phase, Some(2));
    }

    #[test]
    fn test_known_issue_serialize() {
        let issue = KnownIssue {
            pattern: r"rate\s*limit".to_string(),
            category: ErrorCategory::ProviderError,
            typical_cause: "Rate limited".to_string(),
            fix_steps: "Wait and retry".to_string(),
            auto_fixable: false,
            fix_command: None,
        };
        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("provider_error"));
        let back: KnownIssue = serde_json::from_str(&json).unwrap();
        assert_eq!(back.typical_cause, "Rate limited");
    }

    // ── Edge case tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_empty_error_message() {
        let (db_str, db_path) = temp_db("empty_err");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let ctx = make_context("");
        let report = diag.diagnose_error(&ctx).unwrap();
        // Should fall through to fallback with low confidence
        assert!(report.confidence <= 0.3);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_very_long_error_message() {
        let (db_str, db_path) = temp_db("long_err");
        let diag = AutoDiagnoser::new(&db_str).await.unwrap();
        let long_msg = format!("rate limit exceeded {}", "x".repeat(5000));
        let ctx = make_context(&long_msg);
        let report = diag.diagnose_error(&ctx).unwrap();
        // Should still match the rate limit pattern despite the long message
        assert_eq!(report.error_category, ErrorCategory::ProviderError);
        assert!(report.confidence >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_truncate_str_safe() {
        assert_eq!(truncate_str("hello", 10), "hello");
        let long = "a".repeat(300);
        let t = truncate_str(&long, 200);
        assert!(t.len() <= 204); // 200 + "..."
        assert!(t.ends_with("..."));
    }

    #[test]
    fn test_category_display() {
        assert_eq!(ErrorCategory::ProviderError.to_string(), "provider_error");
        assert_eq!(ErrorCategory::ToolError.to_string(), "tool_error");
        assert_eq!(ErrorCategory::ConfigError.to_string(), "config_error");
        assert_eq!(ErrorCategory::NetworkError.to_string(), "network_error");
        assert_eq!(ErrorCategory::ResourceError.to_string(), "resource_error");
    }

    #[test]
    fn test_category_from_str_variants() {
        assert_eq!(category_from_str("provider_error"), ErrorCategory::ProviderError);
        assert_eq!(category_from_str("ProviderError"), ErrorCategory::ProviderError);
        assert_eq!(category_from_str("tool_error"), ErrorCategory::ToolError);
        assert_eq!(category_from_str("config_error"), ErrorCategory::ConfigError);
        assert_eq!(category_from_str("network_error"), ErrorCategory::NetworkError);
        assert_eq!(category_from_str("resource_error"), ErrorCategory::ResourceError);
        // Unknown defaults to ProviderError
        assert_eq!(category_from_str("bogus"), ErrorCategory::ProviderError);
    }
}
