//! Validate exact checkpoint replay and bounded reads on an isolated restored DB.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sui_sentinel::{
    config::Config,
    diagnosis::Store,
    protocols::registry,
    query::Queries,
    state::{Engine, Frame},
};
#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("SENTINEL_RESTORE_DATABASE_URL")?;
    let pool = sqlx::PgPool::connect(&url).await?;
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await?;
    ensure!(
        name.ends_with("_restore"),
        "isolated *_restore database required"
    );
    let scope = "validation-main";
    let raw: Value = sqlx::query_scalar(
        "SELECT raw_frame FROM chain_frames WHERE scope=$1 ORDER BY checkpoint DESC LIMIT 1",
    )
    .bind(scope)
    .fetch_one(&pool)
    .await?;
    let frame: Frame = serde_json::from_value(raw)?;
    let engine = Engine::new(
        Store::connect(&url).await?,
        registry::build(
            "suilend.cached.v1",
            serde_json::to_value(Config::load("deploy/config/suilend.toml")?)?,
        )?,
        scope.into(),
    )?;
    ensure!(
        !engine.apply(&frame).await?,
        "exact replay unexpectedly mutated restore"
    );
    let q = Queries::connect(&url, vec![scope.into()]).await?;
    let result = q
        .execute("summary", serde_json::from_value(json!({"scope":scope}))?)
        .await;
    ensure!(result.get("error").is_none(), "restored query failed");
    let fingerprint:String=sqlx::query_scalar("SELECT md5(string_agg(md5(evaluation::text),'' ORDER BY position_id)) FROM position_evaluations WHERE scope=$1").bind(scope).fetch_one(&pool).await?;
    println!(
        "{}",
        json!({"exact_replay_no_op":true,"checkpoint":result["checkpoint"],"positions":result["data"]["counts"]["positions"],"evaluation_md5":fingerprint,"fresh_risk_supported":false,"hosted_pitr_proven":false})
    );
    Ok(())
}
