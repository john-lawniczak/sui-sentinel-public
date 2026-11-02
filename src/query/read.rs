use super::{Queries, Request};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

impl Queries {
    pub(super) async fn read(&self, kind: &str, request: Request) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let stream: Option<Value> =
            sqlx::query_scalar("SELECT to_jsonb(s) FROM chain_streams s WHERE scope=$1")
                .bind(&request.scope)
                .fetch_optional(&mut *tx)
                .await?;
        let incident_revision: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM query_scope_versions WHERE scope=$1")
                .bind(&request.scope)
                .fetch_optional(&mut *tx)
                .await?;
        let generation = crate::storage::sha256_hex_of(&serde_json::to_vec(&json!([
            request.scope,
            stream,
            incident_revision
        ]))?);
        ensure!(
            request.generation.as_ref().is_none_or(|g| g == &generation),
            "generation_expired"
        );
        let checkpoint = stream.as_ref().and_then(|s| s["checkpoint"].as_i64());
        if let Some(cp) = &request.checkpoint {
            ensure!(
                cp.parse::<i64>().ok() == checkpoint && checkpoint.is_some(),
                "generation_expired"
            );
        }
        let now = time::OffsetDateTime::now_utc();
        let age = stream
            .as_ref()
            .and_then(|s| s["checkpoint_time"].as_str())
            .and_then(|s| {
                time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
            })
            .map(|t| (now - t).whole_seconds());
        let evaluated = stream
            .as_ref()
            .and_then(|s| s["evaluated_checkpoint"].as_i64());
        let reconciled = stream
            .as_ref()
            .and_then(|s| s["reconciled_checkpoint"].as_i64());
        let supported = stream
            .as_ref()
            .is_some_and(|s| s["adapter"] == "suilend.cached.v1");
        let mut next = None;
        let mut resume_after = None;
        let mut history_start = None;
        let mut history_requires_bootstrap = None;
        let data = match kind {
            "status" | "summary" => {
                let counts: Value = sqlx::query_scalar("SELECT jsonb_build_object('positions',(SELECT count(*)::text FROM position_evaluations WHERE scope=$1),'open_incidents',(SELECT count(*)::text FROM incidents WHERE scope=$1 AND resolved_at IS NULL))")
                    .bind(&request.scope).fetch_one(&mut *tx).await?;
                json!({"projection":stream,"counts":counts})
            }
            "positions" => {
                ensure!(stream.is_some(), "stream_unavailable");
                if let Some(id) = &request.id {
                    ensure!(crate::config::is_full_address(id), "invalid_arguments");
                }
                let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('position_id',position_id,'last_recomputed_checkpoint',checkpoint::text,'diagnostics', CASE WHEN octet_length(evaluation::text)<=65536 THEN evaluation ELSE jsonb_build_object('error','evidence_too_large') END) FROM position_evaluations WHERE scope=$1 AND position_id>$2 AND ($3::text IS NULL OR position_id=$3) ORDER BY position_id LIMIT $4")
                    .bind(&request.scope).bind(request.after.as_deref().unwrap_or("")).bind(&request.id).bind(i64::from(request.limit)+1).fetch_all(&mut *tx).await?;
                page(rows, request.limit, "position_id", &mut next)
            }
            "diagnoses" => {
                let after = number(request.after.as_deref().unwrap_or("0"))?;
                let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',id::text,'rule_id',rule_id,'scope',scope,'severity',severity,'state',state,'checkpoint',checkpoint_sequence::text,'valuation_basis',valuation_basis,'opened_at',opened_at,'updated_at',updated_at,'resolved_at',resolved_at,'evidence_id',id::text) FROM incidents WHERE scope=$1 AND id>$2 ORDER BY id LIMIT $3")
                    .bind(&request.scope).bind(after).bind(i64::from(request.limit)+1).fetch_all(&mut *tx).await?;
                page(rows, request.limit, "id", &mut next)
            }
            "changes" => {
                ensure!(stream.is_some(), "stream_unavailable");
                let after = number(request.after.as_deref().unwrap_or("0"))?;
                let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('checkpoint',checkpoint::text,'payload_sha256',payload_hash,'change_count',CASE WHEN jsonb_typeof(raw_frame->'changes')='array' THEN jsonb_array_length(raw_frame->'changes') ELSE NULL END) FROM chain_frames WHERE scope=$1 AND checkpoint>$2 ORDER BY checkpoint LIMIT $3")
                    .bind(&request.scope).bind(after).bind(i64::from(request.limit)+1).fetch_all(&mut *tx).await?;
                page(rows, request.limit, "checkpoint", &mut next)
            }
            "incident_changes" => {
                let after = number(request.after.as_deref().unwrap_or("0"))?;
                ensure!(after <= incident_revision.unwrap_or(0), "invalid_arguments");
                let earliest:Option<(i64,String)>=sqlx::query_as("SELECT revision,event_kind FROM incident_changes WHERE scope=$1 ORDER BY revision LIMIT 1")
                    .bind(&request.scope).fetch_optional(&mut *tx).await?;
                history_start = earliest.as_ref().map(|(r, _)| r.to_string());
                history_requires_bootstrap = Some(earliest.as_ref().map_or(
                    after == 0 && incident_revision.unwrap_or(0) > 0,
                    |(r, k)| (after == 0 && *r > 1) || (after < *r && k == "bootstrap_required"),
                ));
                ensure!(
                    after == 0
                        || earliest
                            .as_ref()
                            .map_or(after == incident_revision.unwrap_or(0), |(first, _)| after
                                >= first - 1),
                    "history_expired"
                );
                let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('cursor',revision::text,'kind',event_kind,'recorded_at',recorded_at,'payload',payload) FROM incident_changes WHERE scope=$1 AND revision>$2 ORDER BY revision LIMIT $3")
                    .bind(&request.scope).bind(after).bind(i64::from(request.limit)+1).fetch_all(&mut *tx).await?;
                let mut result = page(rows, request.limit, "cursor", &mut next);
                if let Some(rows) = result.as_array_mut() {
                    for row in rows.iter_mut() {
                        row["sha256"] =
                            json!(crate::storage::sha256_hex_of(&serde_json::to_vec(row)?));
                    }
                    resume_after = Some(
                        rows.last()
                            .and_then(|r| r["cursor"].as_str())
                            .unwrap_or(request.after.as_deref().unwrap_or("0"))
                            .to_owned(),
                    );
                }
                result
            }
            "evidence" => {
                let id = number(
                    request
                        .id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("invalid_arguments"))?,
                )?;
                let evidence:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('incident_id',id::text,'rule_id',rule_id,'measured_trigger',CASE WHEN octet_length(measured_trigger::text)<=32768 THEN measured_trigger ELSE jsonb_build_object('error','evidence_too_large') END,'evidence',CASE WHEN octet_length(evidence::text)<=32768 THEN evidence ELSE jsonb_build_object('error','evidence_too_large') END) FROM incidents WHERE scope=$1 AND id=$2")
                    .bind(&request.scope).bind(id).fetch_optional(&mut *tx).await?;
                let evidence = evidence.ok_or_else(|| anyhow::anyhow!("evidence_not_found"))?;
                json!({"sha256":crate::storage::sha256_hex_of(&serde_json::to_vec(&evidence)?),"data":evidence})
            }
            _ => anyhow::bail!("unknown_query"),
        };
        tx.commit().await?;
        Ok(
            json!({"schema_version":1,"scope":request.scope,"generation":generation,
            "adapter":stream.as_ref().map(|s|s["adapter"].clone()),
            "configuration_hash":stream.as_ref().map(|s|s["configuration_hash"].clone()),
            "chain_identifier":stream.as_ref().map(|s|s["chain_identifier"].clone()),
            "checkpoint_digest":stream.as_ref().map(|s|s["digest"].clone()),
            "checkpoint":checkpoint.map(|c|c.to_string()),"queried_at":now.format(&time::format_description::well_known::Rfc3339)?,
            "observation_age_seconds":age,"observation_recent":age.is_some_and(|a|(0..=30).contains(&a)),
            "coverage":if stream.is_some() { "committed_projection_only" } else { "no_projection" },
            "projection_complete":supported && checkpoint.is_some() && evaluated==checkpoint,
            "last_reconciled_checkpoint":reconciled.map(|c|c.to_string()),
            "evaluation_lag_checkpoints":checkpoint.zip(evaluated).map(|(c,e)|(c-e).to_string()),
            "reconciliation_lag_checkpoints":checkpoint.zip(reconciled).map(|(c,r)|(c-r).to_string()),
            "provider_availability_lag_seconds":null,"valuation_time":null,"confidence":"unverified_cached_diagnostics",
            "fresh_risk_supported":false,"execution_eligible":false,"untrusted_evidence":true,
            "build":crate::build_identity::current(),"data":data,"next_after":next,"resume_after":resume_after,
            "history_start_cursor":history_start,"history_requires_bootstrap":history_requires_bootstrap,
            "pagination":"State pages require generation + next_after; changes fail with generation_expired. Incident change feed resumes with resume_after without a generation; bootstrap_required means older history is unknown. No historical valuation snapshot is implied."}),
        )
    }
}
fn number(text: &str) -> Result<i64> {
    let n = text
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("invalid_arguments"))?;
    ensure!(n >= 0, "invalid_arguments");
    Ok(n)
}
fn page(mut rows: Vec<Value>, limit: u32, key: &str, next: &mut Option<String>) -> Value {
    if rows.len() > limit as usize {
        rows.truncate(limit as usize);
        *next = rows.last().and_then(|r| r[key].as_str()).map(str::to_owned);
    }
    json!(rows)
}
