//! UserProfile data model with TOML and SQLite persistence.
//!
//! Single-row profile (id=1) storing user preferences, persona config,
//! and alert thresholds. Foundation for persona injection, alerting,
//! timezone-aware scheduling, and system prompt context.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Enums ──────────────────────────────────────────────────────────────

/// How proactively the agent should act without user prompting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactivityLevel {
    Passive,
    Moderate,
    Active,
    Autonomous,
}

impl Default for ProactivityLevel {
    fn default() -> Self {
        Self::Autonomous
    }
}

// ── PersonaConfig ──────────────────────────────────────────────────────

/// Butler/persona identity that shapes the agent's tone and behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonaConfig {
    pub name: String,
    pub style: String,
    pub tone_instructions: String,
    pub proactivity: ProactivityLevel,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            name: "Alfred".to_string(),
            style: "formal-butler".to_string(),
            tone_instructions: "語氣正式簡潔，稱呼用戶為「先生」，自稱用第一人稱。\n\
                能自己處理的事先做完再報告。主動建議時措辭得體，不囉嗦。\n\
                桌面端可用結構化報告格式，Telegram 用簡短訊息。"
                .to_string(),
            proactivity: ProactivityLevel::Autonomous,
        }
    }
}

// ── AlertThresholds ────────────────────────────────────────────────────

/// Thresholds that trigger proactive alerts from the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertThresholds {
    pub deadline_warn_days: u32,
    pub streak_break_days: u32,
    pub budget_warn_percent: f64,
    pub task_failure_count: u32,
    pub idle_days: u32,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            deadline_warn_days: 3,
            streak_break_days: 2,
            budget_warn_percent: 80.0,
            task_failure_count: 3,
            idle_days: 3,
        }
    }
}

// ── UserProfile ────────────────────────────────────────────────────────

/// Top-level user profile with persona, locale, timezone, and alert settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    pub display_name: String,
    pub locale: String,
    pub timezone: String,
    pub persona: PersonaConfig,
    pub alert_thresholds: AlertThresholds,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            display_name: "先生".to_string(),
            locale: "zh-TW".to_string(),
            timezone: "Asia/Taipei".to_string(),
            persona: PersonaConfig::default(),
            alert_thresholds: AlertThresholds::default(),
        }
    }
}

impl UserProfile {
    /// Parse the IANA timezone string into a `chrono_tz::Tz`.
    pub fn parsed_timezone(&self) -> Result<Tz> {
        self.timezone
            .parse::<Tz>()
            .map_err(|e| anyhow!("invalid timezone '{}': {}", self.timezone, e))
    }

    /// Convert a UTC datetime to the user's local timezone.
    pub fn to_local_time(&self, utc: DateTime<Utc>) -> Result<DateTime<Tz>> {
        let tz = self.parsed_timezone()?;
        Ok(utc.with_timezone(&tz))
    }

    /// Generate a system-prompt context block that can be injected into LLM prompts.
    pub fn system_prompt_context(&self) -> String {
        let now_local = self
            .to_local_time(Utc::now())
            .map(|dt| dt.format("%Y-%m-%d %H:%M %Z").to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let body = format!(
            "You are {name}, a {style} AI butler serving {display_name}.\n\
             Locale: {locale} | Timezone: {tz} | Local time: {now}\n\
             Proactivity: {proactivity}",
            name = self.persona.name,
            style = self.persona.style,
            display_name = self.display_name,
            locale = self.locale,
            tz = self.timezone,
            now = now_local,
            proactivity = serde_json::to_string(&self.persona.proactivity)
                .unwrap_or_default()
                .trim_matches('"'),
        );

        if self.persona.tone_instructions.is_empty() {
            body
        } else {
            format!("{}\n{}", self.persona.tone_instructions, body)
        }
    }

    // ── SQLite persistence ─────────────────────────────────────────────

    /// Create the `user_profile` table if it does not exist.
    pub fn create_table(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_profile (
                id          INTEGER PRIMARY KEY CHECK (id = 1),
                display_name TEXT NOT NULL,
                locale       TEXT NOT NULL,
                timezone     TEXT NOT NULL,
                persona_json TEXT NOT NULL,
                alerts_json  TEXT NOT NULL
            );"
        )?;
        Ok(())
    }

    /// UPSERT the profile as a single row (id=1).
    pub fn save(&self, conn: &Connection) -> Result<()> {
        let persona_json = serde_json::to_string(&self.persona)?;
        let alerts_json = serde_json::to_string(&self.alert_thresholds)?;
        conn.execute(
            "INSERT OR REPLACE INTO user_profile
             (id, display_name, locale, timezone, persona_json, alerts_json)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                self.display_name,
                self.locale,
                self.timezone,
                persona_json,
                alerts_json,
            ],
        )?;
        Ok(())
    }

    /// Load the profile from SQLite. Returns `None` if no row exists.
    pub fn load(conn: &Connection) -> Result<Option<Self>> {
        let mut stmt = conn.prepare(
            "SELECT display_name, locale, timezone, persona_json, alerts_json
             FROM user_profile WHERE id = 1"
        )?;

        let mut rows = stmt.query_map([], |row| {
            let display_name: String = row.get(0)?;
            let locale: String = row.get(1)?;
            let timezone: String = row.get(2)?;
            let persona_json: String = row.get(3)?;
            let alerts_json: String = row.get(4)?;
            Ok((display_name, locale, timezone, persona_json, alerts_json))
        })?;

        match rows.next() {
            Some(Ok((display_name, locale, timezone, persona_json, alerts_json))) => {
                let persona: PersonaConfig = serde_json::from_str(&persona_json)?;
                let alert_thresholds: AlertThresholds = serde_json::from_str(&alerts_json)?;
                Ok(Some(Self {
                    display_name,
                    locale,
                    timezone,
                    persona,
                    alert_thresholds,
                }))
            }
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Update a single field in the profile row. Field name is validated
    /// against an allowlist to prevent SQL injection.
    pub fn update_field(conn: &Connection, field: &str, value: &str) -> Result<()> {
        // Allowlist of directly-stored columns
        const ALLOWED_FIELDS: &[&str] = &[
            "display_name",
            "locale",
            "timezone",
            "persona_json",
            "alerts_json",
        ];

        if !ALLOWED_FIELDS.contains(&field) {
            return Err(anyhow!(
                "update_field: '{}' is not an allowed field (allowed: {:?})",
                field,
                ALLOWED_FIELDS
            ));
        }

        // Build query with validated field name — value is still parameterised.
        let sql = format!("UPDATE user_profile SET {} = ?1 WHERE id = 1", field);
        let changed = conn.execute(&sql, params![value])?;
        if changed == 0 {
            return Err(anyhow!("update_field: no profile row to update (save first)"));
        }
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    // Helper: open an in-memory SQLite connection with the table created.
    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        UserProfile::create_table(&conn).unwrap();
        conn
    }

    #[test]
    fn test_proactivity_level_serialization() {
        let json = serde_json::to_string(&ProactivityLevel::Autonomous).unwrap();
        assert_eq!(json, "\"autonomous\"");
        let back: ProactivityLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ProactivityLevel::Autonomous);
    }

    #[test]
    fn test_default_alert_thresholds() {
        let a = AlertThresholds::default();
        assert_eq!(a.deadline_warn_days, 3);
        assert_eq!(a.streak_break_days, 2);
        assert!((a.budget_warn_percent - 80.0).abs() < f64::EPSILON);
        assert_eq!(a.task_failure_count, 3);
        assert_eq!(a.idle_days, 3);
    }

    #[test]
    fn test_default_persona_config() {
        let p = PersonaConfig::default();
        assert_eq!(p.name, "Alfred");
        assert_eq!(p.style, "formal-butler");
        assert_eq!(p.proactivity, ProactivityLevel::Autonomous);
    }

    #[test]
    fn test_default_user_profile() {
        let u = UserProfile::default();
        assert_eq!(u.display_name, "先生");
        assert_eq!(u.locale, "zh-TW");
        assert_eq!(u.timezone, "Asia/Taipei");
    }

    #[test]
    fn test_toml_roundtrip() {
        let original = UserProfile::default();
        let toml_str = toml::to_string(&original).unwrap();
        let restored: UserProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(original.display_name, restored.display_name);
        assert_eq!(original.locale, restored.locale);
        assert_eq!(original.timezone, restored.timezone);
        assert_eq!(original.persona.name, restored.persona.name);
        assert_eq!(original.persona.style, restored.persona.style);
        assert_eq!(original.persona.proactivity, restored.persona.proactivity);
        assert_eq!(
            original.alert_thresholds.deadline_warn_days,
            restored.alert_thresholds.deadline_warn_days
        );
    }

    #[test]
    fn test_timezone_conversion() {
        let profile = UserProfile::default(); // Asia/Taipei
        let utc = Utc.with_ymd_and_hms(2026, 3, 26, 10, 0, 0).unwrap();
        let local = profile.to_local_time(utc).unwrap();
        assert_eq!(local.hour(), 18);
        assert_eq!(local.format("%Z").to_string(), "CST");
    }

    #[test]
    fn test_parse_timezone() {
        let profile = UserProfile::default();
        let tz = profile.parsed_timezone().unwrap();
        assert_eq!(tz, chrono_tz::Asia::Taipei);
    }

    #[test]
    fn test_sqlite_roundtrip() {
        let conn = test_conn();
        let original = UserProfile::default();
        original.save(&conn).unwrap();

        let loaded = UserProfile::load(&conn).unwrap().expect("should have a row");
        assert_eq!(loaded.display_name, "先生");
        assert_eq!(loaded.locale, "zh-TW");
        assert_eq!(loaded.timezone, "Asia/Taipei");
        assert_eq!(loaded.persona.name, "Alfred");
        assert_eq!(loaded.persona.style, "formal-butler");
        assert_eq!(loaded.persona.proactivity, ProactivityLevel::Autonomous);
        assert_eq!(loaded.alert_thresholds.deadline_warn_days, 3);
        assert_eq!(loaded.alert_thresholds.streak_break_days, 2);
        assert!((loaded.alert_thresholds.budget_warn_percent - 80.0).abs() < f64::EPSILON);
        assert_eq!(loaded.alert_thresholds.task_failure_count, 3);
        assert_eq!(loaded.alert_thresholds.idle_days, 3);
    }

    #[test]
    fn test_sqlite_load_empty() {
        let conn = test_conn();
        let loaded = UserProfile::load(&conn).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_system_prompt_context() {
        let profile = UserProfile::default();
        let ctx = profile.system_prompt_context();
        assert!(ctx.contains("Alfred"), "should contain butler name");
        assert!(ctx.contains("先生"), "should contain display_name");
        assert!(ctx.contains("Asia/Taipei"), "should contain timezone");
        assert!(ctx.contains("zh-TW"), "should contain locale");
    }

    #[test]
    fn test_update_field_timezone() {
        let conn = test_conn();
        let profile = UserProfile::default();
        profile.save(&conn).unwrap();

        UserProfile::update_field(&conn, "timezone", "America/New_York").unwrap();
        let loaded = UserProfile::load(&conn).unwrap().expect("should have a row");
        assert_eq!(loaded.timezone, "America/New_York");
    }

    #[test]
    fn test_system_prompt_context_contains_all_fields() {
        let profile = UserProfile {
            display_name: "Master".to_string(),
            locale: "en".to_string(),
            timezone: "America/New_York".to_string(),
            persona: PersonaConfig {
                name: "Jeeves".to_string(),
                style: "formal-butler".to_string(),
                tone_instructions: String::new(),
                proactivity: ProactivityLevel::Autonomous,
            },
            alert_thresholds: AlertThresholds::default(),
        };
        let ctx = profile.system_prompt_context();
        assert!(ctx.contains("Jeeves"), "should contain persona name");
        assert!(ctx.contains("Master"), "should contain display_name");
        assert!(ctx.contains("en"), "should contain locale");
        assert!(ctx.contains("America/New_York"), "should contain timezone");
    }

    #[test]
    fn test_tone_instructions_in_system_prompt() {
        let profile = UserProfile {
            display_name: "Boss".to_string(),
            locale: "en".to_string(),
            timezone: "UTC".to_string(),
            persona: PersonaConfig {
                name: "Smithers".to_string(),
                style: "formal-butler".to_string(),
                tone_instructions: "Always be concise and direct.".to_string(),
                proactivity: ProactivityLevel::Moderate,
            },
            alert_thresholds: AlertThresholds::default(),
        };
        let ctx = profile.system_prompt_context();
        assert!(
            ctx.starts_with("Always be concise and direct."),
            "tone_instructions should be prepended to system prompt; got: {}",
            ctx,
        );
        assert!(ctx.contains("Smithers"), "should still contain persona name");
        assert!(ctx.contains("Boss"), "should still contain display_name");
    }

    #[test]
    fn test_default_tone_instructions_not_empty() {
        let p = PersonaConfig::default();
        assert!(
            !p.tone_instructions.is_empty(),
            "default tone_instructions must not be empty"
        );
        assert!(
            p.tone_instructions.contains("語氣正式簡潔"),
            "default tone_instructions should contain the spec string"
        );
    }
}
