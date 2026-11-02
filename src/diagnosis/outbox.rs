//! Notification outbox (temp.md section 7: "Persist notifications in an
//! outbox, retry with bounded backoff, and track delivery attempts and
//! acknowledgments. Use stable deduplication IDs with downstream
//! receivers"). This module persists and dedups; it does not implement an
//! actual sender (email/webhook/etc.) - that is future work layered on top
//! of `pending`/`mark_delivered`/`record_attempt_failed`.

use anyhow::{Context, Result};
use serde_json::Value;
use time::OffsetDateTime;

use super::Store;

#[derive(Debug, sqlx::FromRow)]
pub struct OutboxEntry {
    pub id: i64,
    pub incident_id: i64,
    pub dedup_id: String,
    pub payload: Value,
    pub attempts: i32,
    pub last_error: Option<String>,
}

impl Store {
    /// Enqueue a notification for `incident_id`, deduplicated on
    /// `dedup_id`. Returns `true` if this call actually inserted a new
    /// row, `false` if an entry with this `dedup_id` already existed (a
    /// retried enqueue of the same logical event, not a new one) - the
    /// `UNIQUE` constraint on `dedup_id` is what makes this safe to call
    /// more than once for the same transition.
    pub async fn enqueue_notification(
        &self,
        incident_id: i64,
        dedup_id: &str,
        payload: &Value,
    ) -> Result<bool> {
        let inserted: Option<(i64,)> = sqlx::query_as(
            "INSERT INTO outbox (incident_id, dedup_id, payload, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (dedup_id) DO NOTHING
             RETURNING id",
        )
        .bind(incident_id)
        .bind(dedup_id)
        .bind(payload)
        .bind(OffsetDateTime::now_utc())
        .fetch_optional(&self.pool)
        .await
        .context("enqueueing outbox notification")?;
        Ok(inserted.is_some())
    }

    /// Bounded read of not-yet-delivered entries, oldest first (temp.md:
    /// "bounded ... evidence reads"; a future sender drains this).
    pub async fn pending_notifications(&self, limit: i64) -> Result<Vec<OutboxEntry>> {
        let limit = limit.clamp(1, 500);
        sqlx::query_as(
            "SELECT id, incident_id, dedup_id, payload, attempts, last_error
             FROM outbox WHERE delivered_at IS NULL
             ORDER BY created_at ASC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("reading pending outbox notifications")
    }

    pub async fn mark_delivered(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE outbox SET delivered_at = $2 WHERE id = $1")
            .bind(id)
            .bind(OffsetDateTime::now_utc())
            .execute(&self.pool)
            .await
            .context("marking outbox entry delivered")?;
        Ok(())
    }

    /// Record a failed delivery attempt without marking the entry
    /// delivered, so a future drain of `pending_notifications` retries it
    /// (temp.md: "retry with bounded backoff" - the backoff policy itself
    /// belongs to the sender that calls this, not to the store).
    pub async fn record_delivery_attempt_failed(&self, id: i64, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE outbox SET attempts = attempts + 1, last_attempt_at = $2, last_error = $3
             WHERE id = $1",
        )
        .bind(id)
        .bind(OffsetDateTime::now_utc())
        .bind(error)
        .execute(&self.pool)
        .await
        .context("recording failed outbox delivery attempt")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::test_support::{store, unique_suffix};
    use serde_json::json;

    async fn incident_id(store: &Store, dedup_key: &str) -> i64 {
        let (incident, _) = store
            .open_or_update_incident(crate::diagnosis::IncidentCandidate {
                dedup_key: dedup_key.to_owned(),
                rule_id: "test_rule".to_owned(),
                rule_version: 1,
                scope: "test_scope".to_owned(),
                severity: "warning".to_owned(),
                valuation_basis: "unverified_cached_inputs".to_owned(),
                checkpoint_sequence: None,
                measured_trigger: json!({}),
                evidence: json!({}),
            })
            .await
            .unwrap();
        incident.id
    }

    #[tokio::test]
    async fn duplicate_dedup_id_does_not_enqueue_twice() {
        let Some(store) = store().await else { return };
        let id = incident_id(&store, &format!("test-outbox-dedup-{}", unique_suffix())).await;
        let dedup_id = format!("outbox-dedup-{}", unique_suffix());
        let payload = json!({"kind": "opened"});

        assert!(
            store
                .enqueue_notification(id, &dedup_id, &payload)
                .await
                .unwrap()
        );
        assert!(
            !store
                .enqueue_notification(id, &dedup_id, &payload)
                .await
                .unwrap(),
            "a retried enqueue with the same dedup_id must not insert a second row"
        );

        let pending = store.pending_notifications(500).await.unwrap();
        assert_eq!(pending.iter().filter(|e| e.dedup_id == dedup_id).count(), 1);
    }

    #[tokio::test]
    async fn delivery_attempts_and_delivered_state_are_tracked() {
        let Some(store) = store().await else { return };
        let id = incident_id(&store, &format!("test-outbox-delivery-{}", unique_suffix())).await;
        let dedup_id = format!("outbox-delivery-{}", unique_suffix());
        store
            .enqueue_notification(id, &dedup_id, &json!({}))
            .await
            .unwrap();
        let entry = store
            .pending_notifications(500)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.dedup_id == dedup_id)
            .unwrap();
        assert_eq!(entry.attempts, 0);

        store
            .record_delivery_attempt_failed(entry.id, "connection refused")
            .await
            .unwrap();
        let entry = store
            .pending_notifications(500)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.dedup_id == dedup_id)
            .unwrap();
        assert_eq!(entry.attempts, 1);
        assert_eq!(entry.last_error.as_deref(), Some("connection refused"));

        store.mark_delivered(entry.id).await.unwrap();
        let still_pending = store
            .pending_notifications(500)
            .await
            .unwrap()
            .into_iter()
            .any(|e| e.dedup_id == dedup_id);
        assert!(!still_pending, "a delivered entry must not be re-drained");
    }
}
