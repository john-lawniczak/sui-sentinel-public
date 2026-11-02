//! Debounce state is committed with signal receipts and incident/outbox writes.
use crate::monitoring::operations::Operations;
use anyhow::Result;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
pub(super) async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    condition: &str,
    now: OffsetDateTime,
    policy: &Operations,
) -> Result<(bool, bool)> {
    let (since, notified): (OffsetDateTime,Option<OffsetDateTime>) = sqlx::query_as(
        "INSERT INTO rule_windows(rule_key,condition,since_at) VALUES ($1,$2,$3)
         ON CONFLICT(rule_key) DO UPDATE SET condition=EXCLUDED.condition,
         since_at=CASE WHEN rule_windows.condition=EXCLUDED.condition THEN rule_windows.since_at ELSE EXCLUDED.since_at END
         RETURNING since_at,last_notified_at")
        .bind(key).bind(condition).bind(now).fetch_one(&mut **tx).await?;
    let delay = if condition == "healthy" {
        policy.resolve_after_seconds
    } else {
        policy.open_after_seconds
    };
    let stable = (now - since).whole_seconds() >= delay as i64;
    let remind =
        notified.is_some_and(|at| (now - at).whole_seconds() >= policy.reminder_seconds as i64);
    Ok((stable, remind))
}
