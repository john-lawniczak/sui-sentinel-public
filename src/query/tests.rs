use super::*;
#[tokio::test]
async fn readonly_pool_and_backpressure_are_enforced() {
    let Some(store) = crate::diagnosis::test_support::store().await else {
        return;
    };
    drop(store);
    let url = std::env::var("SENTINEL_TEST_DATABASE_URL").unwrap();
    let q = Queries::connect(&url, vec!["query-bounds".into(), "other-bounds".into()])
        .await
        .unwrap();
    assert!(
        sqlx::query("CREATE TABLE forbidden_query_write(id INT)")
            .execute(&q.pool)
            .await
            .is_err()
    );
    let all = q.admission.acquire_many(4).await.unwrap();
    let args = serde_json::from_value(json!({"scope":"query-bounds"})).unwrap();
    assert_eq!(
        q.execute("status", args).await["error"]["code"],
        "query_busy"
    );
    drop(all);
    let args = serde_json::from_value(json!({"scope":"query-bounds"})).unwrap();
    let first = q.execute("status", args).await;
    assert!(first.get("error").is_none());
    let args =
        serde_json::from_value(json!({"scope":"other-bounds","generation":first["generation"]}))
            .unwrap();
    assert_eq!(
        q.execute("status", args).await["error"]["code"],
        "generation_expired"
    );
}
