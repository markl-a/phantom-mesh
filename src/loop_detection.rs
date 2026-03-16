//! Advanced Loop Detection — multi-layer detection for agent tool-call loops.
//! Detects: generic repeat, ping-pong patterns, stale results (same tool same output).
//! Replaces the simple LoopDetector in agent_runtime.rs.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tracing::debug;

/// Action the agent loop should take based on loop detection.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopAction {
    /// Continue normally
    Continue,
    /// Inject a nudge message to break the loop
    Warn(String),
    /// Stop the agent — loop is unrecoverable
    Stop(LoopKind),
}

/// Kind of loop detected
#[derive(Debug, Clone, PartialEq)]
pub enum LoopKind {
    /// Same tool calls repeated N times
    GenericRepeat { count: usize },
    /// Alternating between two tools
    PingPong { tool_a: String, tool_b: String },
    /// Same tool producing identical results
    StaleResult { tool: String, result_hash: u64 },
}

/// Configuration for the advanced loop detector
#[derive(Debug, Clone)]
pub struct LoopDetectorConfig {
    /// Number of identical rounds before warning (default: 3)
    pub warn_threshold: usize,
    /// Number of identical rounds before injecting nudge (default: 5)
    pub nudge_threshold: usize,
    /// Number of identical rounds before stopping (default: 8)
    pub stop_threshold: usize,
    /// Same tool returning same result N times (default: 3)
    pub stale_result_threshold: usize,
}

impl Default for LoopDetectorConfig {
    fn default() -> Self {
        Self {
            warn_threshold: 3,
            nudge_threshold: 5,
            stop_threshold: 8,
            stale_result_threshold: 3,
        }
    }
}

/// Advanced multi-layer loop detector.
pub struct AdvancedLoopDetector {
    /// Recent round signatures (hashes of tool call sets)
    round_signatures: VecDeque<u64>,
    /// Per-tool result hashes for stale detection
    result_hashes: HashMap<String, VecDeque<u64>>,
    /// Configuration
    config: LoopDetectorConfig,
    /// Count of consecutive identical round signatures
    consecutive_identical: usize,
    /// Last 4 round signatures for ping-pong detection
    pattern_buffer: VecDeque<u64>,
}

impl AdvancedLoopDetector {
    pub fn new(config: LoopDetectorConfig) -> Self {
        Self {
            round_signatures: VecDeque::with_capacity(config.stop_threshold + 1),
            result_hashes: HashMap::new(),
            config,
            consecutive_identical: 0,
            pattern_buffer: VecDeque::with_capacity(5),
        }
    }

    /// Record a round's tool calls and return the appropriate action.
    pub fn record_round(&mut self, tool_calls: &[(String, String)]) -> LoopAction {
        let sig = self.hash_tool_calls(tool_calls);

        // Track consecutive identical signatures
        if let Some(&last) = self.round_signatures.back() {
            if last == sig {
                self.consecutive_identical += 1;
            } else {
                self.consecutive_identical = 1;
            }
        } else {
            self.consecutive_identical = 1;
        }

        self.round_signatures.push_back(sig);
        if self.round_signatures.len() > self.config.stop_threshold + 2 {
            self.round_signatures.pop_front();
        }

        // Pattern buffer for ping-pong
        self.pattern_buffer.push_back(sig);
        if self.pattern_buffer.len() > 4 {
            self.pattern_buffer.pop_front();
        }

        // Check stop threshold first (most severe)
        if self.consecutive_identical >= self.config.stop_threshold {
            debug!("Loop detected: generic repeat ({} times)", self.consecutive_identical);
            return LoopAction::Stop(LoopKind::GenericRepeat {
                count: self.consecutive_identical,
            });
        }

        // Check ping-pong: A-B-A-B pattern in the last 4 rounds
        if let Some(pp) = self.detect_ping_pong(tool_calls) {
            return LoopAction::Stop(pp);
        }

        // Check nudge threshold
        if self.consecutive_identical >= self.config.nudge_threshold {
            return LoopAction::Warn(self.nudge_message());
        }

        // Check warn threshold
        if self.consecutive_identical >= self.config.warn_threshold {
            debug!("Loop warning: {} consecutive identical rounds", self.consecutive_identical);
            return LoopAction::Warn(format!(
                "Warning: detected {} consecutive identical tool calls. Consider trying a different approach.",
                self.consecutive_identical
            ));
        }

        LoopAction::Continue
    }

    /// Record a tool result and check for stale results.
    pub fn record_result(&mut self, tool_name: &str, result: &str) -> LoopAction {
        let result_hash = self.hash_string(result);

        let hashes = self.result_hashes
            .entry(tool_name.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.config.stale_result_threshold + 1));

        hashes.push_back(result_hash);
        if hashes.len() > self.config.stale_result_threshold + 2 {
            hashes.pop_front();
        }

        // Check if the last N results are all identical
        if hashes.len() >= self.config.stale_result_threshold {
            let recent: Vec<&u64> = hashes.iter().rev().take(self.config.stale_result_threshold).collect();
            if recent.iter().all(|h| **h == result_hash) {
                debug!("Stale result detected: tool '{}' returned same result {} times",
                    tool_name, self.config.stale_result_threshold);
                return LoopAction::Stop(LoopKind::StaleResult {
                    tool: tool_name.to_string(),
                    result_hash,
                });
            }
        }

        LoopAction::Continue
    }

    /// Generate a nudge message to break the loop.
    pub fn nudge_message(&self) -> String {
        "You seem to be repeating the same actions. Please try a completely different approach: \
         use different tools, different arguments, or reconsider whether the current strategy \
         is working. If you have enough information, provide your final answer now."
            .to_string()
    }

    /// Reset all state.
    pub fn reset(&mut self) {
        self.round_signatures.clear();
        self.result_hashes.clear();
        self.consecutive_identical = 0;
        self.pattern_buffer.clear();
    }

    /// Get the current consecutive identical count.
    pub fn consecutive_count(&self) -> usize {
        self.consecutive_identical
    }

    // ── Private helpers ──

    fn hash_tool_calls(&self, tool_calls: &[(String, String)]) -> u64 {
        let mut entries: Vec<String> = tool_calls
            .iter()
            .map(|(name, args)| format!("{}:{}", name, args))
            .collect();
        entries.sort();
        let mut hasher = DefaultHasher::new();
        entries.hash(&mut hasher);
        hasher.finish()
    }

    fn hash_string(&self, s: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    fn detect_ping_pong(&self, current_calls: &[(String, String)]) -> Option<LoopKind> {
        if self.pattern_buffer.len() < 4 {
            return None;
        }

        let buf: Vec<u64> = self.pattern_buffer.iter().copied().collect();
        // A-B-A-B pattern: buf[0]==buf[2] && buf[1]==buf[3] && buf[0]!=buf[1]
        if buf[0] == buf[2] && buf[1] == buf[3] && buf[0] != buf[1] {
            // Extract tool names for reporting
            let tool_a = current_calls.first()
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| "unknown".into());
            let tool_b = "alternating_tool".to_string();

            debug!("Ping-pong loop detected");
            return Some(LoopKind::PingPong { tool_a, tool_b });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_detector() -> AdvancedLoopDetector {
        AdvancedLoopDetector::new(LoopDetectorConfig::default())
    }

    fn small_detector() -> AdvancedLoopDetector {
        AdvancedLoopDetector::new(LoopDetectorConfig {
            warn_threshold: 2,
            nudge_threshold: 3,
            stop_threshold: 4,
            stale_result_threshold: 2,
        })
    }

    fn calls(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, a)| (n.to_string(), a.to_string())).collect()
    }

    // ── Generic Repeat tests ──

    #[test]
    fn test_no_loop_different_calls() {
        let mut d = default_detector();
        assert_eq!(d.record_round(&calls(&[("shell", "ls")])), LoopAction::Continue);
        assert_eq!(d.record_round(&calls(&[("shell", "pwd")])), LoopAction::Continue);
        assert_eq!(d.record_round(&calls(&[("file_read", "a.txt")])), LoopAction::Continue);
    }

    #[test]
    fn test_warn_at_threshold() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls")]);
        assert_eq!(d.record_round(&c), LoopAction::Continue); // 1
        assert_eq!(d.record_round(&c), LoopAction::Continue); // 2
        // 3 = warn_threshold
        match d.record_round(&c) {
            LoopAction::Warn(_) => {}
            other => panic!("Expected Warn, got {:?}", other),
        }
    }

    #[test]
    fn test_nudge_at_threshold() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls")]);
        for _ in 0..4 {
            d.record_round(&c);
        }
        // 5 = nudge_threshold
        match d.record_round(&c) {
            LoopAction::Warn(msg) => {
                assert!(msg.contains("different approach"), "Expected nudge message");
            }
            other => panic!("Expected Warn with nudge, got {:?}", other),
        }
    }

    #[test]
    fn test_stop_at_threshold() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls")]);
        for _ in 0..7 {
            d.record_round(&c);
        }
        // 8 = stop_threshold
        match d.record_round(&c) {
            LoopAction::Stop(LoopKind::GenericRepeat { count }) => {
                assert_eq!(count, 8);
            }
            other => panic!("Expected Stop(GenericRepeat), got {:?}", other),
        }
    }

    #[test]
    fn test_different_call_resets_count() {
        let mut d = default_detector();
        let c1 = calls(&[("shell", "ls")]);
        let c2 = calls(&[("shell", "pwd")]);
        d.record_round(&c1);
        d.record_round(&c1);
        // Break the streak
        assert_eq!(d.record_round(&c2), LoopAction::Continue);
        // Restart
        assert_eq!(d.record_round(&c1), LoopAction::Continue);
        assert_eq!(d.consecutive_count(), 1);
    }

    #[test]
    fn test_multi_tool_calls_per_round() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls"), ("file_read", "a.txt")]);
        assert_eq!(d.record_round(&c), LoopAction::Continue);
        assert_eq!(d.record_round(&c), LoopAction::Continue);
        match d.record_round(&c) {
            LoopAction::Warn(_) => {}
            other => panic!("Expected Warn, got {:?}", other),
        }
    }

    // ── Ping-Pong tests ──

    #[test]
    fn test_ping_pong_detection() {
        let mut d = small_detector();
        let a = calls(&[("shell", "ls")]);
        let b = calls(&[("file_read", "x.txt")]);
        d.record_round(&a); // A
        d.record_round(&b); // B
        d.record_round(&a); // A
        // B → completes A-B-A-B
        match d.record_round(&b) {
            LoopAction::Stop(LoopKind::PingPong { .. }) => {}
            other => panic!("Expected Stop(PingPong), got {:?}", other),
        }
    }

    #[test]
    fn test_no_ping_pong_with_three_different() {
        let mut d = default_detector();
        let a = calls(&[("shell", "ls")]);
        let b = calls(&[("file_read", "x")]);
        let c = calls(&[("shell", "pwd")]);
        assert_eq!(d.record_round(&a), LoopAction::Continue);
        assert_eq!(d.record_round(&b), LoopAction::Continue);
        assert_eq!(d.record_round(&c), LoopAction::Continue);
        assert_eq!(d.record_round(&a), LoopAction::Continue);
    }

    // ── Stale Result tests ──

    #[test]
    fn test_stale_result_detection() {
        let mut d = default_detector();
        assert_eq!(d.record_result("shell", "output1"), LoopAction::Continue);
        assert_eq!(d.record_result("shell", "output1"), LoopAction::Continue);
        match d.record_result("shell", "output1") {
            LoopAction::Stop(LoopKind::StaleResult { tool, .. }) => {
                assert_eq!(tool, "shell");
            }
            other => panic!("Expected Stop(StaleResult), got {:?}", other),
        }
    }

    #[test]
    fn test_stale_result_different_results() {
        let mut d = default_detector();
        assert_eq!(d.record_result("shell", "output1"), LoopAction::Continue);
        assert_eq!(d.record_result("shell", "output2"), LoopAction::Continue);
        assert_eq!(d.record_result("shell", "output3"), LoopAction::Continue);
    }

    #[test]
    fn test_stale_result_different_tools_independent() {
        let mut d = default_detector();
        d.record_result("shell", "same");
        d.record_result("shell", "same");
        // Different tool doesn't count toward shell's stale threshold
        d.record_result("file_read", "same");
        // shell at 2, not at threshold (3)
        assert_eq!(d.record_result("file_read", "same"), LoopAction::Continue);
    }

    #[test]
    fn test_stale_result_broken_by_different() {
        let mut d = small_detector(); // stale_result_threshold = 2
        d.record_result("shell", "out");
        d.record_result("shell", "different"); // breaks the streak
        assert_eq!(d.record_result("shell", "out"), LoopAction::Continue);
    }

    // ── Reset & Misc tests ──

    #[test]
    fn test_reset_clears_state() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls")]);
        d.record_round(&c);
        d.record_round(&c);
        d.record_result("shell", "out");
        d.reset();
        assert_eq!(d.consecutive_count(), 0);
        assert_eq!(d.record_round(&c), LoopAction::Continue);
    }

    #[test]
    fn test_nudge_message_content() {
        let d = default_detector();
        let msg = d.nudge_message();
        assert!(msg.contains("different approach"));
        assert!(msg.contains("final answer"));
    }

    #[test]
    fn test_consecutive_count() {
        let mut d = default_detector();
        let c = calls(&[("shell", "ls")]);
        d.record_round(&c);
        assert_eq!(d.consecutive_count(), 1);
        d.record_round(&c);
        assert_eq!(d.consecutive_count(), 2);
    }
}
