//! Store only readiness inputs alongside the rendered immutable generation.
use crate::{
    analysis::report::Report,
    monitoring::{Freshness, Policy},
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) struct Observation {
    at: Option<OffsetDateTime>,
    complete: bool,
    scope: String,
    markets: Vec<String>,
}
impl Observation {
    pub fn from_report(report: &Report) -> Self {
        Self {
            at: OffsetDateTime::parse(&report.checkpoint.timestamp, &Rfc3339).ok(),
            complete: report.capture_complete
                && !report.markets.is_empty()
                && report.markets.iter().all(|m| {
                    m.captured_positions == m.expected_positions
                        && m.diagnostics_unavailable == 0
                        && m.provisional_owner_positions == 0
                }),
            scope: report.scope.clone(),
            markets: report.markets.iter().map(|m| m.market_id.clone()).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct Health {
    #[serde(skip)]
    pub ready: bool,
    pub readiness_reason: &'static str,
    pub observation_age_seconds: Option<u64>,
    pub observation_freshness: Option<Freshness>,
    pub coverage_complete: bool,
    // Cached aggregates do not provide a common valuation timestamp.
    pub valuation_age_seconds: Option<u64>,
    pub valuation_basis: &'static str,
}

pub(super) fn evaluate(
    observation: Option<&Observation>,
    policy: &Policy,
    now: OffsetDateTime,
    reload_failed: bool,
) -> Health {
    let mut health = Health {
        ready: false,
        readiness_reason: "no_generation",
        observation_age_seconds: None,
        observation_freshness: None,
        coverage_complete: false,
        valuation_age_seconds: None,
        valuation_basis: "unverified_cached_diagnostics",
    };
    let Some(observation) = observation else {
        return health;
    };
    health.coverage_complete = observation.complete;
    let Some(at) = observation.at else {
        health.readiness_reason = "invalid_observation_time";
        return health;
    };
    if at > now {
        health.readiness_reason = "future_observation_time";
        return health;
    }
    let age = (now - at).whole_seconds() as u64;
    let freshness = policy.snapshot_age_status(age);
    health.observation_age_seconds = Some(age);
    health.observation_freshness = Some(freshness);
    let mut expected = policy.readiness.expected_market_ids.clone();
    let mut actual = observation.markets.clone();
    expected.sort();
    actual.sort();
    health.readiness_reason = if reload_failed {
        "reload_failed"
    } else if !observation.complete {
        "incomplete_coverage"
    } else if observation.scope != policy.readiness.expected_scope
        || (!expected.is_empty() && actual != expected)
    {
        "scope_mismatch"
    } else if freshness != Freshness::Fresh {
        "stale_observation"
    } else {
        "ready"
    };
    health.ready = health.readiness_reason == "ready";
    health
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn age_coverage_scope_and_clock_are_independent_of_load_time() {
        let policy = Policy::builtin().unwrap();
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(1);
        let mut input = Observation {
            at: Some(now),
            complete: true,
            scope: "explicit_markets".into(),
            markets: vec!["m".into()],
        };
        assert!(evaluate(Some(&input), &policy, now, false).ready);
        assert!(
            !evaluate(
                Some(&input),
                &policy,
                now + time::Duration::seconds(361),
                false
            )
            .ready
        );
        assert!(!evaluate(Some(&input), &policy, now, true).ready);
        input.complete = false;
        assert_eq!(
            evaluate(Some(&input), &policy, now, false).readiness_reason,
            "incomplete_coverage"
        );
        input.complete = true;
        input.scope = "all".into();
        assert_eq!(
            evaluate(Some(&input), &policy, now, false).readiness_reason,
            "scope_mismatch"
        );
        input.at = Some(now + time::Duration::seconds(1));
        assert_eq!(
            evaluate(Some(&input), &policy, now, false).readiness_reason,
            "future_observation_time"
        );
        input.at = None;
        assert_eq!(
            evaluate(Some(&input), &policy, now, false).readiness_reason,
            "invalid_observation_time"
        );
    }
}
