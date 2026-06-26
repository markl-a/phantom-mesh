//! Example: experimental-memory (H3, FTS5 SQLite backend).
//!
//! Opens a temp DB, inserts three facts, searches them three ways:
//!   (1) plain token search
//!   (2) FTS5 OR-operator search
//!   (3) escaped literal-phrase search of operator-y text
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/skill-docs-target \
//!     cargo run -p phantom-mesh \
//!       --example experimental_memory_example \
//!       --features experimental-memory
//!
//! Expected last line: `experimental-memory OK`. Exit code 0.

use phantom_mesh::skillbank::{escape_fts5_query, SkillMemory, NewMemory};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use a per-process temp file so concurrent runs don't collide.
    let pid = std::process::id();
    let db_path = std::env::temp_dir().join(format!("skill_memory_example_{pid}.db"));
    let _ = std::fs::remove_file(&db_path); // fresh start
    let mem = SkillMemory::open_at(db_path.clone())?;

    for (text, tags) in [
        ("the quick brown fox", "animals"),
        ("rust is a memory-safe language", "programming"),
        ("FTS5 supports unicode tokenization", "sqlite"),
    ] {
        mem.insert(NewMemory {
            kind: "fact",
            source: "seed",
            text,
            tags,
        })
        .await?;
    }

    let hits = mem.search("rust", 10).await?;
    assert_eq!(hits.len(), 1, "expected 1 match for 'rust', got {hits:?}");
    println!("[1] plain search 'rust' -> {} hit", hits.len());

    let many = mem.search("supports OR fox", 10).await?;
    assert_eq!(
        many.len(),
        2,
        "expected 2 matches for OR-query, got {many:?}"
    );
    println!(
        "[2] OR-operator search 'supports OR fox' -> {} hits",
        many.len()
    );

    // Insert a row whose body contains FTS5 operator words.
    mem.insert(NewMemory {
        kind: "fact",
        source: "seed",
        text: "the literal phrase: AND OR NOT NEAR appears here",
        tags: "",
    })
    .await?;
    let escaped = escape_fts5_query("AND OR NOT NEAR");
    let lit = mem.search(&escaped, 10).await?;
    assert_eq!(
        lit.len(),
        1,
        "escaped literal-phrase search must find exactly one"
    );
    println!(
        "[3] escaped literal-phrase {escaped:?} -> {} hit",
        lit.len()
    );

    // Clean up.
    let _ = std::fs::remove_file(&db_path);

    println!("experimental-memory OK");
    Ok(())
}
