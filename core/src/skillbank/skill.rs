//! Skill Document parser.
//!
//! A Skill Document is a Markdown file beginning with a `---`-fenced YAML
//! frontmatter block (matching `docs/skill-schema.json`) followed by
//! free-form Markdown body. This module gives us a typed view + round-trip
//! serializer so the curator (track H1) can read, mutate, and re-emit skills
//! without losing structure.
//!
//! Gated behind the `experimental-curator` cargo feature — the default
//! `cargo build` does not pull `serde_yaml` and does not compile this file.

#![cfg(feature = "experimental-curator")]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Structured frontmatter — mirrors `docs/skill-schema.json`.
///
/// `BTreeMap` for `inputs` (not `HashMap`) so serialization order is
/// deterministic, which is required for the round-trip equality test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub version: String,
    pub description: String,
    pub triggers: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Full Skill Document = frontmatter + Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDocument {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

/// Parse / serialize errors. Kept small + explicit so callers can match.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("missing opening `---` fence on line 1")]
    MissingOpeningFence,
    #[error("missing closing `---` fence")]
    MissingClosingFence,
    #[error("frontmatter is not valid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
}

/// Parse a Skill Document from the raw file contents.
pub fn parse_str(input: &str) -> Result<SkillDocument, SkillError> {
    // Normalize line endings — accept both LF and CRLF on input.
    let normalized = input.replace("\r\n", "\n");

    // Must begin with `---` followed by newline (no leading whitespace allowed).
    let after_open = normalized
        .strip_prefix("---\n")
        .ok_or(SkillError::MissingOpeningFence)?;

    // Find the closing `---` — must be at the start of a line.
    // We look for "\n---\n" or a trailing "\n---" at EOF. The closing fence
    // marks the end of the YAML region; everything after is body.
    let (yaml_str, body_str) = match after_open.find("\n---\n") {
        Some(idx) => {
            let yaml = &after_open[..idx];
            let body = &after_open[idx + "\n---\n".len()..];
            (yaml, body)
        }
        None => {
            // Allow `\n---` at EOF (no body).
            if let Some(stripped) = after_open.strip_suffix("\n---") {
                (stripped, "")
            } else {
                return Err(SkillError::MissingClosingFence);
            }
        }
    };

    let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml_str)?;

    // Normalize body: ensure exactly one trailing newline (round-trip stability).
    let mut body = body_str.to_string();
    while body.ends_with("\n\n") {
        body.pop();
    }
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }

    Ok(SkillDocument { frontmatter, body })
}

/// Serialize a Skill Document back to the canonical `---<yaml>---<body>` form.
/// Body is normalized to end with exactly one `\n`.
pub fn serialize(doc: &SkillDocument) -> Result<String, SkillError> {
    let yaml = serde_yaml::to_string(&doc.frontmatter)?;
    // serde_yaml's to_string already ends with `\n`. We don't want a double
    // newline before the closing fence.
    let yaml_trimmed = yaml.trim_end_matches('\n');

    // Normalize body the same way parse does, so round-trip is stable.
    let mut body = doc.body.clone();
    while body.ends_with("\n\n") {
        body.pop();
    }
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }

    Ok(format!("---\n{yaml_trimmed}\n---\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_struct_serializes_with_required_fields() {
        // Smoke test: just constructs the struct and serializes it via
        // serde_yaml. Doesn't go through parse_str (still stubbed).
        let fm = SkillFrontmatter {
            name: "test".into(),
            version: "0.1.0".into(),
            description: "a test".into(),
            triggers: vec!["t1".into()],
            tools: vec![],
            inputs: BTreeMap::new(),
            outputs: vec![],
            tags: vec![],
            created_at: None,
            author: None,
        };
        let yaml = serde_yaml::to_string(&fm).expect("yaml ok");
        assert!(yaml.contains("name: test"));
        assert!(yaml.contains("version: 0.1.0"));
        // Empty optional collections are skipped:
        assert!(!yaml.contains("tools:"));
        assert!(!yaml.contains("inputs:"));
        assert!(!yaml.contains("tags:"));
    }

    #[test]
    fn parse_str_minimal_document() {
        let input = "\
---
name: hello-world
version: 0.1.0
description: A minimal skill.
triggers:
  - say hello
---
# Hello, world

Body goes here.
";
        let doc = parse_str(input).expect("parse ok");
        assert_eq!(doc.frontmatter.name, "hello-world");
        assert_eq!(doc.frontmatter.version, "0.1.0");
        assert_eq!(doc.frontmatter.description, "A minimal skill.");
        assert_eq!(doc.frontmatter.triggers, vec!["say hello".to_string()]);
        assert!(doc.frontmatter.tools.is_empty());
        assert!(doc.frontmatter.inputs.is_empty());
        assert!(doc.body.starts_with("# Hello, world"));
        assert!(doc.body.ends_with('\n'));
    }

    #[test]
    fn round_trip_minimal_document() {
        let input = "\
---
name: hello-world
version: 0.1.0
description: A minimal skill.
triggers:
  - say hello
---
# Hello, world

Body goes here.
";
        let parsed = parse_str(input).expect("parse ok");
        let emitted = serialize(&parsed).expect("serialize ok");
        let reparsed = parse_str(&emitted).expect("re-parse ok");
        assert_eq!(parsed, reparsed, "round-trip must be lossless");
    }

    #[test]
    fn round_trip_sample_skill_from_docs() {
        // Reads docs/skills/sample-skill.md from the repo root.
        // CARGO_MANIFEST_DIR points to core/, so we go one level up.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("docs/skills/sample-skill.md");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let parsed = parse_str(&raw).expect("parse sample-skill.md");
        assert_eq!(parsed.frontmatter.name, "rebase-onto-main");
        assert!(!parsed.frontmatter.triggers.is_empty());
        assert!(parsed.frontmatter.tools.contains(&"git_status".to_string()));

        let emitted = serialize(&parsed).expect("serialize sample");
        let reparsed = parse_str(&emitted).expect("re-parse sample");
        assert_eq!(parsed, reparsed, "sample skill must round-trip");
    }

    #[test]
    fn parse_missing_opening_fence_errors() {
        let input = "name: bad\nversion: 0.1.0\n";
        let err = parse_str(input).expect_err("must reject");
        assert!(matches!(err, SkillError::MissingOpeningFence));
    }

    #[test]
    fn parse_missing_closing_fence_errors() {
        let input = "---\nname: bad\nversion: 0.1.0\ndescription: x\ntriggers: [t]\n";
        let err = parse_str(input).expect_err("must reject");
        assert!(matches!(err, SkillError::MissingClosingFence));
    }

    #[test]
    fn parse_invalid_yaml_errors() {
        let input = "---\nname: : : not yaml\n---\nbody\n";
        let err = parse_str(input).expect_err("must reject");
        assert!(matches!(err, SkillError::InvalidYaml(_)));
    }

    #[test]
    fn parse_missing_required_field_errors() {
        // Missing `triggers` (required).
        let input = "---\nname: x\nversion: 0.1.0\ndescription: y\n---\nbody\n";
        let err = parse_str(input).expect_err("must reject");
        assert!(
            matches!(err, SkillError::InvalidYaml(_)),
            "missing required serde field surfaces as InvalidYaml, got: {err:?}"
        );
    }

    #[test]
    fn sample_skill_validates_against_json_schema() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.parent().expect("workspace root");

        let schema_path = workspace.join("docs/skill-schema.json");
        let schema_str = std::fs::read_to_string(&schema_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", schema_path.display()));
        let schema: serde_json::Value = serde_json::from_str(&schema_str).expect("schema is json");
        let compiled = jsonschema::JSONSchema::compile(&schema).expect("schema compiles");

        let sample_path = workspace.join("docs/skills/sample-skill.md");
        let sample_raw = std::fs::read_to_string(&sample_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", sample_path.display()));
        let parsed = parse_str(&sample_raw).expect("parse sample");

        // Re-serialize the frontmatter via serde_json so jsonschema can validate it.
        let frontmatter_json =
            serde_json::to_value(&parsed.frontmatter).expect("frontmatter serializes to json");

        let msgs: Option<Vec<String>> = match compiled.validate(&frontmatter_json) {
            Ok(()) => None,
            Err(errors) => Some(errors.map(|e| e.to_string()).collect()),
        };
        if let Some(msgs) = msgs {
            panic!(
                "sample-skill.md frontmatter does NOT validate against docs/skill-schema.json:\n{}",
                msgs.join("\n")
            );
        }
    }
}
