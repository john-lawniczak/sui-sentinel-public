//! Pure per-position stress arithmetic; input verification belongs to the adapter.

mod policy;
pub use policy::{Policy, Scenario};

use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub type Assets = [(String, Decimal)];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub debt_usd: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub shortfall_usd: Decimal,
    pub shortfall_positions: u64,
    /// Configured `price_factors` asset keys that matched at least one
    /// evaluated collateral or debt leg in this scenario.
    #[serde(default)]
    pub matched_price_overrides: BTreeSet<String>,
    /// Configured `price_factors` asset keys that matched none - a likely
    /// typo or an asset that never appears in this capture's exposures.
    /// Illustrative-only; does not gate the stress score or report
    /// generation. Publishing this is explicit so a silently-ignored
    /// override is visible rather than invisible.
    #[serde(default)]
    pub unmatched_price_overrides: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub evaluated_positions: u64,
    pub excluded_positions: u64,
    pub scenarios: Vec<Outcome>,
    pub status: Status,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub score: Option<Decimal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Available,
    UnverifiedCachedInputs,
    IncompleteCoverage,
    NoDebt,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::UnverifiedCachedInputs => "unverified_cached_inputs",
            Self::IncompleteCoverage => "incomplete_coverage",
            Self::NoDebt => "no_debt",
        }
    }
}

/// A caller must establish fresh valuations and validated recovery assumptions
/// before supplying Verified. There is deliberately no CLI flag to bypass this.
pub enum Basis {
    Cached,
    Verified,
}

pub struct Accumulator {
    policy: Policy,
    summary: Summary,
    pending: Vec<(Decimal, Decimal, u64)>,
    matched: Vec<BTreeSet<String>>,
}

impl Accumulator {
    pub fn new(policy: &Policy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy: policy.clone(),
            pending: Vec::with_capacity(policy.scenarios.len()),
            matched: vec![BTreeSet::new(); policy.scenarios.len()],
            summary: Summary {
                evaluated_positions: 0,
                excluded_positions: 0,
                status: Status::UnverifiedCachedInputs,
                score: None,
                scenarios: policy
                    .scenarios
                    .iter()
                    .map(|s| Outcome {
                        name: s.name.clone(),
                        debt_usd: Decimal::ZERO,
                        shortfall_usd: Decimal::ZERO,
                        shortfall_positions: 0,
                        matched_price_overrides: BTreeSet::new(),
                        unmatched_price_overrides: BTreeSet::new(),
                    })
                    .collect(),
            },
        })
    }

    pub fn exclude(&mut self) -> Result<()> {
        self.summary.excluded_positions = self
            .summary
            .excluded_positions
            .checked_add(1)
            .context("position count overflow")?;
        Ok(())
    }

    pub fn add(&mut self, collateral: &Assets, debt: &Assets) -> Result<()> {
        // Calculate the complete position before committing it to the accumulator.
        self.pending.clear();
        for (scenario, total) in self.policy.scenarios.iter().zip(&self.summary.scenarios) {
            let debt = reprice(debt, scenario)?;
            let recovery = reprice(collateral, scenario)?
                .checked_mul(Decimal::ONE - scenario.recovery_discount)
                .context("recovery overflow")?;
            let loss = debt
                .checked_sub(recovery)
                .context("shortfall overflow")?
                .max(Decimal::ZERO);
            self.pending.push((
                total
                    .debt_usd
                    .checked_add(debt)
                    .context("debt total overflow")?,
                total
                    .shortfall_usd
                    .checked_add(loss)
                    .context("loss total overflow")?,
                total
                    .shortfall_positions
                    .checked_add(u64::from(loss > Decimal::ZERO))
                    .context("shortfall count overflow")?,
            ));
        }
        let count = self
            .summary
            .evaluated_positions
            .checked_add(1)
            .context("position count overflow")?;
        // Coverage belongs to committed evaluations too: an overflow in any
        // scenario must not make an excluded position's overrides look applied.
        for (scenario, matched) in self.policy.scenarios.iter().zip(&mut self.matched) {
            track_matches(debt, scenario, matched);
            track_matches(collateral, scenario, matched);
        }
        for (total, &(debt, loss, count)) in self.summary.scenarios.iter_mut().zip(&self.pending) {
            total.debt_usd = debt;
            total.shortfall_usd = loss;
            total.shortfall_positions = count;
        }
        self.summary.evaluated_positions = count;
        Ok(())
    }

    pub fn finish(mut self, complete: bool, basis: Basis) -> Result<Summary> {
        for ((scenario, outcome), matched) in self
            .policy
            .scenarios
            .iter()
            .zip(&mut self.summary.scenarios)
            .zip(&self.matched)
        {
            outcome.matched_price_overrides = matched.clone();
            outcome.unmatched_price_overrides = scenario
                .price_factors
                .keys()
                .filter(|asset| !matched.contains(*asset))
                .cloned()
                .collect();
        }
        self.summary.status = if !complete || self.summary.excluded_positions > 0 {
            Status::IncompleteCoverage
        } else if matches!(basis, Basis::Cached) {
            Status::UnverifiedCachedInputs
        } else if let Some(value) = score(&self.summary.scenarios, self.policy.loss_tolerance)? {
            self.summary.score = Some(value);
            Status::Available
        } else {
            Status::NoDebt
        };
        Ok(self.summary)
    }
}

fn track_matches(assets: &Assets, scenario: &Scenario, matched: &mut BTreeSet<String>) {
    for (asset, _) in assets {
        if scenario.price_factors.contains_key(asset) {
            matched.insert(asset.clone());
        }
    }
}

fn reprice(assets: &Assets, scenario: &Scenario) -> Result<Decimal> {
    assets
        .iter()
        .try_fold(Decimal::ZERO, |total, (asset, amount)| {
            ensure!(*amount >= Decimal::ZERO, "negative stress exposure");
            let factor = scenario
                .price_factors
                .get(asset)
                .map_or(scenario.default_price_factor, |x| x.0);
            total
                .checked_add(amount.checked_mul(factor).context("reprice overflow")?)
                .context("exposure sum overflow")
        })
}

/// Higher is worse. Scenario debt is the denominator; absent debt is not zero risk.
pub fn score(outcomes: &[Outcome], tolerance: Decimal) -> Result<Option<Decimal>> {
    ensure!(
        tolerance > Decimal::ZERO && tolerance <= Decimal::ONE,
        "invalid loss tolerance"
    );
    let mut worst: Option<Decimal> = None;
    for row in outcomes {
        ensure!(
            row.debt_usd >= Decimal::ZERO
                && row.shortfall_usd >= Decimal::ZERO
                && row.shortfall_usd <= row.debt_usd,
            "invalid scenario totals"
        );
        if row.debt_usd > Decimal::ZERO {
            let ratio = row
                .shortfall_usd
                .checked_div(row.debt_usd)
                .context("loss ratio overflow")?;
            worst = Some(worst.map_or(ratio, |old| old.max(ratio)));
        }
    }
    worst
        .map(|loss| {
            loss.checked_div(tolerance)
                .context("score overflow")?
                .min(Decimal::ONE)
                .checked_mul(Decimal::from(100))
                .context("score overflow")
        })
        .transpose()
}
