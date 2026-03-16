//! Blog publish tool — creates MDX posts, updates index.ts, and git pushes
//! to auto-deploy via Vercel. Designed for the markl-ai.space Next.js blog.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

/// Blog publish configuration (from agents.toml [blog] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BlogConfig {
    /// Path to the blog git repo
    #[serde(default)]
    pub repo_path: String,
    /// Default author name
    #[serde(default = "default_author")]
    pub author: String,
    /// Git remote name
    #[serde(default = "default_remote")]
    pub remote: String,
    /// Git branch to push to
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_author() -> String { "Mark Light".to_string() }
fn default_remote() -> String { "origin".to_string() }
fn default_branch() -> String { "main".to_string() }

impl Default for BlogConfig {
    fn default() -> Self {
        Self {
            repo_path: String::new(),
            author: default_author(),
            remote: default_remote(),
            branch: default_branch(),
        }
    }
}

pub struct BlogPublishTool {
    config: BlogConfig,
}

impl BlogPublishTool {
    pub fn new(config: BlogConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for BlogPublishTool {
    fn name(&self) -> &str {
        "blog_publish"
    }

    fn description(&self) -> &str {
        "Publish a blog post to markl-ai.space. Creates MDX file, updates blog index, \
         and git pushes for auto-deploy. Args: title, titleEn, content, contentEn (optional), \
         excerpt, excerptEn, category (AI/ML|Tutorial|Insights|News|Dev Log), tags, \
         readTime, icon (Brain|Code|Sparkles|Lightbulb|Zap|TestTube|Server|Wrench|Repeat), \
         color (Tailwind gradient), featured (bool)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Chinese title for the blog post"
                },
                "titleEn": {
                    "type": "string",
                    "description": "English title for the blog post"
                },
                "content": {
                    "type": "string",
                    "description": "Blog post content in Markdown (Chinese)"
                },
                "contentEn": {
                    "type": "string",
                    "description": "Optional English content. If provided, separated by ---en--- in MDX"
                },
                "excerpt": {
                    "type": "string",
                    "description": "Chinese excerpt/summary for list view"
                },
                "excerptEn": {
                    "type": "string",
                    "description": "English excerpt/summary for list view"
                },
                "category": {
                    "type": "string",
                    "enum": ["AI/ML", "Tutorial", "Insights", "News", "Dev Log"],
                    "description": "Blog category"
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Array of tag strings, e.g. [\"LLM\", \"Tutorial\"]"
                },
                "readTime": {
                    "type": "string",
                    "description": "Estimated reading time, e.g. '8 min'"
                },
                "icon": {
                    "type": "string",
                    "enum": ["Brain", "Code", "Sparkles", "Lightbulb", "Zap", "TestTube", "Server", "Wrench", "Repeat"],
                    "description": "Lucide icon name for the post card"
                },
                "color": {
                    "type": "string",
                    "description": "Tailwind gradient classes, e.g. 'from-purple-500 to-pink-500'"
                },
                "featured": {
                    "type": "boolean",
                    "description": "Whether to feature this post (default: false)"
                },
                "slug": {
                    "type": "string",
                    "description": "Optional URL slug. If not provided, auto-generated from titleEn"
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "If true, create files but don't git push (default: false)"
                }
            },
            "required": ["title", "titleEn", "content", "excerpt", "excerptEn", "category", "tags"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // Validate repo path
        if self.config.repo_path.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Blog repo_path not configured. Add [blog] repo_path to agents.toml".into(),
            });
        }

        let repo = &self.config.repo_path;
        if !std::path::Path::new(repo).join("content").join("blog").exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Blog content dir not found at {}/content/blog/", repo),
            });
        }

        // Extract required fields
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let title_en = args.get("titleEn").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let content_en = args.get("contentEn").and_then(|v| v.as_str()).unwrap_or("");
        let excerpt = args.get("excerpt").and_then(|v| v.as_str()).unwrap_or("");
        let excerpt_en = args.get("excerptEn").and_then(|v| v.as_str()).unwrap_or("");
        let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("Dev Log");
        let read_time = args.get("readTime").and_then(|v| v.as_str()).unwrap_or("5 min");
        let icon = args.get("icon").and_then(|v| v.as_str()).unwrap_or("Brain");
        let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("from-purple-500 to-pink-500");
        let featured = args.get("featured").and_then(|v| v.as_bool()).unwrap_or(false);
        let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);

        if title.is_empty() || content.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required fields: title and content".into(),
            });
        }

        // Validate category
        let valid_cats = ["AI/ML", "Tutorial", "Insights", "News", "Dev Log"];
        if !valid_cats.contains(&category) {
            return Ok(ToolResult {
                success: false,
                output: format!("Invalid category '{}'. Must be one of: {}", category, valid_cats.join(", ")),
            });
        }

        // Generate slug
        let slug = if let Some(s) = args.get("slug").and_then(|v| v.as_str()) {
            s.to_string()
        } else {
            slugify(title_en)
        };

        if slug.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Could not generate slug. Provide titleEn or slug parameter.".into(),
            });
        }

        // Tags
        let tags: Vec<String> = if let Some(arr) = args.get("tags").and_then(|v| v.as_array()) {
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        } else {
            vec![]
        };

        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let author = &self.config.author;

        // 1. Create MDX file
        let tags_yaml: Vec<String> = tags.iter().map(|t| format!("\"{}\"", t)).collect();
        let mut mdx = format!(
            "---\n\
             title: \"{}\"\n\
             titleEn: \"{}\"\n\
             excerpt: \"{}\"\n\
             excerptEn: \"{}\"\n\
             date: \"{}\"\n\
             category: \"{}\"\n\
             tags: [{}]\n\
             readTime: \"{}\"\n\
             author: \"{}\"\n\
             featured: {}\n\
             icon: \"{}\"\n\
             color: \"{}\"\n\
             ---\n\n\
             {}",
            title.replace('"', "\\\""),
            title_en.replace('"', "\\\""),
            excerpt.replace('"', "\\\""),
            excerpt_en.replace('"', "\\\""),
            date,
            category,
            tags_yaml.join(", "),
            read_time,
            author,
            featured,
            icon,
            color,
            content,
        );

        // Add English content if provided
        if !content_en.is_empty() {
            mdx.push_str(&format!("\n\n---en---\n\n{}", content_en));
        }

        let mdx_path = std::path::Path::new(repo)
            .join("content").join("blog").join(format!("{}.mdx", slug));

        if mdx_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Blog post already exists: {}", mdx_path.display()),
            });
        }

        std::fs::write(&mdx_path, &mdx)
            .map_err(|e| anyhow::anyhow!("Failed to write MDX: {}", e))?;

        debug!("Created MDX: {}", mdx_path.display());

        // 2. Update index.ts
        let index_path = std::path::Path::new(repo)
            .join("src").join("data").join("blog").join("index.ts");

        let index_content = std::fs::read_to_string(&index_path)
            .map_err(|e| anyhow::anyhow!("Failed to read index.ts: {}", e))?;

        // Find max ID
        let max_id: i32 = {
            let re = regex::Regex::new(r"id:\s*(\d+)").unwrap();
            re.captures_iter(&index_content)
                .filter_map(|c| c.get(1).and_then(|m| m.as_str().parse().ok()))
                .max()
                .unwrap_or(0)
        };
        let new_id = max_id + 1;

        // Check if icon needs to be added to imports
        let mut updated_content = index_content.clone();
        let import_line_re = regex::Regex::new(
            r#"import \{([^}]+)\} from "lucide-react";"#
        ).unwrap();

        if let Some(cap) = import_line_re.captures(&updated_content) {
            let icons_str = cap.get(1).unwrap().as_str();
            let imported_icons: Vec<&str> = icons_str.split(',').map(|s| s.trim()).collect();
            if !imported_icons.contains(&icon) {
                // Add icon to import
                let new_import = format!("{}, {}", icons_str.trim(), icon);
                updated_content = updated_content.replace(
                    icons_str,
                    &format!(" {} ", new_import),
                );
            }
        }

        // Build tags TypeScript array
        let tags_ts: Vec<String> = tags.iter().map(|t| format!("\"{}\"", t)).collect();

        // Build new blog post entry
        let new_entry = format!(
            "  {{\n\
             \x20   id: {},\n\
             \x20   slug: \"{}\",\n\
             \x20   title: \"{}\",\n\
             \x20   titleEn: \"{}\",\n\
             \x20   excerpt: \"{}\",\n\
             \x20   excerptEn: \"{}\",\n\
             \x20   category: \"{}\",\n\
             \x20   tags: [{}],\n\
             \x20   date: \"{}\",\n\
             \x20   readTime: \"{}\",\n\
             \x20   author: \"{}\",\n\
             \x20   featured: {},\n\
             \x20   icon: {},\n\
             \x20   color: \"{}\",\n\
             \x20 }},",
            new_id,
            slug,
            title.replace('"', "\\\""),
            title_en.replace('"', "\\\""),
            excerpt.replace('"', "\\\""),
            excerpt_en.replace('"', "\\\""),
            category,
            tags_ts.join(", "),
            date,
            read_time,
            author,
            featured,
            icon,
            color,
        );

        // Insert at the top of blogPosts array
        let insert_marker = "export const blogPosts: BlogPost[] = [\n";
        if let Some(pos) = updated_content.find(insert_marker) {
            let insert_pos = pos + insert_marker.len();
            updated_content.insert_str(insert_pos, &format!("{}\n", new_entry));
        } else {
            // Cleanup MDX on failure
            let _ = std::fs::remove_file(&mdx_path);
            return Ok(ToolResult {
                success: false,
                output: "Could not find blogPosts array in index.ts".into(),
            });
        }

        std::fs::write(&index_path, &updated_content)
            .map_err(|e| anyhow::anyhow!("Failed to write index.ts: {}", e))?;

        debug!("Updated index.ts with id={}", new_id);

        // 3. Git add, commit, push
        if !dry_run {
            let mdx_rel = format!("content/blog/{}.mdx", slug);
            let index_rel = "src/data/blog/index.ts";

            let git_result = tokio::process::Command::new("git")
                .args(["add", &mdx_rel, index_rel])
                .current_dir(repo)
                .output()
                .await;

            if let Err(e) = git_result {
                return Ok(ToolResult {
                    success: false,
                    output: format!("git add failed: {}", e),
                });
            }

            let commit_msg = format!("feat: 新增部落格文章「{}」", title);
            let commit_result = tokio::process::Command::new("git")
                .args(["commit", "-m", &commit_msg])
                .current_dir(repo)
                .output()
                .await;

            match commit_result {
                Ok(out) if out.status.success() => {
                    debug!("Committed: {}", commit_msg);
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Ok(ToolResult {
                        success: false,
                        output: format!("git commit failed: {}", stderr),
                    });
                }
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("git commit error: {}", e),
                    });
                }
            }

            let push_result = tokio::process::Command::new("git")
                .args(["push", &self.config.remote, &self.config.branch])
                .current_dir(repo)
                .output()
                .await;

            match push_result {
                Ok(out) if out.status.success() => {
                    debug!("Pushed to {}/{}", self.config.remote, self.config.branch);
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    warn!("git push failed: {}", stderr);
                    return Ok(ToolResult {
                        success: true,  // Committed but push failed
                        output: format!(
                            "Blog post created and committed (id: {}, slug: {}), but push failed: {}. \
                             Push manually with: cd {} && git push {} {}",
                            new_id, slug, stderr, repo, self.config.remote, self.config.branch
                        ),
                    });
                }
                Err(e) => {
                    return Ok(ToolResult {
                        success: true,
                        output: format!(
                            "Blog post committed (id: {}, slug: {}), but push error: {}",
                            new_id, slug, e
                        ),
                    });
                }
            }

            Ok(ToolResult {
                success: true,
                output: format!(
                    "Blog post published!\n\
                     - ID: {}\n\
                     - Slug: {}\n\
                     - MDX: content/blog/{}.mdx\n\
                     - URL: https://markl-ai.space/blog/{}\n\
                     - Vercel will auto-deploy in ~1 minute.",
                    new_id, slug, slug, slug
                ),
            })
        } else {
            Ok(ToolResult {
                success: true,
                output: format!(
                    "Dry run complete.\n\
                     - ID: {}\n\
                     - Slug: {}\n\
                     - MDX created: content/blog/{}.mdx\n\
                     - index.ts updated\n\
                     - Skipped git push (dry_run=true)",
                    new_id, slug, slug
                ),
            })
        }
    }
}

/// Convert a title to a URL-safe slug (kebab-case)
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blog_tool_name() {
        let tool = BlogPublishTool::new(BlogConfig::default());
        assert_eq!(tool.name(), "blog_publish");
    }

    #[test]
    fn test_blog_config_defaults() {
        let config = BlogConfig::default();
        assert_eq!(config.author, "Mark Light");
        assert_eq!(config.remote, "origin");
        assert_eq!(config.branch, "main");
        assert!(config.repo_path.is_empty());
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("AI/ML Trends 2026"), "ai-ml-trends-2026");
        assert_eq!(slugify("How to Build an LLM Agent"), "how-to-build-an-llm-agent");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn test_blog_tool_schema() {
        let tool = BlogPublishTool::new(BlogConfig::default());
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("title").is_some());
        assert!(props.get("content").is_some());
        assert!(props.get("category").is_some());
    }

    #[tokio::test]
    async fn test_blog_no_repo_path() {
        let tool = BlogPublishTool::new(BlogConfig::default());
        let result = tool.execute(json!({
            "title": "Test",
            "titleEn": "Test",
            "content": "Hello",
            "excerpt": "ex",
            "excerptEn": "ex",
            "category": "Dev Log",
            "tags": ["test"]
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("repo_path"));
    }

    #[tokio::test]
    async fn test_blog_invalid_category() {
        let mut config = BlogConfig::default();
        config.repo_path = ".".to_string(); // Will fail path check first
        let tool = BlogPublishTool::new(config);
        let result = tool.execute(json!({
            "title": "Test",
            "titleEn": "Test",
            "content": "Hello",
            "excerpt": "ex",
            "excerptEn": "ex",
            "category": "Invalid",
            "tags": ["test"]
        })).await.unwrap();
        assert!(!result.success);
    }
}
