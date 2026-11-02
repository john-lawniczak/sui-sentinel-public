//! Pure cadence helpers: unique per-run output directories, and deciding
//! whether an overdue tick should be skipped rather than run late (temp.md
//! section 4: "skip/coalesce missed schedule ticks rather than accumulating
//! jobs"). Nothing here reads the system clock - callers supply `now`, so
//! every decision is reproducible in tests.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use time::OffsetDateTime;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A filesystem-safe, monotonically distinguishable directory name for one
/// run: `run-<unix-nanos>-<pid>-<process-local-sequence>`. The sequence guards
/// against two calls landing in the same clock tick (common on fast
/// filesystems or in tests); it is process-local, not crash-safe across
/// restarts, which is fine - uniqueness only needs to hold among
/// concurrently-running or closely-spaced calls within one process, and
/// `storage::create_directory`'s `create_dir` (not `create_dir_all`) still
/// fails loudly on any genuine collision.
pub fn unique_run_directory(base: &Path, now: OffsetDateTime) -> PathBuf {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    base.join(format!(
        "run-{}-{}-{sequence}",
        now.unix_timestamp_nanos().max(0),
        std::process::id()
    ))
}

/// A tick scheduled at `scheduled_at` is overdue once `now` is more than one
/// `interval` past it. An overdue tick is skipped (coalesced into the next
/// one) rather than run late, so a backlog of missed ticks never
/// accumulates into a burst of simultaneous jobs.
pub fn should_skip_overdue_tick(
    scheduled_at: OffsetDateTime,
    now: OffsetDateTime,
    interval: Duration,
) -> bool {
    let overdue_by = now - scheduled_at;
    overdue_by.is_positive() && overdue_by.whole_seconds() as u128 > interval.as_secs() as u128
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration as TimeDuration;

    #[test]
    fn unique_run_directory_never_repeats_for_the_same_instant() {
        let base = Path::new("/tmp/sentinel-runs");
        let now = OffsetDateTime::now_utc();
        let first = unique_run_directory(base, now);
        let second = unique_run_directory(base, now);
        assert_ne!(first, second);
        assert!(first.starts_with(base));
    }

    #[test]
    fn exactly_one_interval_late_is_not_yet_overdue() {
        let scheduled = OffsetDateTime::now_utc();
        let interval = Duration::from_secs(60);
        assert!(!should_skip_overdue_tick(
            scheduled,
            scheduled + TimeDuration::seconds(60),
            interval
        ));
        assert!(should_skip_overdue_tick(
            scheduled,
            scheduled + TimeDuration::seconds(61),
            interval
        ));
    }

    #[test]
    fn a_tick_that_has_not_yet_arrived_is_never_skipped() {
        let scheduled = OffsetDateTime::now_utc();
        let interval = Duration::from_secs(60);
        assert!(!should_skip_overdue_tick(
            scheduled,
            scheduled - TimeDuration::seconds(5),
            interval
        ));
    }
}
