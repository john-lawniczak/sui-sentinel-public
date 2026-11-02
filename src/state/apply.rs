use super::{Change, Engine, Frame};
use crate::model::{Checkpoint, Object};
use anyhow::{Context, Result, ensure};
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};

impl Engine {
    /// Commit raw frame, object versions, dependency indexes, evaluations and
    /// both watermarks together. Exact replay is a no-op; conflicting replay fails.
    pub async fn apply(&self, frame: &Frame) -> Result<bool> {
        ensure!(
            frame.schema_version == 1 && !frame.checkpoint.digest.is_empty(),
            "invalid frame header"
        );
        ensure!(
            frame.changes.len() <= 50_000 && frame.dependency_changes.len() <= 10_000,
            "frame exceeds work budget"
        );
        time::OffsetDateTime::parse(
            &frame.checkpoint.timestamp,
            &time::format_description::well_known::Rfc3339,
        )?;
        let value = serde_json::to_value(frame)?;
        let hash = crate::storage::sha256_hex_of(&serde_json::to_vec(&value)?);
        let cp = i64::try_from(frame.checkpoint.sequence_number)?;
        let mut tx = self.store.pool.begin().await?;
        let (previous,digest,chain,adapter,configuration): (i64,String,String,String,String) = sqlx::query_as(
            "SELECT checkpoint,digest,chain_identifier,adapter,configuration_hash FROM chain_streams WHERE scope=$1 FOR UPDATE")
            .bind(&self.scope).fetch_one(&mut *tx).await?;
        ensure!(
            adapter == self.adapter.id() && configuration == self.adapter.configuration_hash()?,
            "adapter/configuration changed; rebootstrap after review"
        );
        ensure!(chain == frame.chain_identifier, "chain identifier mismatch");
        if cp <= previous {
            let stored: Option<(String,)> = sqlx::query_as(
                "SELECT payload_hash FROM chain_frames WHERE scope=$1 AND checkpoint=$2",
            )
            .bind(&self.scope)
            .bind(cp)
            .fetch_optional(&mut *tx)
            .await?;
            ensure!(
                stored.is_some_and(|(h,)| h == hash),
                "conflicting or unavailable historical frame"
            );
            return Ok(false);
        }
        ensure!(
            Some(cp) == previous.checked_add(1) && frame.previous_digest == digest,
            "checkpoint gap or digest discontinuity"
        );
        sqlx::query("CREATE TEMP TABLE affected_positions (id TEXT PRIMARY KEY) ON COMMIT DROP")
            .execute(&mut *tx)
            .await?;
        let mut changed = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for change in &frame.changes {
            let (id, version) = match change {
                Change::Upsert { object } => (&object.address, object.version),
                Change::Delete {
                    object_id, version, ..
                } => (object_id, *version),
            };
            ensure!(seen.insert(id), "duplicate object change");
            ensure!(
                crate::config::is_full_address(id),
                "invalid changed object ID"
            );
            let old: Option<(i64, Option<serde_json::Value>, String, bool)> = sqlx::query_as(
                "SELECT version,body,kind,permanently_deleted FROM chain_objects WHERE scope=$1 AND object_id=$2",
            )
            .bind(&self.scope)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
            ensure!(
                version > 0
                    && old
                        .as_ref()
                        .is_none_or(|(v, _, _, _)| i64::try_from(version).is_ok_and(|n| n > *v)),
                "non-increasing object version"
            );
            let before = old
                .as_ref()
                .and_then(|(_, body, _, _)| body.as_ref())
                .map(|body| serde_json::from_value::<Object>(body.clone()))
                .transpose()?;
            let inputs_unchanged = match (&before, change) {
                (Some(before), Change::Upsert { object }) => {
                    self.adapter.evaluation_inputs_unchanged(before, object)?
                }
                _ => false,
            };
            if !inputs_unchanged {
                changed.insert(format!("object:{id}"));
                if let Some(before) = &before {
                    changed.extend(self.adapter.project(before)?.invalidates);
                }
            }
            match change {
                Change::Upsert { object } => {
                    ensure!(
                        old.as_ref().is_none_or(|(_, _, _, permanent)| !permanent),
                        "deleted object cannot be resurrected"
                    );
                    let projection = self.adapter.project(object)?;
                    if let Some((_, _, kind, _)) = &old {
                        ensure!(kind == projection.kind, "object kind changed");
                    }
                    if !inputs_unchanged {
                        changed.extend(projection.invalidates.clone());
                    }
                    self.put(&mut tx, object).await?;
                    if projection.kind == "position" {
                        sqlx::query(
                            "INSERT INTO affected_positions VALUES ($1) ON CONFLICT DO NOTHING",
                        )
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
                Change::Delete { wrapped, .. } => {
                    let (_, body, kind, permanent) =
                        old.context("delete references unknown object")?;
                    ensure!(
                        !permanent && (body.is_some() || !wrapped),
                        "object already absent or deleted"
                    );
                    ensure!(
                        kind != "market" && kind != "dependency",
                        "required deployment/market dependency deleted"
                    );
                    sqlx::query("UPDATE chain_objects SET body=NULL,version=$3,permanently_deleted=$4 WHERE scope=$1 AND object_id=$2")
                        .bind(&self.scope).bind(id).bind(i64::try_from(version)?).bind(!wrapped).execute(&mut *tx).await?;
                    sqlx::query(
                        "DELETE FROM position_dependencies WHERE scope=$1 AND position_id=$2",
                    )
                    .bind(&self.scope)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM position_evaluations WHERE scope=$1 AND position_id=$2",
                    )
                    .bind(&self.scope)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        for dependency in &frame.dependency_changes {
            ensure!(
                !dependency.key.is_empty()
                    && dependency.key.len() <= 512
                    && !dependency.provenance.is_empty(),
                "missing dependency provenance"
            );
            changed.insert(dependency.key.clone());
        }
        let changed: Vec<_> = changed.into_iter().collect();
        sqlx::query("INSERT INTO affected_positions SELECT DISTINCT position_id FROM position_dependencies WHERE scope=$1 AND dependency=ANY($2) ON CONFLICT DO NOTHING")
            .bind(&self.scope).bind(&changed).execute(&mut *tx).await?;
        self.evaluate_affected(&mut tx, &frame.checkpoint).await?;
        sqlx::query("INSERT INTO chain_frames VALUES ($1,$2,$3,$4)")
            .bind(&self.scope)
            .bind(cp)
            .bind(hash)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE chain_streams SET checkpoint=$2,digest=$3,evaluated_checkpoint=$2,updated_at=now(),checkpoint_time=$4 WHERE scope=$1")
            .bind(&self.scope).bind(cp).bind(&frame.checkpoint.digest).bind(time::OffsetDateTime::parse(&frame.checkpoint.timestamp,&time::format_description::well_known::Rfc3339)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    pub(super) async fn put(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        object: &Object,
    ) -> Result<()> {
        let projection = self.adapter.project(object)?;
        let subject = if projection.kind == "capability" {
            projection
                .invalidates
                .iter()
                .find_map(|s| s.strip_prefix("position:"))
                .map(str::to_owned)
        } else {
            None
        };
        sqlx::query("INSERT INTO chain_objects(scope,object_id,version,body,kind,subject) VALUES ($1,$2,$3,$4,$5,$6)
            ON CONFLICT(scope,object_id) DO UPDATE SET version=EXCLUDED.version,body=EXCLUDED.body,kind=EXCLUDED.kind,subject=EXCLUDED.subject")
            .bind(&self.scope).bind(&object.address).bind(i64::try_from(object.version)?).bind(serde_json::to_value(object)?)
            .bind(projection.kind).bind(subject).execute(&mut **tx).await?;
        sqlx::query("DELETE FROM position_dependencies WHERE scope=$1 AND position_id=$2")
            .bind(&self.scope)
            .bind(&object.address)
            .execute(&mut **tx)
            .await?;
        for key in projection.dependencies {
            sqlx::query("INSERT INTO position_dependencies VALUES ($1,$2,$3)")
                .bind(&self.scope)
                .bind(&object.address)
                .bind(key)
                .execute(&mut **tx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn evaluate_affected(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        checkpoint: &Checkpoint,
    ) -> Result<()> {
        let affected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM affected_positions)")
            .fetch_one(&mut **tx)
            .await?;
        if !affected {
            return Ok(());
        }
        let common: Vec<(serde_json::Value,)> = sqlx::query_as("SELECT body FROM chain_objects WHERE scope=$1 AND body IS NOT NULL AND kind IN ('market','dependency')")
            .bind(&self.scope).fetch_all(&mut **tx).await?;
        let common: BTreeMap<String, std::sync::Arc<Object>> = common
            .into_iter()
            .map(|(v,)| {
                let o: Object = serde_json::from_value(v)?;
                Ok((o.address.clone(), std::sync::Arc::new(o)))
            })
            .collect::<Result<_>>()?;
        let mut last = String::new();
        loop {
            // Page the work list before fetching object bodies. During bootstrap,
            // PostgreSQL has no statistics for the uncommitted census; joining
            // first can rescan the entire object table for every small batch.
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM affected_positions WHERE id>$1 ORDER BY id LIMIT 256",
            )
            .bind(&last)
            .fetch_all(&mut **tx)
            .await?;
            let Some(end) = ids.last() else {
                break;
            };
            last = end.clone();
            let rows: Vec<(String,serde_json::Value)> = sqlx::query_as("SELECT object_id,body FROM chain_objects
                WHERE scope=$1 AND object_id=ANY($2) AND kind='position' AND body IS NOT NULL ORDER BY object_id")
                .bind(&self.scope).bind(&ids).fetch_all(&mut **tx).await?;
            // A dependency may refer to a deleted/absent position. Advance past
            // empty pages without skipping later live positions or writing SQL
            // with an empty VALUES clause.
            if rows.is_empty() {
                continue;
            }
            let caps: Vec<(String,serde_json::Value)> = sqlx::query_as("SELECT subject,body FROM chain_objects WHERE scope=$1 AND kind='capability' AND body IS NOT NULL AND subject=ANY($2)")
                .bind(&self.scope).bind(&ids).fetch_all(&mut **tx).await?;
            let mut by_position: BTreeMap<String, Vec<Object>> = BTreeMap::new();
            for (id, body) in caps {
                by_position
                    .entry(id)
                    .or_default()
                    .push(serde_json::from_value(body)?);
            }
            let mut evaluated = Vec::with_capacity(rows.len());
            for (id, body) in rows {
                let mut inputs = common.clone();
                for cap in by_position.remove(&id).unwrap_or_default() {
                    inputs.insert(cap.address.clone(), std::sync::Arc::new(cap));
                }
                let value =
                    self.adapter
                        .evaluate(&serde_json::from_value(body)?, &inputs, checkpoint)?;
                evaluated.push((id, value));
            }
            let mut batch = sqlx::QueryBuilder::new(
                "INSERT INTO position_evaluations(scope,position_id,checkpoint,evaluation) ",
            );
            batch.push_values(evaluated, |mut row, (id, value)| {
                row.push_bind(&self.scope)
                    .push_bind(id)
                    .push_bind(checkpoint.sequence_number as i64)
                    .push_bind(value);
            });
            batch.push(" ON CONFLICT(scope,position_id) DO UPDATE SET checkpoint=EXCLUDED.checkpoint,evaluation=EXCLUDED.evaluation");
            batch.build().execute(&mut **tx).await?;
        }
        Ok(())
    }
}
