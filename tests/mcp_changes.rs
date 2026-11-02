use serde_json::{Value, json};
use sui_sentinel::{diagnosis::Store, query::Queries};

#[tokio::test]
async fn incident_feed_resumes_updates_preserves_evidence_and_rolls_back_atomically() {
    let Ok(url) = std::env::var("SENTINEL_TEST_DATABASE_URL") else {
        return;
    };
    Store::connect(&url).await.unwrap().migrate().await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let scope = format!(
        "feed-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let q = Queries::connect(&url, vec![scope.clone()]).await.unwrap();
    let read = |after: Option<&str>| {
        serde_json::from_value(json!({"scope":scope,"after":after,"limit":1})).unwrap()
    };
    let id:i64=sqlx::query_scalar("INSERT INTO incidents(dedup_key,rule_id,rule_version,scope,severity,state,valuation_basis,measured_trigger,evidence,opened_at,updated_at) VALUES($1,'test',1,$1,'high','opened','cached','{}',$2,now(),now()) RETURNING id").bind(&scope).bind(json!({"original":true})).fetch_one(&pool).await.unwrap();
    let first = q.execute("incident_changes", read(None)).await;
    assert_eq!(first["resume_after"], "1");
    assert_eq!(first["data"][0]["payload"]["incident_id"], id.to_string());
    sqlx::query("UPDATE incidents SET state='acknowledged',evidence=$2 WHERE id=$1")
        .bind(id)
        .bind(json!({"updated":true}))
        .execute(&pool)
        .await
        .unwrap();
    let next = q.execute("incident_changes", read(Some("1"))).await;
    assert_eq!(next["resume_after"], "2");
    assert_eq!(next["data"][0]["payload"]["state"], "acknowledged");
    assert_eq!(
        next["data"][0]["payload"]["evidence"],
        json!({"updated":true})
    );
    let again = q.execute("incident_changes", read(None)).await;
    assert_eq!(again["data"][0], first["data"][0]);
    assert_eq!(again["next_after"], "1");
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE incidents SET state='resolved' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let during = q.execute("incident_changes", read(Some("2"))).await;
    assert_eq!(during["data"], json!([]));
    tx.rollback().await.unwrap();
    let after = q.execute("incident_changes", read(Some("2"))).await;
    assert_eq!(after["resume_after"], "2");
    sqlx::query("DELETE FROM incidents WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let deleted = q.execute("incident_changes", read(Some("2"))).await;
    assert_eq!(deleted["resume_after"], "3");
    assert_eq!(deleted["data"][0]["kind"], "deleted");
    assert_eq!(
        deleted["data"][0]["payload"]["evidence"],
        json!({"updated":true})
    );
    // Simulate future archival. A retained floor must never silently skip history.
    sqlx::query("DELETE FROM incident_changes WHERE scope=$1 AND revision<3")
        .bind(&scope)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        q.execute("incident_changes", read(Some("1"))).await["error"]["code"],
        "history_expired"
    );
    assert_eq!(
        q.execute("incident_changes", read(Some("4"))).await["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(
        q.execute("incident_changes", read(None)).await["history_requires_bootstrap"],
        true
    );
    assert_eq!(
        q.execute("incident_changes", read(Some("2"))).await["history_requires_bootstrap"],
        false
    );
    // A migrated baseline marker requires bootstrap only until that cursor is handled.
    sqlx::query(
        "UPDATE incident_changes SET event_kind='bootstrap_required' WHERE scope=$1 AND revision=3",
    )
    .bind(&scope)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        q.execute("incident_changes", read(Some("2"))).await["history_requires_bootstrap"],
        true
    );
    assert_eq!(
        q.execute("incident_changes", read(Some("3"))).await["history_requires_bootstrap"],
        false
    );
    let schema: Value = serde_json::to_value(read(Some("3"))).unwrap();
    assert!(schema["generation"].is_null());
}

#[tokio::test]
async fn same_scope_cursors_cannot_commit_out_of_order() {
    let Ok(url) = std::env::var("SENTINEL_TEST_DATABASE_URL") else {
        return;
    };
    Store::connect(&url).await.unwrap().migrate().await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let scope = format!(
        "ordered-feed-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let insert = "INSERT INTO incidents(dedup_key,rule_id,rule_version,scope,severity,state,valuation_basis,measured_trigger,evidence,opened_at,updated_at) VALUES($1,'test',1,$2,'info','opened','cached','{}','{}',now(),now())";
    let mut first = pool.begin().await.unwrap();
    sqlx::query(insert)
        .bind(format!("{scope}-first"))
        .bind(&scope)
        .execute(&mut *first)
        .await
        .unwrap();
    let second_pool = pool.clone();
    let second_scope = scope.clone();
    let mut second = tokio::spawn(async move {
        sqlx::query(insert)
            .bind(format!("{second_scope}-second"))
            .bind(second_scope)
            .execute(&second_pool)
            .await
            .unwrap();
    });
    // Different incident IDs share the per-scope revision barrier.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut second)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM incident_changes WHERE scope=$1")
        .bind(&scope)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    first.commit().await.unwrap();
    second.await.unwrap();
    let revisions: Vec<i64> = sqlx::query_scalar(
        "SELECT revision FROM incident_changes WHERE scope=$1 ORDER BY revision",
    )
    .bind(scope)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(revisions, vec![1, 2]);
}
