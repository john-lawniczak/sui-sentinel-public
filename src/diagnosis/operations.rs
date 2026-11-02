use super::Store;
use crate::runtime::operations::Operations;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
impl Store {
    pub async fn operations(&self, scope: &str) -> Result<Operations> {
        let (pending, age, attempts, dead): (i64, Option<f64>, i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE o.delivered_at IS NULL AND o.dead_lettered_at IS NULL),
             EXTRACT(EPOCH FROM now()-min(o.created_at) FILTER (WHERE o.delivered_at IS NULL AND o.dead_lettered_at IS NULL))::float8,
             COALESCE(sum(o.attempts),0)::bigint, count(*) FILTER (WHERE o.dead_lettered_at IS NOT NULL)
             FROM outbox o JOIN incidents i ON i.id=o.incident_id WHERE i.scope=$1")
            .bind(scope).fetch_one(&self.pool).await?;
        Ok(Operations {
            database_ok: Some(true),
            notification_pending: Some(pending),
            notification_oldest_age_seconds: age,
            notification_attempts: Some(attempts),
            notification_dead_letters: Some(dead),
            ..Default::default()
        })
    }
    pub async fn evidence_pins(&self, scope: &str) -> Result<BTreeSet<String>> {
        let paths: Vec<String> = sqlx::query_scalar("SELECT evidence->>'output_directory' FROM incidents WHERE scope=$1 AND evidence ? 'output_directory' UNION SELECT o.payload->'incident'->'evidence'->>'output_directory' FROM outbox o JOIN incidents i ON i.id=o.incident_id WHERE i.scope=$1 AND o.payload->'incident'->'evidence' ? 'output_directory' UNION SELECT evidence->'evidence'->>'output_directory' FROM operator_audit WHERE scope=$1 AND evidence->'evidence' ? 'output_directory' LIMIT 10001")
            .bind(scope).fetch_all(&self.pool).await?;
        ensure!(
            paths.len() <= 10000,
            "incident evidence exceeds retention scan budget"
        );
        Ok(paths
            .iter()
            .filter_map(|p| {
                std::path::Path::new(p)
                    .file_name()?
                    .to_str()
                    .map(str::to_owned)
            })
            .collect())
    }
    pub async fn inspect_operations(&self, scope: &str) -> Result<Value> {
        let incidents: Vec<Value> = sqlx::query_scalar(
            "SELECT to_jsonb(i) FROM incidents i WHERE scope=$1 ORDER BY updated_at DESC LIMIT 100",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;
        let notifications: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',o.id,'incident_id',o.incident_id,'dedup_id',o.dedup_id,'attempts',o.attempts,'last_error',o.last_error,'dead_lettered_at',o.dead_lettered_at,'lease_until',o.lease_until) FROM outbox o JOIN incidents i ON i.id=o.incident_id WHERE i.scope=$1 AND o.delivered_at IS NULL ORDER BY o.id LIMIT 100")
            .bind(scope).fetch_all(&self.pool).await?;
        Ok(
            json!({"scope":scope,"incidents":incidents,"pending_notifications":notifications,"limit":100}),
        )
    }
    pub async fn administer(&self, scope: &str, action: &str, id: i64, reason: &str) -> Result<()> {
        ensure!(
            !reason.trim().is_empty() && reason.len() <= 1024,
            "reason must contain 1..1024 bytes"
        );
        let mut tx = self.pool.begin().await?;
        let evidence: Value = match action {
            "acknowledge" => {
                let row: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(i) FROM incidents i WHERE id=$1 AND scope=$2 AND resolved_at IS NULL FOR UPDATE")
                    .bind(id).bind(scope).fetch_optional(&mut *tx).await?;
                let row = row.ok_or_else(|| anyhow::anyhow!("open incident not found in scope"))?;
                sqlx::query("UPDATE incidents SET state='acknowledged' WHERE id=$1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                row
            }
            "retry" => {
                // Lock the incident to serialize admin actions with diagnosis transitions.
                let incident: Option<i64> = sqlx::query_scalar("SELECT i.id FROM incidents i JOIN outbox o ON o.incident_id=i.id WHERE o.id=$1 AND i.scope=$2 FOR UPDATE OF i")
                    .bind(id).bind(scope).fetch_optional(&mut *tx).await?;
                ensure!(incident.is_some(), "notification not found in scope");
                let row: Option<Value> = sqlx::query_scalar("UPDATE outbox o SET dead_lettered_at=NULL,attempts=0,next_attempt_at=now(),lease_until=NULL,lease_token=lease_token+1,last_error=NULL WHERE id=$1 AND delivered_at IS NULL AND dead_lettered_at IS NOT NULL AND (lease_until IS NULL OR lease_until<now()) AND NOT EXISTS (SELECT 1 FROM outbox later WHERE later.incident_id=o.incident_id AND later.id>o.id AND (later.delivered_at IS NOT NULL OR later.lease_until>=now())) RETURNING to_jsonb(o)")
                    .bind(id).fetch_optional(&mut *tx).await?;
                row.ok_or_else(|| anyhow::anyhow!("retry rejected: not dead-lettered, leased, or a later notification was sent"))?
            }
            _ => anyhow::bail!("unknown administration action"),
        };
        sqlx::query("INSERT INTO operator_audit(scope,action,target_id,reason,evidence) VALUES ($1,$2,$3,$4,$5)")
            .bind(scope).bind(action).bind(id).bind(reason).bind(evidence).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
