//! One explicitly requested TEST ONLY notification through the real durable sender.
//! Use a scratch database; credentials come from the environment and are never printed.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sui_sentinel::diagnosis::{IncidentCandidate, Store, delivery::Webhook, transition::Signal};

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("SENTINEL_TEST_DATABASE_URL")?;
    let parsed = reqwest::Url::parse(&url)?;
    ensure!(
        parsed.path().ends_with("_test"),
        "use a scratch _test database"
    );
    let webhook = Webhook::with_format(
        &std::env::var("SENTINEL_WEBHOOK_URL").context("webhook secret is required")?,
        None,
        "discord",
    )?;
    let store = Store::connect(&url).await?;
    store.migrate().await?;
    let now = time::OffsetDateTime::now_utc();
    let scope = format!("TEST-ONLY-discord-{}", now.unix_timestamp());
    let key = format!("{scope}:acceptance");
    store.apply_signal(Signal {
        id: "one-shot-acceptance", observed_at: now, key: &key, scope: &scope,
        notify_unchanged: false,
        candidate: Some(IncidentCandidate {
            dedup_key: key.clone(), rule_id: "TEST_ONLY_no_financial_alert_or_action".into(),
            rule_version: 1, scope: scope.clone(), severity: "info".into(),
            valuation_basis: "operational_acceptance_test".into(), checkpoint_sequence: None,
            measured_trigger: json!({"test_only":true}),
            evidence: json!({"purpose":"One authorized Discord delivery acceptance; production is not activated"}),
        }),
    }).await?;
    ensure!(
        webhook.deliver_one(&store, &scope).await?,
        "test outbox entry missing"
    );
    let pool = sqlx::PgPool::connect(&url).await?;
    let (id, delivered, attempts, error): (i64, bool, i32, Option<String>) = sqlx::query_as(
        "SELECT o.id,o.delivered_at IS NOT NULL,o.attempts,o.last_error FROM outbox o JOIN incidents i ON i.id=o.incident_id WHERE i.scope=$1",
    ).bind(&scope).fetch_one(&pool).await?;
    // Prevent a later generic scratch worker from retrying a failed one-shot test.
    if !delivered {
        sqlx::query("UPDATE outbox SET dead_lettered_at=now() WHERE id=$1")
            .bind(id)
            .execute(&pool)
            .await?;
    }
    println!(
        "{}",
        json!({"test_only":true,"scope":scope,"outbox_id":id.to_string(),
        "discord_http_accepted":delivered,"attempts":attempts,"error":error,
        "human_channel_acknowledgement":false,"production_activated":false})
    );
    ensure!(
        delivered,
        "Discord acceptance failed; no automatic retry scheduled"
    );
    Ok(())
}
