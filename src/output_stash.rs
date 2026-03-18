// Output Stash — save large tool results to disk and return a reference handle
//
// When a tool result exceeds 8000 tokens (~32000 chars), calling code can stash
// the content to `~/.clawtex/stash/` and receive a short handle back instead of
// flooding the LLM context window.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Manages stashing of large tool outputs to disk.
pub struct OutputStash {
    stash_dir: PathBuf,
}

impl OutputStash {
    /// Create a new stash using the default directory (`~/.clawtex/stash/`).
    pub fn new() -> Self {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let stash_dir = PathBuf::from(format!("{}/.clawtex/stash", home));
        Self { stash_dir }
    }

    /// Create a stash that writes to a custom directory (useful in tests).
    pub fn with_dir(stash_dir: PathBuf) -> Self {
        Self { stash_dir }
    }

    /// Ensure the stash directory exists.
    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.stash_dir)
            .with_context(|| format!("Failed to create stash dir: {}", self.stash_dir.display()))
    }

    /// Stash `content` to a timestamped file and return a short handle string.
    ///
    /// The handle looks like:
    /// `[Output stashed to /path/to/file (12345 bytes). Use file_read to access full content.]`
    pub fn stash(&self, content: &str) -> Result<String> {
        self.ensure_dir()?;

        // Build a timestamped filename.
        let timestamp = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        };
        let filename = format!("stash_{}.txt", timestamp);
        let path = self.stash_dir.join(&filename);

        fs::write(&path, content.as_bytes())
            .with_context(|| format!("Failed to write stash file: {}", path.display()))?;

        let size = content.len();
        let path_str = path.to_string_lossy().replace('\\', "/");
        Ok(format!(
            "[Output stashed to {} ({} bytes). Use file_read to access full content.]",
            path_str, size
        ))
    }

    /// Retrieve the full content of a previously stashed file.
    pub fn retrieve(&self, path: &str) -> Result<String> {
        let p = PathBuf::from(path);
        fs::read_to_string(&p)
            .with_context(|| format!("Failed to read stash file: {}", p.display()))
    }
}

impl Default for OutputStash {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_stash() -> (TempDir, OutputStash) {
        let dir = TempDir::new().unwrap();
        let stash = OutputStash::with_dir(dir.path().to_path_buf());
        (dir, stash)
    }

    #[test]
    fn test_stash_small_content() {
        let (_dir, stash) = temp_stash();
        let handle = stash.stash("hello world").unwrap();
        assert!(handle.contains("Output stashed to"));
        assert!(handle.contains("11 bytes"));
        assert!(handle.contains("file_read"));
    }

    #[test]
    fn test_stash_creates_file() {
        let (dir, stash) = temp_stash();
        stash.stash("content here").unwrap();
        let files: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_retrieve_roundtrip() {
        let (_dir, stash) = temp_stash();
        let big = "x".repeat(50_000);
        let handle = stash.stash(&big).unwrap();

        // Extract the path from inside the brackets.
        // Handle format: "[Output stashed to <PATH> (<SIZE> bytes). ...]"
        let after_to = handle.split(" to ").nth(1).unwrap();
        let path = after_to.split(' ').next().unwrap();

        let retrieved = stash.retrieve(path).unwrap();
        assert_eq!(retrieved, big);
    }

    #[test]
    fn test_stash_large_output_byte_count() {
        let (_dir, stash) = temp_stash();
        let content = "a".repeat(32_001);
        let handle = stash.stash(&content).unwrap();
        assert!(handle.contains("32001 bytes"));
    }

    #[test]
    fn test_stash_multiple_files_unique_names() {
        let (dir, stash) = temp_stash();
        // Stash twice — filenames must be unique (timestamp in millis; add tiny
        // sleep to guarantee different timestamps on fast machines).
        let _ = stash.stash("first").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _ = stash.stash("second").unwrap();
        let files: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_retrieve_nonexistent_file() {
        let (_dir, stash) = temp_stash();
        let result = stash.retrieve("/nonexistent/path/file.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_stash_handle_format() {
        let (_dir, stash) = temp_stash();
        let handle = stash.stash("test content").unwrap();
        // Must start with the bracket notation
        assert!(handle.starts_with('['));
        assert!(handle.ends_with(']'));
        assert!(handle.contains("bytes"));
        assert!(handle.contains("file_read"));
    }

    #[test]
    fn test_stash_dir_created_automatically() {
        let parent = TempDir::new().unwrap();
        // Use a nested path that doesn't exist yet.
        let nested = parent.path().join("a").join("b").join("c");
        let stash = OutputStash::with_dir(nested.clone());
        stash.stash("auto-create test").unwrap();
        assert!(nested.exists());
    }
}
