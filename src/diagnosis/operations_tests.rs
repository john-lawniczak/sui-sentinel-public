use super::{IncidentCandidate, test_support, transition::Signal};
use serde_json::json;
use time::{Duration, OffsetDateTime};
fn candidate(scope: &str, key: &str) -> IncidentCandidate {
    IncidentCandidate {
        dedup_key: key.into(),
        rule_id: "test_health".into(),
        rule_version: 1,
        scope: scope.into(),
        severity: "high".into(),
        valuation_basis: "not_applicable".into(),
        checkpoint_sequence: None,
        measured_trigger: json!({}),
        evidence: json!({}),
    }
}
#[tokio::test]
async fn persistent_debounce_recovery_reminders_and_audited_acknowledgment() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let scope = test_support::unique_suffix();
    let key = format!("rule:{scope}");
    let policy = crate::monitoring::operations::Operations {
        open_after_seconds: 10,
        resolve_after_seconds: 20,
        reminder_seconds: 60,
        ..Default::default()
    };
    let start = OffsetDateTime::now_utc();
    for (n, active) in [
        (0, true),
        (5, true),
        (11, true),
        (30, false),
        (35, true),
        (80, true),
        (90, false),
        (111, false),
    ] {
        let id = n.to_string();
        let at = start + Duration::seconds(n);
        for _ in 0..2 {
            store
                .apply_signal_policy(
                    Signal {
                        id: &id,
                        observed_at: at,
                        key: &key,
                        scope: &scope,
                        candidate: active.then(|| candidate(&scope, &key)),
                        notify_unchanged: false,
                    },
                    Some(&policy),
                )
                .await
                .unwrap();
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM incidents WHERE scope=$1")
            .bind(&scope)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, if n < 10 { 0 } else { 1 });
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM outbox WHERE incident_id IN (SELECT id FROM incidents WHERE scope=$1)").bind(&scope).fetch_one(&store.pool).await.unwrap();
    assert_eq!(count, 3, "open, reminder and stable resolution only");
    let id = "reopen";
    store
        .apply_signal(Signal {
            id,
            observed_at: start + Duration::seconds(120),
            key: &key,
            scope: &scope,
            candidate: Some(candidate(&scope, &key)),
            notify_unchanged: false,
        })
        .await
        .unwrap();
    let incident: i64 =
        sqlx::query_scalar("SELECT id FROM incidents WHERE scope=$1 AND resolved_at IS NULL")
            .bind(&scope)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert!(
        store
            .administer("wrong-scope", "acknowledge", incident, "reviewed")
            .await
            .is_err()
    );
    store
        .administer(&scope, "acknowledge", incident, "reviewed provider outage")
        .await
        .unwrap();
    let (actor, action): (String, String) =
        sqlx::query_as("SELECT actor,action FROM operator_audit WHERE scope=$1")
            .bind(&scope)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert!(!actor.is_empty());
    assert_eq!(action, "acknowledge");
    store
        .apply_signal(Signal {
            id: "refresh",
            observed_at: start + Duration::seconds(121),
            key: &key,
            scope: &scope,
            candidate: Some(candidate(&scope, &key)),
            notify_unchanged: false,
        })
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM incidents WHERE id=$1")
        .bind(incident)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(state, "acknowledged");
}
#[tokio::test]
async fn accepted_notification_retries_after_lost_ack_without_repeating_receiver_effect() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let scope = test_support::unique_suffix();
    let key = format!("delivery:{scope}");
    store
        .apply_signal(Signal {
            id: "initial",
            observed_at: OffsetDateTime::now_utc(),
            key: &key,
            scope: &scope,
            candidate: Some(candidate(&scope, &key)),
            notify_unchanged: false,
        })
        .await
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let received = std::sync::Arc::new(tokio::sync::Notify::new());
    let (calls, notify) = (seen.clone(), received.clone());
    let app = axum::Router::new().route(
        "/",
        axum::routing::post(move |headers: axum::http::HeaderMap| {
            let (calls, notify) = (calls.clone(), notify.clone());
            async move {
                let first = {
                    let mut calls = calls.lock().unwrap();
                    calls.push(headers["Idempotency-Key"].to_str().unwrap().into());
                    calls.len() == 1
                };
                notify.notify_one();
                if first {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
                "accepted"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("http://{addr}/");
    let (db, job_scope) = (store.clone(), scope.clone());
    let target = url.clone();
    let worker = tokio::spawn(async move {
        super::delivery::Webhook::new(&target, None)
            .unwrap()
            .deliver_one(&db, &job_scope)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), received.notified())
        .await
        .unwrap();
    worker.abort();
    let _ = worker.await;
    sqlx::query("UPDATE outbox SET lease_until=now()-interval '1 second' WHERE incident_id IN (SELECT id FROM incidents WHERE scope=$1)").bind(&scope).execute(&store.pool).await.unwrap();
    super::delivery::Webhook::new(&url, None)
        .unwrap()
        .deliver_one(&store, &scope)
        .await
        .unwrap();
    {
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        assert_eq!(
            calls
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1
        );
    }
    server.abort();
}
#[tokio::test]
async fn retry_is_scoped_audited_and_rejects_obsolete_delivery() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let scope = test_support::unique_suffix();
    let key = format!("retry:{scope}");
    for id in ["one", "two"] {
        store
            .apply_signal(Signal {
                id,
                observed_at: OffsetDateTime::now_utc(),
                key: &key,
                scope: &scope,
                candidate: Some(candidate(&scope, &key)),
                notify_unchanged: true,
            })
            .await
            .unwrap();
    }
    let ids:Vec<i64>=sqlx::query_scalar("SELECT id FROM outbox WHERE incident_id IN (SELECT id FROM incidents WHERE scope=$1) ORDER BY id").bind(&scope).fetch_all(&store.pool).await.unwrap();
    sqlx::query("UPDATE outbox SET dead_lettered_at=now(),attempts=10 WHERE id=$1")
        .bind(ids[0])
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .administer("other", "retry", ids[0], "receiver fixed")
            .await
            .is_err()
    );
    store
        .administer(&scope, "retry", ids[0], "receiver fixed")
        .await
        .unwrap();
    sqlx::query("UPDATE outbox SET dead_lettered_at=now() WHERE id=$1")
        .bind(ids[0])
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE outbox SET delivered_at=now() WHERE id=$1")
        .bind(ids[1])
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .administer(&scope, "retry", ids[0], "obsolete")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn retention_keeps_historical_notification_evidence_after_incident_updates() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let scope = test_support::unique_suffix();
    let key = format!("pins:{scope}");
    let start = OffsetDateTime::now_utc();
    for (id, directory, seconds) in [("one", "/data/old", 0), ("two", "/data/new", 1)] {
        let mut c = candidate(&scope, &key);
        c.evidence = serde_json::json!({"output_directory": directory});
        store
            .apply_signal(Signal {
                id,
                observed_at: start + time::Duration::seconds(seconds),
                key: &key,
                scope: &scope,
                candidate: Some(c),
                notify_unchanged: true,
            })
            .await
            .unwrap();
    }
    let pins = store.evidence_pins(&scope).await.unwrap();
    assert!(pins.contains("old"));
    assert!(pins.contains("new"));
}
