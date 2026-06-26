//! Example: experimental-tools (H5).
//!
//! Loads the full skill catalog, asserts there are 30 tools with
//! unique names + well-formed schemas, and exercises 3 tools:
//!   (1) skill_calculator: "2 + 3 * 4" -> 14
//!   (2) skill_text_stats: word/line/char counts
//!   (3) skill_uuid_gen:   produces a v4 UUID
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/skill-docs-target \
//!     cargo run -p phantom-mesh \
//!       --example experimental_tools_example \
//!       --features experimental-tools
//!
//! Expected last line: `experimental-tools OK`. Exit code 0.

use phantom_mesh::skillbank::tools::catalog;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cat = catalog();
    assert_eq!(cat.len(), 30, "catalog must have exactly 30 tools");

    // All names unique + every schema well-formed.
    let mut names: Vec<&str> = cat.iter().map(|t| t.name()).collect();
    let n_before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), n_before, "tool names must be unique");
    for t in cat.iter() {
        let s = t.schema();
        assert_eq!(s["type"], "function", "tool {} bad schema type", t.name());
        assert_eq!(
            s["function"]["name"],
            t.name(),
            "tool {} name mismatch",
            t.name()
        );
    }
    println!("[1] catalog OK: 30 unique tools, all schemas well-formed");

    // (a) Calculator
    let calc = cat
        .iter()
        .find(|t| t.name() == "skill_calculator")
        .unwrap();
    let r = calc.call(&json!({"expression": "2 + 3 * 4"})).await?;
    assert_eq!(r["result"], 14.0);
    println!("[2] skill_calculator: '2 + 3 * 4' = {}", r["result"]);

    // (b) Text stats
    let stats = cat
        .iter()
        .find(|t| t.name() == "skill_text_stats")
        .unwrap();
    let r = stats
        .call(&json!({"text": "one two\nthree four five"}))
        .await?;
    println!(
        "[3] skill_text_stats: words={} lines={} chars={}",
        r["words"], r["lines"], r["chars"]
    );

    // (c) UUID gen
    let uuid = cat.iter().find(|t| t.name() == "skill_uuid_gen").unwrap();
    let r = uuid.call(&json!({})).await?;
    let s = r["uuid"].as_str().unwrap();
    assert_eq!(s.len(), 36, "v4 uuid must be 36 chars including hyphens");
    println!("[4] skill_uuid_gen: {s}");

    println!("experimental-tools OK");
    Ok(())
}
