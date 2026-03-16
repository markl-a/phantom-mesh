// SaaS project scaffolding tool
// Copies the SaaS API template from ~/.clawtex/templates/saas-api/
// and replaces placeholder values with actual product info.
// This ensures code_gen always starts with a working, compilable project.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tracing::info;

use super::{Tool, ToolResult};

pub struct ScaffoldSaasTool {
    template_dir: PathBuf,
    workspace_dir: PathBuf,
}

impl ScaffoldSaasTool {
    pub fn new(home: &str) -> Self {
        Self {
            template_dir: PathBuf::from(format!("{}/.clawtex/templates/saas-api", home)),
            workspace_dir: PathBuf::from(format!("{}/.clawtex/workspace", home)),
        }
    }

    fn slugify(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn copy_and_replace(
        src_dir: &Path,
        dst_dir: &Path,
        replacements: &[(String, String)],
    ) -> Result<usize> {
        let mut count = 0;
        if !src_dir.exists() {
            anyhow::bail!("Template directory not found: {}", src_dir.display());
        }
        std::fs::create_dir_all(dst_dir)?;

        for entry in walkdir(src_dir)? {
            let rel = entry.strip_prefix(src_dir)?;
            let dst_path = dst_dir.join(rel);

            if entry.is_dir() {
                std::fs::create_dir_all(&dst_path)?;
            } else {
                // Read file, apply replacements, write
                let content = std::fs::read_to_string(&entry)?;
                let mut replaced = content;
                for (from, to) in replacements {
                    replaced = replaced.replace(from, to);
                }
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dst_path, &replaced)?;
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Recursively collect all file/dir paths under `dir`.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    fn walk(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            paths.push(path.clone());
            if path.is_dir() {
                walk(&path, paths)?;
            }
        }
        Ok(())
    }
    walk(dir, &mut paths)?;
    Ok(paths)
}

#[async_trait]
impl Tool for ScaffoldSaasTool {
    fn name(&self) -> &str { "scaffold_saas" }

    fn description(&self) -> &str {
        "Scaffold a complete SaaS API project from template. \
         Creates a working Express+TypeScript+Stripe project with auth, billing, rate limiting, \
         and Docker support. Use this BEFORE customizing API endpoints."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "product_name": {
                    "type": "string",
                    "description": "Human-readable product name (e.g. 'AI Text Summarizer')"
                },
                "product_slug": {
                    "type": "string",
                    "description": "URL-safe slug (e.g. 'ai-text-summarizer'). Auto-generated from name if omitted."
                },
                "product_description": {
                    "type": "string",
                    "description": "One-line description of the product"
                }
            },
            "required": ["product_name"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let name = args.get("product_name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            return Ok(ToolResult { success: false, output: "Error: 'product_name' is required".into() });
        }

        let slug = args.get("product_slug")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Self::slugify(name));

        let description = args.get("product_description")
            .and_then(|v| v.as_str())
            .unwrap_or("SaaS API service");

        let project_dir = self.workspace_dir.join(&slug);
        if project_dir.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Project directory already exists: {}. Delete it first or use a different name.", project_dir.display()),
            });
        }

        let replacements = vec![
            ("{{PRODUCT_NAME}}".to_string(), name.to_string()),
            ("{{PRODUCT_SLUG}}".to_string(), slug.clone()),
            ("{{PRODUCT_DESCRIPTION}}".to_string(), description.to_string()),
        ];

        match Self::copy_and_replace(&self.template_dir, &project_dir, &replacements) {
            Ok(file_count) => {
                info!("scaffold_saas: created {} in {} ({} files)", slug, project_dir.display(), file_count);
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Project scaffolded successfully!\n\n\
                         Directory: {}\n\
                         Files created: {}\n\
                         Product: {} ({})\n\n\
                         Structure:\n\
                         {}/\n\
                         ├── package.json\n\
                         ├── tsconfig.json\n\
                         ├── Dockerfile\n\
                         ├── .env.example\n\
                         └── src/\n\
                             ├── index.ts (Express app)\n\
                             ├── db.ts (SQLite + API keys)\n\
                             ├── routes/\n\
                             │   ├── api.ts (your API endpoints — CUSTOMIZE THIS)\n\
                             │   ├── billing.ts (Stripe checkout + webhooks)\n\
                             │   └── keys.ts (API key management)\n\
                             ├── middleware/\n\
                             │   ├── auth.ts (API key auth)\n\
                             │   └── rateLimit.ts (tier-based limits)\n\
                             └── billing/\n\
                                 └── stripe.ts (Stripe integration)\n\n\
                         Next steps:\n\
                         1. Customize src/routes/api.ts with your business logic\n\
                         2. Run: cd {} && npm install && npm run build\n\
                         3. Test: npm test",
                        project_dir.display(), file_count, name, slug,
                        slug, slug
                    ),
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Scaffold failed: {}", e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_slugify() {
        assert_eq!(ScaffoldSaasTool::slugify("AI Text Summarizer"), "ai-text-summarizer");
        assert_eq!(ScaffoldSaasTool::slugify("My Cool API!"), "my-cool-api");
        assert_eq!(ScaffoldSaasTool::slugify("hello"), "hello");
    }

    #[tokio::test]
    async fn test_missing_name() {
        let tool = ScaffoldSaasTool::new("/tmp/nonexistent");
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("product_name"));
    }

    #[tokio::test]
    async fn test_template_not_found() {
        let tool = ScaffoldSaasTool::new("/tmp/nonexistent");
        let result = tool.execute(json!({"product_name": "Test"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Template directory not found") || result.output.contains("Scaffold failed"));
    }

    #[tokio::test]
    async fn test_scaffold_with_real_template() {
        // This test only works if the template exists
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let template_dir = format!("{}/.clawtex/templates/saas-api", home);
        if !std::path::Path::new(&template_dir).exists() {
            return; // Skip if template not installed
        }

        let tmp = TempDir::new().unwrap();
        let tool = ScaffoldSaasTool {
            template_dir: PathBuf::from(&template_dir),
            workspace_dir: tmp.path().to_path_buf(),
        };

        let result = tool.execute(json!({
            "product_name": "Test API",
            "product_description": "A test product"
        })).await.unwrap();

        assert!(result.success, "Scaffold failed: {}", result.output);
        assert!(result.output.contains("test-api"));
        assert!(tmp.path().join("test-api/package.json").exists());
        assert!(tmp.path().join("test-api/src/index.ts").exists());
        assert!(tmp.path().join("test-api/src/routes/api.ts").exists());
        assert!(tmp.path().join("test-api/src/billing/stripe.ts").exists());

        // Verify placeholder replacement
        let pkg = std::fs::read_to_string(tmp.path().join("test-api/package.json")).unwrap();
        assert!(pkg.contains("test-api"));
        assert!(!pkg.contains("{{PRODUCT_SLUG}}"));
    }
}
