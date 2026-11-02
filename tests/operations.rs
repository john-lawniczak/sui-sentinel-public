use std::collections::BTreeSet;
use sui_sentinel::{
    monitoring::operations::Operations,
    runtime::{self, RunRecord, State},
    storage,
};
#[tokio::test]
async fn disk_preflight_fails_closed_and_retention_preserves_pins_and_unreplayed_runs() {
    let base = std::env::temp_dir().join(format!(
        "sentinel-retention-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(base.join("runs")).unwrap();
    assert!(runtime::disk::preflight(&base, u64::MAX).await.is_err());
    for i in 0..7 {
        let directory = base.join(format!("run-{i}"));
        std::fs::create_dir(&directory).unwrap();
        let mut run = RunRecord::start(&directory);
        run.finish_ok(State::Published);
        run.finished_at = Some("2020-01-01T00:00:00Z".into());
        runtime::journal::save(&base, &run).unwrap();
        if i != 2 {
            storage::write_pointer(&base.join(format!("runs/run-{i}.applied")), "applied").unwrap();
        }
    }
    storage::write_pointer(&base.join("current"), "run-0").unwrap();
    storage::write_pointer(&base.join("last-good"), "run-1").unwrap();
    let policy = Operations {
        retain_generations: 2,
        retain_seconds: 0,
        ..Default::default()
    };
    let pins = BTreeSet::from(["run-3".to_owned()]);
    assert_eq!(runtime::retention::clean(&base, &policy, &pins).unwrap(), 1);
    for i in [0, 1, 2, 3, 5, 6] {
        assert!(base.join(format!("run-{i}")).exists());
    }
    assert!(!base.join("run-4").exists());
    assert!(runtime::retention::records(&base, 1).is_err());
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn unknown_stale_and_future_monitor_heartbeats_are_never_healthy() {
    let mut ops = runtime::operations::Operations::default();
    assert!(!ops.healthy(100, 30));
    ops.observed_at = 100;
    ops.scheduler_heartbeat = Some(100);
    ops.observer_heartbeat = Some(100);
    ops.delivery_heartbeat = Some(100);
    ops.database_ok = Some(true);
    ops.observer_ok = Some(true);
    assert!(ops.healthy(100, 30));
    assert!(!ops.healthy(131, 30));
    assert!(!ops.healthy(99, 30));
    ops.database_ok = Some(false);
    assert!(!ops.healthy(100, 30));
}
