//! Runtime freshness and coverage policy. Observation freshness does not imply fresh valuation.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
pub mod operations;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub capture: Capture,
    #[serde(default)]
    pub readiness: Readiness,
    #[serde(default)]
    pub operations: operations::Operations,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// T: target interval between scheduled snapshot captures, in seconds.
    pub interval_seconds: u64,
    /// D: capture deadline/timeout budget for a single capture, in seconds.
    pub deadline_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Readiness {
    pub expected_scope: String,
    pub expected_market_ids: Vec<String>,
}
impl Default for Readiness {
    fn default() -> Self {
        Self {
            expected_scope: "explicit_markets".into(),
            expected_market_ids: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Age <= T + D.
    Fresh,
    /// T + D < age <= 2T + D.
    Warn,
    /// Age > 2T + D.
    Critical,
}

impl Policy {
    pub fn builtin() -> Result<Self> {
        Self::parse(include_str!("../deploy/config/monitoring.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("reading monitoring policy {}", path.display()))?;
        Self::parse(&input).with_context(|| format!("invalid monitoring policy {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let policy: Self =
            toml::from_str(text).context("failed to parse monitoring policy TOML")?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported monitoring policy schema"
        );
        ensure!(
            self.capture.interval_seconds > 0,
            "capture interval must be positive"
        );
        ensure!(
            self.capture.interval_seconds <= 86_400,
            "capture interval must be at most 24 hours"
        );
        ensure!(
            self.capture.deadline_seconds > 0
                && self.capture.deadline_seconds <= self.capture.interval_seconds,
            "capture deadline must not exceed the capture interval"
        );
        ensure!(
            !self.readiness.expected_scope.is_empty(),
            "expected scope cannot be empty"
        );
        ensure!(
            self.operations.heartbeat_max_age_seconds >= 10
                && self.operations.heartbeat_max_age_seconds <= 3600,
            "invalid heartbeat age"
        );
        ensure!(
            self.operations.retain_generations >= 2
                && self.operations.max_run_records >= self.operations.retain_generations
                && self.operations.max_run_records <= 10000,
            "invalid retention budget"
        );
        ensure!(
            self.operations.retain_seconds <= 365 * 86400
                && self.operations.open_after_seconds <= 3600
                && self.operations.resolve_after_seconds <= 3600
                && self.operations.reminder_seconds >= 60,
            "invalid operations windows"
        );
        let ids: std::collections::BTreeSet<_> =
            self.readiness.expected_market_ids.iter().collect();
        ensure!(
            ids.len() == self.readiness.expected_market_ids.len(),
            "duplicate expected market"
        );
        ensure!(
            ids.iter().all(|id| crate::config::is_full_address(id)),
            "invalid expected market ID"
        );
        Ok(())
    }

    /// Classify a scheduled snapshot's observed age (now - capture time)
    /// against this policy's warn/critical boundaries: warn above `T + D`,
    /// critical above `2T + D`. Snapshot age alone never establishes
    /// valuation freshness - it is the capture-side half of the contract
    /// only.
    pub fn snapshot_age_status(&self, age_seconds: u64) -> Freshness {
        let warn_at = self.capture.interval_seconds + self.capture.deadline_seconds;
        let critical_at = 2 * self.capture.interval_seconds + self.capture.deadline_seconds;
        if age_seconds > critical_at {
            Freshness::Critical
        } else if age_seconds > warn_at {
            Freshness::Warn
        } else {
            Freshness::Fresh
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(interval: u64, deadline: u64) -> Policy {
        Policy {
            schema_version: 1,
            operations: operations::Operations::default(),
            readiness: Readiness::default(),
            capture: Capture {
                interval_seconds: interval,
                deadline_seconds: deadline,
            },
        }
    }

    #[test]
    fn builtin_policy_is_valid() {
        Policy::builtin().unwrap();
    }

    #[test]
    fn rejects_invalid_schema_interval_or_deadline() {
        assert!(
            Policy::parse(
                "schema_version = 2\n[capture]\ninterval_seconds = 1\ndeadline_seconds = 1\n"
            )
            .is_err()
        );
        let mut zero_interval = policy(300, 60);
        zero_interval.capture.interval_seconds = 0;
        assert!(zero_interval.validate().is_err());
        let mut huge_interval = policy(300, 60);
        huge_interval.capture.interval_seconds = 100_000;
        assert!(huge_interval.validate().is_err());
        assert!(policy(300, 301).validate().is_err());
    }

    #[test]
    fn classifies_exact_warn_and_critical_boundaries() {
        let policy = policy(300, 60);
        // warn_at = 360, critical_at = 660.
        assert_eq!(policy.snapshot_age_status(0), Freshness::Fresh);
        assert_eq!(policy.snapshot_age_status(360), Freshness::Fresh);
        assert_eq!(policy.snapshot_age_status(361), Freshness::Warn);
        assert_eq!(policy.snapshot_age_status(660), Freshness::Warn);
        assert_eq!(policy.snapshot_age_status(661), Freshness::Critical);
        assert_eq!(policy.snapshot_age_status(u64::MAX), Freshness::Critical);
    }
}
