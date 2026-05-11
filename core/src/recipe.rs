//! Recipe export — Tier 2 of CO-EVOLUTION (`docs/CO-EVOLUTION.md` §38-69)
//! and the unit of CONTRIBUTOR-FUNNEL contribution (`docs/CONTRIBUTOR-FUNNEL.md`
//! §3 BROKER LAYER input).
//!
//! A Recipe is a content-addressed, ed25519-signed JSON document that
//! captures one autonomous evolution episode. Storage:
//!
//!   ~/.phantom-mesh/recipes/<sha256-of-body>.json
//!
//! v0.1.0 ships:
//!   - `phantom evolve publish [--private]` — export a recipe locally
//!   - sign with this machine's identity (`phantom keys init` first)
//!   - verify on round-trip (no broker yet)
//!
//! v0.2 adds:
//!   - `--share` (no `--private`) to POST to phantommesh.io broker
//!   - `phantom evolve adopt <url>` to fetch + verify + apply a recipe
//!   - GitHub OAuth link so recipes carry @username

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Path to `~/.phantom-mesh/recipes/`.
pub fn recipes_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("recipes")
}

pub fn ensure_dir() -> Result<()> {
    fs::create_dir_all(recipes_dir())?;
    Ok(())
}

/// Recipe descriptor — what platform / phantom version this was
/// generated against. Used by adopters to filter applicability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Descriptor {
    pub platform: String,
    pub phantom_version: String,
    pub core_sha: String,
    /// Classification hint — "extensions_only" | "scripts_or_tests" |
    /// "core_change" | "sensitive_change". The broker re-classifies
    /// authoritatively from the patch's file paths; this hint is just
    /// a fast pre-check for the local user.
    pub classification: String,
}

/// The canonical body that gets signed. Excludes the `signature` and
/// `author_pubkey` fields themselves (they wrap the body).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeBody {
    /// Content-addressed identifier (sha256 of canonical JSON of this body
    /// minus this field, but in v0.1.0 we just use sha256 of the body's
    /// `goal + plan + dead_ends + journey + patch` concatenation for
    /// simplicity).
    pub recipe_sha: String,

    /// Ephemeral session id from the originating EvolveCheckpoint (so
    /// adopters can cross-reference if the user shared the checkpoint
    /// alongside).
    pub session_id: String,

    /// One-line goal the autoevolve loop was given.
    pub goal: String,

    /// Multi-step plan as the agent saw it.
    pub plan: Vec<String>,

    /// Dead-ends + completed steps from the EvolveCheckpoint (negative
    /// + positive context for adopters).
    pub dead_ends: Vec<String>,
    pub completed_steps: Vec<String>,

    /// Cross-machine journey if the checkpoint was handed off via
    /// /rpc/evolve-handoff before completing.
    pub journey: Vec<JourneyEntry>,

    /// Optional `git format-patch` blob. None = recipe is informational
    /// only (e.g. a documentation update or a goal that didn't need code
    /// changes).
    pub patch: Option<String>,

    /// Platform / version metadata for adopters.
    pub descriptor: Descriptor,

    /// Unix ms when the recipe was published.
    pub published_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JourneyEntry {
    pub node: String,
    pub ts_ms: u64,
    pub note: String,
}

/// The signed envelope that gets written to disk. Body + author + sig.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub body: RecipeBody,
    /// Author's hex-encoded ed25519 public key.
    pub author_pubkey: String,
    /// hex(ed25519_sign(canonical_json(body), priv_key)).
    pub signature: String,
}

/// Compute a stable sha256 over the canonical fields. Used as the
/// content-addressed filename.
pub fn compute_sha(goal: &str, plan: &[String], patch: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(goal.as_bytes());
    hasher.update(b"\n");
    for step in plan {
        hasher.update(step.as_bytes());
        hasher.update(b"\n");
    }
    if let Some(p) = patch {
        hasher.update(p.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Build a signed Recipe from a body. Loads the user's signing key
/// from disk; errors if `phantom keys init` hasn't run.
pub fn sign(body: RecipeBody) -> Result<Recipe> {
    let canonical = serde_json::to_vec(&body)
        .context("serialising recipe body for signing")?;
    let signature = crate::identity::sign_hex(&canonical)
        .context("signing recipe — has `phantom keys init` been run?")?;
    let author_pubkey = crate::identity::load_pub_hex()?;
    Ok(Recipe {
        body,
        author_pubkey,
        signature,
    })
}

/// Verify a Recipe's signature. Returns Ok(true) if valid.
pub fn verify(recipe: &Recipe) -> Result<bool> {
    let canonical = serde_json::to_vec(&recipe.body)?;
    crate::identity::verify(&recipe.author_pubkey, &canonical, &recipe.signature)
}

/// Write a recipe to `~/.phantom-mesh/recipes/<recipe_sha>.json`.
/// Returns the path written.
pub fn save(recipe: &Recipe) -> Result<PathBuf> {
    ensure_dir()?;
    let path = recipes_dir().join(format!("{}.json", recipe.body.recipe_sha));
    let json = serde_json::to_string_pretty(recipe)?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Load a recipe from disk by sha (or full path).
pub fn load(sha_or_path: &str) -> Result<Recipe> {
    let path = if sha_or_path.contains('/') || sha_or_path.ends_with(".json") {
        PathBuf::from(sha_or_path)
    } else {
        recipes_dir().join(format!("{}.json", sha_or_path))
    };
    let s = fs::read_to_string(&path)
        .with_context(|| format!("reading recipe at {}", path.display()))?;
    let r: Recipe = serde_json::from_str(&s)?;
    Ok(r)
}

/// Heuristic classifier — runs over a `git format-patch` blob and
/// decides which CO-EVO Tier the recipe touches. The broker re-runs
/// this authoritatively; this is just a local hint.
pub fn classify_patch(patch: Option<&str>) -> &'static str {
    let Some(p) = patch else { return "extensions_only"; };
    // Sensitive paths (per CO-EVOLUTION.md §107 + SPEC-FREEZE-V1 §2):
    let sensitive = [
        "core/src/auth/",
        "core/src/mesh.rs",
        "core/src/keys.rs",
        "core/src/serve.rs",
        "templates/",
    ];
    for s in sensitive {
        if p.contains(s) {
            return "sensitive_change";
        }
    }
    if p.contains("core/src/") || p.contains("app/src-tauri/") {
        return "core_change";
    }
    if p.contains("scripts/") || p.contains("tests/") || p.contains("docs/") {
        return "scripts_or_tests";
    }
    "extensions_only"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_for_test() -> RecipeBody {
        RecipeBody {
            recipe_sha: compute_sha("fix CJK render", &["read tui.rs".into(), "edit display_width".into()], None),
            session_id: "sess-1".into(),
            goal: "fix CJK render".into(),
            plan: vec!["read tui.rs".into(), "edit display_width".into()],
            dead_ends: vec!["tried unicode-segmentation, didn't work".into()],
            completed_steps: vec!["used unicode-width crate".into()],
            journey: vec![],
            patch: None,
            descriptor: Descriptor {
                platform: "macos-aarch64".into(),
                phantom_version: "0.4.0".into(),
                core_sha: "abc1234567".into(),
                classification: "extensions_only".into(),
            },
            published_at_ms: 1777700000000,
        }
    }

    #[test]
    fn classify_patches_correctly() {
        assert_eq!(classify_patch(None), "extensions_only");
        assert_eq!(classify_patch(Some("--- a/core/src/keys.rs\n+++ b/core/src/keys.rs\n")), "sensitive_change");
        assert_eq!(classify_patch(Some("--- a/core/src/agent.rs\n+++ b/core/src/agent.rs\n")), "core_change");
        assert_eq!(classify_patch(Some("--- a/scripts/build-mac.sh\n+++ b/scripts/build-mac.sh\n")), "scripts_or_tests");
        assert_eq!(classify_patch(Some("--- a/docs/README.md\n+++ b/docs/README.md\n")), "scripts_or_tests");
    }

    #[test]
    fn compute_sha_is_stable() {
        let s1 = compute_sha("goal", &["a".into(), "b".into()], None);
        let s2 = compute_sha("goal", &["a".into(), "b".into()], None);
        assert_eq!(s1, s2);
        let s3 = compute_sha("different", &["a".into(), "b".into()], None);
        assert_ne!(s1, s3);
    }

    #[test]
    fn body_serialises_and_round_trips() {
        let body = body_for_test();
        let json = serde_json::to_string(&body).unwrap();
        let deserialised: RecipeBody = serde_json::from_str(&json).unwrap();
        assert_eq!(body.goal, deserialised.goal);
        assert_eq!(body.recipe_sha, deserialised.recipe_sha);
        assert_eq!(body.dead_ends.len(), deserialised.dead_ends.len());
    }
}
