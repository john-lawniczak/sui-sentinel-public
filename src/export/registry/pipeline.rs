//! Operational metrics use policy and observed status, never hard-coded UX limits.
use super::Status;
use crate::export::metrics::{family, sample};
use crate::monitoring::Policy;
fn add(out: &mut String, name: &str, help: &str, value: Option<f64>) {
    family(
        out,
        name,
        help,
        value.map(|v| sample([], v)).into_iter().collect::<Vec<_>>(),
    );
}
pub(super) fn render(status: &Status, policy: &Policy) -> String {
    let mut out = String::new();
    add(
        &mut out,
        "observation_age_seconds",
        "Age of chain observation; not report load age.",
        status.health.observation_age_seconds.map(|n| n as f64),
    );
    add(
        &mut out,
        "observation_freshness",
        "Policy freshness: 0 fresh, 1 warning, 2 critical; absent without observation.",
        status.health.observation_freshness.map(|s| match s {
            crate::monitoring::Freshness::Fresh => 0.0,
            crate::monitoring::Freshness::Warn => 1.0,
            crate::monitoring::Freshness::Critical => 2.0,
        }),
    );
    add(
        &mut out,
        "coverage_complete",
        "Complete cached diagnostic coverage.",
        Some(u8::from(status.health.coverage_complete) as f64),
    );
    add(
        &mut out,
        "policy_warn_age_seconds",
        "Deployed observation warning threshold.",
        Some((policy.capture.interval_seconds + policy.capture.deadline_seconds) as f64),
    );
    add(
        &mut out,
        "policy_critical_age_seconds",
        "Deployed observation critical threshold.",
        Some((2 * policy.capture.interval_seconds + policy.capture.deadline_seconds) as f64),
    );
    family(
        &mut out,
        "readiness_reason",
        "Current readiness reason.",
        [sample([("reason", status.health.readiness_reason)], 1)],
    );
    let Some(ops) = &status.operations else {
        return out;
    };
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    add(
        &mut out,
        "monitor_healthy",
        "Fresh supervisor, observer and delivery heartbeats with database available.",
        Some(u8::from(ops.healthy(now, policy.operations.heartbeat_max_age_seconds)) as f64),
    );
    for (name, time) in [
        ("scheduler_heartbeat_seconds", ops.scheduler_heartbeat),
        ("observer_heartbeat_seconds", ops.observer_heartbeat),
        ("delivery_heartbeat_seconds", ops.delivery_heartbeat),
    ] {
        add(
            &mut out,
            name,
            "Last successful worker heartbeat (Unix seconds).",
            time.map(|n| n as f64),
        );
    }
    add(
        &mut out,
        "database_available",
        "Most recent observed database availability; check heartbeat age.",
        ops.database_ok.map(|b| u8::from(b) as f64),
    );
    add(
        &mut out,
        "disk_available_bytes",
        "Observed free bytes on the capture filesystem.",
        ops.disk_available_bytes.map(|n| n as f64),
    );
    add(
        &mut out,
        "disk_minimum_bytes",
        "Disk preflight floor from policy.",
        Some(policy.operations.min_free_bytes as f64),
    );
    add(
        &mut out,
        "capture_active",
        "Whether a supervised capture is running.",
        Some(u8::from(ops.capture_active) as f64),
    );
    add(
        &mut out,
        "capture_duration_seconds",
        "Last completed capture duration.",
        ops.last_run_duration_seconds,
    );
    add(
        &mut out,
        "capture_failure_streak",
        "Consecutive failed capture attempts.",
        Some(ops.failure_streak as f64),
    );
    add(
        &mut out,
        "scheduler_missed_ticks",
        "Observed missed scheduling intervals since counter origin.",
        Some(ops.missed_ticks as f64),
    );
    add(
        &mut out,
        "spool_backlog",
        "Observed unapplied lifecycle journal entries.",
        ops.spool_backlog.map(|n| n as f64),
    );
    add(
        &mut out,
        "notification_pending",
        "Pending delivery entries.",
        ops.notification_pending.map(|n| n as f64),
    );
    add(
        &mut out,
        "notification_oldest_age_seconds",
        "Oldest undelivered queue age; absent when empty or unknown.",
        ops.notification_oldest_age_seconds,
    );
    add(
        &mut out,
        "notification_attempts",
        "Total retained delivery attempts.",
        ops.notification_attempts.map(|n| n as f64),
    );
    add(
        &mut out,
        "notification_dead_letters",
        "Entries requiring operator recovery.",
        ops.notification_dead_letters.map(|n| n as f64),
    );
    if let Some(t) = &ops.transport {
        for (name, value) in [
            ("graphql_attempts", t.attempts),
            ("graphql_retries", t.retries),
            ("graphql_throttled", t.throttled),
            ("graphql_response_bytes", t.response_bytes),
        ] {
            add(
                &mut out,
                name,
                "Last observed capture transport counter; may be partial during failure.",
                Some(value as f64),
            );
        }
    }
    out
}
