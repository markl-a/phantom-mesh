use serde::{Deserialize, Serialize};

/// A capability that a node can provide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Capability {
    Shell,
    FileRead,
    FileWrite,
    FileEdit,
    WebSearch,
    HttpRequest,
    ContentGeneration,
    Translation,
    Summarization,
    Calculator,
    Memory,
    LocalLlm,
    CloudLlm,
}

/// Qualification level for a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityQualifier {
    pub level: CapabilityLevel,
    pub allowed_commands: Vec<String>,
}

/// Capability proficiency level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CapabilityLevel {
    Basic,
    Standard,
    Advanced,
}

/// A requirement that can be satisfied by one or more capabilities (OR logic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    /// Any one of these capabilities satisfies the requirement.
    pub alternatives: Vec<Capability>,
}

impl CapabilityRequirement {
    pub fn new(alternatives: Vec<Capability>) -> Self {
        Self { alternatives }
    }

    pub fn single(cap: Capability) -> Self {
        Self { alternatives: vec![cap] }
    }

    pub fn is_satisfied_by(&self, available: &[Capability]) -> bool {
        self.alternatives.iter().any(|req| available.contains(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_requirement_single() {
        let req = CapabilityRequirement::single(Capability::Shell);
        assert!(req.is_satisfied_by(&[Capability::Shell, Capability::FileRead]));
        assert!(!req.is_satisfied_by(&[Capability::FileRead]));
    }

    #[test]
    fn test_capability_requirement_or() {
        let req = CapabilityRequirement::new(vec![Capability::LocalLlm, Capability::CloudLlm]);
        assert!(req.is_satisfied_by(&[Capability::CloudLlm]));
        assert!(req.is_satisfied_by(&[Capability::LocalLlm]));
        assert!(!req.is_satisfied_by(&[Capability::Shell]));
    }

    #[test]
    fn test_capability_serde_roundtrip() {
        let cap = Capability::Shell;
        let json = serde_json::to_string(&cap).unwrap();
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, back);
    }
}
