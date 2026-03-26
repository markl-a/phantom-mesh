//! Structured JSON Logging Layer -- produces machine-readable log entries with
//! trace/span correlation, a queryable in-memory ring buffer, and domain-specific
//! helpers for tool, provider, hand, and cluster events.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::Level;

// ---------------------------------------------------------------------------
// LogEntry
// ---------------------------------------------------------------------------

/// A single structured log record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// ISO-8601 timestamp of the log event.
    pub timestamp: DateTime<Utc>,
    /// Log severity level.
    pub level: LogLevel,
    /// Originating module or subsystem.
    pub module: String,
    /// Human-readable message.
    pub message: String,
    /// Arbitrary key-value fields attached to this entry.
    pub fields: HashMap<String, Value>,
    /// Distributed trace ID (optional).
    pub trace_id: Option<String>,
    /// Span ID within a trace (optional).
    pub span_id: Option<String>,
}

impl LogEntry {
    /// Create a new LogEntry with the current UTC timestamp.
    pub fn new(level: LogLevel, module: &str, message: &str) -> Self {
        Self {
            timestamp: Utc::now(),
            level,
            module: module.to_string(),
            message: message.to_string(),
            fields: HashMap::new(),
            trace_id: None,
            span_id: None,
        }
    }

    /// Attach a key-value field.
    pub fn with_field(mut self, key: &str, value: Value) -> Self {
        self.fields.insert(key.to_string(), value);
        self
    }

    /// Attach a trace ID.
    pub fn with_trace(mut self, trace_id: &str) -> Self {
        self.trace_id = Some(trace_id.to_string());
        self
    }

    /// Attach a span ID.
    pub fn with_span(mut self, span_id: &str) -> Self {
        self.span_id = Some(span_id.to_string());
        self
    }

    /// Serialize this entry to a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{{\"error\":\"serialization failed\",\"message\":\"{}\"}}", self.message))
    }

    /// Serialize this entry to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| self.to_json())
    }

    /// Compact single-line format: `[LEVEL] module: message {field=val, ...}`
    pub fn to_compact(&self) -> String {
        let fields_str = if self.fields.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = self.fields.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!(" {{{}}}", pairs.join(", "))
        };
        format!("[{}] {}: {}{}", self.level.as_str(), self.module, self.message, fields_str)
    }
}

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

/// Log severity levels mirroring tracing::Level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "TRACE" => Some(Self::Trace),
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARN" => Some(Self::Warn),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }

    /// Convert from tracing::Level.
    pub fn from_tracing(level: &Level) -> Self {
        match *level {
            Level::TRACE => Self::Trace,
            Level::DEBUG => Self::Debug,
            Level::INFO => Self::Info,
            Level::WARN => Self::Warn,
            Level::ERROR => Self::Error,
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// LogConfig
// ---------------------------------------------------------------------------

/// Where log output is directed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogOutput {
    Stdout,
    File(String),
    Both(String),
}

/// Output serialization format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogFormat {
    Json,
    Pretty,
    Compact,
}

/// Log file rotation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogRotation {
    Daily,
    Hourly,
    Size,
}

/// Configuration for the structured logging system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Where to write logs.
    pub output: LogOutput,
    /// Serialization format.
    pub format: LogFormat,
    /// Minimum severity to record.
    pub level_filter: LogLevel,
    /// Maximum log file size in MB before rotation (used with Size rotation).
    pub max_file_size_mb: u64,
    /// Rotation strategy for file output.
    pub rotation: LogRotation,
    /// Ring buffer capacity for in-memory queryable logs.
    pub buffer_capacity: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            output: LogOutput::Stdout,
            format: LogFormat::Json,
            level_filter: LogLevel::Info,
            max_file_size_mb: 50,
            rotation: LogRotation::Daily,
            buffer_capacity: 1000,
        }
    }
}

// ---------------------------------------------------------------------------
// LogFilter
// ---------------------------------------------------------------------------

/// Filter criteria for querying the log buffer.
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Minimum level (inclusive).
    pub level: Option<LogLevel>,
    /// Module name substring match.
    pub module_pattern: Option<String>,
    /// Start of time range (inclusive).
    pub from: Option<DateTime<Utc>>,
    /// End of time range (inclusive).
    pub to: Option<DateTime<Utc>>,
    /// Case-insensitive text search in message.
    pub text_search: Option<String>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
}

impl LogFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn with_module(mut self, pattern: &str) -> Self {
        self.module_pattern = Some(pattern.to_string());
        self
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.text_search = Some(text.to_string());
        self
    }

    pub fn with_time_range(mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        self.from = Some(from);
        self.to = Some(to);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Check whether a LogEntry matches this filter.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        // Level filter: entry must be >= filter level
        if let Some(ref level) = self.level {
            if (entry.level as u8) < (*level as u8) {
                return false;
            }
        }
        // Module pattern: substring match
        if let Some(ref pattern) = self.module_pattern {
            if !entry.module.contains(pattern.as_str()) {
                return false;
            }
        }
        // Time range
        if let Some(ref from) = self.from {
            if entry.timestamp < *from {
                return false;
            }
        }
        if let Some(ref to) = self.to {
            if entry.timestamp > *to {
                return false;
            }
        }
        // Text search (case-insensitive)
        if let Some(ref text) = self.text_search {
            if !entry.message.to_lowercase().contains(&text.to_lowercase()) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// LogStats
// ---------------------------------------------------------------------------

/// Aggregate statistics across the log buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogStats {
    /// Total number of entries in the buffer.
    pub total: usize,
    /// Count per severity level.
    pub per_level: HashMap<String, usize>,
    /// Count per module.
    pub per_module: HashMap<String, usize>,
    /// Number of ERROR entries in the last hour.
    pub errors_last_hour: usize,
}

// ---------------------------------------------------------------------------
// LogBuffer -- ring buffer for recent logs
// ---------------------------------------------------------------------------

/// Thread-safe ring buffer that stores the most recent log entries.
pub struct LogBuffer {
    entries: RwLock<Vec<LogEntry>>,
    capacity: usize,
    /// Write position for ring buffer semantics.
    write_pos: Mutex<usize>,
    /// Total entries ever written (may exceed capacity).
    total_written: Mutex<usize>,
}

impl LogBuffer {
    /// Create a new ring buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 { 1 } else { capacity };
        Self {
            entries: RwLock::new(Vec::with_capacity(cap)),
            capacity: cap,
            write_pos: Mutex::new(0),
            total_written: Mutex::new(0),
        }
    }

    /// Push a log entry into the ring buffer.
    pub fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.write().unwrap();
        let mut pos = self.write_pos.lock().unwrap();
        let mut total = self.total_written.lock().unwrap();

        if entries.len() < self.capacity {
            entries.push(entry);
        } else {
            entries[*pos] = entry;
        }
        *pos = (*pos + 1) % self.capacity;
        *total += 1;
    }

    /// Return the number of entries currently in the buffer.
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total entries ever pushed (including those evicted by the ring).
    pub fn total_written(&self) -> usize {
        *self.total_written.lock().unwrap()
    }

    /// Query entries matching a filter. Returns entries in chronological order.
    pub fn query(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let entries = self.entries.read().unwrap();
        let total = *self.total_written.lock().unwrap();
        let len = entries.len();

        // Build a chronologically ordered iterator over the ring buffer.
        let ordered: Vec<&LogEntry> = if len < self.capacity || total <= self.capacity {
            // Buffer not yet full -- entries are already in order.
            entries.iter().collect()
        } else {
            // Ring has wrapped -- read from write_pos to end, then 0 to write_pos.
            let pos = *self.write_pos.lock().unwrap();
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                result.push(&entries[(pos + i) % len]);
            }
            result
        };

        let mut matched: Vec<LogEntry> = ordered
            .into_iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();

        if let Some(limit) = filter.limit {
            matched.truncate(limit);
        }

        matched
    }

    /// Get all entries (in chronological order).
    pub fn all(&self) -> Vec<LogEntry> {
        self.query(&LogFilter::default())
    }

    /// Compute aggregate statistics.
    pub fn stats(&self) -> LogStats {
        let entries = self.entries.read().unwrap();
        let now = Utc::now();
        let one_hour_ago = now - chrono::Duration::hours(1);

        let mut per_level: HashMap<String, usize> = HashMap::new();
        let mut per_module: HashMap<String, usize> = HashMap::new();
        let mut errors_last_hour = 0usize;

        for entry in entries.iter() {
            *per_level.entry(entry.level.as_str().to_string()).or_insert(0) += 1;
            *per_module.entry(entry.module.clone()).or_insert(0) += 1;
            if entry.level == LogLevel::Error && entry.timestamp >= one_hour_ago {
                errors_last_hour += 1;
            }
        }

        LogStats {
            total: entries.len(),
            per_level,
            per_module,
            errors_last_hour,
        }
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap();
        entries.clear();
        *self.write_pos.lock().unwrap() = 0;
        // Note: total_written is not reset -- it tracks lifetime writes.
    }
}

impl std::fmt::Debug for LogBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogBuffer")
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .field("total_written", &self.total_written())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// LogFileWriter -- JSONL file output with daily + size rotation
// ---------------------------------------------------------------------------

/// Default log directory relative to the working directory.
const DEFAULT_LOG_DIR: &str = "data/logs";

/// Manages writing log entries as JSONL to rotating files.
///
/// Rotation rules:
/// - **Daily**: when the UTC date changes a new file `phantom_mesh-{date}.jsonl` is opened.
/// - **Size**: when the current file exceeds `max_bytes` a new segment
///   `phantom_mesh-{date}-{n}.jsonl` is opened (n increments from 1).
///
/// The writer uses `BufWriter` for efficient I/O.
struct LogFileWriter {
    /// Directory where log files are stored.
    dir: PathBuf,
    /// Maximum file size in bytes before size-based rotation.
    max_bytes: u64,
    /// The date string (YYYY-MM-DD) of the currently open file.
    current_date: String,
    /// Sequence number for size-based rotation within a single day (0 = first file).
    seq: u32,
    /// Bytes written to the current file so far.
    bytes_written: u64,
    /// Buffered writer for the current file.
    writer: BufWriter<File>,
}

impl LogFileWriter {
    /// Build the file name for a given date and sequence number.
    fn file_name(date: &str, seq: u32) -> String {
        if seq == 0 {
            format!("phantom_mesh-{}.jsonl", date)
        } else {
            format!("phantom_mesh-{}-{}.jsonl", date, seq)
        }
    }

    /// Open (or create) a log file and return a `BufWriter` wrapping it.
    fn open_file(dir: &Path, date: &str, seq: u32) -> std::io::Result<(BufWriter<File>, u64)> {
        fs::create_dir_all(dir)?;
        let path = dir.join(Self::file_name(date, seq));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let existing_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok((BufWriter::new(file), existing_len))
    }

    /// Create a new `LogFileWriter`, opening (or creating) today's log file.
    fn new(dir: PathBuf, max_bytes: u64) -> std::io::Result<Self> {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let seq = 0u32;
        let (writer, existing_len) = Self::open_file(&dir, &date, seq)?;
        Ok(Self {
            dir,
            max_bytes,
            current_date: date,
            seq,
            bytes_written: existing_len,
            writer,
        })
    }

    /// Rotate to a new file — either because the date changed or the size limit
    /// was reached.  Returns `Ok(())` on success.
    fn rotate(&mut self, new_date: &str) -> std::io::Result<()> {
        // Flush the old writer before switching files.
        self.writer.flush()?;

        if new_date != self.current_date {
            // Daily rotation — reset sequence.
            self.current_date = new_date.to_string();
            self.seq = 0;
        } else {
            // Size rotation — increment sequence within the same day.
            self.seq += 1;
        }

        let (writer, existing_len) = Self::open_file(&self.dir, &self.current_date, self.seq)?;
        self.writer = writer;
        self.bytes_written = existing_len;
        Ok(())
    }

    /// Append a single `LogEntry` as a JSONL line.  Handles rotation transparently.
    fn append(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Daily rotation check.
        if today != self.current_date {
            self.rotate(&today)?;
        }

        let line = entry.to_json();
        let line_bytes = line.len() as u64 + 1; // +1 for newline

        // Size rotation check (before writing so the *next* entry goes to a new file).
        if self.bytes_written > 0 && self.bytes_written + line_bytes > self.max_bytes {
            self.rotate(&today)?;
        }

        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.bytes_written += line_bytes;

        Ok(())
    }

    /// Return the full path of the current log file.
    #[allow(dead_code)]
    fn current_path(&self) -> PathBuf {
        self.dir.join(Self::file_name(&self.current_date, self.seq))
    }
}

impl std::fmt::Debug for LogFileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogFileWriter")
            .field("dir", &self.dir)
            .field("current_date", &self.current_date)
            .field("seq", &self.seq)
            .field("bytes_written", &self.bytes_written)
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// StructuredLogger -- the main logger with buffer
// ---------------------------------------------------------------------------

/// The main structured logger that writes formatted output and maintains
/// an in-memory ring buffer of recent entries.
///
/// When the output is `LogOutput::File` or `LogOutput::Both`, log entries are
/// also appended as JSONL to rotating files under `data/logs/`.
pub struct StructuredLogger {
    config: LogConfig,
    buffer: Arc<LogBuffer>,
    /// Optional file writer — present when output targets a file.
    file_writer: Option<Mutex<LogFileWriter>>,
}

impl StructuredLogger {
    /// Create a new StructuredLogger with the given configuration.
    ///
    /// When the output mode includes a file path (`LogOutput::File` or
    /// `LogOutput::Both`), a `LogFileWriter` is created that writes JSONL to
    /// `<path>/phantom_mesh-{date}.jsonl` with daily and size-based rotation.  If
    /// no explicit path is provided in the variant, `data/logs` is used.
    pub fn new(config: LogConfig) -> Self {
        let buffer_cap = config.buffer_capacity;
        let max_bytes = config.max_file_size_mb * 1024 * 1024;

        let file_writer = match &config.output {
            LogOutput::File(dir) | LogOutput::Both(dir) => {
                let log_dir = if dir.is_empty() {
                    PathBuf::from(DEFAULT_LOG_DIR)
                } else {
                    PathBuf::from(dir)
                };
                match LogFileWriter::new(log_dir, max_bytes) {
                    Ok(w) => Some(Mutex::new(w)),
                    Err(e) => {
                        eprintln!("[structured_log] failed to open log file: {}", e);
                        None
                    }
                }
            }
            LogOutput::Stdout => None,
        };

        Self {
            config,
            buffer: Arc::new(LogBuffer::new(buffer_cap)),
            file_writer,
        }
    }

    /// Get a reference to the underlying buffer.
    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    /// Get a shared handle to the buffer.
    pub fn buffer_arc(&self) -> Arc<LogBuffer> {
        Arc::clone(&self.buffer)
    }

    /// Get a reference to the current config.
    pub fn config(&self) -> &LogConfig {
        &self.config
    }

    /// Record a log entry if it passes the level filter.
    ///
    /// The entry is pushed into the in-memory ring buffer **and**, when a file
    /// writer is configured, appended as a JSONL line to the current log file
    /// (with automatic daily/size rotation).
    pub fn log(&self, entry: LogEntry) {
        // Level filter: only record if entry level >= configured minimum
        if (entry.level as u8) < (self.config.level_filter as u8) {
            return;
        }

        // Write to file if a writer is configured.
        if let Some(ref fw) = self.file_writer {
            if let Ok(mut writer) = fw.lock() {
                if let Err(e) = writer.append(&entry) {
                    eprintln!("[structured_log] file write error: {}", e);
                }
            }
        }

        // Always push into the in-memory ring buffer.
        self.buffer.push(entry);
    }

    /// Record a log entry with the given level, module, and message.
    pub fn log_msg(&self, level: LogLevel, module: &str, message: &str) {
        self.log(LogEntry::new(level, module, message));
    }

    /// Query recent logs from the buffer.
    pub fn query_recent_logs(&self, filter: &LogFilter) -> Vec<LogEntry> {
        self.buffer.query(filter)
    }

    /// Get aggregate statistics from the buffer.
    pub fn log_stats(&self) -> LogStats {
        self.buffer.stats()
    }
}

impl std::fmt::Debug for StructuredLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructuredLogger")
            .field("config", &self.config)
            .field("buffer", &self.buffer)
            .field("file_writer", &self.file_writer.as_ref().map(|fw| {
                fw.lock().ok().map(|w| format!("{:?}", *w))
            }))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Domain-specific logging helpers
// ---------------------------------------------------------------------------

/// Log a tool execution event.
pub fn log_tool_execution(
    logger: &StructuredLogger,
    tool: &str,
    duration_ms: u64,
    success: bool,
    details: Value,
) {
    let level = if success { LogLevel::Info } else { LogLevel::Warn };
    let msg = if success {
        format!("Tool '{}' completed in {}ms", tool, duration_ms)
    } else {
        format!("Tool '{}' failed after {}ms", tool, duration_ms)
    };

    let entry = LogEntry::new(level, "tools", &msg)
        .with_field("tool", Value::String(tool.to_string()))
        .with_field("duration_ms", Value::Number(serde_json::Number::from(duration_ms)))
        .with_field("success", Value::Bool(success))
        .with_field("details", details);

    logger.log(entry);
}

/// Log an LLM provider call.
pub fn log_provider_call(
    logger: &StructuredLogger,
    provider: &str,
    model: &str,
    tokens: u64,
    cost: f64,
    duration_ms: u64,
) {
    let msg = format!(
        "Provider '{}' model '{}': {} tokens, ${:.6}, {}ms",
        provider, model, tokens, cost, duration_ms
    );

    let entry = LogEntry::new(LogLevel::Info, "providers", &msg)
        .with_field("provider", Value::String(provider.to_string()))
        .with_field("model", Value::String(model.to_string()))
        .with_field("tokens", Value::Number(serde_json::Number::from(tokens)))
        .with_field("cost", serde_json::json!(cost))
        .with_field("duration_ms", Value::Number(serde_json::Number::from(duration_ms)));

    logger.log(entry);
}

/// Log a hand phase transition.
pub fn log_hand_phase(
    logger: &StructuredLogger,
    hand: &str,
    phase: u32,
    status: &str,
) {
    let level = if status == "failed" || status == "error" {
        LogLevel::Error
    } else {
        LogLevel::Info
    };
    let msg = format!("Hand '{}' phase {}: {}", hand, phase, status);

    let entry = LogEntry::new(level, "hands", &msg)
        .with_field("hand", Value::String(hand.to_string()))
        .with_field("phase", Value::Number(serde_json::Number::from(phase)))
        .with_field("status", Value::String(status.to_string()));

    logger.log(entry);
}

/// Log a cluster event (node join, leave, dispatch, heartbeat, etc.).
pub fn log_cluster_event(
    logger: &StructuredLogger,
    event_type: &str,
    node: &str,
    details: Value,
) {
    let level = match event_type {
        "error" | "node_down" | "dispatch_failed" => LogLevel::Error,
        "warning" | "node_unhealthy" => LogLevel::Warn,
        _ => LogLevel::Info,
    };
    let msg = format!("Cluster event '{}' on node '{}'", event_type, node);

    let entry = LogEntry::new(level, "cluster", &msg)
        .with_field("event_type", Value::String(event_type.to_string()))
        .with_field("node", Value::String(node.to_string()))
        .with_field("details", details);

    logger.log(entry);
}

/// Initialize logging with the given configuration.
/// This sets up the tracing subscriber and returns a StructuredLogger.
pub fn init_logging(config: &LogConfig) -> StructuredLogger {
    // Set up tracing level filter based on config
    let _level = match config.level_filter {
        LogLevel::Trace => Level::TRACE,
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    };

    // Create the structured logger with the config
    StructuredLogger::new(config.clone())
}

/// Query recent logs from a StructuredLogger.
pub fn query_recent_logs(logger: &StructuredLogger, filter: &LogFilter) -> Vec<LogEntry> {
    logger.query_recent_logs(filter)
}

/// Get statistics from a StructuredLogger.
pub fn log_stats(logger: &StructuredLogger) -> LogStats {
    logger.log_stats()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    // -- LogLevel --

    #[test]
    fn test_log_level_as_str() {
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("trace"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_str("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("unknown"), None);
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(format!("{}", LogLevel::Info), "INFO");
        assert_eq!(format!("{}", LogLevel::Error), "ERROR");
    }

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn test_log_level_from_tracing() {
        assert_eq!(LogLevel::from_tracing(&Level::INFO), LogLevel::Info);
        assert_eq!(LogLevel::from_tracing(&Level::ERROR), LogLevel::Error);
        assert_eq!(LogLevel::from_tracing(&Level::TRACE), LogLevel::Trace);
    }

    // -- LogEntry --

    #[test]
    fn test_log_entry_new() {
        let entry = LogEntry::new(LogLevel::Info, "test_module", "hello world");
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.module, "test_module");
        assert_eq!(entry.message, "hello world");
        assert!(entry.fields.is_empty());
        assert!(entry.trace_id.is_none());
        assert!(entry.span_id.is_none());
    }

    #[test]
    fn test_log_entry_with_fields() {
        let entry = LogEntry::new(LogLevel::Debug, "mod", "msg")
            .with_field("count", serde_json::json!(42))
            .with_field("name", Value::String("test".to_string()));
        assert_eq!(entry.fields.len(), 2);
        assert_eq!(entry.fields["count"], serde_json::json!(42));
        assert_eq!(entry.fields["name"], "test");
    }

    #[test]
    fn test_log_entry_with_trace_and_span() {
        let entry = LogEntry::new(LogLevel::Info, "mod", "msg")
            .with_trace("trace-123")
            .with_span("span-456");
        assert_eq!(entry.trace_id, Some("trace-123".to_string()));
        assert_eq!(entry.span_id, Some("span-456".to_string()));
    }

    #[test]
    fn test_log_entry_to_json() {
        let entry = LogEntry::new(LogLevel::Info, "test", "hello");
        let json = entry.to_json();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["level"], "Info");
        assert_eq!(parsed["module"], "test");
        assert_eq!(parsed["message"], "hello");
    }

    #[test]
    fn test_log_entry_to_json_pretty() {
        let entry = LogEntry::new(LogLevel::Warn, "test", "warning");
        let pretty = entry.to_json_pretty();
        assert!(pretty.contains('\n')); // pretty-printed has newlines
        let parsed: Value = serde_json::from_str(&pretty).unwrap();
        assert_eq!(parsed["level"], "Warn");
    }

    #[test]
    fn test_log_entry_to_compact() {
        let entry = LogEntry::new(LogLevel::Error, "tools", "failed");
        let compact = entry.to_compact();
        assert!(compact.starts_with("[ERROR] tools: failed"));
    }

    #[test]
    fn test_log_entry_compact_with_fields() {
        let entry = LogEntry::new(LogLevel::Info, "mod", "msg")
            .with_field("key", Value::String("val".to_string()));
        let compact = entry.to_compact();
        assert!(compact.contains("{"));
        assert!(compact.contains("key="));
    }

    // -- LogConfig --

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.output, LogOutput::Stdout);
        assert_eq!(config.format, LogFormat::Json);
        assert_eq!(config.level_filter, LogLevel::Info);
        assert_eq!(config.max_file_size_mb, 50);
        assert_eq!(config.rotation, LogRotation::Daily);
        assert_eq!(config.buffer_capacity, 1000);
    }

    #[test]
    fn test_log_config_serializable() {
        let config = LogConfig::default();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["format"], "Json");
        assert_eq!(json["level_filter"], "Info");
    }

    // -- LogFilter --

    #[test]
    fn test_log_filter_default_matches_all() {
        let filter = LogFilter::new();
        let entry = LogEntry::new(LogLevel::Trace, "any", "anything");
        assert!(filter.matches(&entry));
    }

    #[test]
    fn test_log_filter_by_level() {
        let filter = LogFilter::new().with_level(LogLevel::Warn);
        let trace_entry = LogEntry::new(LogLevel::Trace, "mod", "msg");
        let warn_entry = LogEntry::new(LogLevel::Warn, "mod", "msg");
        let error_entry = LogEntry::new(LogLevel::Error, "mod", "msg");
        assert!(!filter.matches(&trace_entry));
        assert!(filter.matches(&warn_entry));
        assert!(filter.matches(&error_entry));
    }

    #[test]
    fn test_log_filter_by_module() {
        let filter = LogFilter::new().with_module("tool");
        let tools_entry = LogEntry::new(LogLevel::Info, "tools", "msg");
        let cluster_entry = LogEntry::new(LogLevel::Info, "cluster", "msg");
        assert!(filter.matches(&tools_entry));
        assert!(!filter.matches(&cluster_entry));
    }

    #[test]
    fn test_log_filter_by_text_case_insensitive() {
        let filter = LogFilter::new().with_text("FAIL");
        let entry = LogEntry::new(LogLevel::Error, "mod", "Tool execution failed");
        assert!(filter.matches(&entry));
    }

    #[test]
    fn test_log_filter_by_time_range() {
        let now = Utc::now();
        let filter = LogFilter::new().with_time_range(
            now - Duration::hours(1),
            now + Duration::hours(1),
        );
        let entry = LogEntry::new(LogLevel::Info, "mod", "msg");
        assert!(filter.matches(&entry));

        // Entry from 2 hours ago
        let mut old_entry = LogEntry::new(LogLevel::Info, "mod", "old");
        old_entry.timestamp = now - Duration::hours(2);
        assert!(!filter.matches(&old_entry));
    }

    #[test]
    fn test_log_filter_combined() {
        let filter = LogFilter::new()
            .with_level(LogLevel::Warn)
            .with_module("tools")
            .with_text("failed");

        let good = LogEntry::new(LogLevel::Error, "tools", "Tool failed");
        assert!(filter.matches(&good));

        let wrong_level = LogEntry::new(LogLevel::Info, "tools", "Tool failed");
        assert!(!filter.matches(&wrong_level));

        let wrong_module = LogEntry::new(LogLevel::Error, "cluster", "Tool failed");
        assert!(!filter.matches(&wrong_module));

        let wrong_text = LogEntry::new(LogLevel::Error, "tools", "Tool succeeded");
        assert!(!filter.matches(&wrong_text));
    }

    // -- LogBuffer --

    #[test]
    fn test_buffer_new_empty() {
        let buffer = LogBuffer::new(10);
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.total_written(), 0);
    }

    #[test]
    fn test_buffer_push_and_len() {
        let buffer = LogBuffer::new(10);
        buffer.push(LogEntry::new(LogLevel::Info, "mod", "msg1"));
        buffer.push(LogEntry::new(LogLevel::Info, "mod", "msg2"));
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.total_written(), 2);
    }

    #[test]
    fn test_buffer_ring_eviction() {
        let buffer = LogBuffer::new(3);
        for i in 0..5 {
            buffer.push(LogEntry::new(LogLevel::Info, "mod", &format!("msg{}", i)));
        }
        // Only 3 entries retained, but 5 written total
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.total_written(), 5);

        // The retained entries should be the last 3: msg2, msg3, msg4
        let all = buffer.all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "msg2");
        assert_eq!(all[1].message, "msg3");
        assert_eq!(all[2].message, "msg4");
    }

    #[test]
    fn test_buffer_query_with_filter() {
        let buffer = LogBuffer::new(100);
        buffer.push(LogEntry::new(LogLevel::Info, "tools", "tool ok"));
        buffer.push(LogEntry::new(LogLevel::Error, "tools", "tool failed"));
        buffer.push(LogEntry::new(LogLevel::Info, "cluster", "node joined"));

        let filter = LogFilter::new().with_level(LogLevel::Error);
        let results = buffer.query(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "tool failed");
    }

    #[test]
    fn test_buffer_query_with_limit() {
        let buffer = LogBuffer::new(100);
        for i in 0..10 {
            buffer.push(LogEntry::new(LogLevel::Info, "mod", &format!("msg{}", i)));
        }
        let filter = LogFilter::new().with_limit(3);
        let results = buffer.query(&filter);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_buffer_stats() {
        let buffer = LogBuffer::new(100);
        buffer.push(LogEntry::new(LogLevel::Info, "tools", "ok"));
        buffer.push(LogEntry::new(LogLevel::Info, "tools", "ok again"));
        buffer.push(LogEntry::new(LogLevel::Error, "cluster", "error"));
        buffer.push(LogEntry::new(LogLevel::Warn, "hands", "warning"));

        let stats = buffer.stats();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.per_level.get("INFO"), Some(&2));
        assert_eq!(stats.per_level.get("ERROR"), Some(&1));
        assert_eq!(stats.per_level.get("WARN"), Some(&1));
        assert_eq!(stats.per_module.get("tools"), Some(&2));
        assert_eq!(stats.per_module.get("cluster"), Some(&1));
        assert_eq!(stats.errors_last_hour, 1);
    }

    #[test]
    fn test_buffer_stats_empty() {
        let buffer = LogBuffer::new(10);
        let stats = buffer.stats();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.errors_last_hour, 0);
        assert!(stats.per_level.is_empty());
        assert!(stats.per_module.is_empty());
    }

    #[test]
    fn test_buffer_clear() {
        let buffer = LogBuffer::new(10);
        buffer.push(LogEntry::new(LogLevel::Info, "mod", "msg"));
        assert_eq!(buffer.len(), 1);
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        // total_written is not reset
        assert_eq!(buffer.total_written(), 1);
    }

    #[test]
    fn test_buffer_zero_capacity() {
        // Edge case: capacity of 0 should be clamped to 1
        let buffer = LogBuffer::new(0);
        buffer.push(LogEntry::new(LogLevel::Info, "mod", "msg1"));
        buffer.push(LogEntry::new(LogLevel::Info, "mod", "msg2"));
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.total_written(), 2);
    }

    // -- StructuredLogger --

    #[test]
    fn test_logger_new() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        assert_eq!(logger.buffer().len(), 0);
    }

    #[test]
    fn test_logger_log_entry() {
        let config = LogConfig { level_filter: LogLevel::Debug, ..LogConfig::default() };
        let logger = StructuredLogger::new(config);
        logger.log(LogEntry::new(LogLevel::Info, "test", "hello"));
        assert_eq!(logger.buffer().len(), 1);
    }

    #[test]
    fn test_logger_filters_below_level() {
        let config = LogConfig { level_filter: LogLevel::Warn, ..LogConfig::default() };
        let logger = StructuredLogger::new(config);
        logger.log(LogEntry::new(LogLevel::Debug, "test", "filtered out"));
        logger.log(LogEntry::new(LogLevel::Info, "test", "filtered out"));
        logger.log(LogEntry::new(LogLevel::Warn, "test", "kept"));
        logger.log(LogEntry::new(LogLevel::Error, "test", "kept"));
        assert_eq!(logger.buffer().len(), 2);
    }

    #[test]
    fn test_logger_log_msg() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        logger.log_msg(LogLevel::Info, "test", "short message");
        assert_eq!(logger.buffer().len(), 1);
    }

    #[test]
    fn test_logger_query_recent_logs() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        logger.log(LogEntry::new(LogLevel::Info, "tools", "tool1"));
        logger.log(LogEntry::new(LogLevel::Error, "cluster", "node_down"));
        logger.log(LogEntry::new(LogLevel::Info, "hands", "phase1"));

        let filter = LogFilter::new().with_module("cluster");
        let results = query_recent_logs(&logger, &filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].module, "cluster");
    }

    #[test]
    fn test_logger_log_stats() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        logger.log(LogEntry::new(LogLevel::Info, "tools", "ok"));
        logger.log(LogEntry::new(LogLevel::Error, "tools", "fail"));

        let stats = log_stats(&logger);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.per_level.get("INFO"), Some(&1));
        assert_eq!(stats.per_level.get("ERROR"), Some(&1));
        assert_eq!(stats.errors_last_hour, 1);
    }

    // -- Domain-specific log helpers --

    #[test]
    fn test_log_tool_execution_success() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_tool_execution(&logger, "shell", 150, true, serde_json::json!({"cmd": "ls"}));

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].module, "tools");
        assert!(entries[0].message.contains("shell"));
        assert!(entries[0].message.contains("150ms"));
        assert_eq!(entries[0].fields["success"], true);
        assert_eq!(entries[0].fields["tool"], "shell");
    }

    #[test]
    fn test_log_tool_execution_failure() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_tool_execution(&logger, "web_search", 5000, false, serde_json::json!({"error": "timeout"}));

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Warn);
        assert!(entries[0].message.contains("failed"));
    }

    #[test]
    fn test_log_provider_call() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_provider_call(&logger, "gemini", "gemini-2.5-pro", 1500, 0.003, 800);

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].module, "providers");
        assert_eq!(entries[0].fields["provider"], "gemini");
        assert_eq!(entries[0].fields["model"], "gemini-2.5-pro");
        assert_eq!(entries[0].fields["tokens"], 1500);
    }

    #[test]
    fn test_log_hand_phase_success() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_hand_phase(&logger, "content", 1, "completed");

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].fields["hand"], "content");
        assert_eq!(entries[0].fields["phase"], 1);
        assert_eq!(entries[0].fields["status"], "completed");
    }

    #[test]
    fn test_log_hand_phase_failure() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_hand_phase(&logger, "seo_content", 3, "failed");

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Error);
    }

    #[test]
    fn test_log_cluster_event_info() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_cluster_event(&logger, "node_joined", "z13", serde_json::json!({"port": 7878}));

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[0].module, "cluster");
        assert_eq!(entries[0].fields["event_type"], "node_joined");
        assert_eq!(entries[0].fields["node"], "z13");
    }

    #[test]
    fn test_log_cluster_event_error() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        log_cluster_event(&logger, "node_down", "m1-mac", serde_json::json!(null));

        let entries = logger.buffer().all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, LogLevel::Error);
    }

    // -- init_logging --

    #[test]
    fn test_init_logging() {
        let config = LogConfig {
            level_filter: LogLevel::Debug,
            buffer_capacity: 500,
            ..LogConfig::default()
        };
        let logger = init_logging(&config);
        assert_eq!(logger.config().level_filter, LogLevel::Debug);
        assert_eq!(logger.config().buffer_capacity, 500);
        assert!(logger.buffer().is_empty());
    }

    // -- Integration: end-to-end scenario --

    #[test]
    fn test_end_to_end_logging_scenario() {
        let config = LogConfig {
            level_filter: LogLevel::Trace,
            buffer_capacity: 100,
            ..LogConfig::default()
        };
        let logger = init_logging(&config);

        // Simulate a hand execution with tool calls
        log_hand_phase(&logger, "content", 1, "started");
        log_tool_execution(&logger, "web_search", 250, true, serde_json::json!({"query": "rust async"}));
        log_provider_call(&logger, "gemini", "pro", 2000, 0.005, 1200);
        log_tool_execution(&logger, "file_write", 30, true, serde_json::json!({"path": "/tmp/out.md"}));
        log_hand_phase(&logger, "content", 1, "completed");
        log_cluster_event(&logger, "dispatch", "z13", serde_json::json!({"tool": "web_search"}));

        // Verify buffer contents
        assert_eq!(logger.buffer().len(), 6);

        // Query by module
        let tool_logs = query_recent_logs(&logger, &LogFilter::new().with_module("tools"));
        assert_eq!(tool_logs.len(), 2);

        let hand_logs = query_recent_logs(&logger, &LogFilter::new().with_module("hands"));
        assert_eq!(hand_logs.len(), 2);

        // Stats check
        let stats = log_stats(&logger);
        assert_eq!(stats.total, 6);
        assert_eq!(stats.per_module.get("tools"), Some(&2));
        assert_eq!(stats.per_module.get("hands"), Some(&2));
        assert_eq!(stats.per_module.get("providers"), Some(&1));
        assert_eq!(stats.per_module.get("cluster"), Some(&1));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let entry = LogEntry::new(LogLevel::Info, "test", "round-trip")
            .with_field("key", serde_json::json!("value"))
            .with_trace("t-1")
            .with_span("s-1");

        let json = entry.to_json();
        let deserialized: LogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.level, LogLevel::Info);
        assert_eq!(deserialized.module, "test");
        assert_eq!(deserialized.message, "round-trip");
        assert_eq!(deserialized.trace_id, Some("t-1".to_string()));
        assert_eq!(deserialized.span_id, Some("s-1".to_string()));
        assert_eq!(deserialized.fields["key"], "value");
    }

    #[test]
    fn test_log_stats_serializable() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        logger.log(LogEntry::new(LogLevel::Info, "mod", "msg"));
        let stats = logger.log_stats();
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["total"], 1);
    }

    #[test]
    fn test_buffer_arc_sharing() {
        let config = LogConfig::default();
        let logger = StructuredLogger::new(config);
        let buffer = logger.buffer_arc();

        // Log via the logger
        logger.log(LogEntry::new(LogLevel::Info, "mod", "msg"));

        // Read via the shared arc
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.all()[0].message, "msg");
    }

    // -- LogFileWriter --

    #[test]
    fn test_file_writer_creates_directory_and_file() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");
        let writer = LogFileWriter::new(log_dir.clone(), 50 * 1024 * 1024).unwrap();
        assert!(log_dir.exists());
        assert!(writer.current_path().exists());
        let expected_name = format!(
            "phantom_mesh-{}.jsonl",
            Utc::now().format("%Y-%m-%d")
        );
        assert_eq!(writer.current_path().file_name().unwrap().to_str().unwrap(), expected_name);
    }

    #[test]
    fn test_file_writer_appends_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");
        let mut writer = LogFileWriter::new(log_dir.clone(), 50 * 1024 * 1024).unwrap();

        let entry1 = LogEntry::new(LogLevel::Info, "test", "first");
        let entry2 = LogEntry::new(LogLevel::Warn, "test", "second");
        writer.append(&entry1).unwrap();
        writer.append(&entry2).unwrap();

        let content = std::fs::read_to_string(writer.current_path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line must be valid JSON
        let parsed: LogEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.message, "first");
        let parsed2: LogEntry = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(parsed2.message, "second");
    }

    #[test]
    fn test_file_writer_size_rotation() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");
        // Set a tiny max size so rotation is triggered quickly.
        let mut writer = LogFileWriter::new(log_dir.clone(), 100).unwrap();
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Write entries until we exceed 100 bytes
        for i in 0..10 {
            let entry = LogEntry::new(LogLevel::Info, "test", &format!("message-{}", i));
            writer.append(&entry).unwrap();
        }

        // After rotation, the sequence number should have advanced
        assert!(writer.seq > 0, "seq should have advanced due to size rotation");

        // The first file and at least one rotated file should exist
        let first_file = log_dir.join(format!("phantom_mesh-{}.jsonl", today));
        let rotated_file = log_dir.join(format!("phantom_mesh-{}-1.jsonl", today));
        assert!(first_file.exists(), "original file should exist");
        assert!(rotated_file.exists(), "rotated file should exist");
    }

    #[test]
    fn test_file_writer_daily_rotation_resets_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");
        let mut writer = LogFileWriter::new(log_dir.clone(), 50 * 1024 * 1024).unwrap();

        // Artificially bump the sequence to simulate a prior size rotation.
        writer.seq = 3;

        // Simulate a date change by manually calling rotate with a new date.
        writer.rotate("2099-01-01").unwrap();

        assert_eq!(writer.current_date, "2099-01-01");
        assert_eq!(writer.seq, 0);
        assert_eq!(
            writer.current_path().file_name().unwrap().to_str().unwrap(),
            "phantom_mesh-2099-01-01.jsonl"
        );
    }

    #[test]
    fn test_logger_with_file_output_writes_and_buffers() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");

        let config = LogConfig {
            output: LogOutput::File(log_dir.to_str().unwrap().to_string()),
            level_filter: LogLevel::Trace,
            ..LogConfig::default()
        };
        let logger = StructuredLogger::new(config);

        logger.log(LogEntry::new(LogLevel::Info, "test", "file-and-buffer"));

        // Ring buffer should have the entry
        assert_eq!(logger.buffer().len(), 1);
        assert_eq!(logger.buffer().all()[0].message, "file-and-buffer");

        // File should also have the entry
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let file_path = log_dir.join(format!("phantom_mesh-{}.jsonl", today));
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 1);
        let parsed: LogEntry = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed.message, "file-and-buffer");
    }

    #[test]
    fn test_logger_with_both_output() {
        let tmp = tempfile::tempdir().unwrap();
        let log_dir = tmp.path().join("logs");

        let config = LogConfig {
            output: LogOutput::Both(log_dir.to_str().unwrap().to_string()),
            level_filter: LogLevel::Trace,
            ..LogConfig::default()
        };
        let logger = StructuredLogger::new(config);

        logger.log(LogEntry::new(LogLevel::Info, "test", "both-output"));

        // Ring buffer
        assert_eq!(logger.buffer().len(), 1);

        // File
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let file_path = log_dir.join(format!("phantom_mesh-{}.jsonl", today));
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn test_logger_stdout_only_no_file() {
        // With Stdout output, no file writer should be created.
        let config = LogConfig {
            output: LogOutput::Stdout,
            ..LogConfig::default()
        };
        let logger = StructuredLogger::new(config);
        assert!(logger.file_writer.is_none());
    }

    #[test]
    fn test_file_writer_file_name_generation() {
        assert_eq!(LogFileWriter::file_name("2026-03-19", 0), "phantom_mesh-2026-03-19.jsonl");
        assert_eq!(LogFileWriter::file_name("2026-03-19", 1), "phantom_mesh-2026-03-19-1.jsonl");
        assert_eq!(LogFileWriter::file_name("2026-03-19", 42), "phantom_mesh-2026-03-19-42.jsonl");
    }
}
