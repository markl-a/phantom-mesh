//! Provider Pricing Store - runtime-editable LLM/API pricing rules.
//!
//! This replaces hardcoded runtime assumptions with a small SQLite-backed rule
//! set that can be changed while the daemon is running.

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::cost_tracker::estimate_cost;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPriceRule {
    pub provider: String,
    pub model_pattern: String,
    pub input_usd_per_1m_tokens: f64,
    pub output_usd_per_1m_tokens: f64,
    pub notes: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPriceEstimate {
    pub provider: String,
    pub model: String,
    pub matched_pattern: Option<String>,
    pub source: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub input_usd: f64,
    pub output_usd: f64,
    pub total_usd: f64,
}

pub struct ProviderPricingStore {
    conn: Mutex<Connection>,
}

impl ProviderPricingStore {
    pub fn new(db_path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_price_rules (
                provider                   TEXT NOT NULL,
                model_pattern              TEXT NOT NULL,
                input_usd_per_1m_tokens    REAL NOT NULL DEFAULT 0.0,
                output_usd_per_1m_tokens   REAL NOT NULL DEFAULT 0.0,
                notes                      TEXT,
                updated_at                 TEXT NOT NULL,
                PRIMARY KEY (provider, model_pattern)
            );
            CREATE INDEX IF NOT EXISTS idx_provider_price_provider
                ON provider_price_rules(provider);",
        )?;

        let me = Self {
            conn: Mutex::new(conn),
        };
        me.ensure_default_rules()?;
        Ok(me)
    }

    pub fn upsert_rule(&self, rule: &ProviderPriceRule) -> Result<()> {
        Self::validate_rule(rule)?;
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_price_rules
             (provider, model_pattern, input_usd_per_1m_tokens, output_usd_per_1m_tokens, notes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider, model_pattern) DO UPDATE SET
                input_usd_per_1m_tokens = excluded.input_usd_per_1m_tokens,
                output_usd_per_1m_tokens = excluded.output_usd_per_1m_tokens,
                notes = excluded.notes,
                updated_at = excluded.updated_at",
            params![
                rule.provider.trim().to_lowercase(),
                rule.model_pattern.trim().to_lowercase(),
                rule.input_usd_per_1m_tokens,
                rule.output_usd_per_1m_tokens,
                rule.notes,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_rule(&self, provider: &str, model_pattern: &str) -> Result<Option<ProviderPriceRule>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT provider, model_pattern, input_usd_per_1m_tokens, output_usd_per_1m_tokens, notes, updated_at
             FROM provider_price_rules
             WHERE provider = ?1 AND model_pattern = ?2",
            params![provider.trim().to_lowercase(), model_pattern.trim().to_lowercase()],
            |row| {
                Ok(ProviderPriceRule {
                    provider: row.get(0)?,
                    model_pattern: row.get(1)?,
                    input_usd_per_1m_tokens: row.get(2)?,
                    output_usd_per_1m_tokens: row.get(3)?,
                    notes: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_rules(&self) -> Result<Vec<ProviderPriceRule>> {
        self.list_rules_for_provider(None)
    }

    pub fn list_rules_for_provider(&self, provider: Option<&str>) -> Result<Vec<ProviderPriceRule>> {
        let conn = self.conn.lock().unwrap();
        if let Some(provider) = provider {
            let mut stmt = conn.prepare(
                "SELECT provider, model_pattern, input_usd_per_1m_tokens, output_usd_per_1m_tokens, notes, updated_at
                 FROM provider_price_rules
                 WHERE provider = ?1
                 ORDER BY provider ASC,
                          CASE WHEN model_pattern = '*' THEN 0 ELSE LENGTH(model_pattern) END DESC,
                          model_pattern ASC",
            )?;
            let rows = stmt.query_map([provider.trim().to_lowercase()], |row| {
                Ok(ProviderPriceRule {
                    provider: row.get(0)?,
                    model_pattern: row.get(1)?,
                    input_usd_per_1m_tokens: row.get(2)?,
                    output_usd_per_1m_tokens: row.get(3)?,
                    notes: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        } else {
            let mut stmt = conn.prepare(
                "SELECT provider, model_pattern, input_usd_per_1m_tokens, output_usd_per_1m_tokens, notes, updated_at
                 FROM provider_price_rules
                 ORDER BY provider ASC,
                          CASE WHEN model_pattern = '*' THEN 0 ELSE LENGTH(model_pattern) END DESC,
                          model_pattern ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ProviderPriceRule {
                    provider: row.get(0)?,
                    model_pattern: row.get(1)?,
                    input_usd_per_1m_tokens: row.get(2)?,
                    output_usd_per_1m_tokens: row.get(3)?,
                    notes: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        }
    }

    pub fn estimate_cost(
        &self,
        provider: &str,
        model: &str,
        tokens_in: u32,
        tokens_out: u32,
    ) -> Result<ProviderPriceEstimate> {
        let provider_normalized = provider.trim().to_lowercase();
        let model_normalized = model.trim().to_lowercase();

        let rules = self.list_rules_for_provider(Some(&provider_normalized))?;
        if let Some(rule) = rules.into_iter().find(|rule| Self::pattern_matches(&rule.model_pattern, &model_normalized)) {
            let input_usd =
                (tokens_in as f64 * rule.input_usd_per_1m_tokens) / 1_000_000.0;
            let output_usd =
                (tokens_out as f64 * rule.output_usd_per_1m_tokens) / 1_000_000.0;
            return Ok(ProviderPriceEstimate {
                provider: provider_normalized,
                model: model.to_string(),
                matched_pattern: Some(rule.model_pattern),
                source: "pricing_store".to_string(),
                tokens_in,
                tokens_out,
                input_usd,
                output_usd,
                total_usd: input_usd + output_usd,
            });
        }

        let total_usd = estimate_cost(provider, model, tokens_in, tokens_out);
        Ok(ProviderPriceEstimate {
            provider: provider_normalized,
            model: model.to_string(),
            matched_pattern: None,
            source: "builtin_fallback".to_string(),
            tokens_in,
            tokens_out,
            input_usd: 0.0,
            output_usd: 0.0,
            total_usd,
        })
    }

    fn ensure_default_rules(&self) -> Result<()> {
        for rule in Self::default_rules() {
            if self.get_rule(&rule.provider, &rule.model_pattern)?.is_none() {
                self.upsert_rule(&rule)?;
            }
        }
        Ok(())
    }

    fn default_rules() -> Vec<ProviderPriceRule> {
        let now = Utc::now().to_rfc3339();
        vec![
            Self::rule("anthropic", "opus", 15.0, 75.0, "Approximate default Anthropic Opus pricing.", &now),
            Self::rule("anthropic", "sonnet", 3.0, 15.0, "Approximate default Anthropic Sonnet pricing.", &now),
            Self::rule("anthropic", "haiku", 0.25, 1.25, "Approximate default Anthropic Haiku pricing.", &now),
            Self::rule("anthropic", "*", 3.0, 15.0, "Anthropic fallback pricing.", &now),
            Self::rule("openai", "gpt-4o", 2.5, 10.0, "Approximate default OpenAI GPT-4o pricing.", &now),
            Self::rule("openai", "gpt-4", 10.0, 30.0, "Approximate default OpenAI GPT-4 pricing.", &now),
            Self::rule("openai", "gpt-3.5", 0.5, 1.5, "Approximate default OpenAI GPT-3.5 pricing.", &now),
            Self::rule("openai", "o1", 15.0, 60.0, "Approximate default OpenAI o1 pricing.", &now),
            Self::rule("openai", "*", 2.5, 10.0, "OpenAI fallback pricing.", &now),
            Self::rule("deepseek", "*", 0.14, 0.28, "Approximate default DeepSeek pricing.", &now),
            Self::rule("together", "*", 0.88, 0.88, "Approximate default Together pricing.", &now),
            Self::rule("openrouter", "*", 0.40, 0.40, "Approximate default OpenRouter pricing.", &now),
            Self::rule("gemini", "*", 0.0, 0.0, "Free-tier or manually-managed Gemini pricing.", &now),
            Self::rule("groq", "*", 0.0, 0.0, "Free-tier or manually-managed Groq pricing.", &now),
            Self::rule("cerebras", "*", 0.0, 0.0, "Free-tier or manually-managed Cerebras pricing.", &now),
            Self::rule("ollama", "*", 0.0, 0.0, "Local model marginal API cost.", &now),
            Self::rule("lmstudio", "*", 0.0, 0.0, "Local model marginal API cost.", &now),
            Self::rule("lemonade", "*", 0.0, 0.0, "Local model marginal API cost.", &now),
        ]
    }

    fn rule(
        provider: &str,
        model_pattern: &str,
        input_usd_per_1m_tokens: f64,
        output_usd_per_1m_tokens: f64,
        notes: &str,
        updated_at: &str,
    ) -> ProviderPriceRule {
        ProviderPriceRule {
            provider: provider.to_string(),
            model_pattern: model_pattern.to_string(),
            input_usd_per_1m_tokens,
            output_usd_per_1m_tokens,
            notes: Some(notes.to_string()),
            updated_at: updated_at.to_string(),
        }
    }

    fn validate_rule(rule: &ProviderPriceRule) -> Result<()> {
        if rule.provider.trim().is_empty() {
            bail!("provider must not be empty");
        }
        if rule.model_pattern.trim().is_empty() {
            bail!("model_pattern must not be empty");
        }
        if rule.input_usd_per_1m_tokens < 0.0 {
            bail!("input_usd_per_1m_tokens must be >= 0");
        }
        if rule.output_usd_per_1m_tokens < 0.0 {
            bail!("output_usd_per_1m_tokens must be >= 0");
        }
        Ok(())
    }

    fn pattern_matches(model_pattern: &str, model: &str) -> bool {
        let pattern = model_pattern.trim().to_lowercase();
        pattern == "*" || model.contains(&pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_rule_match_longest_pattern() {
        let store = ProviderPricingStore::new(":memory:").unwrap();
        let estimate = store
            .estimate_cost("openai", "gpt-4o-mini", 1000, 500)
            .unwrap();
        assert_eq!(estimate.source, "pricing_store");
        assert_eq!(estimate.matched_pattern.as_deref(), Some("gpt-4o"));
        assert!(estimate.total_usd > 0.0);
    }

    #[test]
    fn test_upsert_override_to_zero_cost() {
        let store = ProviderPricingStore::new(":memory:").unwrap();
        store
            .upsert_rule(&ProviderPriceRule {
                provider: "openai".to_string(),
                model_pattern: "gpt-4o".to_string(),
                input_usd_per_1m_tokens: 0.0,
                output_usd_per_1m_tokens: 0.0,
                notes: Some("Subscription bucket".to_string()),
                updated_at: Utc::now().to_rfc3339(),
            })
            .unwrap();

        let estimate = store
            .estimate_cost("openai", "gpt-4o", 5000, 5000)
            .unwrap();
        assert_eq!(estimate.total_usd, 0.0);
    }

    #[test]
    fn test_unknown_provider_falls_back() {
        let store = ProviderPricingStore::new(":memory:").unwrap();
        let estimate = store
            .estimate_cost("unknown", "mystery", 1000, 1000)
            .unwrap();
        assert_eq!(estimate.source, "builtin_fallback");
        assert_eq!(estimate.total_usd, 0.0);
    }
}
