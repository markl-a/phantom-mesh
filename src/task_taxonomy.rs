// task_taxonomy.rs — Structured task classification for smarter cluster dispatch.
//
// Each incoming task (tool invocation or hand execution) is classified into a
// TaskCategory. Each category maps to a TaskProfile that encodes hardware
// affinity, latency SLA, cost ceiling, and preferred/fallback node lists.
// The cluster dispatcher can query the taxonomy to make optimal placement
// decisions without hard-coding routing logic everywhere.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// TaskCategory
// ---------------------------------------------------------------------------

/// High-level task classification bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskCategory {
    /// Code generation, code review, refactoring — benefits from GPU acceleration.
    Code,
    /// Reasoning, analysis, planning — CPU-bound, any node.
    Think,
    /// Web search, data gathering, API calls — network-intensive.
    Research,
    /// Bulk / batch processing — cost-optimized, latency-tolerant.
    Batch,
    /// Local file-system operations — must run on hub node with FS access.
    Local,
    /// Operational / administrative commands — hub-only, fast.
    Ops,
}

impl TaskCategory {
    /// Return a lowercase slug for serialization and logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskCategory::Code => "code",
            TaskCategory::Think => "think",
            TaskCategory::Research => "research",
            TaskCategory::Batch => "batch",
            TaskCategory::Local => "local",
            TaskCategory::Ops => "ops",
        }
    }

    /// All variants in definition order.
    pub fn all() -> &'static [TaskCategory] {
        &[
            TaskCategory::Code,
            TaskCategory::Think,
            TaskCategory::Research,
            TaskCategory::Batch,
            TaskCategory::Local,
            TaskCategory::Ops,
        ]
    }
}

impl std::fmt::Display for TaskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// TaskProfile
// ---------------------------------------------------------------------------

/// Resource and scheduling profile associated with a TaskCategory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProfile {
    /// Which category this profile belongs to.
    pub category: TaskCategory,
    /// Whether a GPU-equipped node is preferred (not strictly required).
    pub gpu_required: bool,
    /// Maximum acceptable end-to-end latency in milliseconds.
    pub latency_target_ms: u64,
    /// Maximum cost in USD we are willing to pay per invocation.
    pub cost_ceiling_usd: f64,
    /// Ordered list of preferred node names (best first).
    pub preferred_nodes: Vec<String>,
    /// Fallback nodes to try if no preferred node is available.
    pub fallback_nodes: Vec<String>,
}

// ---------------------------------------------------------------------------
// TaskTaxonomy
// ---------------------------------------------------------------------------

/// Registry of default profiles, one per TaskCategory.
pub struct TaskTaxonomy {
    profiles: HashMap<TaskCategory, TaskProfile>,
}

impl TaskTaxonomy {
    /// Build the default taxonomy with profiles for every category.
    pub fn new() -> Self {
        let mut profiles = HashMap::new();

        // Code — GPU preferred, Z13 primary, latency 30 s, cost $0.05
        profiles.insert(
            TaskCategory::Code,
            TaskProfile {
                category: TaskCategory::Code,
                gpu_required: true,
                latency_target_ms: 30_000,
                cost_ceiling_usd: 0.05,
                preferred_nodes: vec!["Z13".into()],
                fallback_nodes: vec!["M1Mac".into(), "AYANEO".into(), "Acer".into()],
            },
        );

        // Think — CPU ok, any node, latency 60 s, cost $0.02
        profiles.insert(
            TaskCategory::Think,
            TaskProfile {
                category: TaskCategory::Think,
                gpu_required: false,
                latency_target_ms: 60_000,
                cost_ceiling_usd: 0.02,
                preferred_nodes: vec![
                    "Z13".into(),
                    "M1Mac".into(),
                    "AYANEO".into(),
                    "Acer".into(),
                ],
                fallback_nodes: vec![],
            },
        );

        // Research — network required, any node, latency 120 s, cost $0.10
        profiles.insert(
            TaskCategory::Research,
            TaskProfile {
                category: TaskCategory::Research,
                gpu_required: false,
                latency_target_ms: 120_000,
                cost_ceiling_usd: 0.10,
                preferred_nodes: vec![
                    "Z13".into(),
                    "M1Mac".into(),
                    "AYANEO".into(),
                    "Acer".into(),
                ],
                fallback_nodes: vec![],
            },
        );

        // Batch — cheapest node, latency 600 s, cost $0.01
        profiles.insert(
            TaskCategory::Batch,
            TaskProfile {
                category: TaskCategory::Batch,
                gpu_required: false,
                latency_target_ms: 600_000,
                cost_ceiling_usd: 0.01,
                preferred_nodes: vec!["Acer".into(), "AYANEO".into()],
                fallback_nodes: vec!["M1Mac".into(), "Z13".into()],
            },
        );

        // Local — Z13 only (file system access), latency 10 s, cost $0.00
        profiles.insert(
            TaskCategory::Local,
            TaskProfile {
                category: TaskCategory::Local,
                gpu_required: false,
                latency_target_ms: 10_000,
                cost_ceiling_usd: 0.0,
                preferred_nodes: vec!["Z13".into()],
                fallback_nodes: vec![],
            },
        );

        // Ops — Hub only, latency 5 s, cost $0.00
        profiles.insert(
            TaskCategory::Ops,
            TaskProfile {
                category: TaskCategory::Ops,
                gpu_required: false,
                latency_target_ms: 5_000,
                cost_ceiling_usd: 0.0,
                preferred_nodes: vec!["Z13".into()],
                fallback_nodes: vec![],
            },
        );

        Self { profiles }
    }

    /// Get the profile for a given category.
    pub fn profile_for(&self, category: &TaskCategory) -> &TaskProfile {
        self.profiles
            .get(category)
            .expect("TaskTaxonomy must have a profile for every TaskCategory")
    }

    /// Convenience: classify then look up the profile in one call.
    pub fn classify_and_profile(
        &self,
        tool_name: &str,
        hand_name: Option<&str>,
    ) -> (&TaskCategory, &TaskProfile) {
        let cat = classify(tool_name, hand_name);
        // We need an owned category but return a reference to the key stored
        // in the map. Since we know the map contains all variants, look it up.
        let profile = self.profiles.get(&cat).unwrap();
        (&profile.category, profile)
    }
}

impl Default for TaskTaxonomy {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// classify — heuristic classifier
// ---------------------------------------------------------------------------

/// Classify a task into a `TaskCategory` based on the tool name and optional
/// hand (workflow) name.  Uses simple substring / keyword matching — fast and
/// deterministic, no LLM call needed.
///
/// Priority: hand name hints take precedence over tool name hints so that the
/// calling workflow context is respected.
pub fn classify(tool_name: &str, hand_name: Option<&str>) -> TaskCategory {
    let tool = tool_name.to_ascii_lowercase();
    let hand = hand_name.map(|h| h.to_ascii_lowercase());

    // --- Hand-level overrides (workflow context) ---
    if let Some(ref h) = hand {
        // Ops / administrative hands
        if h.contains("cluster_health")
            || h.contains("review_agents")
            || h.contains("self_evolve")
            || h.contains("self_optimize")
            || h.contains("cluster_evolve")
        {
            return TaskCategory::Ops;
        }

        // Code-centric hands
        if h.contains("code_gen")
            || h.contains("code_review")
            || h.contains("saas_deploy")
            || h.contains("micro_saas")
            || h.contains("game_dev")
            || h.contains("build_mobile")
            || h.contains("data_pipeline")
        {
            return TaskCategory::Code;
        }

        // Research-oriented hands
        if h.contains("researcher")
            || h.contains("market_intel")
            || h.contains("seo_content")
            || h.contains("lead")
            || h.contains("outreach")
        {
            return TaskCategory::Research;
        }

        // Batch / bulk-output hands
        if h.contains("content")
            || h.contains("novel")
            || h.contains("comic")
            || h.contains("music")
            || h.contains("youtube")
            || h.contains("social_scheduler")
            || h.contains("ecommerce_ops")
        {
            return TaskCategory::Batch;
        }

        // Think / reasoning hands
        if h.contains("trading_analysis")
            || h.contains("product_spec")
            || h.contains("auto_report")
            || h.contains("report")
            || h.contains("customer_service")
            || h.contains("freelancer")
            || h.contains("invoice")
            || h.contains("design")
            || h.contains("prompt_evolve")
        {
            return TaskCategory::Think;
        }
    }

    // --- Tool-level classification ---

    // Ops tools — system administration, health, notifications
    if tool.contains("system_info")
        || tool.contains("notification_center")
        || tool.contains("clipboard")
        || tool.contains("calendar")
    {
        return TaskCategory::Ops;
    }

    // Local file-system tools
    if tool.contains("file_read")
        || tool.contains("file_write")
        || tool.contains("file_edit")
        || tool.contains("glob_search")
        || tool.contains("content_search")
        || tool.contains("screenshot")
        || tool.contains("archive_extract")
        || tool.contains("knowledge_import")
    {
        return TaskCategory::Local;
    }

    // Code / generation tools
    if tool.contains("shell")
        || tool.contains("ai_code")
        || tool.contains("skeleton_generate")
        || tool.contains("scaffold_saas")
        || tool.contains("render_deploy")
        || tool.contains("computer_use")
    {
        return TaskCategory::Code;
    }

    // Research / network tools
    if tool.contains("web_search")
        || tool.contains("http_request")
        || tool.contains("browser")
        || tool.contains("rss_reader")
        || tool.contains("weather")
        || tool.contains("email_receive")
    {
        return TaskCategory::Research;
    }

    // Batch / bulk-output tools
    if tool.contains("blog_publish")
        || tool.contains("pdf_export")
        || tool.contains("docx_export")
        || tool.contains("xlsx_export")
        || tool.contains("image_generate")
        || tool.contains("video_compose")
        || tool.contains("youtube_upload")
        || tool.contains("music_generate")
        || tool.contains("tts")
        || tool.contains("qr_generate")
        || tool.contains("csv_parse")
        || tool.contains("json_transform")
        || tool.contains("data_analysis")
    {
        return TaskCategory::Batch;
    }

    // Communication / messaging tools → Think (need reasoning about content)
    if tool.contains("email")
        || tool.contains("twitter")
        || tool.contains("slack")
        || tool.contains("discord")
        || tool.contains("line_notify")
        || tool.contains("whatsapp")
        || tool.contains("stripe")
    {
        return TaskCategory::Think;
    }

    // Delegate tools
    if tool.contains("delegate") || tool.contains("run_hand") {
        return TaskCategory::Think;
    }

    // Summarize / translate — lightweight reasoning
    if tool.contains("summarize") || tool.contains("translate") || tool.contains("calculator") {
        return TaskCategory::Think;
    }

    // Memory tools — local-ish but lightweight
    if tool.contains("memory") {
        return TaskCategory::Local;
    }

    // Vision tool — GPU-accelerated
    if tool.contains("vision") {
        return TaskCategory::Code;
    }

    // Default: Think (safest fallback — any node, moderate latency)
    TaskCategory::Think
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // TaskCategory basics
    // -----------------------------------------------------------------------

    #[test]
    fn test_category_as_str_roundtrip() {
        for cat in TaskCategory::all() {
            let s = cat.as_str();
            assert!(!s.is_empty(), "as_str must return non-empty for {:?}", cat);
        }
    }

    #[test]
    fn test_all_categories_count() {
        assert_eq!(TaskCategory::all().len(), 6);
    }

    #[test]
    fn test_display_matches_as_str() {
        for cat in TaskCategory::all() {
            assert_eq!(format!("{}", cat), cat.as_str());
        }
    }

    // -----------------------------------------------------------------------
    // TaskTaxonomy — default profiles
    // -----------------------------------------------------------------------

    #[test]
    fn test_taxonomy_has_all_profiles() {
        let tax = TaskTaxonomy::new();
        for cat in TaskCategory::all() {
            let profile = tax.profile_for(cat);
            assert_eq!(profile.category, *cat);
        }
    }

    #[test]
    fn test_code_profile_gpu_required() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Code);
        assert!(p.gpu_required, "Code profile should prefer GPU");
        assert_eq!(p.latency_target_ms, 30_000);
        assert!((p.cost_ceiling_usd - 0.05).abs() < f64::EPSILON);
        assert!(p.preferred_nodes.contains(&"Z13".to_string()));
    }

    #[test]
    fn test_think_profile_no_gpu() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Think);
        assert!(!p.gpu_required);
        assert_eq!(p.latency_target_ms, 60_000);
        assert!((p.cost_ceiling_usd - 0.02).abs() < f64::EPSILON);
        assert_eq!(p.preferred_nodes.len(), 4, "Think should accept any node");
    }

    #[test]
    fn test_research_profile() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Research);
        assert!(!p.gpu_required);
        assert_eq!(p.latency_target_ms, 120_000);
        assert!((p.cost_ceiling_usd - 0.10).abs() < f64::EPSILON);
    }

    #[test]
    fn test_batch_profile_cheapest() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Batch);
        assert_eq!(p.latency_target_ms, 600_000);
        assert!((p.cost_ceiling_usd - 0.01).abs() < f64::EPSILON);
        // Cheapest nodes first
        assert_eq!(p.preferred_nodes[0], "Acer");
    }

    #[test]
    fn test_local_profile_z13_only() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Local);
        assert_eq!(p.preferred_nodes, vec!["Z13".to_string()]);
        assert!(p.fallback_nodes.is_empty(), "Local should have no fallback");
        assert_eq!(p.latency_target_ms, 10_000);
    }

    #[test]
    fn test_ops_profile_hub_only() {
        let tax = TaskTaxonomy::new();
        let p = tax.profile_for(&TaskCategory::Ops);
        assert_eq!(p.preferred_nodes, vec!["Z13".to_string()]);
        assert_eq!(p.latency_target_ms, 5_000);
        assert!((p.cost_ceiling_usd - 0.0).abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // classify — tool-name heuristics
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_code_tools() {
        assert_eq!(classify("shell", None), TaskCategory::Code);
        assert_eq!(classify("ai_code", None), TaskCategory::Code);
        assert_eq!(classify("skeleton_generate", None), TaskCategory::Code);
        assert_eq!(classify("scaffold_saas", None), TaskCategory::Code);
        assert_eq!(classify("render_deploy", None), TaskCategory::Code);
        assert_eq!(classify("computer_use", None), TaskCategory::Code);
    }

    #[test]
    fn test_classify_local_tools() {
        assert_eq!(classify("file_read", None), TaskCategory::Local);
        assert_eq!(classify("file_write", None), TaskCategory::Local);
        assert_eq!(classify("file_edit", None), TaskCategory::Local);
        assert_eq!(classify("glob_search", None), TaskCategory::Local);
        assert_eq!(classify("content_search", None), TaskCategory::Local);
        assert_eq!(classify("screenshot", None), TaskCategory::Local);
        assert_eq!(classify("archive_extract", None), TaskCategory::Local);
        assert_eq!(classify("knowledge_import", None), TaskCategory::Local);
    }

    #[test]
    fn test_classify_research_tools() {
        assert_eq!(classify("web_search", None), TaskCategory::Research);
        assert_eq!(classify("http_request", None), TaskCategory::Research);
        assert_eq!(classify("browser", None), TaskCategory::Research);
        assert_eq!(classify("rss_reader", None), TaskCategory::Research);
        assert_eq!(classify("weather", None), TaskCategory::Research);
        assert_eq!(classify("email_receive", None), TaskCategory::Research);
    }

    #[test]
    fn test_classify_batch_tools() {
        assert_eq!(classify("blog_publish", None), TaskCategory::Batch);
        assert_eq!(classify("pdf_export", None), TaskCategory::Batch);
        assert_eq!(classify("image_generate", None), TaskCategory::Batch);
        assert_eq!(classify("video_compose", None), TaskCategory::Batch);
        assert_eq!(classify("youtube_upload", None), TaskCategory::Batch);
        assert_eq!(classify("tts", None), TaskCategory::Batch);
        assert_eq!(classify("csv_parse", None), TaskCategory::Batch);
    }

    #[test]
    fn test_classify_ops_tools() {
        assert_eq!(classify("system_info", None), TaskCategory::Ops);
        assert_eq!(classify("notification_center", None), TaskCategory::Ops);
        assert_eq!(classify("clipboard", None), TaskCategory::Ops);
        assert_eq!(classify("calendar", None), TaskCategory::Ops);
    }

    #[test]
    fn test_classify_think_tools() {
        assert_eq!(classify("email", None), TaskCategory::Think);
        assert_eq!(classify("twitter", None), TaskCategory::Think);
        assert_eq!(classify("slack", None), TaskCategory::Think);
        assert_eq!(classify("summarize", None), TaskCategory::Think);
        assert_eq!(classify("translate", None), TaskCategory::Think);
        assert_eq!(classify("calculator", None), TaskCategory::Think);
        assert_eq!(classify("delegate", None), TaskCategory::Think);
        assert_eq!(classify("run_hand", None), TaskCategory::Think);
    }

    // -----------------------------------------------------------------------
    // classify — hand-name overrides
    // -----------------------------------------------------------------------

    #[test]
    fn test_hand_overrides_tool_classification() {
        // web_search would normally be Research, but code_gen hand forces Code
        assert_eq!(
            classify("web_search", Some("code_gen")),
            TaskCategory::Code
        );
        // file_read would normally be Local, but researcher hand forces Research
        assert_eq!(
            classify("file_read", Some("researcher")),
            TaskCategory::Research
        );
        // shell would normally be Code, but cluster_health hand forces Ops
        assert_eq!(
            classify("shell", Some("cluster_health")),
            TaskCategory::Ops
        );
    }

    #[test]
    fn test_hand_ops_classification() {
        assert_eq!(classify("any_tool", Some("cluster_health")), TaskCategory::Ops);
        assert_eq!(classify("any_tool", Some("review_agents")), TaskCategory::Ops);
        assert_eq!(classify("any_tool", Some("self_evolve")), TaskCategory::Ops);
        assert_eq!(classify("any_tool", Some("self_optimize")), TaskCategory::Ops);
        assert_eq!(classify("any_tool", Some("cluster_evolve")), TaskCategory::Ops);
    }

    #[test]
    fn test_hand_research_classification() {
        assert_eq!(classify("any_tool", Some("researcher")), TaskCategory::Research);
        assert_eq!(classify("any_tool", Some("market_intel")), TaskCategory::Research);
        assert_eq!(classify("any_tool", Some("seo_content")), TaskCategory::Research);
        assert_eq!(classify("any_tool", Some("lead")), TaskCategory::Research);
        assert_eq!(classify("any_tool", Some("outreach")), TaskCategory::Research);
    }

    #[test]
    fn test_hand_batch_classification() {
        assert_eq!(classify("any_tool", Some("content")), TaskCategory::Batch);
        assert_eq!(classify("any_tool", Some("novel")), TaskCategory::Batch);
        assert_eq!(classify("any_tool", Some("comic")), TaskCategory::Batch);
        assert_eq!(classify("any_tool", Some("music")), TaskCategory::Batch);
        assert_eq!(classify("any_tool", Some("youtube")), TaskCategory::Batch);
    }

    #[test]
    fn test_hand_think_classification() {
        assert_eq!(classify("any_tool", Some("trading_analysis")), TaskCategory::Think);
        assert_eq!(classify("any_tool", Some("auto_report")), TaskCategory::Think);
        assert_eq!(classify("any_tool", Some("customer_service")), TaskCategory::Think);
        assert_eq!(classify("any_tool", Some("freelancer")), TaskCategory::Think);
        assert_eq!(classify("any_tool", Some("invoice")), TaskCategory::Think);
    }

    // -----------------------------------------------------------------------
    // classify — default / unknown
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_unknown_defaults_to_think() {
        assert_eq!(classify("totally_unknown_tool", None), TaskCategory::Think);
        assert_eq!(classify("", None), TaskCategory::Think);
    }

    // -----------------------------------------------------------------------
    // classify_and_profile convenience
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_and_profile() {
        let tax = TaskTaxonomy::new();
        let (cat, profile) = tax.classify_and_profile("ai_code", None);
        assert_eq!(*cat, TaskCategory::Code);
        assert!(profile.gpu_required);
    }

    // -----------------------------------------------------------------------
    // Case-insensitivity
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_case_insensitive() {
        assert_eq!(classify("Web_Search", None), TaskCategory::Research);
        assert_eq!(classify("FILE_READ", None), TaskCategory::Local);
        assert_eq!(classify("AI_CODE", None), TaskCategory::Code);
        assert_eq!(classify("Shell", Some("Code_Gen")), TaskCategory::Code);
    }

    // -----------------------------------------------------------------------
    // Serde round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_category_serde_roundtrip() {
        for cat in TaskCategory::all() {
            let json = serde_json::to_string(cat).unwrap();
            let back: TaskCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(*cat, back);
        }
    }

    #[test]
    fn test_profile_serde_roundtrip() {
        let tax = TaskTaxonomy::new();
        let profile = tax.profile_for(&TaskCategory::Code);
        let json = serde_json::to_string(profile).unwrap();
        let back: TaskProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.category, TaskCategory::Code);
        assert!(back.gpu_required);
        assert_eq!(back.latency_target_ms, 30_000);
    }
}
