//! Distinct operational diagnoses with persisted debounce, recovery and reminders.
use super::{IncidentCandidate, Store, transition::Signal};
use crate::{monitoring::Policy, runtime::operations::Operations};
use anyhow::Result;
use serde_json::{Value, json};
use time::OffsetDateTime;
pub async fn observe(
    store: &Store,
    scope: &str,
    status: &Value,
    ops: &Operations,
    policy: &Policy,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let reason = status["readiness_reason"]
        .as_str()
        .unwrap_or("no_generation");
    let freshness = status["observation_freshness"].as_str();
    let rules = [
        ("missing_observation", reason == "no_generation"),
        ("coverage_missing", reason == "incomplete_coverage"),
        (
            "snapshot_freshness",
            matches!(freshness, Some("warn" | "critical")),
        ),
        ("scope_mismatch", reason == "scope_mismatch"),
        ("publication_failure", reason == "reload_failed"),
        (
            "unsupported_deployment",
            ops.last_error.as_deref() == Some("unsupported_deployment"),
        ),
        (
            "provider_failure",
            ops.last_error.as_deref() == Some("provider_failure"),
        ),
        (
            "disk_pressure",
            ops.disk_available_bytes
                .is_some_and(|n| n < policy.operations.min_free_bytes),
        ),
        (
            "delivery_failure",
            ops.notification_dead_letters.is_some_and(|n| n > 0)
                || ops
                    .notification_oldest_age_seconds
                    .is_some_and(|n| n > 300.0),
        ),
    ];
    let policy_hash = crate::storage::sha256_hex_of(&serde_json::to_vec(policy)?);
    for (rule, active) in rules {
        let key = format!("{rule}:{scope}");
        let id = now.unix_timestamp_nanos().to_string();
        let candidate = active.then(|| IncidentCandidate {
            dedup_key:key.clone(), rule_id:rule.into(), rule_version:3, scope:scope.into(),
            severity:if rule == "snapshot_freshness" && freshness == Some("warn") {"medium"} else {"high"}.into(),
            valuation_basis:"not_applicable".into(), checkpoint_sequence:None,
            measured_trigger:json!({"status":status,"operations":ops}),
            evidence:json!({"policy_sha256":policy_hash,"capability":"cached_monitoring","expected_markets":policy.readiness.expected_market_ids}),
        });
        store
            .apply_signal_policy(
                Signal {
                    id: &id,
                    observed_at: now,
                    key: &key,
                    scope,
                    candidate,
                    notify_unchanged: false,
                },
                Some(&policy.operations),
            )
            .await?;
    }
    Ok(())
}
