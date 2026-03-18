/// ConfigWatcher — polls `agents.toml` for changes every 10 seconds.
///
/// When the file's mtime changes, the new TOML content is re-parsed and broadcast
/// via a `tokio::sync::watch` channel. Invalid TOML is silently ignored so the
/// daemon continues running with the last-known-good config.
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Minimal representation of the agents section in agents.toml.
/// Extend with more top-level sections as hot-reload coverage grows.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct WatchedConfig {
    /// `[agent.*]` table — agent name → config
    #[serde(default)]
    pub agent: HashMap<String, AgentEntry>,
}

/// Per-agent entry (only the fields needed for hot-reload decisions).
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct AgentEntry {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub instructions: Option<String>,
}

// ── ConfigWatcher ─────────────────────────────────────────────────────────────

/// Polls a TOML config file for changes and broadcasts new parsed configs.
pub struct ConfigWatcher {
    path: PathBuf,
    poll_interval: Duration,
}

impl ConfigWatcher {
    /// Create a watcher for the given path.
    ///
    /// `poll_interval` defaults to 10 seconds if `None`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            poll_interval: Duration::from_secs(10),
        }
    }

    /// Override the polling interval (useful in tests).
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Spawn a background Tokio task that polls the file and sends updates
    /// through `tx` whenever the mtime changes and the new TOML is valid.
    ///
    /// The task runs until the `watch::Sender` is dropped (all receivers gone).
    pub fn start(self, tx: tokio::sync::watch::Sender<WatchedConfig>) {
        tokio::spawn(async move {
            let mut last_modified: Option<SystemTime> = None;

            loop {
                tokio::time::sleep(self.poll_interval).await;

                // Check if the sender still has receivers; exit if not.
                if tx.is_closed() {
                    break;
                }

                match Self::read_mtime(&self.path) {
                    None => {
                        // File doesn't exist yet — keep waiting
                    }
                    Some(mtime) => {
                        let changed = last_modified.map_or(true, |prev| prev != mtime);
                        if changed {
                            match Self::parse_config(&self.path) {
                                Ok(cfg) => {
                                    info!(
                                        "config_watcher: reloaded {:?}",
                                        self.path.display()
                                    );
                                    last_modified = Some(mtime);
                                    // send() only fails when all receivers are dropped
                                    let _ = tx.send(cfg);
                                }
                                Err(e) => {
                                    warn!(
                                        "config_watcher: parse error in {:?} — keeping old config. Error: {}",
                                        self.path.display(),
                                        e
                                    );
                                    // Update mtime so we don't spam the log on every poll
                                    last_modified = Some(mtime);
                                }
                            }
                        }
                    }
                }
            }

            info!("config_watcher: background task exiting (all receivers dropped)");
        });
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn read_mtime(path: &PathBuf) -> Option<SystemTime> {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
    }

    fn parse_config(path: &PathBuf) -> Result<WatchedConfig> {
        let content = std::fs::read_to_string(path)?;
        let cfg: WatchedConfig = toml::from_str(&content)?;
        Ok(cfg)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Helper: write TOML text to a temp file and return it.
    fn write_temp(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("temp file");
        f.write_all(content.as_bytes()).expect("write");
        f
    }

    // ── parse_config ──────────────────────────────────────────────────────────

    #[test]
    fn test_parse_valid_config() {
        let toml = r#"
[agent.master]
provider = "gemini"
model = "gemini-2.0-flash"
tools = ["shell", "file_read"]
instructions = "Be helpful."
"#;
        let f = write_temp(toml);
        let cfg = ConfigWatcher::parse_config(&f.path().to_path_buf())
            .expect("should parse");

        assert!(cfg.agent.contains_key("master"));
        let master = &cfg.agent["master"];
        assert_eq!(master.provider.as_deref(), Some("gemini"));
        assert_eq!(master.model.as_deref(), Some("gemini-2.0-flash"));
        let tools = master.tools.as_ref().expect("tools should be set");
        assert_eq!(tools, &["shell", "file_read"]);
        assert_eq!(master.instructions.as_deref(), Some("Be helpful."));
    }

    #[test]
    fn test_parse_multiple_agents() {
        let toml = r#"
[agent.master]
provider = "ollama"
model = "qwen3:8b"

[agent.coder]
provider = "chatgpt"
model = "gpt-4o"
tools = ["shell"]
"#;
        let f = write_temp(toml);
        let cfg = ConfigWatcher::parse_config(&f.path().to_path_buf())
            .expect("should parse");

        assert_eq!(cfg.agent.len(), 2);
        assert!(cfg.agent.contains_key("master"));
        assert!(cfg.agent.contains_key("coder"));
    }

    #[test]
    fn test_parse_empty_file_returns_default() {
        let f = write_temp("");
        let cfg = ConfigWatcher::parse_config(&f.path().to_path_buf())
            .expect("empty TOML should give default");
        assert!(cfg.agent.is_empty());
    }

    #[test]
    fn test_parse_invalid_toml_returns_error() {
        let f = write_temp("this is [[[not valid toml");
        let result = ConfigWatcher::parse_config(&f.path().to_path_buf());
        assert!(result.is_err(), "invalid TOML should return Err");
    }

    #[test]
    fn test_parse_nonexistent_file_returns_error() {
        let path = PathBuf::from("/tmp/clawtex-nonexistent-config-99999.toml");
        let result = ConfigWatcher::parse_config(&path);
        assert!(result.is_err());
    }

    // ── mtime detection ───────────────────────────────────────────────────────

    #[test]
    fn test_mtime_present_for_existing_file() {
        let f = write_temp("[agent.test]\nprovider = \"ollama\"\n");
        let mtime = ConfigWatcher::read_mtime(&f.path().to_path_buf());
        assert!(mtime.is_some());
    }

    #[test]
    fn test_mtime_none_for_missing_file() {
        let path = PathBuf::from("/tmp/clawtex-no-such-file-77777.toml");
        assert!(ConfigWatcher::read_mtime(&path).is_none());
    }

    // ── end-to-end: detect file change via watch channel ─────────────────────

    #[tokio::test]
    async fn test_detects_file_change() {
        use std::io::Write;

        let mut f = NamedTempFile::new().expect("temp file");
        // Initial content
        writeln!(f, "[agent.master]\nprovider = \"ollama\"").unwrap();

        let path = f.path().to_path_buf();
        let initial = ConfigWatcher::parse_config(&path).unwrap();
        let (tx, mut rx) = tokio::sync::watch::channel(initial);

        // Very short polling interval for the test
        let watcher = ConfigWatcher::new(path.clone()).with_interval(Duration::from_millis(50));
        watcher.start(tx);

        // Give the task one poll cycle to record the current mtime
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Rewrite the file with different content
        // Use a small sleep to guarantee an mtime change on fast file systems
        tokio::time::sleep(Duration::from_millis(10)).await;
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            writeln!(file, "[agent.master]\nprovider = \"gemini\"").unwrap();
        }

        // Wait for the watcher to pick up the change (up to 1 second)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            tokio::time::sleep(Duration::from_millis(60)).await;
            if rx.has_changed().unwrap_or(false) {
                rx.mark_unchanged();
                let cfg = rx.borrow().clone();
                assert_eq!(
                    cfg.agent.get("master").and_then(|a| a.provider.as_deref()),
                    Some("gemini"),
                    "reloaded config should contain new provider"
                );
                return;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("config_watcher did not detect file change within 1 second");
            }
        }
    }

    #[tokio::test]
    async fn test_invalid_config_keeps_old_config() {
        use std::io::Write;

        let mut f = NamedTempFile::new().expect("temp file");
        writeln!(f, "[agent.master]\nprovider = \"ollama\"").unwrap();

        let path = f.path().to_path_buf();
        let initial = ConfigWatcher::parse_config(&path).unwrap();
        let (tx, mut rx) = tokio::sync::watch::channel(initial.clone());

        let watcher = ConfigWatcher::new(path.clone()).with_interval(Duration::from_millis(50));
        watcher.start(tx);

        // Let the watcher record the initial mtime (first poll cycle sees the file,
        // records mtime, and sends the initial parsed config — consume that update).
        tokio::time::sleep(Duration::from_millis(150)).await;
        // Drain any initial update so we have a clean baseline.
        rx.mark_unchanged();

        // Write invalid TOML — guarantee a new mtime
        tokio::time::sleep(Duration::from_millis(10)).await;
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            writeln!(file, "[[[[invalid toml garbage").unwrap();
        }

        // Give the watcher time to attempt a reload
        tokio::time::sleep(Duration::from_millis(300)).await;

        // The channel value should remain unchanged (bad TOML must not be sent)
        let current = rx.borrow().clone();
        assert_eq!(
            current.agent.get("master").and_then(|a| a.provider.as_deref()),
            Some("ollama"),
            "bad TOML should not overwrite the channel value"
        );
        // No new update should have been pushed to the channel
        assert!(
            !rx.has_changed().unwrap_or(true),
            "invalid TOML should not trigger a watch channel update"
        );
    }

    #[test]
    fn test_with_interval_overrides_default() {
        let path = PathBuf::from("/tmp/test.toml");
        let watcher = ConfigWatcher::new(path).with_interval(Duration::from_secs(5));
        assert_eq!(watcher.poll_interval, Duration::from_secs(5));
    }
}
