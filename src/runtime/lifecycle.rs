//! Persisted lifecycle records; unknown transport counters are null.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Scheduled,
    Collecting,
    Captured,
    Analyzing,
    Validating,
    Published,
    Failed,
    /// Not part of temp.md's listed states; added so a skipped tick (lock
    /// held by another run, or the tick itself was overdue and coalesced)
    /// is distinguishable from `Failed` - nothing went wrong, the run
    /// simply did not start.
    Skipped,
}

#[derive(Serialize, Deserialize)]
pub struct RunRecord {
    pub state: State,
    pub output_directory: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_seconds: Option<f64>,
    pub checkpoint: Option<u64>,
    pub position_count: Option<u64>,
    pub market_count: Option<u64>,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
    /// Transport retries are not yet wired into run accounting.
    pub retries: Option<u64>,
    #[serde(default)]
    pub transport: Option<crate::ingest::graphql::Transport>,
    #[serde(default)]
    pub phases: std::collections::BTreeMap<String, String>,
    #[serde(skip, default = "Instant::now")]
    start_instant: Instant,
}

impl RunRecord {
    pub fn start(output_directory: &std::path::Path) -> Self {
        Self {
            state: State::Scheduled,
            output_directory: output_directory.display().to_string(),
            started_at: now_rfc3339(),
            finished_at: None,
            duration_seconds: None,
            checkpoint: None,
            position_count: None,
            market_count: None,
            error_category: None,
            error_message: None,
            retries: None,
            transport: None,
            phases: Default::default(),
            start_instant: Instant::now(),
        }
    }

    pub fn transition(&mut self, state: State) {
        self.state = state;
    }

    pub fn record_capture(&mut self, checkpoint: u64, position_count: u64, market_count: u64) {
        self.checkpoint = Some(checkpoint);
        self.position_count = Some(position_count);
        self.market_count = Some(market_count);
    }

    pub fn finish_ok(&mut self, state: State) {
        self.state = state;
        self.finish_common();
    }

    pub fn finish_failed(&mut self, category: &'static str, error: &anyhow::Error) {
        self.state = State::Failed;
        self.error_category = Some(category.to_owned());
        self.error_message = Some(error.to_string());
        self.finish_common();
    }

    fn finish_common(&mut self) {
        self.finished_at = Some(now_rfc3339());
        self.duration_seconds = Some(duration_seconds(self.start_instant.elapsed()));
    }
}

fn duration_seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unavailable".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_success_transition_and_duration() {
        let mut run = RunRecord::start(std::path::Path::new("/tmp/example"));
        assert_eq!(run.state, State::Scheduled);
        run.transition(State::Collecting);
        run.record_capture(42, 10, 1);
        run.finish_ok(State::Published);
        assert_eq!(run.state, State::Published);
        assert_eq!(run.checkpoint, Some(42));
        assert!(run.finished_at.is_some());
        assert!(run.duration_seconds.unwrap() >= 0.0);
        assert!(run.error_category.is_none());
    }

    #[test]
    fn records_failure_category_and_message() {
        let mut run = RunRecord::start(std::path::Path::new("/tmp/example"));
        run.finish_failed("capture_io_error", &anyhow::anyhow!("transport closed"));
        assert_eq!(run.state, State::Failed);
        assert_eq!(run.error_category.as_deref(), Some("capture_io_error"));
        assert_eq!(run.error_message.as_deref(), Some("transport closed"));
        assert!(run.finished_at.is_some());
    }

    #[test]
    fn serializes_without_leaking_the_internal_timer() {
        let run = RunRecord::start(std::path::Path::new("/tmp/example"));
        let value = serde_json::to_value(&run).unwrap();
        assert!(value.get("start_instant").is_none());
        assert!(value["retries"].is_null());
    }
}
