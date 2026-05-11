use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Priority-ordered instruction files. Higher index = lower priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionSource {
    PhantomMd,
    ClaudeMd,
    AgentsMd,
}

impl InstructionSource {
    fn filename(&self) -> &'static str {
        match self {
            Self::PhantomMd => "PHANTOM.md",
            Self::ClaudeMd => "CLAUDE.md",
            Self::AgentsMd => "AGENTS.md",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::PhantomMd => "PHANTOM.md",
            Self::ClaudeMd => "CLAUDE.md",
            Self::AgentsMd => "AGENTS.md",
        }
    }
}

/// Detected project type with extracted metadata.
#[derive(Debug, Clone)]
pub enum ProjectType {
    Rust {
        name: String,
        version: String,
        description: Option<String>,
        dependencies: Vec<String>,
    },
    Node {
        name: String,
        description: Option<String>,
        scripts: Vec<String>,
        dependencies: Vec<String>,
    },
    Python {
        name: String,
        python_version: Option<String>,
    },
    Go {
        module: String,
    },
    Unknown,
}

impl ProjectType {
    fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Git state for the working tree.
#[derive(Debug, Clone, Default)]
pub struct GitContext {
    pub branch: Option<String>,
    pub last_commit: Option<String>,
    pub is_dirty: bool,
    pub uncommitted_files: usize,
}

/// Full project context, detected from the filesystem.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub cwd: PathBuf,

    /// The instruction file found (PHANTOM.md > CLAUDE.md > AGENTS.md).
    pub instruction_source: Option<InstructionSource>,
    pub instruction_content: Option<String>,

    /// First 100 lines of README.md (if present).
    pub readme_excerpt: Option<String>,

    /// Contents of .phantom/config.toml (if present).
    pub phantom_config: Option<String>,

    /// Detected project type.
    pub project_type: ProjectType,

    /// Git state.
    pub git: GitContext,
}

impl ProjectContext {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// Detect and load all project context starting from `cwd`, walking up to
    /// 3 parent directories.
    pub async fn detect(cwd: &Path) -> Self {
        let mut ctx = ProjectContext {
            cwd: cwd.to_path_buf(),
            instruction_source: None,
            instruction_content: None,
            readme_excerpt: None,
            phantom_config: None,
            project_type: ProjectType::Unknown,
            git: GitContext::default(),
        };

        // Collect candidate directories: cwd + up to 3 parents.
        let dirs: Vec<PathBuf> = {
            let mut v = vec![cwd.to_path_buf()];
            let mut cur = cwd.to_path_buf();
            for _ in 0..3 {
                match cur.parent() {
                    Some(p) if p != cur => {
                        cur = p.to_path_buf();
                        v.push(cur.clone());
                    }
                    _ => break,
                }
            }
            v
        };

        // 1. Instruction files (PHANTOM.md > CLAUDE.md > AGENTS.md) — use the
        //    nearest dir that has any of them, with priority order.
        'outer: for dir in &dirs {
            for source in &[
                InstructionSource::PhantomMd,
                InstructionSource::ClaudeMd,
                InstructionSource::AgentsMd,
            ] {
                let path = dir.join(source.filename());
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    let trimmed = content.trim().to_string();
                    if !trimmed.is_empty() {
                        tracing::info!(
                            "Loaded project instructions from {}",
                            path.display()
                        );
                        ctx.instruction_source = Some(source.clone());
                        ctx.instruction_content = Some(trimmed);
                        break 'outer;
                    }
                }
            }
        }

        // 2. README.md — first 100 lines from the nearest match.
        for dir in &dirs {
            let path = dir.join("README.md");
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if !content.trim().is_empty() {
                    let excerpt: String = content
                        .lines()
                        .take(100)
                        .collect::<Vec<_>>()
                        .join("\n");
                    ctx.readme_excerpt = Some(excerpt);
                    break;
                }
            }
        }

        // 3. .phantom/config.toml
        for dir in &dirs {
            let path = dir.join(".phantom").join("config.toml");
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if !content.trim().is_empty() {
                    ctx.phantom_config = Some(content.trim().to_string());
                    break;
                }
            }
        }

        // 4. Project type detection — search nearest dir first.
        for dir in &dirs {
            let pt = detect_project_type(dir).await;
            if !pt.is_unknown() {
                ctx.project_type = pt;
                break;
            }
        }

        // 5. Git context — run from cwd.
        ctx.git = load_git_context(cwd).await;

        ctx
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Returns `true` if a `PHANTOM.md` file was found and loaded.
    pub fn has_phantom_md(&self) -> bool {
        matches!(self.instruction_source, Some(InstructionSource::PhantomMd))
    }

    /// Returns the content of the highest-priority instruction file found
    /// (PHANTOM.md, CLAUDE.md, or AGENTS.md), if any.
    pub fn project_instructions(&self) -> Option<&str> {
        self.instruction_content.as_deref()
    }

    /// Full system-prompt context block.
    pub fn to_system_context(&self) -> String {
        let mut out = String::new();

        // Header line with git info
        let git_info = self.git_summary_inline();
        out.push_str("# Project Context\n");
        out.push_str(&format!(
            "Directory: {}",
            self.cwd.display()
        ));
        if !git_info.is_empty() {
            out.push_str(&format!(" ({})", git_info));
        }
        out.push('\n');

        // Last commit
        if let Some(commit) = &self.git.last_commit {
            out.push_str(&format!("Last commit: \"{}\"\n", commit));
        }

        // Project instructions
        if let (Some(src), Some(content)) =
            (&self.instruction_source, &self.instruction_content)
        {
            out.push('\n');
            out.push_str(&format!(
                "## Project Instructions (from {})\n",
                src.label()
            ));
            out.push_str(content);
            out.push('\n');
        }

        // Project type
        let type_str = self.project_type_summary();
        if !type_str.is_empty() {
            out.push('\n');
            out.push_str("## Project Type\n");
            out.push_str(&type_str);
            out.push('\n');
        }

        // README excerpt
        if let Some(readme) = &self.readme_excerpt {
            out.push('\n');
            out.push_str("## README (excerpt)\n");
            out.push_str(readme);
            out.push('\n');
        }

        // Phantom config
        if let Some(cfg) = &self.phantom_config {
            out.push('\n');
            out.push_str("## .phantom/config.toml\n```toml\n");
            out.push_str(cfg);
            out.push_str("\n```\n");
        }

        out
    }

    /// Short version of the context (under 200 chars).
    pub fn to_system_context_brief(&self) -> String {
        let dir = self.cwd.display().to_string();
        let git = self.git_summary_inline();
        let proj = self.project_type_oneliner();

        let mut parts = vec![dir];
        if !git.is_empty() {
            parts.push(git);
        }
        if !proj.is_empty() {
            parts.push(proj);
        }

        let full = parts.join(" | ");
        if full.len() > 200 {
            full[..200].to_string()
        } else {
            full
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn git_summary_inline(&self) -> String {
        let mut parts = Vec::new();
        if let Some(branch) = &self.git.branch {
            parts.push(format!("git: {}", branch));
        }
        if self.git.uncommitted_files > 0 {
            parts.push(format!("{} uncommitted files", self.git.uncommitted_files));
        }
        parts.join(", ")
    }

    fn project_type_summary(&self) -> String {
        match &self.project_type {
            ProjectType::Rust {
                name,
                version,
                description,
                dependencies,
            } => {
                let mut s = format!("Rust crate: {} v{}", name, version);
                if let Some(desc) = description {
                    s.push_str(&format!("\nDescription: {}", desc));
                }
                if !dependencies.is_empty() {
                    let shown = &dependencies[..dependencies.len().min(5)];
                    let rest = dependencies.len().saturating_sub(5);
                    if rest > 0 {
                        s.push_str(&format!(
                            "\nDependencies: {} (+{} more)",
                            shown.join(", "),
                            rest
                        ));
                    } else {
                        s.push_str(&format!("\nDependencies: {}", shown.join(", ")));
                    }
                }
                s
            }
            ProjectType::Node {
                name,
                description,
                scripts,
                dependencies,
            } => {
                let mut s = format!("Node.js package: {}", name);
                if let Some(desc) = description {
                    s.push_str(&format!("\nDescription: {}", desc));
                }
                if !scripts.is_empty() {
                    s.push_str(&format!("\nScripts: {}", scripts.join(", ")));
                }
                if !dependencies.is_empty() {
                    let shown = &dependencies[..dependencies.len().min(5)];
                    let rest = dependencies.len().saturating_sub(5);
                    if rest > 0 {
                        s.push_str(&format!(
                            "\nDependencies: {} (+{} more)",
                            shown.join(", "),
                            rest
                        ));
                    } else {
                        s.push_str(&format!("\nDependencies: {}", shown.join(", ")));
                    }
                }
                s
            }
            ProjectType::Python { name, python_version } => {
                let mut s = format!("Python project: {}", name);
                if let Some(pv) = python_version {
                    s.push_str(&format!("\nPython: {}", pv));
                }
                s
            }
            ProjectType::Go { module } => format!("Go module: {}", module),
            ProjectType::Unknown => String::new(),
        }
    }

    fn project_type_oneliner(&self) -> String {
        match &self.project_type {
            ProjectType::Rust { name, version, .. } => {
                format!("Rust: {} v{}", name, version)
            }
            ProjectType::Node { name, .. } => format!("Node: {}", name),
            ProjectType::Python { name, .. } => format!("Python: {}", name),
            ProjectType::Go { module } => format!("Go: {}", module),
            ProjectType::Unknown => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Project-type detection helpers
// ---------------------------------------------------------------------------

async fn detect_project_type(dir: &Path) -> ProjectType {
    // Rust — Cargo.toml
    let cargo_path = dir.join("Cargo.toml");
    if let Ok(content) = tokio::fs::read_to_string(&cargo_path).await {
        if let Some(pt) = parse_cargo_toml(&content) {
            return pt;
        }
    }

    // Node — package.json
    let pkg_path = dir.join("package.json");
    if let Ok(content) = tokio::fs::read_to_string(&pkg_path).await {
        if let Some(pt) = parse_package_json(&content) {
            return pt;
        }
    }

    // Python — pyproject.toml
    let pyproject_path = dir.join("pyproject.toml");
    if let Ok(content) = tokio::fs::read_to_string(&pyproject_path).await {
        if let Some(pt) = parse_pyproject_toml(&content) {
            return pt;
        }
    }

    // Python — setup.py (minimal: just look for name=)
    let setup_path = dir.join("setup.py");
    if let Ok(content) = tokio::fs::read_to_string(&setup_path).await {
        if let Some(pt) = parse_setup_py(&content) {
            return pt;
        }
    }

    // Go — go.mod
    let gomod_path = dir.join("go.mod");
    if let Ok(content) = tokio::fs::read_to_string(&gomod_path).await {
        if let Some(pt) = parse_go_mod(&content) {
            return pt;
        }
    }

    ProjectType::Unknown
}

fn parse_cargo_toml(content: &str) -> Option<ProjectType> {
    // Use the `toml` crate (already in Cargo.toml) via serde_json round-trip
    // is cumbersome; do a simple line-by-line parse instead to avoid pulling
    // in toml::Value (which *is* available — use it).
    let val: toml::Value = content.parse().ok()?;

    let pkg = val.get("package")?;
    let name = pkg.get("name")?.as_str()?.to_string();
    let version = pkg
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    let description = pkg
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut dependencies: Vec<String> = Vec::new();
    if let Some(deps) = val.get("dependencies").and_then(|v| v.as_table()) {
        for k in deps.keys() {
            dependencies.push(k.clone());
        }
    }
    // Also include dev-dependencies? Keep it simple — only runtime deps.

    Some(ProjectType::Rust {
        name,
        version,
        description,
        dependencies,
    })
}

fn parse_package_json(content: &str) -> Option<ProjectType> {
    let val: serde_json::Value = serde_json::from_str(content).ok()?;

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let description = val
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let scripts: Vec<String> = val
        .get("scripts")
        .and_then(|v| v.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let mut dependencies: Vec<String> = Vec::new();
    for key in &["dependencies", "devDependencies"] {
        if let Some(obj) = val.get(*key).and_then(|v| v.as_object()) {
            for k in obj.keys() {
                if !dependencies.contains(k) {
                    dependencies.push(k.clone());
                }
            }
        }
    }

    Some(ProjectType::Node {
        name,
        description,
        scripts,
        dependencies,
    })
}

fn parse_pyproject_toml(content: &str) -> Option<ProjectType> {
    let val: toml::Value = content.parse().ok()?;

    // PEP 517/518 style: [project]
    if let Some(project) = val.get("project") {
        let name = project.get("name")?.as_str()?.to_string();
        let python_version = project
            .get("requires-python")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Some(ProjectType::Python { name, python_version });
    }

    // Poetry style: [tool.poetry]
    if let Some(poetry) = val
        .get("tool")
        .and_then(|t| t.get("poetry"))
    {
        let name = poetry.get("name")?.as_str()?.to_string();
        let python_version = poetry
            .get("dependencies")
            .and_then(|d| d.get("python"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Some(ProjectType::Python { name, python_version });
    }

    None
}

fn parse_setup_py(content: &str) -> Option<ProjectType> {
    // Minimal heuristic: look for name="..." or name='...' in setup() call.
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") {
            // e.g.  name="myproject",  or  name='myproject'
            let after_eq = trimmed.splitn(2, '=').nth(1)?.trim();
            let name = after_eq
                .trim_matches(|c| c == '"' || c == '\'' || c == ',')
                .to_string();
            if !name.is_empty() {
                return Some(ProjectType::Python {
                    name,
                    python_version: None,
                });
            }
        }
    }
    None
}

fn parse_go_mod(content: &str) -> Option<ProjectType> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let module = rest.trim().to_string();
            if !module.is_empty() {
                return Some(ProjectType::Go { module });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Git context
// ---------------------------------------------------------------------------

async fn load_git_context(cwd: &Path) -> GitContext {
    let mut ctx = GitContext::default();

    // Branch name
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .await
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() && branch != "HEAD" {
                ctx.branch = Some(branch);
            }
        }
    }

    // Last commit message (first line only)
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["log", "-1", "--pretty=%s"])
        .current_dir(cwd)
        .output()
        .await
    {
        if output.status.success() {
            let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !msg.is_empty() {
                ctx.last_commit = Some(msg);
            }
        }
    }

    // Dirty / uncommitted files
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .await
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let count = text.lines().filter(|l| !l.trim().is_empty()).count();
            ctx.uncommitted_files = count;
            ctx.is_dirty = count > 0;
        }
    }

    ctx
}

// ---------------------------------------------------------------------------
// Legacy top-level functions (preserved for backward compatibility)
// ---------------------------------------------------------------------------

/// Walk up from `start_dir` looking for PHANTOM.md or .phantom-mesh/context.md.
/// Returns the content if found, None if not found.
pub async fn load_project_context(start_dir: &Path) -> Option<String> {
    let mut dir = start_dir.to_path_buf();

    loop {
        // Check for PHANTOM.md
        let candidate = dir.join("PHANTOM.md");
        if candidate.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&candidate).await {
                if !content.trim().is_empty() {
                    tracing::info!("Loaded project context from {}", candidate.display());
                    return Some(format!(
                        "## Project Context (from {})\n\n{}",
                        candidate.display(),
                        content.trim()
                    ));
                }
            }
        }

        // Check for .phantom-mesh/context.md
        let alt = dir.join(".phantom-mesh").join("context.md");
        if alt.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&alt).await {
                if !content.trim().is_empty() {
                    tracing::info!("Loaded project context from {}", alt.display());
                    return Some(format!(
                        "## Project Context (from {})\n\n{}",
                        alt.display(),
                        content.trim()
                    ));
                }
            }
        }

        // Walk up
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }

        // Stop at home dir or filesystem root
        if let Some(home) = dirs::home_dir() {
            if dir == home || dir == PathBuf::from("/") {
                break;
            }
        }
    }
    None
}

/// Load from a specific path (for when cwd is known from config).
pub async fn load_from_path(path: &Path) -> Option<String> {
    if let Ok(content) = tokio::fs::read_to_string(path).await {
        if !content.trim().is_empty() {
            return Some(format!("## Project Context\n\n{}", content.trim()));
        }
    }
    None
}

/// Load context from current working directory.
pub async fn load_cwd_context() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    load_project_context(&cwd).await
}
