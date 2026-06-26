//! SPEC-61 S1..S40 scenario catalog: parse `appendix/scenarios-S1-S40.csv` →
//! `Vec<Scenario>` + the SPEC-61 §19 meta-validators. (Bodies land in Task 7.)
//!
//! No `csv` crate dependency (would touch Cargo.lock, a hot file) — a small
//! RFC-4180-subset parser handles the quoted `given/when/then` fields, which
//! contain commas. `platforms`/`testIds` are `+`-delimited multi-value columns.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub id: String,
    pub category: Category,
    pub given: String,
    pub when: String,
    pub then: String,
    pub test_ids: Vec<String>,
    pub automation: Automation,
    pub platforms: Vec<Platform>,
    pub priority: Priority,
    /// DRIFT (DOCUMENTATION-CHARTER): SPEC-61 §8.2 specifies `manual_reason` as a
    /// hidden 10th CSV column (present only for `Manual` rows). Modelled here as an
    /// optional field so `T-scenarios-manual-justified` can assert it is non-empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_reason: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Onboarding,
    Capture,
    Coach,
    SkillBank,
    Cluster,
    Disaster,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Automation {
    Auto,
    Manual,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    IOS,
    Android,
    MacOS,
    Windows,
    Linux,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(thiserror::Error, Debug)]
pub enum ScenarioError {
    #[error("scenario CSV not found or unreadable: {0}")]
    Io(String),
    #[error("scenario CSV parse error: {0}")]
    Parse(String),
}

use std::path::{Path, PathBuf};

const CATALOG_REL: &str = "docs/superpowers/specs/v060-deep-spec/appendix/scenarios-S1-S40.csv";

/// Walk up from the current dir to find the repo root (the dir containing the
/// scenario CSV). Lets the shipped binary + tests locate the catalog without a
/// baked absolute path.
pub fn find_repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(CATALOG_REL).is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Parse the catalog from the canonical CSV path under the repo.
pub fn load_catalog() -> Result<Vec<Scenario>, ScenarioError> {
    let root = find_repo_root()
        .ok_or_else(|| ScenarioError::Io(format!("could not locate {CATALOG_REL} above CWD")))?;
    load_catalog_at(&root)
}

/// Parse the catalog rooted at an explicit repo root.
pub fn load_catalog_at(repo_root: &Path) -> Result<Vec<Scenario>, ScenarioError> {
    let path = repo_root.join(CATALOG_REL);
    let csv = std::fs::read_to_string(&path).map_err(|e| ScenarioError::Io(format!("{path:?}: {e}")))?;
    parse_catalog(&csv)
}

/// Split one CSV line into fields, honouring `"`-quoted fields (which may contain
/// commas) with `""` as the escaped-quote. RFC-4180 subset (no embedded newlines —
/// SPEC-61 §7.1 forbids them in given/when/then).
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => fields.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
    }
    fields.push(cur);
    fields
}

fn parse_category(s: &str) -> Result<Category, ScenarioError> {
    Ok(match s {
        "onboarding" => Category::Onboarding,
        "capture" => Category::Capture,
        "coach" => Category::Coach,
        "skillbank" => Category::SkillBank,
        "cluster" => Category::Cluster,
        "disaster" => Category::Disaster,
        other => return Err(ScenarioError::Parse(format!("unknown category: {other}"))),
    })
}

fn parse_platform(s: &str) -> Result<Platform, ScenarioError> {
    Ok(match s {
        "ios" => Platform::IOS,
        "android" => Platform::Android,
        "macos" => Platform::MacOS,
        "windows" => Platform::Windows,
        "linux" => Platform::Linux,
        other => return Err(ScenarioError::Parse(format!("unknown platform: {other}"))),
    })
}

fn parse_priority(s: &str) -> Result<Priority, ScenarioError> {
    Ok(match s {
        "Critical" => Priority::Critical,
        "High" => Priority::High,
        "Medium" => Priority::Medium,
        "Low" => Priority::Low,
        other => return Err(ScenarioError::Parse(format!("unknown priority: {other}"))),
    })
}

fn parse_automation(s: &str) -> Result<Automation, ScenarioError> {
    Ok(match s {
        "auto" => Automation::Auto,
        "manual" => Automation::Manual,
        other => return Err(ScenarioError::Parse(format!("unknown automation: {other}"))),
    })
}

/// Parse a catalog from a CSV string. Header order (SPEC-61 §7.1):
/// `id,category,platforms,priority,given,when,then,testIds,automation,manualReason`.
/// `platforms`/`testIds` are `+`-delimited; `manualReason` (10th, optional) is the
/// DRIFT-noted hidden column for `Manual` rows.
pub fn parse_catalog(csv: &str) -> Result<Vec<Scenario>, ScenarioError> {
    let mut scenarios = Vec::new();
    let mut lines = csv.lines();
    // header
    let header = lines
        .next()
        .ok_or_else(|| ScenarioError::Parse("empty CSV".into()))?;
    if !header.trim_end_matches('\r').starts_with("id,category,platforms,priority") {
        return Err(ScenarioError::Parse(format!("unexpected header: {header}")));
    }

    for (i, raw) in lines.enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let f = parse_csv_line(line);
        if f.len() < 9 {
            return Err(ScenarioError::Parse(format!(
                "row {} has {} fields, need >= 9",
                i + 2,
                f.len()
            )));
        }
        let platforms = f[2]
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_platform)
            .collect::<Result<Vec<_>, _>>()?;
        let test_ids: Vec<String> = f[7]
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let manual_reason = f
            .get(9)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        scenarios.push(Scenario {
            id: f[0].trim().to_string(),
            category: parse_category(f[1].trim())?,
            given: f[4].clone(),
            when: f[5].clone(),
            then: f[6].clone(),
            test_ids,
            automation: parse_automation(f[8].trim())?,
            platforms,
            priority: parse_priority(f[3].trim())?,
            manual_reason,
        });
    }
    Ok(scenarios)
}

// ── SPEC-61 §19 meta-validators ──────────────────────────────────────────────

/// `T-scenarios-catalog-complete`: exactly 40 rows, ids S1..S40 contiguous.
pub fn validate_count_contiguous(s: &[Scenario]) -> Result<(), String> {
    if s.len() != 40 {
        return Err(format!("expected 40 scenarios, found {}", s.len()));
    }
    for (idx, sc) in s.iter().enumerate() {
        let want = format!("S{}", idx + 1);
        if sc.id != want {
            return Err(format!("row {} id = {} (expected {want})", idx + 1, sc.id));
        }
    }
    Ok(())
}

/// `T-scenarios-auto-ratio`: Automation::Auto count / total.
pub fn auto_ratio(s: &[Scenario]) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let auto = s.iter().filter(|x| x.automation == Automation::Auto).count();
    auto as f64 / s.len() as f64
}

/// `T-scenarios-manual-justified`: every Manual row has a non-empty manual_reason.
pub fn validate_manual_justified(s: &[Scenario]) -> Result<(), String> {
    for sc in s.iter().filter(|x| x.automation == Automation::Manual) {
        if sc.manual_reason.as_deref().unwrap_or("").trim().is_empty() {
            return Err(format!("{} is manual but has no manual_reason", sc.id));
        }
    }
    Ok(())
}

/// `T-scenarios-test-id-resolve`: every testId greps to >= 1 SPEC-*.md under
/// v060-deep-spec/. Returns the unresolved ids (empty ⇒ all resolve).
pub fn validate_test_ids_resolve(s: &[Scenario], repo_root: &Path) -> Vec<String> {
    let spec_dir = repo_root.join("docs/superpowers/specs/v060-deep-spec");
    let mut corpus = String::new();
    collect_spec_md(&spec_dir, &mut corpus);

    let mut unresolved = Vec::new();
    for sc in s {
        for tid in &sc.test_ids {
            if !corpus.contains(tid.as_str()) {
                unresolved.push(format!("{}: {}", sc.id, tid));
            }
        }
    }
    unresolved
}

fn collect_spec_md(dir: &Path, out: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_spec_md(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
            if let Ok(t) = std::fs::read_to_string(&p) {
                out.push_str(&t);
                out.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
    }

    /// RED-first (SPEC-61 §19): catalog is exactly 40 contiguous S1..S40 rows.
    #[test]
    fn catalog_has_40_contiguous_scenarios() {
        let cat = load_catalog_at(&repo_root()).expect("load catalog");
        validate_count_contiguous(&cat).expect("40 contiguous S1..S40");
        assert_eq!(cat.len(), 40);
        assert_eq!(cat[0].id, "S1");
        assert_eq!(cat[39].id, "S40");
    }

    /// RED-first (SPEC-61 §19 G4): automation ratio >= 80% (actual: 36/40 = 90%).
    #[test]
    fn auto_ratio_at_least_80pct() {
        let cat = load_catalog_at(&repo_root()).expect("load catalog");
        let ratio = auto_ratio(&cat);
        assert!(ratio >= 0.80, "auto ratio {ratio} < 0.80");
        let manual = cat.iter().filter(|s| s.automation == Automation::Manual).count();
        assert_eq!(manual, 4, "exactly S6/S9/S35/S40 are manual");
    }

    #[test]
    fn manual_rows_are_justified() {
        let cat = load_catalog_at(&repo_root()).expect("load catalog");
        validate_manual_justified(&cat).expect("every manual row has a reason");
        // the 4 manual scenarios are exactly S6, S9, S35, S40.
        let manual_ids: Vec<&str> = cat
            .iter()
            .filter(|s| s.automation == Automation::Manual)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(manual_ids, vec!["S6", "S9", "S35", "S40"]);
    }

    #[test]
    fn priority_distribution_matches_spec() {
        let cat = load_catalog_at(&repo_root()).expect("load catalog");
        let count = |p: Priority| cat.iter().filter(|s| s.priority == p).count();
        assert_eq!(count(Priority::Critical), 14);
        assert_eq!(count(Priority::High), 22);
        assert_eq!(count(Priority::Medium), 4);
        assert_eq!(count(Priority::Low), 0);
    }

    #[test]
    fn parse_csv_line_handles_quoted_commas() {
        let f = parse_csv_line(r#"S1,onboarding,ios,Critical,"a, b","c","d",T-x+T-y,auto,"#);
        assert_eq!(f[4], "a, b");
        assert_eq!(f[7], "T-x+T-y");
        assert_eq!(f.len(), 10);
    }
}
