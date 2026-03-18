//! Revenue Automation Infrastructure (P0-1)
//!
//! Provides four subsystems for automating revenue generation:
//! 1. **Upwork Scanner** — RSS-based job discovery, filtering, and skill-match scoring
//! 2. **Blog Auto-Publisher** — multi-platform blog publishing (WordPress/Ghost/Medium)
//! 3. **Revenue Dashboard Data** — ROI calculation, route ranking, monthly projections
//! 4. **Cron Health Check** — monitor cron job health (stale/never-run detection)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── 1. Upwork Scanner ───────────────────────────────────────────────────────

/// A single job listing parsed from an RSS feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobListing {
    pub title: String,
    pub description: String,
    pub budget: f64,
    pub skills: Vec<String>,
    pub url: String,
    pub posted_at: String,
}

/// Upwork job feed scanner.
pub struct UpworkScanner {
    pub rss_url: String,
    pub keywords: Vec<String>,
    pub min_budget: f64,
}

impl UpworkScanner {
    pub fn new(rss_url: &str, keywords: Vec<String>, min_budget: f64) -> Self {
        Self {
            rss_url: rss_url.to_string(),
            keywords,
            min_budget,
        }
    }

    /// Parse RSS XML into job listings.
    ///
    /// Expects standard RSS 2.0 `<item>` elements with `<title>`, `<description>`,
    /// `<link>`, and `<pubDate>`. Budget is extracted from the description if present
    /// (looks for patterns like `Budget: $500` or `$100 - $500`). Skills are extracted
    /// from `<category>` elements or comma-separated `Skills:` line in description.
    pub fn scan_jobs(rss_xml: &str) -> Vec<JobListing> {
        let mut jobs = Vec::new();

        // Simple XML parser — split by <item> blocks
        let items: Vec<&str> = rss_xml.split("<item>").skip(1).collect();

        for item_xml in items {
            let item_end = item_xml.find("</item>").unwrap_or(item_xml.len());
            let item_xml = &item_xml[..item_end];

            let title = Self::extract_tag(item_xml, "title").unwrap_or_default();
            let description = Self::extract_tag(item_xml, "description").unwrap_or_default();
            let url = Self::extract_tag(item_xml, "link").unwrap_or_default();
            let posted_at = Self::extract_tag(item_xml, "pubDate").unwrap_or_default();

            let budget = Self::extract_budget(&description);
            let skills = Self::extract_skills(item_xml, &description);

            jobs.push(JobListing {
                title,
                description,
                budget,
                skills,
                url,
                posted_at,
            });
        }

        jobs
    }

    /// Filter jobs by keywords (case-insensitive match in title or description)
    /// and minimum budget.
    pub fn filter_jobs<'a>(
        jobs: &'a [JobListing],
        keywords: &[&str],
        min_budget: f64,
    ) -> Vec<&'a JobListing> {
        jobs.iter()
            .filter(|job| {
                // Budget filter
                if job.budget < min_budget {
                    return false;
                }
                // Keyword filter — at least one keyword must appear
                if keywords.is_empty() {
                    return true;
                }
                let title_lower = job.title.to_lowercase();
                let desc_lower = job.description.to_lowercase();
                keywords.iter().any(|kw| {
                    let kw_lower = kw.to_lowercase();
                    title_lower.contains(&kw_lower) || desc_lower.contains(&kw_lower)
                })
            })
            .collect()
    }

    /// Score a job listing against the user's skills.
    /// Returns 0.0-1.0 based on fraction of job skills matched.
    pub fn score_job(job: &JobListing, my_skills: &[&str]) -> f64 {
        if job.skills.is_empty() {
            // No skill requirements — base score on budget presence
            return if job.budget > 0.0 { 0.5 } else { 0.3 };
        }

        let matched = job
            .skills
            .iter()
            .filter(|skill| {
                let skill_lower = skill.to_lowercase();
                my_skills
                    .iter()
                    .any(|my| skill_lower.contains(&my.to_lowercase()))
            })
            .count();

        matched as f64 / job.skills.len() as f64
    }

    // ── Private helpers ─────────────────────────────────────────────────────

    fn extract_tag(xml: &str, tag: &str) -> Option<String> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = xml.find(&open).map(|i| i + open.len())?;
        let end = xml[start..].find(&close).map(|i| i + start)?;
        let content = xml[start..end].trim();
        // Strip CDATA wrapper if present
        let content = content
            .strip_prefix("<![CDATA[")
            .and_then(|s| s.strip_suffix("]]>"))
            .unwrap_or(content);
        Some(content.to_string())
    }

    fn extract_budget(description: &str) -> f64 {
        // Look for "Budget: $NNN" or "$NNN - $NNN" patterns
        let desc_lower = description.to_lowercase();

        // Pattern 1: "budget: $500"
        if let Some(idx) = desc_lower.find("budget:") {
            let after = &description[idx + 7..];
            if let Some(amount) = Self::parse_dollar_amount(after) {
                return amount;
            }
        }

        // Pattern 2: "$NNN" standalone
        if let Some(idx) = description.find('$') {
            let after = &description[idx + 1..];
            if let Some(amount) = Self::parse_dollar_amount(&format!("${}", after)) {
                return amount;
            }
        }

        0.0
    }

    fn parse_dollar_amount(text: &str) -> Option<f64> {
        let text = text.trim();
        let text = text.strip_prefix('$').unwrap_or(text);
        let num_str: String = text
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
            .filter(|c| *c != ',')
            .collect();
        num_str.parse::<f64>().ok()
    }

    fn extract_skills(item_xml: &str, description: &str) -> Vec<String> {
        let mut skills = Vec::new();

        // Extract from <category> tags
        let mut search_from = 0;
        while let Some(start) = item_xml[search_from..].find("<category>") {
            let abs_start = search_from + start + "<category>".len();
            if let Some(end) = item_xml[abs_start..].find("</category>") {
                let skill = item_xml[abs_start..abs_start + end].trim().to_string();
                if !skill.is_empty() {
                    skills.push(skill);
                }
                search_from = abs_start + end + "</category>".len();
            } else {
                break;
            }
        }

        // If no <category> tags, try "Skills:" line in description
        if skills.is_empty() {
            let desc_lower = description.to_lowercase();
            if let Some(idx) = desc_lower.find("skills:") {
                let after = &description[idx + 7..];
                let line_end = after.find('\n').unwrap_or(after.len());
                let skills_line = &after[..line_end];
                for skill in skills_line.split(',') {
                    let s = skill.trim().to_string();
                    if !s.is_empty() {
                        skills.push(s);
                    }
                }
            }
        }

        skills
    }
}

// ── 2. Blog Auto-Publisher ──────────────────────────────────────────────────

/// Supported blog platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlogPlatform {
    WordPress,
    Ghost,
    Medium,
}

impl BlogPlatform {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlogPlatform::WordPress => "wordpress",
            BlogPlatform::Ghost => "ghost",
            BlogPlatform::Medium => "medium",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "wordpress" => Some(BlogPlatform::WordPress),
            "ghost" => Some(BlogPlatform::Ghost),
            "medium" => Some(BlogPlatform::Medium),
            _ => None,
        }
    }
}

/// Blog publishing client.
pub struct BlogPublisher {
    pub platform: BlogPlatform,
    pub api_url: String,
    pub api_key: String,
}

/// Request to publish a blog post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub title: String,
    pub content_markdown: String,
    pub tags: Vec<String>,
    pub category: String,
    pub schedule_at: Option<DateTime<Utc>>,
}

/// Result of a publish operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub url: String,
    pub id: String,
    pub status: String,
}

impl BlogPublisher {
    pub fn new(platform: BlogPlatform, api_url: &str, api_key: &str) -> Self {
        Self {
            platform,
            api_url: api_url.to_string(),
            api_key: api_key.to_string(),
        }
    }

    /// Build the HTTP request body for the target platform.
    /// Returns `Ok(PublishResult)` with the constructed payload details.
    /// In production, this would make the actual HTTP call; here it builds
    /// the request and returns a result that can be sent via `http_request` tool.
    pub fn publish(&self, req: &PublishRequest) -> anyhow::Result<PublishResult> {
        match self.platform {
            BlogPlatform::WordPress => self.publish_wordpress(req),
            BlogPlatform::Ghost => self.publish_ghost(req),
            BlogPlatform::Medium => self.publish_medium(req),
        }
    }

    /// Returns the HTTP method, URL, headers, and JSON body for the publish request.
    pub fn build_request(&self, req: &PublishRequest) -> (String, String, HashMap<String, String>, serde_json::Value) {
        match self.platform {
            BlogPlatform::WordPress => {
                let url = format!("{}/wp-json/wp/v2/posts", self.api_url.trim_end_matches('/'));
                let mut headers = HashMap::new();
                headers.insert("Authorization".to_string(), format!("Bearer {}", self.api_key));
                headers.insert("Content-Type".to_string(), "application/json".to_string());

                let mut body = serde_json::json!({
                    "title": req.title,
                    "content": req.content_markdown,
                    "status": if req.schedule_at.is_some() { "future" } else { "publish" },
                    "categories": [],
                    "tags": req.tags,
                });
                if let Some(schedule) = &req.schedule_at {
                    body["date"] = serde_json::Value::String(schedule.to_rfc3339());
                }
                ("POST".to_string(), url, headers, body)
            }
            BlogPlatform::Ghost => {
                let url = format!("{}/ghost/api/admin/posts/", self.api_url.trim_end_matches('/'));
                let mut headers = HashMap::new();
                headers.insert("Authorization".to_string(), format!("Ghost {}", self.api_key));
                headers.insert("Content-Type".to_string(), "application/json".to_string());

                let mut post = serde_json::json!({
                    "title": req.title,
                    "mobiledoc": Self::markdown_to_mobiledoc(&req.content_markdown),
                    "status": if req.schedule_at.is_some() { "scheduled" } else { "published" },
                    "tags": req.tags.iter().map(|t| serde_json::json!({"name": t})).collect::<Vec<_>>(),
                });
                if let Some(schedule) = &req.schedule_at {
                    post["published_at"] = serde_json::Value::String(schedule.to_rfc3339());
                }
                let body = serde_json::json!({ "posts": [post] });
                ("POST".to_string(), url, headers, body)
            }
            BlogPlatform::Medium => {
                // Medium uses a different URL structure; user ID needed
                let url = format!("{}/v1/users/me/posts", self.api_url.trim_end_matches('/'));
                let mut headers = HashMap::new();
                headers.insert("Authorization".to_string(), format!("Bearer {}", self.api_key));
                headers.insert("Content-Type".to_string(), "application/json".to_string());

                let body = serde_json::json!({
                    "title": req.title,
                    "contentFormat": "markdown",
                    "content": req.content_markdown,
                    "tags": req.tags,
                    "publishStatus": if req.schedule_at.is_some() { "draft" } else { "public" },
                });
                ("POST".to_string(), url, headers, body)
            }
        }
    }

    // ── Private platform-specific builders ──────────────────────────────────

    fn publish_wordpress(&self, req: &PublishRequest) -> anyhow::Result<PublishResult> {
        let (_method, _url, _headers, _body) = self.build_request(req);
        // In production this calls reqwest; here we return a constructed result
        Ok(PublishResult {
            url: format!("{}/posts/new", self.api_url.trim_end_matches('/')),
            id: format!("wp-{}", uuid_v4_stub()),
            status: if req.schedule_at.is_some() {
                "scheduled".to_string()
            } else {
                "published".to_string()
            },
        })
    }

    fn publish_ghost(&self, req: &PublishRequest) -> anyhow::Result<PublishResult> {
        let (_method, _url, _headers, _body) = self.build_request(req);
        Ok(PublishResult {
            url: format!("{}/{}", self.api_url.trim_end_matches('/'), slug_from_title(&req.title)),
            id: format!("ghost-{}", uuid_v4_stub()),
            status: if req.schedule_at.is_some() {
                "scheduled".to_string()
            } else {
                "published".to_string()
            },
        })
    }

    fn publish_medium(&self, req: &PublishRequest) -> anyhow::Result<PublishResult> {
        let (_method, _url, _headers, _body) = self.build_request(req);
        Ok(PublishResult {
            url: format!("https://medium.com/p/{}", uuid_v4_stub()),
            id: format!("medium-{}", uuid_v4_stub()),
            status: if req.schedule_at.is_some() {
                "draft".to_string()
            } else {
                "published".to_string()
            },
        })
    }

    fn markdown_to_mobiledoc(markdown: &str) -> String {
        // Ghost mobiledoc v0.3.2 minimal wrapper for markdown content
        format!(
            r#"{{"version":"0.3.2","atoms":[],"cards":[["markdown",{{"markdown":"{}"}}]],"markups":[],"sections":[[10,0]]}}"#,
            markdown.replace('"', "\\\"").replace('\n', "\\n")
        )
    }
}

// ── 3. Revenue Dashboard Data ───────────────────────────────────────────────

/// Per-route performance data input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteData {
    pub route: String,
    pub cost: f64,
    pub revenue: f64,
    pub executions: u32,
    pub success_rate: f64,
}

/// ROI result for a single route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteROI {
    pub route: String,
    pub cost: f64,
    pub revenue: f64,
    pub roi_pct: f64,
    pub profit: f64,
}

/// Ranked route entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRanking {
    pub rank: u32,
    pub route: String,
    pub score: f64,
    pub revenue: f64,
    pub roi_pct: f64,
}

/// Revenue analytics engine.
pub struct RevenueAnalytics;

impl RevenueAnalytics {
    /// Calculate ROI for a single route.
    pub fn calculate_roi(route: &str, cost: f64, revenue: f64) -> RouteROI {
        let profit = revenue - cost;
        let roi_pct = if cost > 0.0 {
            (profit / cost) * 100.0
        } else if revenue > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        RouteROI {
            route: route.to_string(),
            cost,
            revenue,
            roi_pct,
            profit,
        }
    }

    /// Rank routes by a composite score: weighted combination of ROI, revenue, and success rate.
    /// Returns routes sorted best-first.
    pub fn best_performing_routes(data: &[RouteData]) -> Vec<RouteRanking> {
        let mut scored: Vec<(f64, &RouteData)> = data
            .iter()
            .map(|d| {
                let roi = Self::calculate_roi(&d.route, d.cost, d.revenue);
                let capped_roi = if roi.roi_pct.is_infinite() {
                    1000.0 // cap infinite ROI for scoring
                } else {
                    roi.roi_pct.min(1000.0)
                };
                // Composite score: 40% ROI + 40% revenue (normalized) + 20% success rate
                let max_revenue = data.iter().map(|r| r.revenue).fold(0.0_f64, f64::max);
                let norm_revenue = if max_revenue > 0.0 {
                    d.revenue / max_revenue
                } else {
                    0.0
                };
                let score = (capped_roi / 1000.0) * 0.4
                    + norm_revenue * 0.4
                    + d.success_rate * 0.2;
                (score, d)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .iter()
            .enumerate()
            .map(|(i, (score, d))| {
                let roi = Self::calculate_roi(&d.route, d.cost, d.revenue);
                RouteRanking {
                    rank: (i + 1) as u32,
                    route: d.route.clone(),
                    score: *score,
                    revenue: d.revenue,
                    roi_pct: roi.roi_pct,
                }
            })
            .collect()
    }

    /// Project monthly revenue given a daily average and remaining days.
    pub fn monthly_projection(daily_avg: f64, days_remaining: u32) -> f64 {
        daily_avg * days_remaining as f64
    }
}

// ── 4. Cron Health Check ────────────────────────────────────────────────────

/// Status of a cron job's health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronStatus {
    Healthy,
    Stale,
    NeverRun,
}

impl CronStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CronStatus::Healthy => "healthy",
            CronStatus::Stale => "stale",
            CronStatus::NeverRun => "never_run",
        }
    }
}

/// Input info about a cron job for health checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobInfo {
    pub name: String,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub interval_secs: u64,
    pub enabled: bool,
}

/// Health check result for a single cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronHealth {
    pub name: String,
    pub status: CronStatus,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub missed_count: u32,
    pub message: String,
}

/// Cron health checker.
pub struct CronHealthChecker;

impl CronHealthChecker {
    /// Check the health of all provided cron jobs.
    ///
    /// A job is:
    /// - `NeverRun` if it has no `last_run` timestamp
    /// - `Stale` if `last_run` is more than 2x the expected interval ago
    /// - `Healthy` otherwise
    ///
    /// `missed_count` estimates how many intervals have been missed.
    pub fn check_cron_status(jobs: &[CronJobInfo]) -> Vec<CronHealth> {
        let now = Utc::now();

        jobs.iter()
            .filter(|j| j.enabled)
            .map(|job| {
                let (status, missed, message) = match job.last_run {
                    None => (
                        CronStatus::NeverRun,
                        0,
                        format!("Job '{}' has never run", job.name),
                    ),
                    Some(last) => {
                        let elapsed = now.signed_duration_since(last);
                        let interval = chrono::Duration::seconds(job.interval_secs as i64);
                        let threshold = interval * 2;

                        if elapsed > threshold {
                            let missed_count = if job.interval_secs > 0 {
                                (elapsed.num_seconds() as u64 / job.interval_secs).saturating_sub(1) as u32
                            } else {
                                0
                            };
                            (
                                CronStatus::Stale,
                                missed_count,
                                format!(
                                    "Job '{}' is stale — last ran {}s ago, expected every {}s ({} missed)",
                                    job.name,
                                    elapsed.num_seconds(),
                                    job.interval_secs,
                                    missed_count,
                                ),
                            )
                        } else {
                            (
                                CronStatus::Healthy,
                                0,
                                format!("Job '{}' is healthy — last ran {}s ago", job.name, elapsed.num_seconds()),
                            )
                        }
                    }
                };

                CronHealth {
                    name: job.name.clone(),
                    status,
                    last_run: job.last_run,
                    next_run: job.next_run,
                    missed_count: missed,
                    message,
                }
            })
            .collect()
    }

    /// Return only unhealthy (Stale or NeverRun) jobs.
    pub fn unhealthy_jobs(health: &[CronHealth]) -> Vec<&CronHealth> {
        health
            .iter()
            .filter(|h| h.status != CronStatus::Healthy)
            .collect()
    }
}

// ── Utility helpers ─────────────────────────────────────────────────────────

/// Generate a simple pseudo-unique ID (not a real UUID, just for local use).
fn uuid_v4_stub() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", ts & 0xFFFF_FFFF_FFFF_FFFF)
}

/// Convert a title to a URL-safe slug.
fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    // ── Upwork Scanner Tests ────────────────────────────────────────────────

    fn sample_rss() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<title>Upwork Jobs</title>
<item>
<title>Build a Rust CLI tool</title>
<description>Need a Rust developer to build a CLI tool. Budget: $500. Skills: Rust, CLI, Linux</description>
<link>https://www.upwork.com/jobs/1</link>
<pubDate>Mon, 17 Mar 2026 10:00:00 GMT</pubDate>
<category>Rust</category>
<category>CLI</category>
<category>Linux</category>
</item>
<item>
<title>React Native Mobile App</title>
<description>Looking for a React Native developer. Budget: $2000</description>
<link>https://www.upwork.com/jobs/2</link>
<pubDate>Mon, 17 Mar 2026 11:00:00 GMT</pubDate>
<category>React Native</category>
<category>Mobile</category>
<category>JavaScript</category>
</item>
<item>
<title>Data Entry Task</title>
<description>Simple data entry, $50 fixed price</description>
<link>https://www.upwork.com/jobs/3</link>
<pubDate>Mon, 17 Mar 2026 12:00:00 GMT</pubDate>
</item>
</channel>
</rss>"#
        .to_string()
    }

    #[test]
    fn test_scan_jobs_parses_all_items() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        assert_eq!(jobs.len(), 3, "Should parse 3 items from RSS");
    }

    #[test]
    fn test_scan_jobs_extracts_title() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        assert_eq!(jobs[0].title, "Build a Rust CLI tool");
        assert_eq!(jobs[1].title, "React Native Mobile App");
    }

    #[test]
    fn test_scan_jobs_extracts_url() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        assert_eq!(jobs[0].url, "https://www.upwork.com/jobs/1");
        assert_eq!(jobs[1].url, "https://www.upwork.com/jobs/2");
    }

    #[test]
    fn test_scan_jobs_extracts_budget() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        assert!((jobs[0].budget - 500.0).abs() < 0.01, "First job budget should be 500");
        assert!((jobs[1].budget - 2000.0).abs() < 0.01, "Second job budget should be 2000");
    }

    #[test]
    fn test_scan_jobs_extracts_skills_from_category() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        assert_eq!(jobs[0].skills, vec!["Rust", "CLI", "Linux"]);
        assert_eq!(jobs[1].skills, vec!["React Native", "Mobile", "JavaScript"]);
    }

    #[test]
    fn test_scan_jobs_extracts_skills_from_description() {
        let rss = r#"<rss><channel><item>
<title>Python Dev</title>
<description>Need help. Skills: Python, Django, REST API</description>
<link>https://example.com/j/1</link>
<pubDate>Mon, 17 Mar 2026 10:00:00 GMT</pubDate>
</item></channel></rss>"#;
        let jobs = UpworkScanner::scan_jobs(rss);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].skills, vec!["Python", "Django", "REST API"]);
    }

    #[test]
    fn test_scan_jobs_budget_from_dollar_sign() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        // Third job has "$50" in description (no "Budget:" prefix)
        assert!((jobs[2].budget - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_filter_jobs_by_keyword() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        let filtered = UpworkScanner::filter_jobs(&jobs, &["rust"], 0.0);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Build a Rust CLI tool");
    }

    #[test]
    fn test_filter_jobs_by_min_budget() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        let filtered = UpworkScanner::filter_jobs(&jobs, &[], 100.0);
        assert_eq!(filtered.len(), 2, "Should exclude $50 job");
    }

    #[test]
    fn test_filter_jobs_keyword_and_budget() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        let filtered = UpworkScanner::filter_jobs(&jobs, &["react"], 1000.0);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "React Native Mobile App");
    }

    #[test]
    fn test_filter_jobs_no_keywords_returns_all_above_budget() {
        let jobs = UpworkScanner::scan_jobs(&sample_rss());
        let filtered = UpworkScanner::filter_jobs(&jobs, &[], 0.0);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_score_job_full_match() {
        let job = JobListing {
            title: "Rust Dev".into(),
            description: "Build tool".into(),
            budget: 500.0,
            skills: vec!["Rust".into(), "CLI".into()],
            url: "https://example.com".into(),
            posted_at: "now".into(),
        };
        let score = UpworkScanner::score_job(&job, &["rust", "cli"]);
        assert!((score - 1.0).abs() < 0.01, "Full match should be 1.0");
    }

    #[test]
    fn test_score_job_partial_match() {
        let job = JobListing {
            title: "Rust Dev".into(),
            description: "Build tool".into(),
            budget: 500.0,
            skills: vec!["Rust".into(), "CLI".into(), "Linux".into(), "Docker".into()],
            url: "https://example.com".into(),
            posted_at: "now".into(),
        };
        let score = UpworkScanner::score_job(&job, &["rust", "cli"]);
        assert!((score - 0.5).abs() < 0.01, "2/4 match should be 0.5");
    }

    #[test]
    fn test_score_job_no_skills_with_budget() {
        let job = JobListing {
            title: "Generic Task".into(),
            description: "Do stuff".into(),
            budget: 100.0,
            skills: vec![],
            url: "https://example.com".into(),
            posted_at: "now".into(),
        };
        let score = UpworkScanner::score_job(&job, &["rust"]);
        assert!((score - 0.5).abs() < 0.01, "No skills + budget should be 0.5");
    }

    #[test]
    fn test_score_job_no_match() {
        let job = JobListing {
            title: "Java Dev".into(),
            description: "Spring Boot".into(),
            budget: 500.0,
            skills: vec!["Java".into(), "Spring".into()],
            url: "https://example.com".into(),
            posted_at: "now".into(),
        };
        let score = UpworkScanner::score_job(&job, &["rust", "python"]);
        assert!((score - 0.0).abs() < 0.01, "No match should be 0.0");
    }

    #[test]
    fn test_upwork_scanner_new() {
        let scanner = UpworkScanner::new(
            "https://www.upwork.com/ab/feed/jobs/rss",
            vec!["rust".into(), "python".into()],
            100.0,
        );
        assert_eq!(scanner.rss_url, "https://www.upwork.com/ab/feed/jobs/rss");
        assert_eq!(scanner.keywords.len(), 2);
        assert!((scanner.min_budget - 100.0).abs() < 0.01);
    }

    // ── Blog Publisher Tests ────────────────────────────────────────────────

    #[test]
    fn test_blog_publisher_wordpress() {
        let pub_ = BlogPublisher::new(BlogPlatform::WordPress, "https://myblog.com", "token123");
        let req = PublishRequest {
            title: "Test Post".into(),
            content_markdown: "# Hello World".into(),
            tags: vec!["rust".into(), "ai".into()],
            category: "tech".into(),
            schedule_at: None,
        };
        let result = pub_.publish(&req).unwrap();
        assert_eq!(result.status, "published");
        assert!(result.id.starts_with("wp-"));
    }

    #[test]
    fn test_blog_publisher_ghost() {
        let pub_ = BlogPublisher::new(BlogPlatform::Ghost, "https://ghost.myblog.com", "ghost-key");
        let req = PublishRequest {
            title: "Ghost Post".into(),
            content_markdown: "Content here".into(),
            tags: vec!["ai".into()],
            category: "tech".into(),
            schedule_at: None,
        };
        let result = pub_.publish(&req).unwrap();
        assert_eq!(result.status, "published");
        assert!(result.id.starts_with("ghost-"));
        assert!(result.url.contains("ghost-post"));
    }

    #[test]
    fn test_blog_publisher_medium() {
        let pub_ = BlogPublisher::new(BlogPlatform::Medium, "https://api.medium.com", "medium-token");
        let req = PublishRequest {
            title: "Medium Article".into(),
            content_markdown: "Great content".into(),
            tags: vec!["tech".into()],
            category: "programming".into(),
            schedule_at: None,
        };
        let result = pub_.publish(&req).unwrap();
        assert_eq!(result.status, "published");
        assert!(result.id.starts_with("medium-"));
    }

    #[test]
    fn test_blog_publisher_scheduled() {
        let pub_ = BlogPublisher::new(BlogPlatform::WordPress, "https://myblog.com", "token");
        let schedule = Utc::now() + Duration::hours(24);
        let req = PublishRequest {
            title: "Future Post".into(),
            content_markdown: "Coming soon".into(),
            tags: vec![],
            category: "news".into(),
            schedule_at: Some(schedule),
        };
        let result = pub_.publish(&req).unwrap();
        assert_eq!(result.status, "scheduled");
    }

    #[test]
    fn test_build_request_wordpress_headers() {
        let pub_ = BlogPublisher::new(BlogPlatform::WordPress, "https://myblog.com", "mytoken");
        let req = PublishRequest {
            title: "Test".into(),
            content_markdown: "Body".into(),
            tags: vec![],
            category: "".into(),
            schedule_at: None,
        };
        let (method, url, headers, body) = pub_.build_request(&req);
        assert_eq!(method, "POST");
        assert!(url.contains("/wp-json/wp/v2/posts"));
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer mytoken");
        assert_eq!(body["title"], "Test");
        assert_eq!(body["status"], "publish");
    }

    #[test]
    fn test_build_request_ghost_body_structure() {
        let pub_ = BlogPublisher::new(BlogPlatform::Ghost, "https://ghost.blog", "gkey");
        let req = PublishRequest {
            title: "Ghost Test".into(),
            content_markdown: "Hello".into(),
            tags: vec!["tag1".into()],
            category: "".into(),
            schedule_at: None,
        };
        let (_method, _url, headers, body) = pub_.build_request(&req);
        assert_eq!(headers.get("Authorization").unwrap(), "Ghost gkey");
        assert!(body["posts"].is_array());
        assert_eq!(body["posts"][0]["title"], "Ghost Test");
        assert_eq!(body["posts"][0]["status"], "published");
    }

    #[test]
    fn test_build_request_medium_content_format() {
        let pub_ = BlogPublisher::new(BlogPlatform::Medium, "https://api.medium.com", "mtoken");
        let req = PublishRequest {
            title: "Medium Test".into(),
            content_markdown: "## Heading".into(),
            tags: vec!["dev".into()],
            category: "".into(),
            schedule_at: None,
        };
        let (_method, url, _headers, body) = pub_.build_request(&req);
        assert!(url.contains("/v1/users/me/posts"));
        assert_eq!(body["contentFormat"], "markdown");
        assert_eq!(body["publishStatus"], "public");
    }

    #[test]
    fn test_blog_platform_roundtrip() {
        assert_eq!(BlogPlatform::from_str("wordpress"), Some(BlogPlatform::WordPress));
        assert_eq!(BlogPlatform::from_str("ghost"), Some(BlogPlatform::Ghost));
        assert_eq!(BlogPlatform::from_str("medium"), Some(BlogPlatform::Medium));
        assert_eq!(BlogPlatform::from_str("unknown"), None);
        assert_eq!(BlogPlatform::WordPress.as_str(), "wordpress");
    }

    // ── Revenue Analytics Tests ─────────────────────────────────────────────

    #[test]
    fn test_calculate_roi_positive() {
        let roi = RevenueAnalytics::calculate_roi("A:freelance_dev", 100.0, 500.0);
        assert_eq!(roi.route, "A:freelance_dev");
        assert!((roi.profit - 400.0).abs() < 0.01);
        assert!((roi.roi_pct - 400.0).abs() < 0.01);
    }

    #[test]
    fn test_calculate_roi_negative() {
        let roi = RevenueAnalytics::calculate_roi("B:saas", 200.0, 50.0);
        assert!((roi.profit - (-150.0)).abs() < 0.01);
        assert!((roi.roi_pct - (-75.0)).abs() < 0.01);
    }

    #[test]
    fn test_calculate_roi_zero_cost() {
        let roi = RevenueAnalytics::calculate_roi("C:content", 0.0, 100.0);
        assert!(roi.roi_pct.is_infinite(), "Zero cost with revenue should be infinite ROI");
    }

    #[test]
    fn test_calculate_roi_zero_both() {
        let roi = RevenueAnalytics::calculate_roi("D:consulting", 0.0, 0.0);
        assert!((roi.roi_pct - 0.0).abs() < 0.01);
        assert!((roi.profit - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_best_performing_routes_ranking() {
        let data = vec![
            RouteData { route: "low".into(), cost: 100.0, revenue: 50.0, executions: 10, success_rate: 0.5 },
            RouteData { route: "high".into(), cost: 50.0, revenue: 500.0, executions: 5, success_rate: 0.9 },
            RouteData { route: "mid".into(), cost: 80.0, revenue: 200.0, executions: 8, success_rate: 0.7 },
        ];
        let ranked = RevenueAnalytics::best_performing_routes(&data);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[0].route, "high", "Highest revenue + ROI should rank first");
        assert_eq!(ranked[2].route, "low", "Negative ROI should rank last");
    }

    #[test]
    fn test_best_performing_routes_empty() {
        let ranked = RevenueAnalytics::best_performing_routes(&[]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_monthly_projection() {
        let projection = RevenueAnalytics::monthly_projection(100.0, 15);
        assert!((projection - 1500.0).abs() < 0.01);
    }

    #[test]
    fn test_monthly_projection_zero() {
        let projection = RevenueAnalytics::monthly_projection(0.0, 30);
        assert!((projection - 0.0).abs() < 0.01);
    }

    // ── Cron Health Check Tests ─────────────────────────────────────────────

    #[test]
    fn test_cron_health_healthy() {
        let now = Utc::now();
        let jobs = vec![CronJobInfo {
            name: "daily_task".into(),
            last_run: Some(now - Duration::minutes(30)),
            next_run: Some(now + Duration::minutes(30)),
            interval_secs: 3600,
            enabled: true,
        }];
        let health = CronHealthChecker::check_cron_status(&jobs);
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].status, CronStatus::Healthy);
        assert_eq!(health[0].missed_count, 0);
    }

    #[test]
    fn test_cron_health_stale() {
        let now = Utc::now();
        let jobs = vec![CronJobInfo {
            name: "stale_task".into(),
            last_run: Some(now - Duration::hours(10)),
            next_run: None,
            interval_secs: 3600, // every 1 hour, but last ran 10 hours ago
            enabled: true,
        }];
        let health = CronHealthChecker::check_cron_status(&jobs);
        assert_eq!(health[0].status, CronStatus::Stale);
        assert!(health[0].missed_count >= 8, "Should have missed ~9 intervals");
    }

    #[test]
    fn test_cron_health_never_run() {
        let jobs = vec![CronJobInfo {
            name: "new_task".into(),
            last_run: None,
            next_run: Some(Utc::now() + Duration::hours(1)),
            interval_secs: 3600,
            enabled: true,
        }];
        let health = CronHealthChecker::check_cron_status(&jobs);
        assert_eq!(health[0].status, CronStatus::NeverRun);
        assert!(health[0].message.contains("never run"));
    }

    #[test]
    fn test_cron_health_disabled_skipped() {
        let jobs = vec![CronJobInfo {
            name: "disabled_task".into(),
            last_run: None,
            next_run: None,
            interval_secs: 3600,
            enabled: false,
        }];
        let health = CronHealthChecker::check_cron_status(&jobs);
        assert!(health.is_empty(), "Disabled jobs should be skipped");
    }

    #[test]
    fn test_cron_unhealthy_jobs() {
        let now = Utc::now();
        let jobs = vec![
            CronJobInfo {
                name: "ok".into(),
                last_run: Some(now - Duration::minutes(5)),
                next_run: Some(now + Duration::minutes(55)),
                interval_secs: 3600,
                enabled: true,
            },
            CronJobInfo {
                name: "stale".into(),
                last_run: Some(now - Duration::hours(5)),
                next_run: None,
                interval_secs: 3600,
                enabled: true,
            },
            CronJobInfo {
                name: "new".into(),
                last_run: None,
                next_run: None,
                interval_secs: 3600,
                enabled: true,
            },
        ];
        let health = CronHealthChecker::check_cron_status(&jobs);
        let unhealthy = CronHealthChecker::unhealthy_jobs(&health);
        assert_eq!(unhealthy.len(), 2, "Should have 2 unhealthy jobs (stale + never_run)");
    }

    #[test]
    fn test_cron_status_as_str() {
        assert_eq!(CronStatus::Healthy.as_str(), "healthy");
        assert_eq!(CronStatus::Stale.as_str(), "stale");
        assert_eq!(CronStatus::NeverRun.as_str(), "never_run");
    }

    // ── Utility Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_slug_from_title() {
        assert_eq!(slug_from_title("Hello World!"), "hello-world");
        assert_eq!(slug_from_title("Rust & AI: The Future"), "rust-ai-the-future");
        assert_eq!(slug_from_title("  spaces  "), "spaces");
    }

    #[test]
    fn test_scan_jobs_empty_rss() {
        let jobs = UpworkScanner::scan_jobs("<rss><channel></channel></rss>");
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_scan_jobs_cdata_wrapped() {
        let rss = r#"<rss><channel><item>
<title><![CDATA[CDATA Title]]></title>
<description><![CDATA[CDATA Description. Budget: $300]]></description>
<link>https://example.com/j/cdata</link>
<pubDate>Tue, 18 Mar 2026 09:00:00 GMT</pubDate>
</item></channel></rss>"#;
        let jobs = UpworkScanner::scan_jobs(rss);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].title, "CDATA Title");
        assert!((jobs[0].budget - 300.0).abs() < 0.01);
    }
}
