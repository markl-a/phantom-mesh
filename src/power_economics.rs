//! Power Economics — persistent per-node hardware cost assumptions and
//! profitability helpers for local cluster execution.
//!
//! This module models the cost of running local hardware using:
//! - electricity cost (USD per kWh)
//! - depreciation cost (USD per hour)
//! - cooling overhead (USD per hour)
//!
//! It intentionally does not try to sample live wattage from the OS.
//! Instead it stores editable assumptions per node and produces consistent
//! estimates that higher-level scheduling can rely on.

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePowerProfile {
    pub node_id: String,
    pub idle_watts: f64,
    pub active_watts: f64,
    pub electricity_usd_per_kwh: f64,
    pub depreciation_usd_per_hour: f64,
    pub cooling_usd_per_hour: f64,
    pub notes: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyNodeCost {
    pub node_id: String,
    pub load_factor: f64,
    pub avg_watts: f64,
    pub electricity_usd_per_hour: f64,
    pub depreciation_usd_per_hour: f64,
    pub cooling_usd_per_hour: f64,
    pub total_usd_per_hour: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerCostEstimate {
    pub node_id: String,
    pub duration_secs: f64,
    pub load_factor: f64,
    pub avg_watts: f64,
    pub electricity_usd: f64,
    pub depreciation_usd: f64,
    pub cooling_usd: f64,
    pub total_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitabilityAssessment {
    pub node_id: String,
    pub load_factor: f64,
    pub expected_revenue_per_hour_usd: f64,
    pub api_cost_per_hour_usd: f64,
    pub avg_watts: f64,
    pub electricity_usd_per_hour: f64,
    pub depreciation_usd_per_hour: f64,
    pub cooling_usd_per_hour: f64,
    pub node_cost_usd_per_hour: f64,
    pub break_even_revenue_usd_per_hour: f64,
    pub aggressive_utilization_floor_usd_per_hour: f64,
    pub projected_profit_usd_per_hour: f64,
    pub should_run: bool,
    pub should_saturate: bool,
}

pub struct PowerEconomics {
    conn: Mutex<Connection>,
}

impl PowerEconomics {
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS node_power_profiles (
                    node_id                    TEXT PRIMARY KEY,
                    idle_watts                 REAL NOT NULL,
                    active_watts               REAL NOT NULL,
                    electricity_usd_per_kwh    REAL NOT NULL DEFAULT 0.10,
                    depreciation_usd_per_hour  REAL NOT NULL DEFAULT 0.0,
                    cooling_usd_per_hour       REAL NOT NULL DEFAULT 0.0,
                    notes                      TEXT,
                    updated_at                 TEXT NOT NULL
                );",
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        let me = Self {
            conn: Mutex::new(conn),
        };
        me.ensure_default_profiles()?;
        Ok(me)
    }

    pub fn upsert_profile(&self, profile: &NodePowerProfile) -> Result<()> {
        Self::validate_profile(profile)?;

        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO node_power_profiles
             (node_id, idle_watts, active_watts, electricity_usd_per_kwh,
              depreciation_usd_per_hour, cooling_usd_per_hour, notes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(node_id) DO UPDATE SET
                idle_watts = excluded.idle_watts,
                active_watts = excluded.active_watts,
                electricity_usd_per_kwh = excluded.electricity_usd_per_kwh,
                depreciation_usd_per_hour = excluded.depreciation_usd_per_hour,
                cooling_usd_per_hour = excluded.cooling_usd_per_hour,
                notes = excluded.notes,
                updated_at = excluded.updated_at",
            params![
                profile.node_id,
                profile.idle_watts,
                profile.active_watts,
                profile.electricity_usd_per_kwh,
                profile.depreciation_usd_per_hour,
                profile.cooling_usd_per_hour,
                profile.notes,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_profile(&self, node_id: &str) -> Result<Option<NodePowerProfile>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT node_id, idle_watts, active_watts, electricity_usd_per_kwh,
                    depreciation_usd_per_hour, cooling_usd_per_hour, notes, updated_at
             FROM node_power_profiles
             WHERE node_id = ?1",
            params![node_id],
            |row| {
                Ok(NodePowerProfile {
                    node_id: row.get(0)?,
                    idle_watts: row.get(1)?,
                    active_watts: row.get(2)?,
                    electricity_usd_per_kwh: row.get(3)?,
                    depreciation_usd_per_hour: row.get(4)?,
                    cooling_usd_per_hour: row.get(5)?,
                    notes: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_profiles(&self) -> Result<Vec<NodePowerProfile>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT node_id, idle_watts, active_watts, electricity_usd_per_kwh,
                    depreciation_usd_per_hour, cooling_usd_per_hour, notes, updated_at
             FROM node_power_profiles
             ORDER BY node_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NodePowerProfile {
                node_id: row.get(0)?,
                idle_watts: row.get(1)?,
                active_watts: row.get(2)?,
                electricity_usd_per_kwh: row.get(3)?,
                depreciation_usd_per_hour: row.get(4)?,
                cooling_usd_per_hour: row.get(5)?,
                notes: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn estimate_hourly_cost(&self, node_id: &str, load_factor: f64) -> Result<HourlyNodeCost> {
        let profile = self
            .get_profile(node_id)?
            .ok_or_else(|| anyhow::anyhow!("Unknown power profile for node '{}'", node_id))?;
        Ok(Self::hourly_cost_from_profile(&profile, load_factor))
    }

    pub fn estimate_run_cost(
        &self,
        node_id: &str,
        duration_secs: f64,
        load_factor: f64,
    ) -> Result<PowerCostEstimate> {
        if duration_secs < 0.0 {
            bail!("duration_secs must be >= 0");
        }
        let hourly = self.estimate_hourly_cost(node_id, load_factor)?;
        let duration_hours = duration_secs / 3600.0;
        Ok(PowerCostEstimate {
            node_id: node_id.to_string(),
            duration_secs,
            load_factor: hourly.load_factor,
            avg_watts: hourly.avg_watts,
            electricity_usd: hourly.electricity_usd_per_hour * duration_hours,
            depreciation_usd: hourly.depreciation_usd_per_hour * duration_hours,
            cooling_usd: hourly.cooling_usd_per_hour * duration_hours,
            total_usd: hourly.total_usd_per_hour * duration_hours,
        })
    }

    pub fn assess_profitability(
        &self,
        node_id: &str,
        expected_revenue_per_hour_usd: f64,
        api_cost_per_hour_usd: f64,
        load_factor: f64,
    ) -> Result<ProfitabilityAssessment> {
        let hourly = self.estimate_hourly_cost(node_id, load_factor)?;
        let node_cost = hourly.total_usd_per_hour;
        let break_even = api_cost_per_hour_usd + node_cost;
        let aggressive_floor = api_cost_per_hour_usd
            + hourly.depreciation_usd_per_hour
            + hourly.cooling_usd_per_hour
            + (hourly.electricity_usd_per_hour * 2.0);
        let projected_profit = expected_revenue_per_hour_usd - break_even;

        Ok(ProfitabilityAssessment {
            node_id: node_id.to_string(),
            load_factor: hourly.load_factor,
            expected_revenue_per_hour_usd,
            api_cost_per_hour_usd,
            avg_watts: hourly.avg_watts,
            electricity_usd_per_hour: hourly.electricity_usd_per_hour,
            depreciation_usd_per_hour: hourly.depreciation_usd_per_hour,
            cooling_usd_per_hour: hourly.cooling_usd_per_hour,
            node_cost_usd_per_hour: node_cost,
            break_even_revenue_usd_per_hour: break_even,
            aggressive_utilization_floor_usd_per_hour: aggressive_floor,
            projected_profit_usd_per_hour: projected_profit,
            should_run: expected_revenue_per_hour_usd > break_even,
            should_saturate: expected_revenue_per_hour_usd >= aggressive_floor,
        })
    }

    fn ensure_default_profiles(&self) -> Result<()> {
        for profile in Self::default_profiles() {
            if self.get_profile(&profile.node_id)?.is_none() {
                self.upsert_profile(&profile)?;
            }
        }
        Ok(())
    }

    fn default_profiles() -> Vec<NodePowerProfile> {
        let now = Utc::now().to_rfc3339();
        vec![
            NodePowerProfile {
                node_id: "local".to_string(),
                idle_watts: 20.0,
                active_watts: 65.0,
                electricity_usd_per_kwh: 0.10,
                depreciation_usd_per_hour: 0.030,
                cooling_usd_per_hour: 0.010,
                notes: Some("Default hub profile; adjust to match the actual host.".to_string()),
                updated_at: now.clone(),
            },
            NodePowerProfile {
                node_id: "Z13".to_string(),
                idle_watts: 18.0,
                active_watts: 70.0,
                electricity_usd_per_kwh: 0.10,
                depreciation_usd_per_hour: 0.050,
                cooling_usd_per_hour: 0.015,
                notes: Some("High-performance mobile workstation profile.".to_string()),
                updated_at: now.clone(),
            },
            NodePowerProfile {
                node_id: "M1Mac".to_string(),
                idle_watts: 7.0,
                active_watts: 28.0,
                electricity_usd_per_kwh: 0.10,
                depreciation_usd_per_hour: 0.020,
                cooling_usd_per_hour: 0.005,
                notes: Some("Efficient desktop/mini profile.".to_string()),
                updated_at: now.clone(),
            },
            NodePowerProfile {
                node_id: "AYANEO".to_string(),
                idle_watts: 10.0,
                active_watts: 30.0,
                electricity_usd_per_kwh: 0.10,
                depreciation_usd_per_hour: 0.020,
                cooling_usd_per_hour: 0.008,
                notes: Some("Portable handheld/low-power worker profile.".to_string()),
                updated_at: now.clone(),
            },
            NodePowerProfile {
                node_id: "Acer".to_string(),
                idle_watts: 12.0,
                active_watts: 35.0,
                electricity_usd_per_kwh: 0.10,
                depreciation_usd_per_hour: 0.018,
                cooling_usd_per_hour: 0.008,
                notes: Some("Budget laptop or mini-PC batch worker profile.".to_string()),
                updated_at: now,
            },
        ]
    }

    fn validate_profile(profile: &NodePowerProfile) -> Result<()> {
        if profile.node_id.trim().is_empty() {
            bail!("node_id must not be empty");
        }
        if profile.idle_watts < 0.0 {
            bail!("idle_watts must be >= 0");
        }
        if profile.active_watts < profile.idle_watts {
            bail!("active_watts must be >= idle_watts");
        }
        if profile.electricity_usd_per_kwh < 0.0 {
            bail!("electricity_usd_per_kwh must be >= 0");
        }
        if profile.depreciation_usd_per_hour < 0.0 {
            bail!("depreciation_usd_per_hour must be >= 0");
        }
        if profile.cooling_usd_per_hour < 0.0 {
            bail!("cooling_usd_per_hour must be >= 0");
        }
        Ok(())
    }

    fn hourly_cost_from_profile(profile: &NodePowerProfile, load_factor: f64) -> HourlyNodeCost {
        let load = load_factor.clamp(0.0, 1.0);
        let active_watts = profile.active_watts.max(profile.idle_watts);
        let avg_watts = profile.idle_watts + (active_watts - profile.idle_watts) * load;
        let electricity = (avg_watts / 1000.0) * profile.electricity_usd_per_kwh;
        let total = electricity + profile.depreciation_usd_per_hour + profile.cooling_usd_per_hour;

        HourlyNodeCost {
            node_id: profile.node_id.clone(),
            load_factor: load,
            avg_watts,
            electricity_usd_per_hour: electricity,
            depreciation_usd_per_hour: profile.depreciation_usd_per_hour,
            cooling_usd_per_hour: profile.cooling_usd_per_hour,
            total_usd_per_hour: total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile(node_id: &str) -> NodePowerProfile {
        NodePowerProfile {
            node_id: node_id.to_string(),
            idle_watts: 10.0,
            active_watts: 50.0,
            electricity_usd_per_kwh: 0.20,
            depreciation_usd_per_hour: 0.30,
            cooling_usd_per_hour: 0.10,
            notes: None,
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_default_profiles_seeded() {
        let pe = PowerEconomics::new(":memory:").await.unwrap();
        let profiles = pe.list_profiles().unwrap();
        assert!(profiles.iter().any(|p| p.node_id == "local"));
        assert!(profiles.iter().any(|p| p.node_id == "Z13"));
    }

    #[tokio::test]
    async fn test_estimate_hourly_cost_uses_load_factor() {
        let pe = PowerEconomics::new(":memory:").await.unwrap();
        pe.upsert_profile(&sample_profile("bench")).unwrap();

        let hourly = pe.estimate_hourly_cost("bench", 0.5).unwrap();
        assert!((hourly.avg_watts - 30.0).abs() < 0.0001);
        assert!((hourly.electricity_usd_per_hour - 0.006).abs() < 0.0001);
        assert!((hourly.total_usd_per_hour - 0.406).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_estimate_run_cost_scales_by_duration() {
        let pe = PowerEconomics::new(":memory:").await.unwrap();
        pe.upsert_profile(&sample_profile("run")).unwrap();

        let estimate = pe.estimate_run_cost("run", 1800.0, 1.0).unwrap();
        // avg watts = 50; electricity = 0.05 kWh * 0.20 * 0.5h = 0.005
        assert!((estimate.electricity_usd - 0.005).abs() < 0.0001);
        assert!((estimate.depreciation_usd - 0.15).abs() < 0.0001);
        assert!((estimate.cooling_usd - 0.05).abs() < 0.0001);
        assert!((estimate.total_usd - 0.205).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_profitability_assessment_has_two_thresholds() {
        let pe = PowerEconomics::new(":memory:").await.unwrap();
        pe.upsert_profile(&sample_profile("profit")).unwrap();

        let assessment = pe
            .assess_profitability("profit", 1.0, 0.1, 1.0)
            .unwrap();
        assert!(assessment.should_run);
        assert!(assessment.should_saturate);
        assert!(assessment.projected_profit_usd_per_hour > 0.0);
        assert!(assessment.aggressive_utilization_floor_usd_per_hour >= assessment.break_even_revenue_usd_per_hour);
    }

    #[tokio::test]
    async fn test_invalid_profile_rejected() {
        let pe = PowerEconomics::new(":memory:").await.unwrap();
        let mut bad = sample_profile("bad");
        bad.active_watts = 5.0;
        assert!(pe.upsert_profile(&bad).is_err());
    }
}
