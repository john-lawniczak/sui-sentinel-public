//! Mutates ONLY an explicitly named restored benchmark database with synthetic clocks.
use anyhow::{Result, ensure};
use std::sync::Arc;
use sui_sentinel::{
    config::Config,
    diagnosis::Store,
    model::{Checkpoint, Object},
    protocols::suilend_incremental::Suilend,
    state::{Change, Engine, Frame},
};
#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("SENTINEL_BENCH_DATABASE_URL")?;
    let pool = sqlx::PgPool::connect(&url).await?;
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await?;
    ensure!(
        name.ends_with("_restore"),
        "benchmark requires an isolated *_restore database"
    );
    let store = Store::connect(&url).await?;
    store.migrate().await?;
    let scope = "validation-main";
    let engine = Engine::new(
        store,
        Arc::new(Suilend {
            config: Config::load("deploy/config/suilend.toml")?,
        }),
        scope.into(),
    )?;
    let mixed = std::env::var("SENTINEL_BENCH_MIXED").as_deref() == Ok("true");
    let config = Config::load("deploy/config/suilend.toml")?;
    let market: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM chain_objects WHERE scope=$1 AND object_id=$2")
            .bind(scope)
            .bind(&config.suilend.lending_market_id)
            .fetch_one(&pool)
            .await?;
    let mut market: Object = serde_json::from_value(market)?;
    let before:String=sqlx::query_scalar("SELECT md5(string_agg(md5(evaluation::text),'' ORDER BY position_id)) FROM position_evaluations WHERE scope=$1").bind(scope).fetch_one(&pool).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM position_evaluations WHERE scope=$1")
        .bind(scope)
        .fetch_one(&pool)
        .await?;
    let (mut cp, mut digest, chain): (i64, String, String) = sqlx::query_as(
        "SELECT checkpoint,digest,chain_identifier FROM chain_streams WHERE scope=$1",
    )
    .bind(scope)
    .fetch_one(&pool)
    .await?;
    let body: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM chain_objects WHERE scope=$1 AND object_id=$2")
            .bind(scope)
            .bind(format!("0x{:064x}", 6))
            .fetch_one(&pool)
            .await?;
    let mut clock: Object = serde_json::from_value(body)?;
    let queries = sui_sentinel::query::Queries::connect(&url, vec![scope.into()]).await?;
    let mut readers = Vec::new();
    for _ in 0..2 {
        let q = queries.clone();
        readers.push(tokio::spawn(async move {
            let mut times = Vec::new();
            for i in 0..100 {
                let request = serde_json::from_value(
                    serde_json::json!({"scope":"validation-main","limit":25}),
                )
                .unwrap();
                let start = std::time::Instant::now();
                let answer = q
                    .execute(if i % 2 == 0 { "summary" } else { "positions" }, request)
                    .await;
                ensure!(
                    answer.get("error").is_none(),
                    "bounded query failed during benchmark"
                );
                times.push(start.elapsed().as_secs_f64());
            }
            Ok::<_, anyhow::Error>(times)
        }));
    }
    let mut durations = Vec::new();
    for i in 0..100 {
        cp += 1;
        clock.version += 1;
        clock.contents["timestamp_ms"] = serde_json::json!(
            (sui_sentinel::model::integer(&clock.contents, "timestamp_ms")? + 250).to_string()
        );
        let checkpoint = Checkpoint {
            sequence_number: cp as u64,
            digest: format!("synthetic-benchmark-{cp}"),
            timestamp: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)?,
        };
        let mut changes = vec![Change::Upsert {
            object: clock.clone(),
        }];
        if mixed {
            market.version += 1;
            changes.push(Change::Upsert {
                object: market.clone(),
            });
        }
        let frame = Frame {
            schema_version: 1,
            chain_identifier: chain.clone(),
            checkpoint: checkpoint.clone(),
            previous_digest: digest,
            changes,
            dependency_changes: vec![],
            source: Some(serde_json::json!({"synthetic_benchmark":true,"iteration":i})),
        };
        let start = std::time::Instant::now();
        engine.apply(&frame).await?;
        durations.push(start.elapsed().as_secs_f64());
        digest = checkpoint.digest;
    }
    let after:String=sqlx::query_scalar("SELECT md5(string_agg(md5(evaluation::text),'' ORDER BY position_id)) FROM position_evaluations WHERE scope=$1").bind(scope).fetch_one(&pool).await?;
    ensure!(
        before == after,
        "cached evaluations changed on clock-only input"
    );
    let mut query_times = Vec::new();
    for reader in readers {
        query_times.extend(reader.await??);
    }
    query_times.sort_by(f64::total_cmp);
    durations.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"workload":if mixed { "100 synthetic clock and unchanged-reserve market updates over restored full main-market book" } else { "100 synthetic clock-only checkpoints over restored full main-market book" },"positions":count,"query_concurrency":2,"queries":query_times.len(),"query_p95_seconds":query_times[189],"median_seconds":durations[49],"p95_seconds":durations[94],"p99_seconds":durations[98],"checkpoints_per_second":100.0/durations.iter().sum::<f64>(),"evaluation_hash_unchanged":true,"limitation":"Not a live provider or mixed-mutation benchmark; synthetic database must never resume mainnet ingestion."})
    );
    Ok(())
}
