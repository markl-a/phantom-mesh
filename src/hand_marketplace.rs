//! Hand Marketplace — discover, package, and install Hand workflow packages.
//!
//! Provides:
//! - `HandPackage` metadata for marketplace listings
//! - `MarketplaceIndex` with search and filtering
//! - Packaging hands into a portable archive format
//! - Installing hand packages from archive bytes
//! - Validating hand package structure and TOML

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Hand Package metadata
// ---------------------------------------------------------------------------

/// Metadata for a hand package in the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandPackage {
    /// Unique hand name (e.g., "lead", "content", "seo_content").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic version string (e.g., "1.0.0").
    pub version: String,
    /// Author name or handle.
    pub author: String,
    /// Tools required by this hand.
    pub tools_required: Vec<String>,
    /// Number of phases in the workflow.
    pub phases_count: usize,
    /// Number of downloads (marketplace metric).
    #[serde(default)]
    pub downloads: u64,
    /// Average rating (0.0 - 5.0).
    #[serde(default)]
    pub rating: f64,
    /// Category (e.g., "research", "content", "automation").
    #[serde(default)]
    pub category: String,
    /// Tags for search (e.g., ["seo", "blog", "marketing"]).
    #[serde(default)]
    pub tags: Vec<String>,
}

impl HandPackage {
    /// Check if this package matches a search query.
    /// Matches against name, description, category, author, and tags (case-insensitive).
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
            || self.category.to_lowercase().contains(&q)
            || self.author.to_lowercase().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
    }
}

// ---------------------------------------------------------------------------
// Marketplace Index
// ---------------------------------------------------------------------------

/// In-memory index of available hand packages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceIndex {
    /// All known packages.
    pub packages: Vec<HandPackage>,
    /// Last updated timestamp (unix seconds).
    #[serde(default)]
    pub last_updated: u64,
}

impl MarketplaceIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            last_updated: 0,
        }
    }

    /// Add a package to the index.
    pub fn add(&mut self, package: HandPackage) {
        // Replace if already exists (same name + version).
        self.packages
            .retain(|p| !(p.name == package.name && p.version == package.version));
        self.packages.push(package);
    }

    /// Search packages by query string. Returns matching packages sorted by relevance.
    pub fn search(&self, query: &str) -> Vec<&HandPackage> {
        if query.is_empty() {
            return self.packages.iter().collect();
        }
        let q = query.to_lowercase();
        let mut results: Vec<(&HandPackage, u32)> = self
            .packages
            .iter()
            .filter(|p| p.matches(&q))
            .map(|p| {
                // Score: exact name match = 100, name contains = 50, else = 10
                let score = if p.name.to_lowercase() == q {
                    100
                } else if p.name.to_lowercase().contains(&q) {
                    50
                } else {
                    10
                };
                (p, score)
            })
            .collect();
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.into_iter().map(|(p, _)| p).collect()
    }

    /// Get packages by category.
    pub fn by_category(&self, category: &str) -> Vec<&HandPackage> {
        self.packages
            .iter()
            .filter(|p| p.category.eq_ignore_ascii_case(category))
            .collect()
    }

    /// Get the top N packages by downloads.
    pub fn top_downloads(&self, n: usize) -> Vec<&HandPackage> {
        let mut sorted: Vec<&HandPackage> = self.packages.iter().collect();
        sorted.sort_by(|a, b| b.downloads.cmp(&a.downloads));
        sorted.into_iter().take(n).collect()
    }

    /// Get the top N packages by rating.
    pub fn top_rated(&self, n: usize) -> Vec<&HandPackage> {
        let mut sorted: Vec<&HandPackage> = self.packages.iter().collect();
        sorted.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(std::cmp::Ordering::Equal));
        sorted.into_iter().take(n).collect()
    }

    /// Total package count.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

impl Default for MarketplaceIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Package Archive Format
// ---------------------------------------------------------------------------
//
// Simple binary archive format (no external crate needed):
//
//   MAGIC (8 bytes): "CLAWHPKG"
//   VERSION (1 byte): 0x01
//   FILE_COUNT (u32 LE)
//   For each file:
//     NAME_LEN (u32 LE)
//     NAME (NAME_LEN bytes, UTF-8)
//     DATA_LEN (u32 LE)
//     DATA (DATA_LEN bytes)

const ARCHIVE_MAGIC: &[u8; 8] = b"CLAWHPKG";
const ARCHIVE_VERSION: u8 = 0x01;

/// An entry in a hand package archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Pack a list of file entries into the binary archive format.
pub fn pack_archive(entries: &[ArchiveEntry]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(ARCHIVE_MAGIC);
    buf.push(ARCHIVE_VERSION);
    buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());

    for entry in entries {
        let name_bytes = entry.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&entry.data);
    }

    buf
}

/// Unpack the binary archive format into file entries.
pub fn unpack_archive(data: &[u8]) -> Result<Vec<ArchiveEntry>> {
    if data.len() < 13 {
        return Err(anyhow!("Archive too small (need at least 13 bytes)"));
    }

    if &data[0..8] != ARCHIVE_MAGIC {
        return Err(anyhow!("Invalid archive magic (expected CLAWHPKG)"));
    }

    if data[8] != ARCHIVE_VERSION {
        return Err(anyhow!(
            "Unsupported archive version {} (expected {})",
            data[8],
            ARCHIVE_VERSION
        ));
    }

    let file_count = u32::from_le_bytes([data[9], data[10], data[11], data[12]]) as usize;
    let mut offset = 13;
    let mut entries = Vec::with_capacity(file_count);

    for i in 0..file_count {
        if offset + 4 > data.len() {
            return Err(anyhow!("Truncated archive at file {} name length", i));
        }
        let name_len =
            u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
                as usize;
        offset += 4;

        if offset + name_len > data.len() {
            return Err(anyhow!("Truncated archive at file {} name data", i));
        }
        let name = std::str::from_utf8(&data[offset..offset + name_len])
            .map_err(|_| anyhow!("Invalid UTF-8 in file {} name", i))?
            .to_string();
        offset += name_len;

        if offset + 4 > data.len() {
            return Err(anyhow!("Truncated archive at file {} data length", i));
        }
        let data_len =
            u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
                as usize;
        offset += 4;

        if offset + data_len > data.len() {
            return Err(anyhow!("Truncated archive at file {} data", i));
        }
        let file_data = data[offset..offset + data_len].to_vec();
        offset += data_len;

        entries.push(ArchiveEntry {
            name,
            data: file_data,
        });
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Package / Install operations
// ---------------------------------------------------------------------------

/// Package a hand directory into archive bytes.
///
/// Reads `hand.toml` (required) and any other files in the directory,
/// and bundles them into the CLAWHPKG archive format.
pub fn package_hand(hand_dir: &str) -> Result<Vec<u8>> {
    let dir = Path::new(hand_dir);
    if !dir.is_dir() {
        return Err(anyhow!("Hand directory does not exist: {}", hand_dir));
    }

    let hand_toml_path = dir.join("hand.toml");
    if !hand_toml_path.exists() {
        return Err(anyhow!("Missing hand.toml in {}", hand_dir));
    }

    let mut entries = Vec::new();

    // Collect all files in the directory (non-recursive for safety).
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let data = std::fs::read(&path)?;
            entries.push(ArchiveEntry {
                name: file_name,
                data,
            });
        }
    }

    if entries.is_empty() {
        return Err(anyhow!("No files found in hand directory"));
    }

    Ok(pack_archive(&entries))
}

/// Install a hand from package bytes into the hands directory.
///
/// Extracts the archive, validates it contains a valid hand.toml,
/// and writes the files to `{hands_dir}/{hand_name}/`.
///
/// Returns the installed hand name.
pub fn install_hand(package_bytes: &[u8], hands_dir: &str) -> Result<String> {
    let entries = unpack_archive(package_bytes)?;

    // Find and validate hand.toml
    let hand_toml_entry = entries
        .iter()
        .find(|e| e.name == "hand.toml")
        .ok_or_else(|| anyhow!("Package does not contain hand.toml"))?;

    let hand_toml_str = std::str::from_utf8(&hand_toml_entry.data)
        .map_err(|_| anyhow!("hand.toml is not valid UTF-8"))?;

    let hand_meta: HashMap<String, toml::Value> = toml::from_str(hand_toml_str)
        .map_err(|e| anyhow!("Invalid TOML in hand.toml: {}", e))?;

    let hand_name = hand_meta
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("hand.toml missing 'name' field"))?
        .to_string();

    // Validate name is safe for filesystem
    if hand_name.is_empty()
        || !hand_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(anyhow!(
            "Invalid hand name '{}' — must be alphanumeric with hyphens/underscores",
            hand_name
        ));
    }

    // Create target directory
    let target_dir = Path::new(hands_dir).join(&hand_name);
    std::fs::create_dir_all(&target_dir)?;

    // Write all files
    for entry in &entries {
        // Prevent path traversal
        if entry.name.contains("..") || entry.name.contains('/') || entry.name.contains('\\') {
            return Err(anyhow!(
                "Suspicious file name in package: '{}'",
                entry.name
            ));
        }
        let file_path = target_dir.join(&entry.name);
        std::fs::write(&file_path, &entry.data)?;
    }

    Ok(hand_name)
}

/// Validate a hand package without installing it.
///
/// Checks:
/// - Valid CLAWHPKG archive format
/// - Contains hand.toml
/// - hand.toml is valid TOML with required fields
/// - Returns the parsed HandPackage metadata
pub fn validate_hand_package(data: &[u8]) -> Result<HandPackage> {
    let entries = unpack_archive(data)?;

    let hand_toml_entry = entries
        .iter()
        .find(|e| e.name == "hand.toml")
        .ok_or_else(|| anyhow!("Package does not contain hand.toml"))?;

    let hand_toml_str = std::str::from_utf8(&hand_toml_entry.data)
        .map_err(|_| anyhow!("hand.toml is not valid UTF-8"))?;

    let hand_meta: HashMap<String, toml::Value> = toml::from_str(hand_toml_str)
        .map_err(|e| anyhow!("Invalid TOML in hand.toml: {}", e))?;

    let name = hand_meta
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("hand.toml missing 'name' field"))?
        .to_string();

    let description = hand_meta
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("No description")
        .to_string();

    let version = hand_meta
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0")
        .to_string();

    let author = hand_meta
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let category = hand_meta
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tools_required = hand_meta
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let phases_count = hand_meta
        .get("phases")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    Ok(HandPackage {
        name,
        description,
        version,
        author,
        tools_required,
        phases_count,
        downloads: 0,
        rating: 0.0,
        category,
        tags: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_package(name: &str) -> HandPackage {
        HandPackage {
            name: name.to_string(),
            description: format!("A {} hand", name),
            version: "1.0.0".to_string(),
            author: "clawtex".to_string(),
            tools_required: vec!["web_search".to_string()],
            phases_count: 3,
            downloads: 100,
            rating: 4.5,
            category: "research".to_string(),
            tags: vec!["ai".to_string(), "automation".to_string()],
        }
    }

    fn sample_index() -> MarketplaceIndex {
        let mut idx = MarketplaceIndex::new();
        idx.add(HandPackage {
            name: "lead".to_string(),
            description: "Lead generation workflow".to_string(),
            version: "1.0.0".to_string(),
            author: "clawtex".to_string(),
            tools_required: vec!["web_search".to_string(), "email_send".to_string()],
            phases_count: 4,
            downloads: 500,
            rating: 4.8,
            category: "marketing".to_string(),
            tags: vec!["leads".to_string(), "sales".to_string()],
        });
        idx.add(HandPackage {
            name: "content".to_string(),
            description: "Content creation pipeline".to_string(),
            version: "2.0.0".to_string(),
            author: "clawtex".to_string(),
            tools_required: vec!["web_search".to_string(), "file_write".to_string()],
            phases_count: 4,
            downloads: 300,
            rating: 4.2,
            category: "content".to_string(),
            tags: vec!["blog".to_string(), "writing".to_string()],
        });
        idx.add(HandPackage {
            name: "seo_content".to_string(),
            description: "SEO-optimized content generator".to_string(),
            version: "1.2.0".to_string(),
            author: "community".to_string(),
            tools_required: vec!["web_search".to_string()],
            phases_count: 5,
            downloads: 200,
            rating: 3.9,
            category: "content".to_string(),
            tags: vec!["seo".to_string(), "marketing".to_string()],
        });
        idx
    }

    fn make_hand_toml(name: &str) -> String {
        format!(
            r#"name = "{name}"
description = "Test hand"
version = "1.0.0"
author = "test"
category = "test"
provider = "auto"
output_format = "markdown"
tools = ["web_search", "file_write"]

[[phases]]
name = "research"
system_prompt = "Research the topic."

[[phases]]
name = "write"
system_prompt = "Write the content."
"#
        )
    }

    // -- HandPackage tests --

    #[test]
    fn test_hand_package_serialize() {
        let pkg = sample_package("test");
        let json = serde_json::to_string(&pkg).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"rating\":4.5"));
        assert!(json.contains("\"phases_count\":3"));
    }

    #[test]
    fn test_hand_package_matches_name() {
        let pkg = sample_package("lead");
        assert!(pkg.matches("lead"));
        assert!(pkg.matches("Lead")); // case-insensitive
        assert!(!pkg.matches("xyz"));
    }

    #[test]
    fn test_hand_package_matches_description() {
        let pkg = sample_package("lead");
        assert!(pkg.matches("hand")); // "A lead hand"
    }

    #[test]
    fn test_hand_package_matches_tag() {
        let pkg = sample_package("lead");
        assert!(pkg.matches("automation"));
        assert!(pkg.matches("AI")); // case-insensitive
    }

    #[test]
    fn test_hand_package_matches_author() {
        let pkg = sample_package("lead");
        assert!(pkg.matches("clawtex"));
    }

    #[test]
    fn test_hand_package_matches_category() {
        let pkg = sample_package("lead");
        assert!(pkg.matches("research"));
    }

    // -- MarketplaceIndex tests --

    #[test]
    fn test_index_new_is_empty() {
        let idx = MarketplaceIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_index_add_and_len() {
        let mut idx = MarketplaceIndex::new();
        idx.add(sample_package("a"));
        idx.add(sample_package("b"));
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn test_index_add_replaces_same_name_version() {
        let mut idx = MarketplaceIndex::new();
        let mut pkg1 = sample_package("lead");
        pkg1.downloads = 100;
        idx.add(pkg1);

        let mut pkg2 = sample_package("lead");
        pkg2.downloads = 200;
        idx.add(pkg2);

        assert_eq!(idx.len(), 1);
        assert_eq!(idx.packages[0].downloads, 200);
    }

    #[test]
    fn test_search_empty_query_returns_all() {
        let idx = sample_index();
        let results = idx.search("");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_search_by_name() {
        let idx = sample_index();
        let results = idx.search("lead");
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "lead");
    }

    #[test]
    fn test_search_by_tag() {
        let idx = sample_index();
        let results = idx.search("seo");
        assert!(!results.is_empty());
        assert!(results.iter().any(|p| p.name == "seo_content"));
    }

    #[test]
    fn test_search_no_match() {
        let idx = sample_index();
        let results = idx.search("zzz_nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_exact_name_ranked_first() {
        let idx = sample_index();
        let results = idx.search("content");
        assert!(results.len() >= 2);
        // "content" should rank higher than "seo_content" (exact vs contains)
        assert_eq!(results[0].name, "content");
    }

    #[test]
    fn test_by_category() {
        let idx = sample_index();
        let content = idx.by_category("content");
        assert_eq!(content.len(), 2);
        assert!(content.iter().all(|p| p.category == "content"));
    }

    #[test]
    fn test_top_downloads() {
        let idx = sample_index();
        let top = idx.top_downloads(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "lead"); // 500 downloads
        assert_eq!(top[1].name, "content"); // 300 downloads
    }

    #[test]
    fn test_top_rated() {
        let idx = sample_index();
        let top = idx.top_rated(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "lead"); // 4.8
        assert_eq!(top[1].name, "content"); // 4.2
    }

    // -- Archive format tests --

    #[test]
    fn test_pack_unpack_roundtrip() {
        let entries = vec![
            ArchiveEntry {
                name: "hand.toml".to_string(),
                data: b"name = \"test\"".to_vec(),
            },
            ArchiveEntry {
                name: "readme.txt".to_string(),
                data: b"Hello world".to_vec(),
            },
        ];
        let packed = pack_archive(&entries);
        let unpacked = unpack_archive(&packed).unwrap();
        assert_eq!(unpacked.len(), 2);
        assert_eq!(unpacked[0].name, "hand.toml");
        assert_eq!(unpacked[0].data, b"name = \"test\"");
        assert_eq!(unpacked[1].name, "readme.txt");
        assert_eq!(unpacked[1].data, b"Hello world");
    }

    #[test]
    fn test_pack_empty_entries() {
        let packed = pack_archive(&[]);
        let unpacked = unpack_archive(&packed).unwrap();
        assert!(unpacked.is_empty());
    }

    #[test]
    fn test_unpack_invalid_magic() {
        let data = b"BADMAGIC\x01\x00\x00\x00\x00";
        let result = unpack_archive(data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("magic"));
    }

    #[test]
    fn test_unpack_too_small() {
        let data = b"SHORT";
        let result = unpack_archive(data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn test_unpack_wrong_version() {
        let mut data = Vec::new();
        data.extend_from_slice(ARCHIVE_MAGIC);
        data.push(0xFF); // bad version
        data.extend_from_slice(&0u32.to_le_bytes());
        let result = unpack_archive(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("version"));
    }

    #[test]
    fn test_unpack_truncated_file() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: b"content".to_vec(),
        }];
        let packed = pack_archive(&entries);
        // Truncate to cut off file data
        let truncated = &packed[..packed.len() - 3];
        let result = unpack_archive(truncated);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Truncated"));
    }

    // -- Package / Install tests (using tempdir) --

    #[test]
    fn test_package_hand_directory() {
        let dir = tempfile::tempdir().unwrap();
        let hand_toml = make_hand_toml("test_hand");
        std::fs::write(dir.path().join("hand.toml"), &hand_toml).unwrap();
        std::fs::write(dir.path().join("extra.txt"), "extra data").unwrap();

        let archive = package_hand(dir.path().to_str().unwrap()).unwrap();
        assert!(!archive.is_empty());

        // Verify it can be unpacked
        let entries = unpack_archive(&archive).unwrap();
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hand.toml"));
        assert!(names.contains(&"extra.txt"));
    }

    #[test]
    fn test_package_hand_missing_dir() {
        let result = package_hand("/nonexistent/path/to/hand");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_package_hand_missing_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "no hand.toml here").unwrap();
        let result = package_hand(dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing hand.toml"));
    }

    #[test]
    fn test_install_hand_from_archive() {
        // Create a hand package
        let src_dir = tempfile::tempdir().unwrap();
        let hand_toml = make_hand_toml("my_hand");
        std::fs::write(src_dir.path().join("hand.toml"), &hand_toml).unwrap();

        let archive = package_hand(src_dir.path().to_str().unwrap()).unwrap();

        // Install it
        let hands_dir = tempfile::tempdir().unwrap();
        let name = install_hand(&archive, hands_dir.path().to_str().unwrap()).unwrap();
        assert_eq!(name, "my_hand");

        // Verify files were written
        let installed_toml = hands_dir.path().join("my_hand").join("hand.toml");
        assert!(installed_toml.exists());
        let content = std::fs::read_to_string(installed_toml).unwrap();
        assert!(content.contains("name = \"my_hand\""));
    }

    #[test]
    fn test_install_hand_rejects_path_traversal() {
        let entries = vec![
            ArchiveEntry {
                name: "hand.toml".to_string(),
                data: make_hand_toml("ok_name").into_bytes(),
            },
            ArchiveEntry {
                name: "../evil.txt".to_string(),
                data: b"malicious".to_vec(),
            },
        ];
        let archive = pack_archive(&entries);
        let hands_dir = tempfile::tempdir().unwrap();
        let result = install_hand(&archive, hands_dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Suspicious"));
    }

    #[test]
    fn test_install_hand_rejects_missing_toml() {
        let entries = vec![ArchiveEntry {
            name: "readme.txt".to_string(),
            data: b"no hand.toml".to_vec(),
        }];
        let archive = pack_archive(&entries);
        let hands_dir = tempfile::tempdir().unwrap();
        let result = install_hand(&archive, hands_dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hand.toml"));
    }

    #[test]
    fn test_install_hand_rejects_invalid_name() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: b"name = \"bad name!\"".to_vec(),
        }];
        let archive = pack_archive(&entries);
        let hands_dir = tempfile::tempdir().unwrap();
        let result = install_hand(&archive, hands_dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid hand name"));
    }

    // -- Validate hand package tests --

    #[test]
    fn test_validate_hand_package_good() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: make_hand_toml("good_hand").into_bytes(),
        }];
        let archive = pack_archive(&entries);
        let pkg = validate_hand_package(&archive).unwrap();
        assert_eq!(pkg.name, "good_hand");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.author, "test");
        assert_eq!(pkg.tools_required, vec!["web_search", "file_write"]);
        assert_eq!(pkg.phases_count, 2);
        assert_eq!(pkg.category, "test");
    }

    #[test]
    fn test_validate_hand_package_missing_name() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: b"description = \"no name\"".to_vec(),
        }];
        let archive = pack_archive(&entries);
        let result = validate_hand_package(&archive);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_validate_hand_package_invalid_toml() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: b"this is not [valid toml".to_vec(),
        }];
        let archive = pack_archive(&entries);
        let result = validate_hand_package(&archive);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid TOML"));
    }

    #[test]
    fn test_validate_hand_package_defaults() {
        let entries = vec![ArchiveEntry {
            name: "hand.toml".to_string(),
            data: b"name = \"minimal\"".to_vec(),
        }];
        let archive = pack_archive(&entries);
        let pkg = validate_hand_package(&archive).unwrap();
        assert_eq!(pkg.name, "minimal");
        assert_eq!(pkg.description, "No description");
        assert_eq!(pkg.version, "0.1.0");
        assert_eq!(pkg.author, "unknown");
        assert!(pkg.tools_required.is_empty());
        assert_eq!(pkg.phases_count, 0);
    }

    // -- Index serialization --

    #[test]
    fn test_marketplace_index_serialize() {
        let idx = sample_index();
        let json = serde_json::to_string(&idx).unwrap();
        let parsed: MarketplaceIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), idx.len());
        assert_eq!(parsed.packages[0].name, idx.packages[0].name);
    }

    #[test]
    fn test_marketplace_default() {
        let idx = MarketplaceIndex::default();
        assert!(idx.is_empty());
    }

    // -- End-to-end: package -> validate -> install --

    #[test]
    fn test_end_to_end_package_validate_install() {
        // 1. Create hand directory
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("hand.toml"), make_hand_toml("e2e_hand")).unwrap();
        std::fs::write(src.path().join("prompts.txt"), "extra prompts").unwrap();

        // 2. Package
        let archive = package_hand(src.path().to_str().unwrap()).unwrap();

        // 3. Validate
        let pkg = validate_hand_package(&archive).unwrap();
        assert_eq!(pkg.name, "e2e_hand");

        // 4. Install
        let dest = tempfile::tempdir().unwrap();
        let installed = install_hand(&archive, dest.path().to_str().unwrap()).unwrap();
        assert_eq!(installed, "e2e_hand");

        // 5. Verify files
        assert!(dest.path().join("e2e_hand").join("hand.toml").exists());
        assert!(dest.path().join("e2e_hand").join("prompts.txt").exists());
    }
}
