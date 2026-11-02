//! Observational rules. Every production signal has a stable identity and time.
use super::{IncidentCandidate, Store, transition::Signal};
use crate::{monitoring, runtime};
use anyhow::Result;
use serde_json::json;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub async fn apply_capture_outcome(
    store: &Store,
    scope: &str,
    run: &runtime::RunRecord,
) -> Result<()> {
    if !matches!(
        run.state,
        runtime::State::Failed | runtime::State::Published
    ) {
        return Ok(());
    }
    let key = format!("capture_failure:{scope}");
    let id = format!("{}:{}", run.output_directory, run.started_at);
    let at = OffsetDateTime::parse(&run.started_at, &Rfc3339)?;
    let candidate = if run.state == runtime::State::Failed {
        Some(IncidentCandidate {
            dedup_key: key.clone(),
            rule_id: "capture_failure".into(),
            rule_version: 2,
            scope: scope.into(),
            severity: "high".into(),
            valuation_basis: "not_applicable".into(),
            checkpoint_sequence: run.checkpoint.map(i64::try_from).transpose()?,
            measured_trigger: json!({"error_category":run.error_category,"error_message":run.error_message}),
            evidence: json!({"output_directory":run.output_directory,"run_started_at":run.started_at}),
        })
    } else {
        None
    };
    store
        .apply_signal(Signal {
            id: &id,
            observed_at: at,
            key: &key,
            scope,
            candidate,
            notify_unchanged: true,
        })
        .await
}

/// `signal_id` must be retained when retrying an observation. `observed_at`
/// orders signals so historical replay cannot override newer diagnoses.
pub async fn observe_snapshot_freshness(
    store: &Store,
    scope: &str,
    signal_id: &str,
    observed_at: OffsetDateTime,
    age_seconds: Option<u64>,
    freshness: monitoring::Freshness,
) -> Result<()> {
    let key = format!("snapshot_freshness:{scope}");
    let candidate = if freshness == monitoring::Freshness::Fresh {
        None
    } else {
        Some(IncidentCandidate {
            dedup_key: key.clone(),
            rule_id: "snapshot_freshness".into(),
            rule_version: 2,
            scope: scope.into(),
            severity: if freshness == monitoring::Freshness::Critical {
                "high"
            } else {
                "medium"
            }
            .into(),
            valuation_basis: "not_applicable".into(),
            checkpoint_sequence: None,
            measured_trigger: json!({"age_seconds":age_seconds}),
            evidence: json!({"observed_at":observed_at.to_string()}),
        })
    };
    store
        .apply_signal(Signal {
            id: signal_id,
            observed_at,
            key: &key,
            scope,
            candidate,
            notify_unchanged: false,
        })
        .await
}

#[cfg(test)]
async fn apply_snapshot_freshness(
    store: &Store,
    scope: &str,
    age: u64,
    freshness: monitoring::Freshness,
) -> Result<()> {
    let at = OffsetDateTime::now_utc();
    observe_snapshot_freshness(
        store,
        scope,
        &at.unix_timestamp_nanos().to_string(),
        at,
        Some(age),
        freshness,
    )
    .await
}
#[cfg(test)]
mod tests;
