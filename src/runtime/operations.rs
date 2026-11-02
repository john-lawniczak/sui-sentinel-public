//! A bounded atomic status document. Exporters read it outside presentation locks.
use crate::{runtime::RunRecord, storage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Operations {
    pub observed_at: i64,
    pub scheduler_heartbeat: Option<i64>,
    pub observer_heartbeat: Option<i64>,
    pub delivery_heartbeat: Option<i64>,
    pub database_ok: Option<bool>,
    pub observer_ok: Option<bool>,
    pub disk_available_bytes: Option<u64>,
    pub capture_active: bool,
    pub capture_phase: Option<String>,
    pub last_run_duration_seconds: Option<f64>,
    pub failure_streak: u64,
    pub missed_ticks: u64,
    pub spool_backlog: Option<u64>,
    pub notification_pending: Option<i64>,
    pub notification_oldest_age_seconds: Option<f64>,
    pub notification_attempts: Option<i64>,
    pub notification_dead_letters: Option<i64>,
    pub last_run: Option<String>,
    pub last_error: Option<String>,
    pub transport: Option<crate::ingest::graphql::Transport>,
}
impl Operations {
    pub fn save(&self, base: &Path) -> Result<()> {
        storage::write_pointer(&base.join("operations.json"), &serde_json::to_string(self)?)
    }
    pub fn record(&mut self, run: &RunRecord) {
        self.last_run = Some(run.output_directory.clone());
        self.last_run_duration_seconds = run.duration_seconds;
        self.transport = run.transport.clone();
        match run.state {
            super::State::Failed => {
                self.failure_streak += 1;
                self.last_error = run.error_category.clone();
            }
            super::State::Published => {
                self.failure_streak = 0;
                self.last_error = None;
            }
            _ => {}
        }
    }
    pub fn healthy(&self, now: i64, maximum_age: u64) -> bool {
        let recent = |value: Option<i64>| {
            value.is_some_and(|t| t <= now && now.saturating_sub(t) <= maximum_age as i64)
        };
        recent(Some(self.observed_at))
            && recent(self.scheduler_heartbeat)
            && recent(self.observer_heartbeat)
            && recent(self.delivery_heartbeat)
            && self.database_ok == Some(true)
            && self.observer_ok == Some(true)
    }
}
