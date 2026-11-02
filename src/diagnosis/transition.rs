//! Serialize a rule/scope in PostgreSQL, not a process-local lock. The signal
//! receipt, cursor, incident and notification form one atomic transaction.
use super::{IncidentCandidate, Store};
use anyhow::{Result, ensure};
use serde_json::json;
use time::OffsetDateTime;

pub struct Signal<'a> {
    pub id: &'a str,
    pub observed_at: OffsetDateTime,
    pub key: &'a str,
    pub scope: &'a str,
    pub candidate: Option<IncidentCandidate>,
    /// Capture failures notify per failed run. Freshness notifies transitions only.
    pub notify_unchanged: bool,
}

impl Store {
    pub async fn apply_signal(&self, signal: Signal<'_>) -> Result<()> {
        self.apply_signal_policy(signal, None).await
    }
    pub async fn apply_signal_policy(
        &self,
        signal: Signal<'_>,
        policy: Option<&crate::monitoring::operations::Operations>,
    ) -> Result<()> {
        let hash = crate::storage::sha256_hex_of(&serde_json::to_vec(&json!({
            "at": signal.observed_at.unix_timestamp_nanos().to_string(),
            "scope": signal.scope, "candidate": signal.candidate,
            "notify_unchanged": signal.notify_unchanged, "policy":policy,
        }))?);
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(signal.key)
            .execute(&mut *tx)
            .await?;
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT payload_hash FROM diagnosis_signals WHERE rule_key=$1 AND signal_id=$2",
        )
        .bind(signal.key)
        .bind(signal.id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((existing,)) = existing {
            ensure!(existing == hash, "replayed signal has different contents");
            return Ok(());
        }
        sqlx::query("INSERT INTO diagnosis_signals VALUES ($1,$2,$3,$4)")
            .bind(signal.key)
            .bind(signal.id)
            .bind(signal.observed_at)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        let latest: Option<(OffsetDateTime,)> =
            sqlx::query_as("SELECT observed_at FROM diagnosis_cursors WHERE rule_key=$1")
                .bind(signal.key)
                .fetch_optional(&mut *tx)
                .await?;
        if latest.is_some_and(|(at,)| at > signal.observed_at) {
            tx.commit().await?;
            return Ok(()); // Old replay cannot reopen an incident resolved by newer evidence.
        }
        let mut reminder = false;
        if let Some(policy) = policy {
            let condition = signal
                .candidate
                .as_ref()
                .map(|c| c.severity.as_str())
                .unwrap_or("healthy");
            let (stable, remind) =
                super::windows::observe(&mut tx, signal.key, condition, signal.observed_at, policy)
                    .await?;
            reminder = remind;
            if !stable {
                sqlx::query("INSERT INTO diagnosis_cursors VALUES ($1,$2) ON CONFLICT (rule_key) DO UPDATE SET observed_at=EXCLUDED.observed_at")
                    .bind(signal.key).bind(signal.observed_at).execute(&mut *tx).await?;
                tx.commit().await?;
                return Ok(());
            }
        }
        let current: Option<(i64, String)> = sqlx::query_as(
            "SELECT id,severity FROM incidents WHERE dedup_key=$1 AND resolved_at IS NULL FOR UPDATE")
            .bind(signal.key).fetch_optional(&mut *tx).await?;
        let mut notify = true;
        let change = if let Some(candidate) = signal.candidate {
            ensure!(
                candidate.dedup_key == signal.key && candidate.scope == signal.scope,
                "signal scope mismatch"
            );
            let transition = if current.is_some() {
                "updated"
            } else {
                "opened"
            };
            notify = reminder
                || signal.notify_unchanged
                || current
                    .as_ref()
                    .is_none_or(|(_, severity)| severity != &candidate.severity);
            let (id,): (i64,) = sqlx::query_as(
                "INSERT INTO incidents (dedup_key,rule_id,rule_version,scope,severity,state,
                 checkpoint_sequence,valuation_basis,measured_trigger,evidence,opened_at,updated_at)
                 VALUES ($1,$2,$3,$4,$5,'opened',$6,$7,$8,$9,$10,$10)
                 ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO UPDATE SET
                 severity=EXCLUDED.severity, rule_version=EXCLUDED.rule_version, state=CASE WHEN incidents.state='acknowledged' AND incidents.severity=EXCLUDED.severity THEN 'acknowledged' ELSE 'updated' END, checkpoint_sequence=EXCLUDED.checkpoint_sequence,
                 valuation_basis=EXCLUDED.valuation_basis, measured_trigger=EXCLUDED.measured_trigger,
                 evidence=EXCLUDED.evidence, updated_at=EXCLUDED.updated_at RETURNING id")
                .bind(signal.key).bind(candidate.rule_id).bind(candidate.rule_version).bind(signal.scope)
                .bind(candidate.severity).bind(candidate.checkpoint_sequence).bind(candidate.valuation_basis)
                .bind(candidate.measured_trigger).bind(candidate.evidence).bind(signal.observed_at)
                .fetch_one(&mut *tx).await?;
            Some((id, transition))
        } else if let Some((id, _)) = current {
            sqlx::query(
                "UPDATE incidents SET state='resolved',updated_at=$2,resolved_at=$2 WHERE id=$1",
            )
            .bind(id)
            .bind(signal.observed_at)
            .execute(&mut *tx)
            .await?;
            Some((id, "resolved"))
        } else {
            None
        };
        if let Some((id, transition)) = change.filter(|_| notify) {
            let dedup =
                crate::storage::sha256_hex_of(&serde_json::to_vec(&(signal.key, signal.id))?);
            if policy.is_some() {
                sqlx::query("UPDATE rule_windows SET last_notified_at=$2 WHERE rule_key=$1")
                    .bind(signal.key)
                    .bind(signal.observed_at)
                    .execute(&mut *tx)
                    .await?;
            }
            // Include full stored evidence for an actionable notification.
            let (incident,): (serde_json::Value,) =
                sqlx::query_as("SELECT to_jsonb(i) FROM incidents i WHERE id=$1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            sqlx::query("INSERT INTO outbox (incident_id,dedup_id,payload,created_at) VALUES ($1,$2,$3,$4)")
                .bind(id).bind(dedup).bind(json!({"incident_id":id,"transition":transition,"scope":signal.scope,"incident":incident}))
                .bind(signal.observed_at).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO diagnosis_cursors VALUES ($1,$2) ON CONFLICT (rule_key) DO UPDATE SET observed_at=EXCLUDED.observed_at")
            .bind(signal.key).bind(signal.observed_at).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
