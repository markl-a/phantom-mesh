use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// Risk level for an adaptation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdaptationRisk {
    /// Auto-apply without confirmation
    Safe,
    /// Needs user confirmation
    Normal,
    /// Must have explicit approval
    Dangerous,
}

/// System adaptations that can be applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Adaptation {
    // Safe
    AdjustScaling { delta: i32 },
    ReorderProviderTier { provider: String, new_priority: u32 },

    // Normal
    RebalanceTasks { from_node: String, to_node: String },
    InstallCapability { capability: String },
    SwitchClusterProfile { profile: String },

    // Dangerous
    RemoveNode { node_id: String, reason: String },
    DisableProvider { provider: String, reason: String },
}

impl Adaptation {
    pub fn risk_level(&self) -> AdaptationRisk {
        match self {
            Adaptation::AdjustScaling { .. } | Adaptation::ReorderProviderTier { .. } => {
                AdaptationRisk::Safe
            }
            Adaptation::RebalanceTasks { .. }
            | Adaptation::InstallCapability { .. }
            | Adaptation::SwitchClusterProfile { .. } => AdaptationRisk::Normal,
            Adaptation::RemoveNode { .. } | Adaptation::DisableProvider { .. } => {
                AdaptationRisk::Dangerous
            }
        }
    }

    /// Returns a unique key for deduplication of equivalent adaptations.
    pub fn adaptation_key(&self) -> String {
        match self {
            Adaptation::AdjustScaling { delta } => format!("AdjustScaling:{}", delta),
            Adaptation::ReorderProviderTier {
                provider,
                new_priority,
            } => format!("ReorderProviderTier:{}:{}", provider, new_priority),
            Adaptation::RebalanceTasks {
                from_node,
                to_node,
            } => format!("RebalanceTasks:{}:{}", from_node, to_node),
            Adaptation::InstallCapability { capability } => {
                format!("InstallCapability:{}", capability)
            }
            Adaptation::SwitchClusterProfile { profile } => {
                format!("SwitchClusterProfile:{}", profile)
            }
            Adaptation::RemoveNode { node_id, reason } => {
                format!("RemoveNode:{}:{}", node_id, reason)
            }
            Adaptation::DisableProvider { provider, reason } => {
                format!("DisableProvider:{}:{}", provider, reason)
            }
        }
    }

    pub fn description(&self) -> String {
        match self {
            Adaptation::AdjustScaling { delta } => format!("Adjust scaling by {}", delta),
            Adaptation::ReorderProviderTier {
                provider,
                new_priority,
            } => format!("Reorder {} to priority {}", provider, new_priority),
            Adaptation::RebalanceTasks {
                from_node,
                to_node,
            } => format!("Rebalance tasks from {} to {}", from_node, to_node),
            Adaptation::InstallCapability { capability } => {
                format!("Install capability: {}", capability)
            }
            Adaptation::SwitchClusterProfile { profile } => {
                format!("Switch cluster profile to: {}", profile)
            }
            Adaptation::RemoveNode { node_id, reason } => {
                format!("Remove node {}: {}", node_id, reason)
            }
            Adaptation::DisableProvider { provider, reason } => {
                format!("Disable provider {}: {}", provider, reason)
            }
        }
    }
}

/// Input data for pattern analysis.
#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    /// Per-provider average latency in ms
    pub provider_latencies: HashMap<String, f64>,
    /// Per-provider failure count in analysis window
    pub provider_failures: HashMap<String, u32>,
    /// Per-node task count in analysis window
    pub node_task_counts: HashMap<String, u32>,
    /// Per-node success rate (0.0-1.0)
    pub node_success_rates: HashMap<String, f64>,
    /// Missing capabilities encountered
    pub missing_capabilities: Vec<String>,
    /// Local LLM latency trend (recent average ms)
    pub local_latency_ms: Option<f64>,
    /// Previous local LLM latency (for trend detection)
    pub prev_local_latency_ms: Option<f64>,
}

/// Pending adaptation with approval state.
#[derive(Debug, Clone)]
pub struct PendingAdaptation {
    pub id: u64,
    pub adaptation: Adaptation,
    pub risk: AdaptationRisk,
    pub created_at: u64,
    pub approved: Option<bool>,
}

/// Architecture adaptor — analyzes system metrics and suggests/applies adaptations.
pub struct ArchitectureAdaptor {
    /// Threshold for provider failure count to trigger disable
    pub failure_threshold: u32,
    /// Threshold for node load imbalance ratio
    pub imbalance_ratio: f64,
    /// Threshold for local latency increase (ms)
    pub latency_increase_threshold: f64,
    /// Applied safe adaptations
    applied: Vec<Adaptation>,
    /// Pending normal/dangerous adaptations
    pending: Vec<PendingAdaptation>,
    /// Next adaptation ID
    next_id: u64,
}

impl ArchitectureAdaptor {
    pub fn new() -> Self {
        Self {
            failure_threshold: 10,
            imbalance_ratio: 3.0,
            latency_increase_threshold: 500.0,
            applied: Vec::new(),
            pending: Vec::new(),
            next_id: 1,
        }
    }

    /// Analyze system metrics and generate adaptations.
    pub fn analyze(&mut self, metrics: &SystemMetrics) -> Vec<Adaptation> {
        let mut adaptations = Vec::new();

        // Pattern 1: Provider with high failure rate -> DisableProvider (Dangerous)
        for (provider, failures) in &metrics.provider_failures {
            if *failures >= self.failure_threshold {
                adaptations.push(Adaptation::DisableProvider {
                    provider: provider.clone(),
                    reason: format!("{} failures in analysis window", failures),
                });
            }
        }

        // Pattern 2: Local LLM latency rising -> ReorderProviderTier (Safe)
        if let (Some(current), Some(prev)) =
            (metrics.local_latency_ms, metrics.prev_local_latency_ms)
        {
            if current - prev > self.latency_increase_threshold {
                adaptations.push(Adaptation::ReorderProviderTier {
                    provider: "local".to_string(),
                    new_priority: 99,
                });
            }
        }

        // Pattern 3: Node load imbalance -> RebalanceTasks (Normal)
        if metrics.node_task_counts.len() >= 2 {
            let max = metrics.node_task_counts.values().max().copied().unwrap_or(0);
            let min = metrics.node_task_counts.values().min().copied().unwrap_or(0);
            if (min == 0 && max > 0)
                || (min > 0 && (max as f64 / min as f64) > self.imbalance_ratio)
            {
                let max_node = metrics
                    .node_task_counts
                    .iter()
                    .max_by_key(|(_, v)| *v)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_default();
                let min_node = metrics
                    .node_task_counts
                    .iter()
                    .min_by_key(|(_, v)| *v)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_default();
                adaptations.push(Adaptation::RebalanceTasks {
                    from_node: max_node,
                    to_node: min_node,
                });
            }
        }

        // Pattern 4: Missing capabilities -> InstallCapability (Normal)
        for cap in &metrics.missing_capabilities {
            adaptations.push(Adaptation::InstallCapability {
                capability: cap.clone(),
            });
        }

        // Pattern 5: High task load overall -> AdjustScaling (Safe)
        let total_tasks: u32 = metrics.node_task_counts.values().sum();
        let node_count = metrics.node_task_counts.len() as u32;
        if node_count > 0 && total_tasks / node_count > 50 {
            adaptations.push(Adaptation::AdjustScaling { delta: 2 });
        }

        // Auto-apply safe adaptations, queue normal/dangerous
        let mut result = Vec::new();
        for adaptation in adaptations {
            let key = adaptation.adaptation_key();
            let risk = adaptation.risk_level();
            match risk {
                AdaptationRisk::Safe => {
                    // Dedupe: skip if an equivalent safe adaptation was already applied
                    if self.applied.iter().any(|a| a.adaptation_key() == key) {
                        continue;
                    }
                    info!(
                        "ArchitectureAdaptor: auto-applying safe adaptation: {}",
                        adaptation.description()
                    );
                    self.applied.push(adaptation.clone());
                    result.push(adaptation);
                }
                AdaptationRisk::Normal | AdaptationRisk::Dangerous => {
                    // Dedupe: skip if an equivalent pending adaptation already exists (unapproved)
                    if self
                        .pending
                        .iter()
                        .any(|p| p.approved.is_none() && p.adaptation.adaptation_key() == key)
                    {
                        continue;
                    }
                    let id = self.next_id;
                    self.next_id += 1;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    warn!(
                        "ArchitectureAdaptor: {:?} adaptation needs approval: {}",
                        risk,
                        adaptation.description()
                    );
                    self.pending.push(PendingAdaptation {
                        id,
                        adaptation: adaptation.clone(),
                        risk,
                        created_at: now,
                        approved: None,
                    });
                    result.push(adaptation);
                }
            }
        }

        result
    }

    /// Get pending adaptations that need approval.
    pub fn pending_approvals(&self) -> Vec<&PendingAdaptation> {
        self.pending.iter().filter(|p| p.approved.is_none()).collect()
    }

    /// Approve a pending adaptation. Only proceeds if not already decided.
    pub fn approve(&mut self, id: u64) -> bool {
        if let Some(pending) = self
            .pending
            .iter_mut()
            .find(|p| p.id == id && p.approved.is_none())
        {
            pending.approved = Some(true);
            self.applied.push(pending.adaptation.clone());
            info!(
                "ArchitectureAdaptor: approved adaptation #{}: {}",
                id,
                pending.adaptation.description()
            );
            true
        } else {
            false
        }
    }

    /// Reject a pending adaptation. Only proceeds if not already decided.
    pub fn reject(&mut self, id: u64) -> bool {
        if let Some(pending) = self
            .pending
            .iter_mut()
            .find(|p| p.id == id && p.approved.is_none())
        {
            pending.approved = Some(false);
            info!("ArchitectureAdaptor: rejected adaptation #{}", id);
            true
        } else {
            false
        }
    }

    /// Get all applied adaptations.
    pub fn applied_adaptations(&self) -> &[Adaptation] {
        &self.applied
    }
}

impl Default for ArchitectureAdaptor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_classification() {
        assert_eq!(
            Adaptation::AdjustScaling { delta: 1 }.risk_level(),
            AdaptationRisk::Safe
        );
        assert_eq!(
            Adaptation::ReorderProviderTier {
                provider: "p".into(),
                new_priority: 1
            }
            .risk_level(),
            AdaptationRisk::Safe
        );
        assert_eq!(
            Adaptation::RebalanceTasks {
                from_node: "a".into(),
                to_node: "b".into()
            }
            .risk_level(),
            AdaptationRisk::Normal
        );
        assert_eq!(
            Adaptation::InstallCapability {
                capability: "c".into()
            }
            .risk_level(),
            AdaptationRisk::Normal
        );
        assert_eq!(
            Adaptation::SwitchClusterProfile {
                profile: "p".into()
            }
            .risk_level(),
            AdaptationRisk::Normal
        );
        assert_eq!(
            Adaptation::RemoveNode {
                node_id: "n".into(),
                reason: "r".into()
            }
            .risk_level(),
            AdaptationRisk::Dangerous
        );
        assert_eq!(
            Adaptation::DisableProvider {
                provider: "p".into(),
                reason: "r".into()
            }
            .risk_level(),
            AdaptationRisk::Dangerous
        );
    }

    #[test]
    fn test_provider_failure_detection() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("openai".to_string(), 15);

        let results = adaptor.analyze(&metrics);
        assert!(results.iter().any(|a| matches!(a, Adaptation::DisableProvider { provider, .. } if provider == "openai")));
    }

    #[test]
    fn test_latency_increase_detection() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics.local_latency_ms = Some(1500.0);
        metrics.prev_local_latency_ms = Some(800.0);

        let results = adaptor.analyze(&metrics);
        assert!(results.iter().any(
            |a| matches!(a, Adaptation::ReorderProviderTier { provider, new_priority } if provider == "local" && *new_priority == 99)
        ));
        // Safe adaptation should be auto-applied
        assert!(adaptor.applied_adaptations().iter().any(
            |a| matches!(a, Adaptation::ReorderProviderTier { provider, .. } if provider == "local")
        ));
    }

    #[test]
    fn test_load_imbalance_detection() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics.node_task_counts.insert("node_a".to_string(), 40);
        metrics.node_task_counts.insert("node_b".to_string(), 10);

        let results = adaptor.analyze(&metrics);
        assert!(results
            .iter()
            .any(|a| matches!(a, Adaptation::RebalanceTasks { .. })));
        // Normal adaptation should be queued, not applied
        assert!(adaptor
            .pending_approvals()
            .iter()
            .any(|p| matches!(&p.adaptation, Adaptation::RebalanceTasks { .. })));
    }

    #[test]
    fn test_missing_capability_detection() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .missing_capabilities
            .push("code_review".to_string());

        let results = adaptor.analyze(&metrics);
        assert!(results.iter().any(
            |a| matches!(a, Adaptation::InstallCapability { capability } if capability == "code_review")
        ));
    }

    #[test]
    fn test_high_load_scaling() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        // Average of 60 tasks per node (> 50 threshold)
        metrics.node_task_counts.insert("node_a".to_string(), 60);
        metrics.node_task_counts.insert("node_b".to_string(), 60);

        let results = adaptor.analyze(&metrics);
        assert!(results
            .iter()
            .any(|a| matches!(a, Adaptation::AdjustScaling { delta: 2 })));
    }

    #[test]
    fn test_approve_reject() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("bad_provider".to_string(), 20);

        adaptor.analyze(&metrics);
        let pending = adaptor.pending_approvals();
        assert_eq!(pending.len(), 1);
        let id = pending[0].id;

        // Approve it
        assert!(adaptor.approve(id));
        assert!(adaptor.pending_approvals().is_empty());
        assert!(adaptor.applied_adaptations().iter().any(
            |a| matches!(a, Adaptation::DisableProvider { provider, .. } if provider == "bad_provider")
        ));

        // Try approving non-existent
        assert!(!adaptor.approve(999));

        // Test reject path
        let mut adaptor2 = ArchitectureAdaptor::new();
        metrics.provider_failures.clear();
        metrics
            .missing_capabilities
            .push("test_cap".to_string());
        adaptor2.analyze(&metrics);
        let pending2 = adaptor2.pending_approvals();
        assert_eq!(pending2.len(), 1);
        let id2 = pending2[0].id;
        assert!(adaptor2.reject(id2));
        assert!(adaptor2.pending_approvals().is_empty());
        assert!(!adaptor2.reject(999));
    }

    #[test]
    fn test_no_adaptations_on_healthy_system() {
        let mut adaptor = ArchitectureAdaptor::new();
        let metrics = SystemMetrics::default();

        let results = adaptor.analyze(&metrics);
        assert!(results.is_empty());
        assert!(adaptor.applied_adaptations().is_empty());
        assert!(adaptor.pending_approvals().is_empty());
    }

    #[test]
    fn test_safe_auto_applied() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        // Trigger latency increase (Safe adaptation)
        metrics.local_latency_ms = Some(2000.0);
        metrics.prev_local_latency_ms = Some(1000.0);

        adaptor.analyze(&metrics);
        // Safe adaptations go directly to applied
        assert!(!adaptor.applied_adaptations().is_empty());
        // No pending approvals for safe adaptations
        assert!(adaptor.pending_approvals().is_empty());
    }

    #[test]
    fn test_normal_queued() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .missing_capabilities
            .push("vision".to_string());

        adaptor.analyze(&metrics);
        // Normal adaptations go to pending, not applied
        assert!(adaptor.applied_adaptations().is_empty());
        assert_eq!(adaptor.pending_approvals().len(), 1);
        assert_eq!(adaptor.pending_approvals()[0].risk, AdaptationRisk::Normal);
    }

    #[test]
    fn test_dedupe_repeated_analyze() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("openai".to_string(), 15);
        metrics
            .missing_capabilities
            .push("vision".to_string());
        metrics.local_latency_ms = Some(2000.0);
        metrics.prev_local_latency_ms = Some(1000.0);

        // First analyze: should generate adaptations
        let results1 = adaptor.analyze(&metrics);
        assert!(!results1.is_empty());
        let pending_count = adaptor.pending_approvals().len();
        let applied_count = adaptor.applied_adaptations().len();

        // Second analyze with same metrics: should NOT duplicate
        let results2 = adaptor.analyze(&metrics);
        assert!(results2.is_empty());
        assert_eq!(adaptor.pending_approvals().len(), pending_count);
        assert_eq!(adaptor.applied_adaptations().len(), applied_count);
    }

    #[test]
    fn test_imbalance_min_zero() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        // Extreme imbalance: one node has tasks, other has zero
        metrics.node_task_counts.insert("node_a".to_string(), 10);
        metrics.node_task_counts.insert("node_b".to_string(), 0);

        let results = adaptor.analyze(&metrics);
        assert!(results
            .iter()
            .any(|a| matches!(a, Adaptation::RebalanceTasks { .. })));
        assert!(adaptor
            .pending_approvals()
            .iter()
            .any(|p| matches!(&p.adaptation, Adaptation::RebalanceTasks {
                from_node, to_node
            } if from_node == "node_a" && to_node == "node_b")));
    }

    #[test]
    fn test_approve_idempotent() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("bad_provider".to_string(), 20);

        adaptor.analyze(&metrics);
        let id = adaptor.pending_approvals()[0].id;

        // First approve succeeds
        assert!(adaptor.approve(id));
        assert_eq!(adaptor.applied_adaptations().len(), 1);

        // Second approve on same ID returns false (already decided)
        assert!(!adaptor.approve(id));
        // applied list should NOT have a duplicate
        assert_eq!(adaptor.applied_adaptations().len(), 1);
    }

    #[test]
    fn test_approve_after_reject_returns_false() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("bad_provider".to_string(), 20);

        adaptor.analyze(&metrics);
        let id = adaptor.pending_approvals()[0].id;

        // Reject first
        assert!(adaptor.reject(id));
        // Then try to approve -> should fail
        assert!(!adaptor.approve(id));
        // Nothing should be applied
        assert!(adaptor.applied_adaptations().is_empty());
    }

    #[test]
    fn test_reject_after_approve_returns_false() {
        let mut adaptor = ArchitectureAdaptor::new();
        let mut metrics = SystemMetrics::default();
        metrics
            .provider_failures
            .insert("bad_provider".to_string(), 20);

        adaptor.analyze(&metrics);
        let id = adaptor.pending_approvals()[0].id;

        // Approve first
        assert!(adaptor.approve(id));
        // Then try to reject -> should fail
        assert!(!adaptor.reject(id));
        // Should still be applied
        assert_eq!(adaptor.applied_adaptations().len(), 1);
    }
}
