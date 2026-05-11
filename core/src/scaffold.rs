/// Detects the project type at `cwd` by checking for well-known manifest files.
///
/// Returns one of: `"Rust"`, `"Node.js"`, `"Python"`, `"Go"`, `"Unknown"`.
pub fn detect_project_type(cwd: &std::path::Path) -> &'static str {
    if cwd.join("Cargo.toml").exists() {
        "Rust"
    } else if cwd.join("package.json").exists() {
        "Node.js"
    } else if cwd.join("pyproject.toml").exists() || cwd.join("setup.py").exists() {
        "Python"
    } else if cwd.join("go.mod").exists() {
        "Go"
    } else {
        "Unknown"
    }
}

/// Analyzes the project at `cwd` and returns the content of a `PHANTOM.md` file.
///
/// This function is pure — it performs only read operations and returns a `String`.
/// The caller is responsible for writing the result to disk.
///
/// An async wrapper [`generate_phantom_md_async`] is also available for use in
/// async contexts (e.g. the `phantom init` CLI command).
pub fn generate_phantom_md(cwd: &std::path::Path) -> String {
    use std::fs;

    // ── 1. Project type ───────────────────────────────────────────────────
    let project_type = detect_project_type(cwd);

    // ── 2. Project name & description ────────────────────────────────────
    let (project_name, description) = extract_name_and_description(cwd, project_type);

    // ── 3. Key top-level directories ──────────────────────────────────────
    let skip_dirs = ["target", "node_modules", ".git", ".venv", "__pycache__"];
    let mut key_dirs: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(cwd) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !skip_dirs.contains(&name.as_str()) && !name.starts_with('.'))
            .collect();
        names.sort();
        key_dirs = names;
    }

    let dirs_bullet = if key_dirs.is_empty() {
        "- (none found)".to_string()
    } else {
        key_dirs.iter().map(|d| format!("- {}/", d)).collect::<Vec<_>>().join("\n")
    };

    // ── 4. Key source files (up to 30) ────────────────────────────────────
    let key_files = collect_key_files(cwd, 30);
    let key_files_bullet = if key_files.is_empty() {
        "- (none found)".to_string()
    } else {
        key_files
            .iter()
            .map(|(rel, label)| format!("- `{}` — {}", rel, label))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // ── 5. File-type counts ───────────────────────────────────────────────
    let (primary_language, file_counts) = detect_primary_language(cwd, project_type);

    let counts_summary = if file_counts.is_empty() {
        String::new()
    } else {
        let mut pairs: Vec<_> = file_counts.iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1));
        pairs
            .iter()
            .take(5)
            .map(|(ext, n)| format!(".{}: {}", ext, n))
            .collect::<Vec<_>>()
            .join(", ")
    };

    // ── 6. Build / test commands ──────────────────────────────────────────
    let (build_cmd, test_cmd, check_cmd) = build_test_commands(cwd, project_type);

    // ── 7. README excerpt ─────────────────────────────────────────────────
    let readme_context = read_readme_excerpt(cwd);

    // ── 8. Existing docs ──────────────────────────────────────────────────
    let mut existing_docs: Vec<String> = Vec::new();
    for doc in &["README.md", "ARCHITECTURE.md", "CONTRIBUTING.md", "CHANGELOG.md"] {
        if cwd.join(doc).exists() {
            existing_docs.push(format!("- `{}`", doc));
        }
    }
    let docs_section = if existing_docs.is_empty() {
        "- (none found)".to_string()
    } else {
        existing_docs.join("\n")
    };

    // ── 9. Assemble PHANTOM.md ────────────────────────────────────────────
    let counts_line = if counts_summary.is_empty() {
        String::new()
    } else {
        format!("\n\n**File counts:** {}", counts_summary)
    };

    let readme_section = if readme_context.is_empty() {
        String::new()
    } else {
        format!("\n\n## README Excerpt\n\n{}", readme_context)
    };

    format!(
        r#"# Project: {project_name}

## Overview

{description}

## Project Type

{project_type}

## Key Files
{key_files_bullet}

## Directory Structure
{dirs_bullet}{counts_line}

## Existing Docs
{docs_section}

## Build & Test

```bash
{build_cmd}     # build
{test_cmd}      # run tests
{check_cmd}     # type check / lint
```

## Agent Instructions

- Always run `{check_cmd}` after editing {primary_language} files
- Read files before editing them
- Create tests for new functionality
- Follow existing code style
- Prefer editing existing files over creating new ones
- Check git status before committing{readme_section}
"#,
        project_name = project_name,
        description = description,
        project_type = project_type,
        key_files_bullet = key_files_bullet,
        dirs_bullet = dirs_bullet,
        counts_line = counts_line,
        docs_section = docs_section,
        build_cmd = build_cmd,
        test_cmd = test_cmd,
        check_cmd = check_cmd,
        primary_language = primary_language,
        readme_section = readme_section,
    )
}

/// Async wrapper around [`generate_phantom_md`] for use in async contexts.
///
/// Internally delegates to the synchronous implementation via
/// [`tokio::task::spawn_blocking`] so disk I/O does not block the async runtime.
///
/// # Example
/// ```no_run
/// # async fn example() {
/// let content = phantom_mesh::scaffold::generate_phantom_md_async(
///     std::path::Path::new(".")
/// ).await;
/// tokio::fs::write("PHANTOM.md", content).await.unwrap();
/// # }
/// ```
pub async fn generate_phantom_md_async(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || generate_phantom_md(&cwd))
        .await
        .unwrap_or_default()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extracts `(name, description)` from the relevant manifest, falling back to
/// the directory name / a placeholder string.
fn extract_name_and_description(
    cwd: &std::path::Path,
    project_type: &str,
) -> (String, String) {
    use std::fs;

    let fallback_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    match project_type {
        "Rust" => {
            let content = fs::read_to_string(cwd.join("Cargo.toml")).unwrap_or_default();
            let name = extract_toml_field(&content, "name")
                .unwrap_or_else(|| fallback_name.clone());
            let desc = extract_toml_field(&content, "description")
                .unwrap_or_else(|| "TODO: Add project description".to_string());
            (name, desc)
        }
        "Node.js" => {
            let content = fs::read_to_string(cwd.join("package.json")).unwrap_or_default();
            let name = extract_json_string_field(&content, "name")
                .unwrap_or_else(|| fallback_name.clone());
            let desc = extract_json_string_field(&content, "description")
                .unwrap_or_else(|| "TODO: Add project description".to_string());
            (name, desc)
        }
        "Python" => {
            let pyproject = cwd.join("pyproject.toml");
            if pyproject.exists() {
                let content = fs::read_to_string(&pyproject).unwrap_or_default();
                let name = extract_toml_field(&content, "name")
                    .unwrap_or_else(|| fallback_name.clone());
                let desc = extract_toml_field(&content, "description")
                    .unwrap_or_else(|| "TODO: Add project description".to_string());
                return (name, desc);
            }
            (fallback_name, "TODO: Add project description".to_string())
        }
        "Go" => {
            let content = fs::read_to_string(cwd.join("go.mod")).unwrap_or_default();
            // First line of go.mod is typically: `module <name>`
            let name = content
                .lines()
                .next()
                .and_then(|l| l.strip_prefix("module "))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| fallback_name.clone());
            (name, "TODO: Add project description".to_string())
        }
        _ => (fallback_name, "TODO: Add project description".to_string()),
    }
}

/// Walk `src/` (or the project root) and collect up to `max` notable source
/// files together with a short label describing their role.
fn collect_key_files(cwd: &std::path::Path, max: usize) -> Vec<(String, &'static str)> {
    use std::fs;

    // Well-known entry points and their labels.
    let known: &[(&str, &str)] = &[
        ("src/main.rs",     "binary entry point"),
        ("src/lib.rs",      "library root"),
        ("src/bin",         "additional binaries"),
        ("main.go",         "Go entry point"),
        ("cmd/main.go",     "Go entry point"),
        ("index.ts",        "TypeScript entry point"),
        ("index.js",        "JavaScript entry point"),
        ("src/index.ts",    "TypeScript entry point"),
        ("src/index.js",    "JavaScript entry point"),
        ("main.py",         "Python entry point"),
        ("app.py",          "Python app entry"),
        ("pyproject.toml",  "Python project manifest"),
        ("Cargo.toml",      "Rust workspace/crate manifest"),
        ("package.json",    "Node.js package manifest"),
        ("go.mod",          "Go module manifest"),
        ("tsconfig.json",   "TypeScript configuration"),
        ("tests",           "test directory"),
        ("__tests__",       "JavaScript test directory"),
        ("spec",            "spec/test directory"),
        ("ARCHITECTURE.md", "architecture docs"),
    ];

    let mut result: Vec<(String, &'static str)> = Vec::new();

    for (rel, label) in known {
        if result.len() >= max {
            break;
        }
        if cwd.join(rel).exists() {
            result.push((rel.to_string(), label));
        }
    }

    // Also enumerate top-level source files in src/ up to the limit.
    let src_dir = cwd.join("src");
    if src_dir.is_dir() && result.len() < max {
        if let Ok(entries) = fs::read_dir(&src_dir) {
            let mut extras: Vec<String> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let already = result.iter().any(|(p, _)| p.ends_with(&name));
                    if already {
                        return None;
                    }
                    let ext = std::path::Path::new(&name)
                        .extension()?
                        .to_string_lossy()
                        .into_owned();
                    if matches!(ext.as_str(), "rs" | "ts" | "tsx" | "js" | "py" | "go") {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            extras.sort();
            for name in extras {
                if result.len() >= max {
                    break;
                }
                result.push((format!("src/{}", name), "source module"));
            }
        }
    }

    result
}

/// Counts source file extensions in `src/` (or the project root) and returns
/// `(primary_language_label, extension_count_map)`.
fn detect_primary_language(
    cwd: &std::path::Path,
    project_type: &str,
) -> (String, std::collections::HashMap<String, usize>) {
    use std::collections::HashMap;
    use std::fs;

    let search_dir = {
        let src = cwd.join("src");
        if src.is_dir() { src } else { cwd.to_path_buf() }
    };

    let mut counts: HashMap<String, usize> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&search_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(ext) = entry.path().extension() {
                    *counts.entry(ext.to_string_lossy().into_owned()).or_insert(0) += 1;
                }
            }
        }
    }

    let label = if let Some(top_ext) = counts
        .iter()
        .max_by_key(|(_, v)| *v)
        .map(|(k, _)| k.as_str())
    {
        match top_ext {
            "rs"   => "Rust",
            "ts" | "tsx" => "TypeScript",
            "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
            "py"   => "Python",
            "go"   => "Go",
            "java" => "Java",
            "cpp" | "cc" | "cxx" => "C++",
            "c"    => "C",
            "rb"   => "Ruby",
            "swift" => "Swift",
            "kt"   => "Kotlin",
            _      => "",
        }
    } else {
        ""
    };

    let label = if label.is_empty() {
        match project_type {
            "Rust"    => "Rust",
            "Node.js" => "JavaScript/TypeScript",
            "Python"  => "Python",
            "Go"      => "Go",
            _         => "Unknown",
        }
    } else {
        label
    };

    (label.to_string(), counts)
}

/// Returns `(build_command, test_command, check_command)` based on project type.
fn build_test_commands(
    cwd: &std::path::Path,
    project_type: &str,
) -> (String, String, String) {
    use std::fs;

    match project_type {
        "Rust" => (
            "cargo build".to_string(),
            "cargo test".to_string(),
            "cargo check".to_string(),
        ),
        "Node.js" => {
            let content = fs::read_to_string(cwd.join("package.json")).unwrap_or_default();
            let build = if content.contains("\"build\"") { "npm run build" } else { "npm install" };
            let test  = "npm test"; // both arms of a prior if/else were "npm test" — collapsed.
            let check = if content.contains("\"lint\"")  { "npm run lint" } else { "npx tsc --noEmit" };
            (build.to_string(), test.to_string(), check.to_string())
        }
        "Python" => (
            "python -m build".to_string(),
            "pytest".to_string(),
            "ruff check .".to_string(),
        ),
        "Go" => (
            "go build ./...".to_string(),
            "go test ./...".to_string(),
            "go vet ./...".to_string(),
        ),
        _ => (
            "make build".to_string(),
            "make test".to_string(),
            "make check".to_string(),
        ),
    }
}

/// Reads up to 500 characters from `README.md` at `cwd`, if it exists.
fn read_readme_excerpt(cwd: &std::path::Path) -> String {
    use std::fs;

    let readme_path = cwd.join("README.md");
    if !readme_path.exists() {
        return String::new();
    }
    match fs::read_to_string(&readme_path) {
        Ok(content) => {
            let limit = 500;
            if content.len() <= limit {
                content
            } else {
                let mut end = limit;
                while !content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…", &content[..end])
            }
        }
        Err(_) => String::new(),
    }
}

// ── Parsing helpers (no I/O) ─────────────────────────────────────────────────

/// Minimal TOML string-field extractor (no external crate).
/// Handles both `key = "value"` and `key = 'value'` under `[package]`.
fn extract_toml_field(content: &str, field: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(field) {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim();
                for q in &['"', '\''] {
                    if rest.starts_with(*q) {
                        if let Some(end) = rest[1..].find(*q) {
                            return Some(rest[1..1 + end].to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Minimal JSON string-field extractor for flat `"key": "value"` patterns.
fn extract_json_string_field(content: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\"", field);
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&needle) {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix(':') {
                let rest = rest.trim().trim_end_matches(',');
                if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
                    return Some(rest[1..rest.len() - 1].to_string());
                }
            }
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    // ── detect_project_type ───────────────────────────────────────────────

    #[test]
    fn detect_rust_project() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
        assert_eq!(detect_project_type(&path), "Rust");
    }

    #[test]
    fn detect_node_project() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("package.json"), "{\"name\": \"my-app\"}").unwrap();
        assert_eq!(detect_project_type(&path), "Node.js");
    }

    #[test]
    fn detect_python_project() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("pyproject.toml"), "").unwrap();
        assert_eq!(detect_project_type(&path), "Python");
    }

    #[test]
    fn detect_go_project() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("go.mod"), "module example.com/foo\n").unwrap();
        assert_eq!(detect_project_type(&path), "Go");
    }

    #[test]
    fn detect_unknown_project() {
        let (_dir, path) = tmp_dir();
        assert_eq!(detect_project_type(&path), "Unknown");
    }

    // ── generate_phantom_md ───────────────────────────────────────────────

    #[test]
    fn generate_phantom_md_rust() {
        let (_dir, path) = tmp_dir();
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"test-crate\"\ndescription = \"A test crate\"\n",
        )
        .unwrap();
        fs::create_dir(path.join("src")).unwrap();
        fs::write(path.join("src").join("main.rs"), "fn main() {}").unwrap();

        let md = generate_phantom_md(&path);
        assert!(md.contains("test-crate"), "should contain crate name");
        assert!(md.contains("A test crate"), "should contain description");
        assert!(md.contains("cargo build"), "should contain build command");
        assert!(md.contains("cargo test"), "should contain test command");
        assert!(md.contains("Rust"), "should mention Rust");
        assert!(md.contains("src/main.rs"), "should list entry point");
    }

    #[test]
    fn generate_phantom_md_node() {
        let (_dir, path) = tmp_dir();
        fs::write(
            path.join("package.json"),
            "{\n  \"name\": \"my-app\",\n  \"description\": \"A node app\",\n  \"scripts\": { \"build\": \"tsc\", \"test\": \"jest\" }\n}\n",
        )
        .unwrap();

        let md = generate_phantom_md(&path);
        assert!(md.contains("my-app"), "should contain package name");
        assert!(md.contains("A node app"), "should contain description");
        assert!(md.contains("npm run build"), "should contain build command");
        assert!(md.contains("npm test"), "should contain test command");
        assert!(md.contains("Node.js"), "should mention Node.js");
    }

    #[test]
    fn generate_phantom_md_go() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("go.mod"), "module example.com/myapp\n\ngo 1.21\n").unwrap();

        let md = generate_phantom_md(&path);
        assert!(md.contains("example.com/myapp"), "should contain module name");
        assert!(md.contains("go build ./..."), "should contain build command");
        assert!(md.contains("go test ./..."), "should contain test command");
        assert!(md.contains("Go"), "should mention Go");
    }

    #[test]
    fn generate_phantom_md_includes_readme() {
        let (_dir, path) = tmp_dir();
        fs::write(path.join("README.md"), "Hello from README").unwrap();
        let md = generate_phantom_md(&path);
        assert!(md.contains("Hello from README"));
    }

    #[test]
    fn generate_phantom_md_truncates_long_readme() {
        let (_dir, path) = tmp_dir();
        let long = "x".repeat(600);
        fs::write(path.join("README.md"), &long).unwrap();
        let md = generate_phantom_md(&path);
        assert!(md.contains('…'), "long README should be truncated with ellipsis");
    }

    #[test]
    fn generate_phantom_md_unknown_project() {
        let (_dir, path) = tmp_dir();
        let md = generate_phantom_md(&path);
        assert!(md.contains("# Project:"), "should have project header");
        assert!(md.contains("TODO: Add project description"), "should have placeholder description");
        assert!(md.contains("Unknown"), "should mention Unknown project type");
    }

    #[tokio::test]
    async fn generate_phantom_md_async_works() {
        let (_dir, path) = tmp_dir();
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"async-crate\"\ndescription = \"Async test\"\n",
        )
        .unwrap();

        let md = generate_phantom_md_async(&path).await;
        assert!(md.contains("async-crate"), "async wrapper should produce same output");
        assert!(md.contains("Rust"));
    }

    // ── Parsing helpers ───────────────────────────────────────────────────

    #[test]
    fn toml_field_extraction() {
        let content = "[package]\nname = \"phantom-mesh\"\ndescription = \"An agent mesh\"\n";
        assert_eq!(
            extract_toml_field(content, "name"),
            Some("phantom-mesh".to_string())
        );
        assert_eq!(
            extract_toml_field(content, "description"),
            Some("An agent mesh".to_string())
        );
        assert_eq!(extract_toml_field(content, "version"), None);
    }

    #[test]
    fn json_field_extraction() {
        let multi = "{\n  \"name\": \"my-pkg\",\n  \"description\": \"A package\"\n}";
        assert_eq!(
            extract_json_string_field(multi, "name"),
            Some("my-pkg".to_string())
        );
        assert_eq!(
            extract_json_string_field(multi, "description"),
            Some("A package".to_string())
        );
    }
}
