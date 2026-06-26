//! Example: experimental-curator (H1 + H2).
//!
//! Demonstrates the public surface of `phantom_mesh::skillbank::curator` and
//! `phantom_mesh::skillbank::skill` without making any network calls:
//!
//!   1. Build a judge prompt from an EvolveCheckpoint and verify it
//!      contains the rubric version marker.
//!   2. Parse a (fake) judge reply and turn it into a JudgeVerdict.
//!   3. Parse a tiny Skill Document and round-trip it through serialize.
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/skill-docs-target \
//!     cargo run -p phantom-mesh \
//!       --example experimental_curator_example \
//!       --features experimental-curator
//!
//! Expected last line: `experimental-curator OK`. Exit code 0.

use phantom_mesh::evolve_checkpoint::EvolveCheckpoint;
use phantom_mesh::skillbank::skill::{parse_str, serialize};
use phantom_mesh::skillbank::{
    build_judge_user_prompt, parse_judge_reply, verdict_from_parsed, DEFAULT_JUDGE_MODEL,
    RUBRIC_VERSION,
};

fn main() {
    // (1) Curator: build the prompt a judge would see.
    let cp = EvolveCheckpoint::new("fix the lint", "check", "doc-example-node");
    let prompt = build_judge_user_prompt(&cp);
    assert!(
        prompt.contains(RUBRIC_VERSION),
        "rubric version must appear in prompt"
    );
    println!(
        "[1] judge prompt built ({} bytes, rubric={})",
        prompt.len(),
        RUBRIC_VERSION
    );

    // (2) Curator: parse a hypothetical judge reply and build a verdict.
    let raw = r#"{"score": 8, "rationale": "clean fix, all tests green"}"#;
    let (score, rationale) = parse_judge_reply(raw).expect("parse judge reply");
    assert_eq!(score, 8);
    let verdict = verdict_from_parsed(score, rationale.clone(), DEFAULT_JUDGE_MODEL.to_string(), 0);
    assert_eq!(verdict.rubric_version, RUBRIC_VERSION);
    println!(
        "[2] verdict parsed: score={} model={}",
        verdict.score, verdict.model
    );

    // (3) Skill: parse + round-trip a minimal Skill Document.
    let src = "---\nname: hello\nversion: 0.1.0\ndescription: minimal\ntriggers:\n  - say hi\n---\nbody line\n";
    let doc = parse_str(src).expect("parse skill");
    let back = serialize(&doc).expect("serialize skill");
    let reparsed = parse_str(&back).expect("reparse");
    assert_eq!(doc, reparsed, "skill round-trip must be lossless");
    println!(
        "[3] skill round-trip OK (name={}, version={})",
        doc.frontmatter.name, doc.frontmatter.version
    );

    println!("experimental-curator OK");
}
