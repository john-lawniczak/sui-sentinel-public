use super::*;
use crate::diagnosis::{State, test_support::store};

#[tokio::test]
async fn capture_failure_opens_then_resolves_across_runs() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-scope-{}",
        crate::diagnosis::test_support::unique_suffix()
    );

    let mut failed = runtime::RunRecord::start(std::path::Path::new("/tmp/example"));
    failed.finish_failed("transport_error", &anyhow::anyhow!("connection reset"));
    apply_capture_outcome(&store, &scope, &failed)
        .await
        .unwrap();
    let open = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.scope == scope)
        .unwrap();
    assert_eq!(open.state, State::Opened);
    assert_eq!(open.severity, "high");

    let mut published = runtime::RunRecord::start(std::path::Path::new("/tmp/example"));
    published.record_capture(1, 1, 1);
    published.finish_ok(runtime::State::Published);
    apply_capture_outcome(&store, &scope, &published)
        .await
        .unwrap();
    let resolved = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.scope == scope)
        .unwrap();
    assert_eq!(resolved.id, open.id);
    assert_eq!(resolved.state, State::Resolved);
}

/// Regression: a dedup_id built only from the error category (not the
/// run itself) made every repeat of the same failure after the second
/// occurrence silently vanish - the ON CONFLICT (dedup_id) DO NOTHING
/// path treated a genuinely new failed run as a duplicate of the
/// previous one just because the error category matched.
#[tokio::test]
async fn each_repeated_failure_with_the_same_error_category_gets_its_own_notification() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-scope-repeat-{}",
        crate::diagnosis::test_support::unique_suffix()
    );

    let mut incident_id = None;
    for _ in 0..3 {
        let mut failed = runtime::RunRecord::start(std::path::Path::new("/tmp/example"));
        failed.finish_failed("transport_error", &anyhow::anyhow!("connection reset"));
        apply_capture_outcome(&store, &scope, &failed)
            .await
            .unwrap();
        incident_id = Some(
            store
                .list_incidents(500)
                .await
                .unwrap()
                .into_iter()
                .find(|i| i.scope == scope)
                .unwrap()
                .id,
        );
        // started_at has sub-second precision, but force a gap anyway
        // so this test's correctness never depends on the clock's
        // actual resolution.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let incident_id = incident_id.unwrap();
    let notifications = store
        .pending_notifications(500)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.incident_id == incident_id)
        .count();
    assert_eq!(
        notifications, 3,
        "each of the 3 failed runs must produce its own notification, \
         not collapse into one after the first repeat"
    );
}

#[tokio::test]
async fn a_skipped_run_neither_opens_nor_resolves_anything() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-scope-skip-{}",
        crate::diagnosis::test_support::unique_suffix()
    );

    let mut skipped = runtime::RunRecord::start(std::path::Path::new("/tmp/example"));
    skipped.finish_ok(runtime::State::Skipped);
    apply_capture_outcome(&store, &scope, &skipped)
        .await
        .unwrap();
    assert!(
        !store
            .list_incidents(500)
            .await
            .unwrap()
            .iter()
            .any(|i| i.scope == scope)
    );
}

#[tokio::test]
async fn snapshot_freshness_warn_then_fresh_resolves() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-freshness-{}",
        crate::diagnosis::test_support::unique_suffix()
    );

    apply_snapshot_freshness(&store, &scope, 1000, monitoring::Freshness::Warn)
        .await
        .unwrap();
    let open = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.scope == scope)
        .unwrap();
    assert_eq!(open.state, State::Opened);
    assert_eq!(open.severity, "medium");

    apply_snapshot_freshness(&store, &scope, 10, monitoring::Freshness::Fresh)
        .await
        .unwrap();
    let resolved = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.scope == scope)
        .unwrap();
    assert_eq!(resolved.id, open.id);
    assert_eq!(resolved.state, State::Resolved);
}

/// Stable severity refreshes stored evidence without generating a notification storm.
#[tokio::test]
async fn unchanged_freshness_severity_does_not_flood_notifications() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-freshness-repeat-{}",
        crate::diagnosis::test_support::unique_suffix()
    );

    let mut incident_id = None;
    for age_seconds in [1000, 1100, 1200] {
        apply_snapshot_freshness(&store, &scope, age_seconds, monitoring::Freshness::Warn)
            .await
            .unwrap();
        incident_id = Some(
            store
                .list_incidents(500)
                .await
                .unwrap()
                .into_iter()
                .find(|i| i.scope == scope)
                .unwrap()
                .id,
        );
    }
    let incident_id = incident_id.unwrap();
    let notifications = store
        .pending_notifications(500)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.incident_id == incident_id)
        .count();
    assert_eq!(
        notifications, 1,
        "unchanged severity should update evidence without flooding the receiver"
    );
}

#[tokio::test]
async fn recurring_freshness_incidents_at_the_same_ages_each_notify() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "test-freshness-cycles-{}",
        crate::diagnosis::test_support::unique_suffix()
    );
    for _ in 0..2 {
        apply_snapshot_freshness(&store, &scope, 1000, monitoring::Freshness::Warn)
            .await
            .unwrap();
        apply_snapshot_freshness(&store, &scope, 10, monitoring::Freshness::Fresh)
            .await
            .unwrap();
    }
    let incidents: Vec<_> = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .filter(|incident| incident.scope == scope)
        .collect();
    assert_eq!(incidents.len(), 2);
    let pending = store.pending_notifications(500).await.unwrap();
    for incident in incidents {
        assert_eq!(
            pending
                .iter()
                .filter(|entry| entry.incident_id == incident.id)
                .count(),
            2,
            "each separate incident needs its own opened and resolved notifications"
        );
    }
}

#[tokio::test]
async fn capture_signal_replay_and_out_of_order_history_cannot_reopen_or_duplicate() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "atomic-replay-{}",
        crate::diagnosis::test_support::unique_suffix()
    );
    let mut failed = runtime::RunRecord::start(std::path::Path::new("/test/one"));
    failed.started_at = "2026-01-01T00:00:00Z".into();
    failed.finish_failed("test", &anyhow::anyhow!("failure"));
    let (a, b) = tokio::join!(
        apply_capture_outcome(&store, &scope, &failed),
        apply_capture_outcome(&store, &scope, &failed)
    );
    a.unwrap();
    b.unwrap();
    let mut ok = runtime::RunRecord::start(std::path::Path::new("/test/two"));
    ok.started_at = "2026-01-01T00:01:00Z".into();
    ok.finish_ok(runtime::State::Published);
    apply_capture_outcome(&store, &scope, &ok).await.unwrap();
    apply_capture_outcome(&store, &scope, &failed)
        .await
        .unwrap();
    failed.started_at = "2026-01-01T00:00:30Z".into();
    apply_capture_outcome(&store, &scope, &failed)
        .await
        .unwrap();
    let incidents: Vec<_> = store
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .filter(|i| i.scope == scope)
        .collect();
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].state, State::Resolved);
    assert_eq!(
        store
            .pending_notifications(500)
            .await
            .unwrap()
            .iter()
            .filter(|n| n.incident_id == incidents[0].id)
            .count(),
        2
    );
}

#[tokio::test]
async fn an_outbox_failure_rolls_back_the_signal_and_incident_together() {
    let Some(store) = store().await else { return };
    let scope = format!(
        "atomic-rollback-{}",
        crate::diagnosis::test_support::unique_suffix()
    );
    let mut run = runtime::RunRecord::start(std::path::Path::new("/test/rollback"));
    run.finish_failed("test", &anyhow::anyhow!("failed"));
    let key = format!("capture_failure:{scope}");
    let signal_id = format!("{}:{}", run.output_directory, run.started_at);
    let dedup = crate::storage::sha256_hex_of(&serde_json::to_vec(&(&key, &signal_id)).unwrap());
    let (other, _) = store
        .open_or_update_incident(super::super::IncidentCandidate {
            dedup_key: format!("other:{scope}"),
            rule_id: "other".into(),
            rule_version: 1,
            scope: scope.clone(),
            severity: "high".into(),
            valuation_basis: "not_applicable".into(),
            checkpoint_sequence: None,
            measured_trigger: json!({}),
            evidence: json!({}),
        })
        .await
        .unwrap();
    store
        .enqueue_notification(other.id, &dedup, &json!({}))
        .await
        .unwrap();
    assert!(apply_capture_outcome(&store, &scope, &run).await.is_err());
    let (signals,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM diagnosis_signals WHERE rule_key=$1")
            .bind(&key)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let (incidents,): (i64,) = sqlx::query_as("SELECT count(*) FROM incidents WHERE dedup_key=$1")
        .bind(&key)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!((signals, incidents), (0, 0));
    sqlx::query("DELETE FROM outbox WHERE dedup_id=$1")
        .bind(&dedup)
        .execute(&store.pool)
        .await
        .unwrap();
    apply_capture_outcome(&store, &scope, &run).await.unwrap();
}
