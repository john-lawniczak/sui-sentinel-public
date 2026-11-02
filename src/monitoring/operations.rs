use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Operations {
    pub require_monitor: bool,
    pub heartbeat_max_age_seconds: u64,
    pub min_free_bytes: u64,
    pub retain_generations: usize,
    pub retain_seconds: u64,
    pub max_run_records: usize,
    pub open_after_seconds: u64,
    pub resolve_after_seconds: u64,
    pub reminder_seconds: u64,
}
impl Default for Operations {
    fn default() -> Self {
        Self {
            require_monitor: false,
            heartbeat_max_age_seconds: 90,
            min_free_bytes: 5 * 1024 * 1024 * 1024,
            retain_generations: 8,
            retain_seconds: 86400,
            max_run_records: 10000,
            open_after_seconds: 15,
            resolve_after_seconds: 30,
            reminder_seconds: 3600,
        }
    }
}
