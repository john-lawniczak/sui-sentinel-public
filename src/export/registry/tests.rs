use super::*;
use crate::{analysis::report, storage};
use std::fs;

fn scratch() -> PathBuf {
    // A per-process atomic counter, not just a nanosecond timestamp -
    // see storage::tests::scratch for why a timestamp alone collides
    // under parallel test execution.
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "sui-sentinel-registry-test-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir")
}

fn publish_generation(base: &Path, id: &str) {
    fs::create_dir(base.join(id)).unwrap();
    let dir = base.join(id).join("analysis");
    let mut report = report::generate(&fixture(), &dir).unwrap();
    report.checkpoint.timestamp = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    fs::write(
        dir.join("report.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    fs::remove_file(dir.join("checksums.json")).unwrap();
    storage::write_checksums(&dir, &["positions.ndjson", "report.json"]).unwrap();
    storage::write_pointer(&base.join("current"), id).unwrap();
}

#[test]
fn missing_pointer_is_not_ready_but_not_an_error() {
    let base = scratch();
    let registry = Registry::new(base.clone());
    assert!(!registry.try_reload().unwrap());
    let (presentation, status) = registry.snapshot();
    assert!(presentation.is_none());
    assert!(!status.ready);
    assert!(status.last_reload_error.is_none());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn loads_and_swaps_to_a_new_generation_keeping_the_last_good_one_on_failure() {
    let base = scratch();
    publish_generation(&base, "gen-a");
    let registry = Registry::new(base.clone());
    assert!(registry.try_reload().unwrap());
    let (presentation, status) = registry.snapshot();
    assert!(presentation.is_some());
    assert_eq!(status.current_generation.as_deref(), Some("gen-a"));

    // The pointer now names a generation that was never published -
    // reload must fail without discarding the still-good "gen-a".
    storage::write_pointer(&base.join("current"), "gen-missing").unwrap();
    assert!(!registry.try_reload().unwrap());
    let (presentation, status) = registry.snapshot();
    assert!(presentation.is_some());
    assert_eq!(status.current_generation.as_deref(), Some("gen-a"));
    assert!(status.last_reload_error.is_some());

    // A genuinely new generation still swaps in correctly afterward.
    publish_generation(&base, "gen-b");
    storage::write_pointer(&base.join("current"), "gen-b").unwrap();
    assert!(registry.try_reload().unwrap());
    let (_, status) = registry.snapshot();
    assert_eq!(status.current_generation.as_deref(), Some("gen-b"));
    assert!(status.last_reload_error.is_none());

    fs::remove_dir_all(&base).unwrap();
}

/// The exact per-field semantics of this text (which samples appear
/// under which conditions) are unit-tested directly against the pure
/// `operational::render` function; this only checks `Registry` wires
/// its real lock-held state into that function correctly.
#[test]
fn operational_metrics_reflect_registry_state_through_a_reload() {
    let base = scratch();
    let registry = Registry::new(base.clone());
    assert!(registry.operational_metrics().contains("exporter_ready 0"));

    publish_generation(&base, "gen-a");
    assert!(registry.try_reload().unwrap());
    assert!(registry.operational_metrics().contains("exporter_ready 1"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn repeated_reload_of_the_same_generation_is_a_no_op() {
    let base = scratch();
    publish_generation(&base, "gen-a");
    let registry = Registry::new(base.clone());
    assert!(registry.try_reload().unwrap());
    assert!(
        !registry.try_reload().unwrap(),
        "reloading the same already-current generation must not report a swap"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn lost_empty_and_unreadable_pointers_preserve_last_good_and_report_failure() {
    let base = scratch();
    publish_generation(&base, "gen-a");
    let registry = Registry::new(base.clone());
    assert!(registry.try_reload().unwrap());
    for state in ["missing", "empty", "directory"] {
        fs::remove_file(base.join("current")).unwrap();
        match state {
            "empty" => fs::write(base.join("current"), " \n").unwrap(),
            "directory" => fs::create_dir(base.join("current")).unwrap(),
            _ => {}
        }
        assert!(!registry.try_reload().unwrap());
        let (presentation, status) = registry.snapshot();
        assert!(presentation.is_some());
        assert_eq!(status.current_generation.as_deref(), Some("gen-a"));
        assert!(
            status.last_reload_error.is_some(),
            "missing failure for {state}"
        );
        if state == "directory" {
            fs::remove_dir(base.join("current")).unwrap();
        }
        storage::write_pointer(&base.join("current"), "gen-a").unwrap();
        registry.try_reload().unwrap();
        assert!(registry.snapshot().1.last_reload_error.is_none());
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn generation_pointer_cannot_select_a_report_outside_its_base() {
    let base = scratch();
    let outside = scratch();
    publish_generation(&outside, "gen-outside");
    let registry = Registry::new(base.clone());
    let sibling = format!(
        "../{}/gen-outside",
        outside.file_name().unwrap().to_str().unwrap()
    );
    for id in [sibling, outside.join("gen-outside").display().to_string()] {
        storage::write_pointer(&base.join("current"), &id).unwrap();
        assert!(!registry.try_reload().unwrap());
        assert!(registry.snapshot().0.is_none());
        assert!(registry.snapshot().1.last_reload_error.is_some());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("gen-outside"), base.join("linked")).unwrap();
        storage::write_pointer(&base.join("current"), "linked").unwrap();
        assert!(!registry.try_reload().unwrap());
        assert!(registry.snapshot().0.is_none());
        assert!(registry.snapshot().1.last_reload_error.is_some());
    }
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn required_monitor_health_expires_independently_of_report_load() {
    let base = scratch();
    publish_generation(&base, "gen-ops");
    let mut policy = crate::monitoring::Policy::builtin().unwrap();
    policy.operations.require_monitor = true;
    let registry = Registry::with_policy(base.clone(), policy);
    registry.try_reload().unwrap();
    assert_eq!(
        registry.status().health.readiness_reason,
        "monitor_unhealthy"
    );
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let mut ops = crate::runtime::operations::Operations {
        observed_at: now,
        scheduler_heartbeat: Some(now),
        observer_heartbeat: Some(now),
        delivery_heartbeat: Some(now),
        database_ok: Some(true),
        observer_ok: Some(true),
        ..Default::default()
    };
    ops.save(&base).unwrap();
    registry.try_reload().unwrap();
    assert!(registry.status().ready);
    ops.delivery_heartbeat = Some(now - 100);
    ops.save(&base).unwrap();
    registry.try_reload().unwrap();
    assert!(!registry.status().ready);
    assert!(registry.snapshot().0.is_some());
    std::fs::remove_dir_all(base).unwrap();
}
