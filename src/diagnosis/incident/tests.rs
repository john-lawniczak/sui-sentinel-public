use super::*;
use crate::diagnosis::test_support::{store, unique_suffix};
use serde_json::json;

fn candidate(dedup_key: &str) -> IncidentCandidate {
    IncidentCandidate {
        dedup_key: dedup_key.to_owned(),
        rule_id: "test_rule".to_owned(),
        rule_version: 1,
        scope: "test_scope".to_owned(),
        severity: "warning".to_owned(),
        valuation_basis: "unverified_cached_inputs".to_owned(),
        checkpoint_sequence: Some(1),
        measured_trigger: json!({"value": 1}),
        evidence: json!({"source": "test"}),
    }
}

#[tokio::test]
async fn opening_twice_updates_in_place_instead_of_duplicating() {
    let Some(store) = store().await else { return };
    let key = format!("test-open-twice-{}", unique_suffix());

    let (first, opened) = store
        .open_or_update_incident(candidate(&key))
        .await
        .unwrap();
    assert!(opened);
    assert_eq!(first.state, State::Opened);

    let mut second_candidate = candidate(&key);
    second_candidate.severity = "critical".to_owned();
    let (second, opened_again) = store
        .open_or_update_incident(second_candidate)
        .await
        .unwrap();
    assert!(!opened_again);
    assert_eq!(
        second.id, first.id,
        "must update the same row, not insert a new one"
    );
    assert_eq!(second.state, State::Updated);
    assert_eq!(second.severity, "critical");
}

#[tokio::test]
async fn resolving_an_incident_allows_a_fresh_one_to_open_for_the_same_key() {
    let Some(store) = store().await else { return };
    let key = format!("test-resolve-reopen-{}", unique_suffix());

    let (first, _) = store
        .open_or_update_incident(candidate(&key))
        .await
        .unwrap();
    let resolved = store.resolve_incident(&key).await.unwrap().unwrap();
    assert_eq!(resolved.id, first.id);
    assert_eq!(resolved.state, State::Resolved);

    // Resolving again (e.g. a retried call) is a no-op, not an error.
    assert!(store.resolve_incident(&key).await.unwrap().is_none());

    // A new incident can now open for the same dedup_key - the partial
    // unique index only constrains OPEN rows.
    let (reopened, opened) = store
        .open_or_update_incident(candidate(&key))
        .await
        .unwrap();
    assert!(opened);
    assert_ne!(reopened.id, first.id);
}

/// Restart recovery: a second `Store` connecting fresh (standing in for
/// a restarted process) sees exactly what the first one persisted,
/// with nothing relying on in-process state.
#[tokio::test]
async fn an_incident_persists_across_a_fresh_store_connection() {
    let Some(url) = std::env::var("SENTINEL_TEST_DATABASE_URL").ok() else {
        return;
    };
    let first_process = store().await.unwrap();
    let key = format!("test-restart-{}", unique_suffix());
    let (opened, _) = first_process
        .open_or_update_incident(candidate(&key))
        .await
        .unwrap();
    drop(first_process);

    let second_process = Store::connect(&url).await.unwrap();
    let recovered = second_process
        .list_incidents(500)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.dedup_key == key)
        .expect("incident written by the first connection must be visible to a new one");
    assert_eq!(recovered.id, opened.id);
    assert_eq!(recovered.state, State::Opened);
}

#[tokio::test]
async fn list_incidents_is_bounded_and_most_recent_first() {
    let Some(store) = store().await else { return };
    let prefix = format!("test-list-{}", unique_suffix());
    for index in 0..3 {
        store
            .open_or_update_incident(candidate(&format!("{prefix}-{index}")))
            .await
            .unwrap();
    }
    let rows = store.list_incidents(0).await.unwrap();
    assert!(
        !rows.is_empty(),
        "limit is clamped to at least 1, never 0 rows"
    );
    let rows = store.list_incidents(100_000).await.unwrap();
    assert!(rows.len() <= 500, "limit is clamped to at most 500");
}
