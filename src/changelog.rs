//! Automated changelog generation from git commits.
//!
//! Parses conventional commit messages (feat:, fix:, docs:, etc.),
//! groups them by category and version, and generates markdown changelogs.

use std::collections::HashMap;
use std::fmt;

/// Commit category derived from conventional commit prefixes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommitCategory {
    Breaking,
    Feature,
    Fix,
    Refactor,
    Docs,
    Test,
    Chore,
}

impl fmt::Display for CommitCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitCategory::Breaking => write!(f, "Breaking Changes"),
            CommitCategory::Feature => write!(f, "Features"),
            CommitCategory::Fix => write!(f, "Bug Fixes"),
            CommitCategory::Refactor => write!(f, "Refactoring"),
            CommitCategory::Docs => write!(f, "Documentation"),
            CommitCategory::Test => write!(f, "Tests"),
            CommitCategory::Chore => write!(f, "Chores"),
        }
    }
}

/// A single parsed changelog entry from a git commit.
#[derive(Debug, Clone)]
pub struct ChangelogEntry {
    pub commit_hash: String,
    pub date: String,
    pub category: CommitCategory,
    pub title: String,
    pub body: Option<String>,
    pub author: String,
}

/// A versioned collection of changelog entries.
#[derive(Debug, Clone)]
pub struct ChangelogVersion {
    pub version: String,
    pub date: String,
    pub entries: Vec<ChangelogEntry>,
}

/// Full changelog with versioned and unreleased sections.
#[derive(Debug, Clone)]
pub struct Changelog {
    pub versions: Vec<ChangelogVersion>,
    pub unreleased: Vec<ChangelogEntry>,
}

impl Changelog {
    /// Create an empty changelog.
    pub fn new() -> Self {
        Self {
            versions: Vec::new(),
            unreleased: Vec::new(),
        }
    }

    /// Add an unreleased entry.
    pub fn add_unreleased(&mut self, entry: ChangelogEntry) {
        self.unreleased.push(entry);
    }

    /// Add a version section.
    pub fn add_version(&mut self, version: ChangelogVersion) {
        self.versions.push(version);
    }

    /// Generate the full changelog as markdown.
    pub fn to_markdown(&self) -> String {
        let mut output = String::from("# Changelog\n\n");
        output.push_str("All notable changes to this project will be documented in this file.\n\n");

        if !self.unreleased.is_empty() {
            output.push_str("## [Unreleased]\n\n");
            output.push_str(&generate_grouped_entries(&self.unreleased));
            output.push('\n');
        }

        for version in &self.versions {
            output.push_str(&format_version_section(version));
            output.push('\n');
        }

        output
    }
}

impl Default for Changelog {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about a changelog.
#[derive(Debug, Clone)]
pub struct ChangelogStats {
    pub commit_count: usize,
    pub by_category: HashMap<CommitCategory, usize>,
    pub contributors: Vec<String>,
    pub date_range: Option<(String, String)>,
}

impl ChangelogStats {
    /// Compute stats from a slice of changelog entries.
    pub fn from_entries(entries: &[ChangelogEntry]) -> Self {
        let commit_count = entries.len();

        let mut by_category: HashMap<CommitCategory, usize> = HashMap::new();
        for entry in entries {
            *by_category.entry(entry.category.clone()).or_insert(0) += 1;
        }

        let mut contributors_set: Vec<String> = entries
            .iter()
            .map(|e| e.author.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        contributors_set.sort();

        let date_range = if entries.is_empty() {
            None
        } else {
            let mut dates: Vec<&str> = entries.iter().map(|e| e.date.as_str()).collect();
            dates.sort();
            Some((dates[0].to_string(), dates[dates.len() - 1].to_string()))
        };

        ChangelogStats {
            commit_count,
            by_category,
            contributors: contributors_set,
            date_range,
        }
    }
}

/// Categorize a commit message into a CommitCategory.
///
/// Checks for conventional commit prefixes: feat:, fix:, docs:, refactor:,
/// test:, chore:, BREAKING:. Falls back to Chore for unrecognized messages.
pub fn categorize_commit(message: &str) -> CommitCategory {
    let msg = message.trim();

    // Check for BREAKING prefix first (highest priority)
    if msg.starts_with("BREAKING:") || msg.starts_with("BREAKING CHANGE:") {
        return CommitCategory::Breaking;
    }

    // Check for breaking change indicated by exclamation mark before colon
    if let Some(colon_pos) = msg.find(':') {
        let prefix = &msg[..colon_pos];
        if prefix.ends_with('!') {
            return CommitCategory::Breaking;
        }
    }

    // Standard conventional commit prefixes
    let lower = msg.to_lowercase();
    if lower.starts_with("feat:") || lower.starts_with("feat(") {
        CommitCategory::Feature
    } else if lower.starts_with("fix:") || lower.starts_with("fix(") {
        CommitCategory::Fix
    } else if lower.starts_with("docs:") || lower.starts_with("docs(") {
        CommitCategory::Docs
    } else if lower.starts_with("refactor:") || lower.starts_with("refactor(") {
        CommitCategory::Refactor
    } else if lower.starts_with("test:") || lower.starts_with("test(") || lower.starts_with("tests:") {
        CommitCategory::Test
    } else if lower.starts_with("chore:") || lower.starts_with("chore(") {
        CommitCategory::Chore
    } else {
        CommitCategory::Chore
    }
}

/// Parse a conventional commit message into a ChangelogEntry.
///
/// Returns None if the message does not follow conventional commit format.
/// Recognizes: feat:, fix:, docs:, refactor:, test:, chore:, BREAKING:
///
/// Also handles scoped commits like feat(scope): description.
pub fn parse_conventional_commit(message: &str) -> Option<ChangelogEntry> {
    let msg = message.trim();
    if msg.is_empty() {
        return None;
    }

    // Split into first line and optional body
    let (first_line, body) = if let Some(pos) = msg.find('\n') {
        let first = &msg[..pos];
        let rest = msg[pos + 1..].trim();
        (first, if rest.is_empty() { None } else { Some(rest.to_string()) })
    } else {
        (msg, None)
    };

    // Check for BREAKING prefix
    if first_line.starts_with("BREAKING:") {
        let title = first_line["BREAKING:".len()..].trim().to_string();
        if title.is_empty() {
            return None;
        }
        return Some(ChangelogEntry {
            commit_hash: String::new(),
            date: String::new(),
            category: CommitCategory::Breaking,
            title,
            body,
            author: String::new(),
        });
    }

    if first_line.starts_with("BREAKING CHANGE:") {
        let title = first_line["BREAKING CHANGE:".len()..].trim().to_string();
        if title.is_empty() {
            return None;
        }
        return Some(ChangelogEntry {
            commit_hash: String::new(),
            date: String::new(),
            category: CommitCategory::Breaking,
            title,
            body,
            author: String::new(),
        });
    }

    // Match pattern: type[(scope)][!]: description
    let colon_pos = first_line.find(':')?;
    let prefix = &first_line[..colon_pos];
    let description = first_line[colon_pos + 1..].trim();

    if description.is_empty() {
        return None;
    }

    // Extract the type keyword (before optional scope and !)
    let type_str = if let Some(paren_pos) = prefix.find('(') {
        &prefix[..paren_pos]
    } else if prefix.ends_with('!') {
        &prefix[..prefix.len() - 1]
    } else {
        prefix
    };

    let is_breaking = prefix.ends_with('!');

    let category = if is_breaking {
        CommitCategory::Breaking
    } else {
        match type_str.to_lowercase().as_str() {
            "feat" => CommitCategory::Feature,
            "fix" => CommitCategory::Fix,
            "docs" => CommitCategory::Docs,
            "refactor" => CommitCategory::Refactor,
            "test" | "tests" => CommitCategory::Test,
            "chore" => CommitCategory::Chore,
            _ => return None, // Not a recognized conventional commit
        }
    };

    Some(ChangelogEntry {
        commit_hash: String::new(),
        date: String::new(),
        category,
        title: description.to_string(),
        body,
        author: String::new(),
    })
}

/// Group entries by category and render as markdown sections.
fn generate_grouped_entries(entries: &[ChangelogEntry]) -> String {
    // Use BTreeMap-like sorted order via sorted keys
    let mut groups: HashMap<CommitCategory, Vec<&ChangelogEntry>> = HashMap::new();
    for entry in entries {
        groups.entry(entry.category.clone()).or_default().push(entry);
    }

    let mut output = String::new();

    // Render in a consistent category order
    let category_order = [
        CommitCategory::Breaking,
        CommitCategory::Feature,
        CommitCategory::Fix,
        CommitCategory::Refactor,
        CommitCategory::Docs,
        CommitCategory::Test,
        CommitCategory::Chore,
    ];

    for cat in &category_order {
        if let Some(cat_entries) = groups.get(cat) {
            output.push_str(&format!("### {}\n\n", cat));
            for entry in cat_entries {
                let hash_suffix = if entry.commit_hash.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", &entry.commit_hash)
                };
                output.push_str(&format!("- {}{}\n", entry.title, hash_suffix));
            }
            output.push('\n');
        }
    }

    output
}

/// Generate a changelog in markdown format grouped by category.
pub fn generate_changelog(entries: &[ChangelogEntry]) -> String {
    if entries.is_empty() {
        return "# Changelog\n\nNo changes recorded.\n".to_string();
    }

    let mut output = String::from("# Changelog\n\n");
    output.push_str(&generate_grouped_entries(entries));
    output
}

/// Format a single version section as markdown.
pub fn format_version_section(version: &ChangelogVersion) -> String {
    let mut output = format!("## [{}] - {}\n\n", version.version, version.date);
    if version.entries.is_empty() {
        output.push_str("No changes in this release.\n\n");
    } else {
        output.push_str(&generate_grouped_entries(&version.entries));
    }
    output
}

/// Generate release notes for a range of commits between two references.
///
/// The `from` and `to` parameters are git references (tags, branches, SHAs).
/// The entries should already be filtered to only include commits in that range.
pub fn generate_release_notes(from: &str, to: &str, entries: &[ChangelogEntry]) -> String {
    let mut output = format!("# Release Notes: {} -> {}\n\n", from, to);

    if entries.is_empty() {
        output.push_str("No notable changes in this release.\n");
        return output;
    }

    let stats = ChangelogStats::from_entries(entries);
    output.push_str(&format!(
        "**{}** commits from **{}** contributor(s)\n\n",
        stats.commit_count,
        stats.contributors.len()
    ));

    if let Some((start, end)) = &stats.date_range {
        output.push_str(&format!("Date range: {} to {}\n\n", start, end));
    }

    output.push_str(&generate_grouped_entries(entries));

    // Summary section
    output.push_str("### Summary\n\n");
    let mut category_order: Vec<_> = stats.by_category.iter().collect();
    category_order.sort_by_key(|(cat, _)| format!("{}", cat));
    for (cat, count) in &category_order {
        output.push_str(&format!("- {}: {}\n", cat, count));
    }
    output.push('\n');

    if !stats.contributors.is_empty() {
        output.push_str("### Contributors\n\n");
        for contributor in &stats.contributors {
            output.push_str(&format!("- {}\n", contributor));
        }
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(category: CommitCategory, title: &str, hash: &str) -> ChangelogEntry {
        ChangelogEntry {
            commit_hash: hash.to_string(),
            date: "2026-03-18".to_string(),
            category,
            title: title.to_string(),
            body: None,
            author: "dev".to_string(),
        }
    }

    // --- categorize_commit tests ---

    #[test]
    fn test_categorize_feat() {
        assert_eq!(categorize_commit("feat: add budget system"), CommitCategory::Feature);
    }

    #[test]
    fn test_categorize_fix() {
        assert_eq!(categorize_commit("fix: shell tests on macOS"), CommitCategory::Fix);
    }

    #[test]
    fn test_categorize_docs() {
        assert_eq!(categorize_commit("docs: update README"), CommitCategory::Docs);
    }

    #[test]
    fn test_categorize_refactor() {
        assert_eq!(categorize_commit("refactor: extract helper fn"), CommitCategory::Refactor);
    }

    #[test]
    fn test_categorize_test() {
        assert_eq!(categorize_commit("test: add integration tests"), CommitCategory::Test);
    }

    #[test]
    fn test_categorize_chore() {
        assert_eq!(categorize_commit("chore: bump dependencies"), CommitCategory::Chore);
    }

    #[test]
    fn test_categorize_breaking() {
        assert_eq!(categorize_commit("BREAKING: remove old API"), CommitCategory::Breaking);
    }

    #[test]
    fn test_categorize_breaking_change() {
        assert_eq!(categorize_commit("BREAKING CHANGE: new config format"), CommitCategory::Breaking);
    }

    #[test]
    fn test_categorize_breaking_with_exclamation() {
        assert_eq!(categorize_commit("feat!: redesign API"), CommitCategory::Breaking);
    }

    #[test]
    fn test_categorize_unknown_falls_to_chore() {
        assert_eq!(categorize_commit("update something random"), CommitCategory::Chore);
    }

    #[test]
    fn test_categorize_scoped_feat() {
        assert_eq!(categorize_commit("feat(cluster): add batch dispatch"), CommitCategory::Feature);
    }

    #[test]
    fn test_categorize_tests_plural() {
        assert_eq!(categorize_commit("tests: add more coverage"), CommitCategory::Test);
    }

    // --- parse_conventional_commit tests ---

    #[test]
    fn test_parse_feat_commit() {
        let entry = parse_conventional_commit("feat: add budget downgrade system").unwrap();
        assert_eq!(entry.category, CommitCategory::Feature);
        assert_eq!(entry.title, "add budget downgrade system");
        assert!(entry.body.is_none());
    }

    #[test]
    fn test_parse_fix_commit() {
        let entry = parse_conventional_commit("fix: shell tests on macOS").unwrap();
        assert_eq!(entry.category, CommitCategory::Fix);
        assert_eq!(entry.title, "shell tests on macOS");
    }

    #[test]
    fn test_parse_breaking_commit() {
        let entry = parse_conventional_commit("BREAKING: remove legacy provider API").unwrap();
        assert_eq!(entry.category, CommitCategory::Breaking);
        assert_eq!(entry.title, "remove legacy provider API");
    }

    #[test]
    fn test_parse_breaking_change_commit() {
        let entry = parse_conventional_commit("BREAKING CHANGE: new config schema").unwrap();
        assert_eq!(entry.category, CommitCategory::Breaking);
        assert_eq!(entry.title, "new config schema");
    }

    #[test]
    fn test_parse_scoped_commit() {
        let entry = parse_conventional_commit("feat(tools): add vision tool").unwrap();
        assert_eq!(entry.category, CommitCategory::Feature);
        assert_eq!(entry.title, "add vision tool");
    }

    #[test]
    fn test_parse_commit_with_body() {
        let msg = "fix: resolve UTF-8 truncation panic\n\nThe shell output was being truncated at byte boundaries\ninstead of character boundaries.";
        let entry = parse_conventional_commit(msg).unwrap();
        assert_eq!(entry.category, CommitCategory::Fix);
        assert_eq!(entry.title, "resolve UTF-8 truncation panic");
        assert!(entry.body.is_some());
        assert!(entry.body.unwrap().contains("byte boundaries"));
    }

    #[test]
    fn test_parse_empty_message_returns_none() {
        assert!(parse_conventional_commit("").is_none());
    }

    #[test]
    fn test_parse_non_conventional_returns_none() {
        assert!(parse_conventional_commit("just a random commit message").is_none());
    }

    #[test]
    fn test_parse_empty_description_returns_none() {
        assert!(parse_conventional_commit("feat:").is_none());
        assert!(parse_conventional_commit("fix: ").is_none());
    }

    #[test]
    fn test_parse_breaking_exclamation_commit() {
        let entry = parse_conventional_commit("refactor!: rewrite provider trait").unwrap();
        assert_eq!(entry.category, CommitCategory::Breaking);
        assert_eq!(entry.title, "rewrite provider trait");
    }

    #[test]
    fn test_parse_unknown_prefix_returns_none() {
        assert!(parse_conventional_commit("banana: something").is_none());
    }

    // --- generate_changelog tests ---

    #[test]
    fn test_generate_changelog_empty() {
        let result = generate_changelog(&[]);
        assert!(result.contains("No changes recorded"));
    }

    #[test]
    fn test_generate_changelog_grouped_by_category() {
        let entries = vec![
            make_entry(CommitCategory::Feature, "add budget system", "abc1234"),
            make_entry(CommitCategory::Fix, "fix shell tests", "def5678"),
            make_entry(CommitCategory::Feature, "add cluster dispatch", "ghi9012"),
        ];
        let result = generate_changelog(&entries);
        assert!(result.contains("### Features"));
        assert!(result.contains("### Bug Fixes"));
        assert!(result.contains("- add budget system (abc1234)"));
        assert!(result.contains("- fix shell tests (def5678)"));
        assert!(result.contains("- add cluster dispatch (ghi9012)"));
    }

    #[test]
    fn test_generate_changelog_category_order() {
        let entries = vec![
            make_entry(CommitCategory::Chore, "bump deps", "aaa"),
            make_entry(CommitCategory::Feature, "new feature", "bbb"),
            make_entry(CommitCategory::Breaking, "remove old API", "ccc"),
        ];
        let result = generate_changelog(&entries);
        let breaking_pos = result.find("### Breaking Changes").unwrap();
        let feature_pos = result.find("### Features").unwrap();
        let chore_pos = result.find("### Chores").unwrap();
        // Breaking should come first, then Features, then Chores
        assert!(breaking_pos < feature_pos);
        assert!(feature_pos < chore_pos);
    }

    // --- format_version_section tests ---

    #[test]
    fn test_format_version_section_basic() {
        let version = ChangelogVersion {
            version: "0.2.0".to_string(),
            date: "2026-03-18".to_string(),
            entries: vec![
                make_entry(CommitCategory::Feature, "add budget downgrade system", "abc1234"),
                make_entry(CommitCategory::Fix, "fix shell tests on macOS", "def5678"),
            ],
        };
        let result = format_version_section(&version);
        assert!(result.contains("## [0.2.0] - 2026-03-18"));
        assert!(result.contains("### Features"));
        assert!(result.contains("- add budget downgrade system (abc1234)"));
        assert!(result.contains("### Bug Fixes"));
        assert!(result.contains("- fix shell tests on macOS (def5678)"));
    }

    #[test]
    fn test_format_version_section_empty() {
        let version = ChangelogVersion {
            version: "0.1.0".to_string(),
            date: "2026-01-01".to_string(),
            entries: vec![],
        };
        let result = format_version_section(&version);
        assert!(result.contains("No changes in this release"));
    }

    // --- generate_release_notes tests ---

    #[test]
    fn test_release_notes_empty() {
        let result = generate_release_notes("v0.1.0", "v0.2.0", &[]);
        assert!(result.contains("Release Notes: v0.1.0 -> v0.2.0"));
        assert!(result.contains("No notable changes"));
    }

    #[test]
    fn test_release_notes_with_entries() {
        let entries = vec![
            make_entry(CommitCategory::Feature, "add new tool", "abc"),
            make_entry(CommitCategory::Fix, "fix crash", "def"),
        ];
        let result = generate_release_notes("v0.1.0", "v0.2.0", &entries);
        assert!(result.contains("Release Notes: v0.1.0 -> v0.2.0"));
        assert!(result.contains("**2** commits"));
        assert!(result.contains("**1** contributor(s)"));
        assert!(result.contains("### Features"));
        assert!(result.contains("### Bug Fixes"));
        assert!(result.contains("### Summary"));
        assert!(result.contains("### Contributors"));
        assert!(result.contains("- dev"));
    }

    #[test]
    fn test_release_notes_multiple_contributors() {
        let mut e1 = make_entry(CommitCategory::Feature, "add X", "aaa");
        e1.author = "alice".to_string();
        let mut e2 = make_entry(CommitCategory::Fix, "fix Y", "bbb");
        e2.author = "bob".to_string();
        let mut e3 = make_entry(CommitCategory::Docs, "update docs", "ccc");
        e3.author = "alice".to_string();

        let result = generate_release_notes("v1.0", "v1.1", &[e1, e2, e3]);
        assert!(result.contains("**3** commits"));
        assert!(result.contains("**2** contributor(s)"));
        assert!(result.contains("- alice"));
        assert!(result.contains("- bob"));
    }

    // --- ChangelogStats tests ---

    #[test]
    fn test_stats_empty() {
        let stats = ChangelogStats::from_entries(&[]);
        assert_eq!(stats.commit_count, 0);
        assert!(stats.by_category.is_empty());
        assert!(stats.contributors.is_empty());
        assert!(stats.date_range.is_none());
    }

    #[test]
    fn test_stats_counts() {
        let entries = vec![
            make_entry(CommitCategory::Feature, "a", "1"),
            make_entry(CommitCategory::Feature, "b", "2"),
            make_entry(CommitCategory::Fix, "c", "3"),
        ];
        let stats = ChangelogStats::from_entries(&entries);
        assert_eq!(stats.commit_count, 3);
        assert_eq!(*stats.by_category.get(&CommitCategory::Feature).unwrap(), 2);
        assert_eq!(*stats.by_category.get(&CommitCategory::Fix).unwrap(), 1);
    }

    #[test]
    fn test_stats_date_range() {
        let mut e1 = make_entry(CommitCategory::Feature, "a", "1");
        e1.date = "2026-03-15".to_string();
        let mut e2 = make_entry(CommitCategory::Fix, "b", "2");
        e2.date = "2026-03-18".to_string();
        let mut e3 = make_entry(CommitCategory::Docs, "c", "3");
        e3.date = "2026-03-16".to_string();

        let stats = ChangelogStats::from_entries(&[e1, e2, e3]);
        let (start, end) = stats.date_range.unwrap();
        assert_eq!(start, "2026-03-15");
        assert_eq!(end, "2026-03-18");
    }

    #[test]
    fn test_stats_contributors_deduped() {
        let mut e1 = make_entry(CommitCategory::Feature, "a", "1");
        e1.author = "alice".to_string();
        let mut e2 = make_entry(CommitCategory::Fix, "b", "2");
        e2.author = "alice".to_string();
        let mut e3 = make_entry(CommitCategory::Docs, "c", "3");
        e3.author = "bob".to_string();

        let stats = ChangelogStats::from_entries(&[e1, e2, e3]);
        assert_eq!(stats.contributors.len(), 2);
        assert!(stats.contributors.contains(&"alice".to_string()));
        assert!(stats.contributors.contains(&"bob".to_string()));
    }

    // --- Changelog struct tests ---

    #[test]
    fn test_changelog_new() {
        let cl = Changelog::new();
        assert!(cl.versions.is_empty());
        assert!(cl.unreleased.is_empty());
    }

    #[test]
    fn test_changelog_add_unreleased() {
        let mut cl = Changelog::new();
        cl.add_unreleased(make_entry(CommitCategory::Feature, "new thing", "abc"));
        assert_eq!(cl.unreleased.len(), 1);
    }

    #[test]
    fn test_changelog_to_markdown_unreleased() {
        let mut cl = Changelog::new();
        cl.add_unreleased(make_entry(CommitCategory::Feature, "new feature", "abc"));
        cl.add_unreleased(make_entry(CommitCategory::Fix, "fix bug", "def"));
        let md = cl.to_markdown();
        assert!(md.contains("# Changelog"));
        assert!(md.contains("## [Unreleased]"));
        assert!(md.contains("### Features"));
        assert!(md.contains("- new feature (abc)"));
        assert!(md.contains("### Bug Fixes"));
        assert!(md.contains("- fix bug (def)"));
    }

    #[test]
    fn test_changelog_to_markdown_with_versions() {
        let mut cl = Changelog::new();
        cl.add_version(ChangelogVersion {
            version: "0.2.0".to_string(),
            date: "2026-03-18".to_string(),
            entries: vec![make_entry(CommitCategory::Feature, "add X", "aaa")],
        });
        cl.add_version(ChangelogVersion {
            version: "0.1.0".to_string(),
            date: "2026-03-01".to_string(),
            entries: vec![make_entry(CommitCategory::Chore, "initial", "bbb")],
        });
        let md = cl.to_markdown();
        assert!(md.contains("## [0.2.0] - 2026-03-18"));
        assert!(md.contains("## [0.1.0] - 2026-03-01"));
    }

    #[test]
    fn test_changelog_default() {
        let cl = Changelog::default();
        assert!(cl.versions.is_empty());
        assert!(cl.unreleased.is_empty());
    }

    // --- CommitCategory display tests ---

    #[test]
    fn test_category_display() {
        assert_eq!(format!("{}", CommitCategory::Breaking), "Breaking Changes");
        assert_eq!(format!("{}", CommitCategory::Feature), "Features");
        assert_eq!(format!("{}", CommitCategory::Fix), "Bug Fixes");
        assert_eq!(format!("{}", CommitCategory::Refactor), "Refactoring");
        assert_eq!(format!("{}", CommitCategory::Docs), "Documentation");
        assert_eq!(format!("{}", CommitCategory::Test), "Tests");
        assert_eq!(format!("{}", CommitCategory::Chore), "Chores");
    }

    // --- Edge case tests ---

    #[test]
    fn test_parse_whitespace_only() {
        assert!(parse_conventional_commit("   ").is_none());
    }

    #[test]
    fn test_generate_changelog_single_category() {
        let entries = vec![
            make_entry(CommitCategory::Fix, "fix A", "111"),
            make_entry(CommitCategory::Fix, "fix B", "222"),
        ];
        let result = generate_changelog(&entries);
        assert!(result.contains("### Bug Fixes"));
        assert!(!result.contains("### Features"));
        assert!(result.contains("- fix A (111)"));
        assert!(result.contains("- fix B (222)"));
    }

    #[test]
    fn test_entry_without_hash() {
        let entry = make_entry(CommitCategory::Feature, "no hash feature", "");
        let entries = vec![entry];
        let result = generate_changelog(&entries);
        // Should not have trailing parentheses for empty hash
        assert!(result.contains("- no hash feature\n"));
        assert!(!result.contains("()"));
    }

    #[test]
    fn test_parse_docs_commit() {
        let entry = parse_conventional_commit("docs: add API reference").unwrap();
        assert_eq!(entry.category, CommitCategory::Docs);
        assert_eq!(entry.title, "add API reference");
    }

    #[test]
    fn test_parse_refactor_commit() {
        let entry = parse_conventional_commit("refactor: simplify router logic").unwrap();
        assert_eq!(entry.category, CommitCategory::Refactor);
        assert_eq!(entry.title, "simplify router logic");
    }

    #[test]
    fn test_parse_chore_commit() {
        let entry = parse_conventional_commit("chore: update CI config").unwrap();
        assert_eq!(entry.category, CommitCategory::Chore);
        assert_eq!(entry.title, "update CI config");
    }

    #[test]
    fn test_parse_test_commit() {
        let entry = parse_conventional_commit("test: add integration tests for cluster").unwrap();
        assert_eq!(entry.category, CommitCategory::Test);
        assert_eq!(entry.title, "add integration tests for cluster");
    }
}
