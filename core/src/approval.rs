//! Session-cached tool approvals.
//!
//! The agent loop calls an optional `ToolGate` before every tool
//! invocation (see `agent::run_with_callbacks_gated`). Without
//! caching, an interactive session re-prompts the user on every
//! repeated call — same `shell rm -rf node_modules`, same `git push`,
//! over and over — which gets old fast.
//!
//! [`ToolApprovalCache`] sits *behind* the user's gate function and
//! stores the decisions keyed on a canonical fingerprint of
//! `(tool_name, args)`. Callers wrap their base gate with
//! [`cached_gate`] to get a gate that:
//!
//!   1. Looks up the fingerprint in the cache; returns the cached
//!      decision if present.
//!   2. Otherwise calls the base gate, stores the result, and returns
//!      it.
//!
//! Modeled on Codex's `with_cached_approval` (see
//! `references/codex/codex-rs/core/src/tools/sandboxing.rs:71-117`).
//! Codex's variant lives inside the sandbox subsystem; ours decouples
//! the cache from the policy so any caller (TUI prompt, CI auto-deny,
//! headless allow-all) can plug in.
//!
//! Fingerprinting strategy:
//!
//! * `tool_name` is included verbatim — `shell` vs `file_write` matter.
//! * `args` is canonicalised via `serde_json::Value`'s sorted-key
//!   serialisation so semantically-equivalent JSON ({"a":1,"b":2}
//!   vs {"b":2,"a":1}) maps to the same key.
//! * No hashing — the fingerprint is the full JSON string. Cheap and
//!   debuggable; tool args are usually <1 KB.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::agent::ToolGateDecision;

#[derive(Default)]
pub struct ToolApprovalCache {
    inner: RwLock<HashMap<String, ToolGateDecision>>,
}

impl ToolApprovalCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Build the canonical fingerprint for a tool call. Public so
    /// tests + future cache inspection tooling can reuse the same
    /// keying scheme.
    pub fn fingerprint(name: &str, args: &Value) -> String {
        // serde_json::to_string preserves insertion order, which
        // would let `{"a":1,"b":2}` and `{"b":2,"a":1}` collide
        // into different cache entries. Round-tripping through a
        // `BTreeMap` (in `canonicalise`) gives us a key-sorted
        // representation — same fingerprint regardless of how the
        // model serialised the arguments.
        let canon = canonicalise(args);
        format!("{}::{}", name, canon)
    }

    pub fn check(&self, name: &str, args: &Value) -> Option<ToolGateDecision> {
        let key = Self::fingerprint(name, args);
        self.inner.read().ok().and_then(|g| g.get(&key).cloned())
    }

    pub fn remember(&self, name: &str, args: &Value, decision: ToolGateDecision) {
        let key = Self::fingerprint(name, args);
        if let Ok(mut g) = self.inner.write() {
            g.insert(key, decision);
        }
    }

    /// Drop every cached decision. Use at session boundary
    /// (`/clear`, agent switch, `spectyn logout`) to avoid one
    /// session's allowances bleeding into the next.
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.write() {
            g.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Wrap a base gate with a session cache. The returned closure has
/// the same signature as `ToolGate` so callers can pass it to
/// `AgentRuntime::run_with_callbacks_gated` unchanged.
///
/// Caveat: `Deny(reason)` decisions are cached too — that's
/// intentional, otherwise a per-call random reason string would re-
/// trigger the underlying gate every time. If callers want
/// "remember allows, re-ask on deny" semantics, they should wrap a
/// gate that consults the cache only for allows (an example sketched
/// in the unit tests below).
pub fn cached_gate<G>(
    cache: Arc<ToolApprovalCache>,
    base: G,
) -> impl Fn(&str, &Value) -> ToolGateDecision + Send + Sync + 'static
where
    G: Fn(&str, &Value) -> ToolGateDecision + Send + Sync + 'static,
{
    move |name: &str, args: &Value| -> ToolGateDecision {
        if let Some(cached) = cache.check(name, args) {
            return cached;
        }
        let decision = base(name, args);
        cache.remember(name, args, decision.clone());
        decision
    }
}

/// Sort object keys recursively + serialise. We avoid `serde_json`'s
/// `Map` (which is order-preserving by feature) by walking the
/// `Value` tree and rebuilding objects with sorted keys via
/// `BTreeMap`.
fn canonicalise(v: &Value) -> String {
    fn walk(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<String, Value> =
                    map.iter().map(|(k, v)| (k.clone(), walk(v))).collect();
                serde_json::to_value(sorted).unwrap_or_else(|_| Value::Null)
            }
            Value::Array(arr) => Value::Array(arr.iter().map(walk).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&walk(v)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn fingerprint_is_key_order_invariant() {
        let a = json!({"path": "/tmp/x", "mode": "r"});
        let b = json!({"mode": "r", "path": "/tmp/x"});
        assert_eq!(
            ToolApprovalCache::fingerprint("file_read", &a),
            ToolApprovalCache::fingerprint("file_read", &b),
        );
    }

    #[test]
    fn fingerprint_distinguishes_tools_and_args() {
        let args = json!({"path": "/tmp/x"});
        assert_ne!(
            ToolApprovalCache::fingerprint("file_read", &args),
            ToolApprovalCache::fingerprint("file_write", &args),
        );
        let other = json!({"path": "/tmp/y"});
        assert_ne!(
            ToolApprovalCache::fingerprint("file_read", &args),
            ToolApprovalCache::fingerprint("file_read", &other),
        );
    }

    #[test]
    fn nested_objects_and_arrays_canonicalise() {
        let a = json!({"opts": {"a": 1, "b": 2}, "list": [{"x": 1, "y": 2}]});
        let b = json!({"list": [{"y": 2, "x": 1}], "opts": {"b": 2, "a": 1}});
        assert_eq!(
            ToolApprovalCache::fingerprint("t", &a),
            ToolApprovalCache::fingerprint("t", &b),
        );
    }

    #[test]
    fn cache_round_trips() {
        let c = ToolApprovalCache::new();
        let args = json!({"path": "/tmp"});
        assert!(c.check("file_read", &args).is_none());
        c.remember("file_read", &args, ToolGateDecision::Allow);
        assert_eq!(c.check("file_read", &args), Some(ToolGateDecision::Allow));
        c.remember(
            "shell",
            &json!({"cmd": "rm -rf /"}),
            ToolGateDecision::Deny("nope".into()),
        );
        assert_eq!(c.len(), 2);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn cached_gate_calls_base_only_once_per_fingerprint() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_base = calls.clone();
        let base = move |_name: &str, _args: &Value| -> ToolGateDecision {
            calls_for_base.fetch_add(1, Ordering::SeqCst);
            ToolGateDecision::Allow
        };
        let cache = ToolApprovalCache::new();
        let gate = cached_gate(cache, base);
        let args = json!({"path": "/tmp/x"});

        assert_eq!(gate("file_read", &args), ToolGateDecision::Allow);
        assert_eq!(gate("file_read", &args), ToolGateDecision::Allow);
        assert_eq!(gate("file_read", &args), ToolGateDecision::Allow);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "base gate must run once");

        // Different args → cache miss → base runs again.
        let other = json!({"path": "/tmp/y"});
        assert_eq!(gate("file_read", &other), ToolGateDecision::Allow);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cached_gate_remembers_denies_too() {
        let cache = ToolApprovalCache::new();
        let base = |_name: &str, _args: &Value| -> ToolGateDecision {
            ToolGateDecision::Deny("scary".into())
        };
        let gate = cached_gate(cache.clone(), base);
        let args = json!({"cmd": "rm -rf /"});
        gate("shell", &args);
        // Cached: subsequent calls return the same deny without
        // hitting `base`.
        let cached = cache.check("shell", &args);
        assert_eq!(cached, Some(ToolGateDecision::Deny("scary".into())));
    }
}
