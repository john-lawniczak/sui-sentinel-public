//! Single-host capture supervision. Production work runs in killable child processes.
pub mod disk;
pub mod journal;
pub mod lifecycle;
pub mod lock;
pub mod operations;
pub mod process;
pub mod progress;
pub mod retention;
pub mod schedule;

use std::{future::Future, path::Path, time::Duration};

use anyhow::Result;
use time::OffsetDateTime;

pub use lifecycle::{RunRecord, State};

/// What a capture closure reports back on success. Deliberately minimal -
/// only what this orchestrator itself needs to record, not a duplicate of
/// the full `Manifest`.
pub struct CaptureOutcome {
    pub checkpoint: u64,
    pub position_count: u64,
    pub market_count: u64,
}

/// Run one supervised capture attempt and finalize it under the same lock.
/// `publish` must validate the artifacts and update the publication pointer;
/// only successful finalization records Published. Returns `Ok` except an
/// internal bug (the lifecycle record's `state` and `error_*` fields carry
/// the actual outcome - lock contention and capture failure are both
/// reported, not propagated as `Err`, so a caller logging/publishing the
/// record does not need a second error-handling path).
pub async fn run_supervised_capture<F, Fut, P>(
    base: &Path,
    lock_path: &Path,
    now: OffsetDateTime,
    deadline: Duration,
    capture: F,
    publish: P,
) -> Result<RunRecord>
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: Future<Output = Result<CaptureOutcome>>,
    P: FnOnce(&Path, &CaptureOutcome) -> Result<()>,
{
    let output_directory = schedule::unique_run_directory(base, now);
    let mut run = RunRecord::start(&output_directory);

    let held = match lock::HostLock::try_acquire(lock_path) {
        Ok(held) => held,
        Err(error) => {
            run.finish_failed("lock_io_error", &error);
            return Ok(run);
        }
    };
    let Some(_held) = held else {
        run.finish_ok(State::Skipped);
        return Ok(run);
    };

    journal::recover(base)?;
    let started = std::time::Instant::now();
    run.transition(State::Collecting);
    journal::save(base, &run)?;
    match tokio::time::timeout(deadline, capture(output_directory.clone())).await {
        Ok(Ok(_)) if started.elapsed() >= deadline => {
            run.finish_failed(
                "deadline_exceeded",
                &anyhow::anyhow!("capture completed after deadline"),
            );
        }
        Ok(Ok(outcome)) => {
            run.record_capture(
                outcome.checkpoint,
                outcome.position_count,
                outcome.market_count,
            );
            run.transition(State::Validating);
            journal::save(base, &run)?;
            match publish(&output_directory, &outcome) {
                Ok(()) => run.finish_ok(State::Published),
                Err(error) => run.finish_failed("publication_error", &error),
            }
        }
        Ok(Err(error)) => run.finish_failed(
            if error.is::<process::DeadlineExceeded>() {
                "deadline_exceeded"
            } else {
                "capture_error"
            },
            &error,
        ),
        Err(_elapsed) => run.finish_failed(
            "deadline_exceeded",
            &anyhow::anyhow!("capture exceeded the {:?} deadline budget", deadline),
        ),
    }
    if let Ok(progress) =
        crate::storage::read_json::<progress::Progress>(&progress::path(base, &output_directory))
    {
        run.retries = Some(progress.transport.retries);
        run.transport = Some(progress.transport);
        run.phases = progress.phases;
        if run.state == State::Failed
            && let Some(kind) = progress.failure_kind
        {
            run.error_category = Some(kind);
        }
    }
    journal::save(base, &run)?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> std::path::PathBuf {
        // A per-process atomic counter, not just a nanosecond timestamp -
        // see storage::tests::scratch for why a timestamp alone collides
        // under parallel test execution.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sui-sentinel-runtime-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn successful_capture_publishes_and_records_counts() {
        let scratch = scratch_dir();
        let run = run_supervised_capture(
            &scratch,
            &scratch.join("lock"),
            OffsetDateTime::now_utc(),
            Duration::from_secs(5),
            |_out| async {
                Ok(CaptureOutcome {
                    checkpoint: 42,
                    position_count: 10,
                    market_count: 1,
                })
            },
            |directory, _| {
                assert!(lock::HostLock::try_acquire(&scratch.join("lock"))?.is_none());
                crate::storage::write_pointer(
                    &scratch.join("current"),
                    directory.file_name().unwrap().to_str().unwrap(),
                )
            },
        )
        .await
        .unwrap();
        assert_eq!(run.state, State::Published);
        assert_eq!(run.checkpoint, Some(42));
        assert!(scratch.join("current").exists());
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[tokio::test]
    async fn failed_capture_is_recorded_not_propagated() {
        let scratch = scratch_dir();
        let run = run_supervised_capture(
            &scratch,
            &scratch.join("lock"),
            OffsetDateTime::now_utc(),
            Duration::from_secs(5),
            |_out| async { Err(anyhow::anyhow!("transport closed")) },
            |_, _| panic!("failed capture must not publish"),
        )
        .await
        .unwrap();
        assert_eq!(run.state, State::Failed);
        assert_eq!(run.error_category.as_deref(), Some("capture_error"));
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[tokio::test]
    async fn capture_exceeding_the_deadline_is_recorded_as_failed() {
        let scratch = scratch_dir();
        let run = run_supervised_capture(
            &scratch,
            &scratch.join("lock"),
            OffsetDateTime::now_utc(),
            Duration::from_millis(10),
            |_out| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(CaptureOutcome {
                    checkpoint: 1,
                    position_count: 0,
                    market_count: 0,
                })
            },
            |_, _| panic!("timed-out capture must not publish"),
        )
        .await
        .unwrap();
        assert_eq!(run.state, State::Failed);
        assert_eq!(run.error_category.as_deref(), Some("deadline_exceeded"));
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[tokio::test]
    async fn an_overlapping_run_is_skipped_not_failed() {
        let scratch = scratch_dir();
        let lock_path = scratch.join("lock");
        let held = lock::HostLock::try_acquire(&lock_path).unwrap();
        assert!(held.is_some());
        let run = run_supervised_capture(
            &scratch,
            &lock_path,
            OffsetDateTime::now_utc(),
            Duration::from_secs(5),
            |_out| async { panic!("capture must not run while the lock is held elsewhere") },
            |_, _| panic!("skipped capture must not publish"),
        )
        .await
        .unwrap();
        assert_eq!(run.state, State::Skipped);
        drop(held);
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[tokio::test]
    async fn publication_failure_does_not_report_a_published_run() {
        let scratch = scratch_dir();
        crate::storage::write_pointer(&scratch.join("current"), "last-good").unwrap();
        let run = run_supervised_capture(
            &scratch,
            &scratch.join("lock"),
            OffsetDateTime::now_utc(),
            Duration::from_secs(5),
            |_| async {
                Ok(CaptureOutcome {
                    checkpoint: 42,
                    position_count: 10,
                    market_count: 1,
                })
            },
            |_, _| Err(anyhow::anyhow!("candidate failed validation")),
        )
        .await
        .unwrap();
        assert_eq!(run.state, State::Failed);
        assert_eq!(run.error_category.as_deref(), Some("publication_error"));
        assert_eq!(
            std::fs::read_to_string(scratch.join("current")).unwrap(),
            "last-good"
        );
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
