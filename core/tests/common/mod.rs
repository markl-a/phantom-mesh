//! Shared helpers for integration tests.
//!
//! Lives at `tests/common/mod.rs` (a subdirectory module), NOT `tests/common.rs`
//! — so cargo treats it as a shared module included via `mod common;`, not as
//! its own test binary. Each consumer adds `mod common;` (+ `use
//! common::workspace_tempdir;`).
#![allow(dead_code)]

/// A `TempDir` the file/search tools will actually operate on.
///
/// `phantom_mesh::tools::{file, search}` enforce a workspace allow-list (cwd +
/// `~/.phantom-mesh` + `PHANTOM_EXTRA_ALLOWED_ROOTS`). A bare
/// `tempfile::tempdir()` lands in `/tmp` — outside that list — so the tools
/// reject the path with "path outside workspace" before a test can assert.
/// Create the temp dir under the crate dir (`CARGO_MANIFEST_DIR`), which is the
/// cwd root during `cargo test`.
///
/// It is the crate root, **not** `target/`: `search::glob`/`content` shell out
/// to `rg`, which honors `.gitignore`, and `target/` is ignored (→ "no files
/// found"). The crate root is not ignored, so rg sees the temp files. The file
/// tools accept either, so the crate root works for all callers.
///
/// `TempDir`'s `Drop` removes the directory (including on panic unwind), so no
/// stray dirs are left under `core/` in normal runs.
pub fn workspace_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("phantom-test-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("create temp dir under crate dir")
}
