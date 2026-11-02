use anyhow::{Result, ensure};
use rust_decimal::Decimal;

use super::metrics::{family, sample};
use crate::analysis::{report::Report, stress::Status};

pub fn validate(report: &Report) -> Result<()> {
    if let Some(policy) = &report.stress_policy {
        policy.validate()?;
    }
    for market in &report.markets {
        let Some(summary) = &market.stress else {
            ensure!(
                report.stress_policy.is_none(),
                "stress policy without market results"
            );
            continue;
        };
        let policy = report
            .stress_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("stress results without policy"))?;
        ensure!(
            summary.score.is_none(),
            "cached inputs cannot publish an economic score"
        );
        let expected_status = if !report.capture_complete || summary.excluded_positions > 0 {
            Status::IncompleteCoverage
        } else {
            Status::UnverifiedCachedInputs
        };
        ensure!(
            summary.status == expected_status,
            "stress status disagrees with coverage or valuation basis"
        );
        ensure!(
            summary
                .evaluated_positions
                .checked_add(summary.excluded_positions)
                == Some(market.captured_positions),
            "stress coverage mismatch"
        );
        ensure!(
            summary.evaluated_positions <= market.diagnostics_available,
            "stress exceeds diagnostic coverage"
        );
        let unknown = market
            .cached_health_buckets
            .get("unknown_zero_cached_debt")
            .map_or(0, |b| b.positions);
        ensure!(
            market.diagnostics_unavailable.checked_add(unknown) == Some(summary.excluded_positions),
            "stress exclusions disagree with diagnostic gaps"
        );
        ensure!(
            summary.scenarios.len() == policy.scenarios.len(),
            "stress scenario count mismatch"
        );
        for (result, scenario) in summary.scenarios.iter().zip(&policy.scenarios) {
            ensure!(
                result.name == scenario.name
                    && result.debt_usd >= Decimal::ZERO
                    && result.shortfall_usd >= Decimal::ZERO
                    && result.shortfall_usd <= result.debt_usd
                    && result.shortfall_positions <= summary.evaluated_positions
                    && (result.shortfall_usd.is_zero() == (result.shortfall_positions == 0))
                    && (summary.evaluated_positions > 0 || result.debt_usd.is_zero()),
                "invalid stress result"
            );
        }
    }
    Ok(())
}

pub fn render(report: &Report, out: &mut String) {
    family(
        out,
        "economic_score_available",
        "One only when the market economic score is supported by verified inputs.",
        report.markets.iter().map(|m| {
            sample(
                [("market", m.market_id.as_str())],
                u8::from(m.stress.as_ref().is_some_and(|s| s.score.is_some())),
            )
        }),
    );
    family(
        out,
        "economic_score",
        "Economic stress score from zero to 100, higher is worse; absent when unavailable.",
        report.markets.iter().filter_map(|m| {
            m.stress
                .as_ref()?
                .score
                .map(|s| sample([("market", m.market_id.as_str())], s))
        }),
    );
    family(
        out,
        "stress_status",
        "Stress model availability; cached scenario outputs are illustrative only.",
        report.markets.iter().map(|m| {
            sample(
                [
                    ("market", m.market_id.as_str()),
                    (
                        "status",
                        m.stress
                            .as_ref()
                            .map_or("not_computed", |s| s.status.as_str()),
                    ),
                ],
                1,
            )
        }),
    );
    family(
        out,
        "stress_positions",
        "Positions included or excluded from cached sensitivity calculations.",
        report
            .markets
            .iter()
            .filter_map(|m| m.stress.as_ref().map(|s| (m, s)))
            .flat_map(|(m, s)| {
                [
                    sample(
                        [("market", m.market_id.as_str()), ("status", "evaluated")],
                        s.evaluated_positions,
                    ),
                    sample(
                        [("market", m.market_id.as_str()), ("status", "excluded")],
                        s.excluded_positions,
                    ),
                ]
            }),
    );
    family(
        out,
        "cached_stress_debt_usd",
        "Hypothetical repriced debt on evaluated positions; cached inputs and assumed recoveries.",
        report.markets.iter().flat_map(|m| {
            m.stress.iter().flat_map(|s| s.scenarios.iter()).map(|s| {
                sample(
                    [
                        ("market", m.market_id.as_str()),
                        ("scenario", s.name.as_str()),
                    ],
                    s.debt_usd,
                )
            })
        }),
    );
    family(
        out,
        "cached_stress_shortfall_usd",
        "Sum of per-position hypothetical shortfalls; not measured bad debt or expected loss.",
        report.markets.iter().flat_map(|m| {
            m.stress.iter().flat_map(|s| s.scenarios.iter()).map(|s| {
                sample(
                    [
                        ("market", m.market_id.as_str()),
                        ("scenario", s.name.as_str()),
                    ],
                    s.shortfall_usd,
                )
            })
        }),
    );
    family(
        out,
        "stress_loss_tolerance_ratio",
        "Policy loss rate that corresponds to a score of 100; not a probability.",
        report
            .stress_policy
            .iter()
            .map(|p| sample([], p.loss_tolerance)),
    );
}
