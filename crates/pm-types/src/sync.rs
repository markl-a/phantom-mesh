use serde::{Deserialize, Serialize};

/// Strategy for synchronizing data across nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SyncStrategy {
    /// Events are append-only; conflicts are impossible.
    AppendOnly,
    /// Last write wins based on `updated_at` timestamp.
    LastWriteWins,
    /// Data stays on the local node only; never synced.
    LocalOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_strategy_serde() {
        let strategy = SyncStrategy::AppendOnly;
        let json = serde_json::to_string(&strategy).unwrap();
        let back: SyncStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SyncStrategy::AppendOnly);
    }

    #[test]
    fn test_sync_strategy_equality() {
        assert_ne!(SyncStrategy::AppendOnly, SyncStrategy::LastWriteWins);
        assert_ne!(SyncStrategy::LastWriteWins, SyncStrategy::LocalOnly);
    }
}
