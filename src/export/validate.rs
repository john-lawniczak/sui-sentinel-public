use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{analysis::report::Report, config::is_full_address};

pub const BUCKETS: [&str; 7] = [
    "below_1",
    "1_to_1_05",
    "1_05_to_1_1",
    "1_1_to_1_25",
    "at_least_1_25",
    "no_debt",
    "unknown_zero_cached_debt",
];

pub fn report(report: &Report) -> Result<OffsetDateTime> {
    ensure!(report.schema_version == 1, "unsupported report schema");
    ensure!(
        report.calculation_status == "unverified_cached_diagnostics"
            && report.overall_grade.is_none(),
        "unsupported valuation status or grade; exporter only supports cached diagnostics"
    );
    ensure!(
        matches!(
            report.scope.as_str(),
            "explicit_markets" | "all_discovered_type_instances_not_an_endorsement_registry"
        ),
        "unknown report scope"
    );
    ensure!(
        !report.checkpoint.digest.is_empty() && report.checkpoint.sequence_number < (1 << 53),
        "invalid checkpoint provenance"
    );
    let timestamp = OffsetDateTime::parse(&report.checkpoint.timestamp, &Rfc3339)
        .context("invalid checkpoint timestamp")?;
    ensure!(
        timestamp.unix_timestamp() >= 0,
        "checkpoint predates Unix epoch"
    );
    ensure!(
        !report.markets.is_empty() && report.markets.len() <= 256,
        "expected 1..=256 markets"
    );
    let mut markets = HashSet::new();
    for market in &report.markets {
        ensure!(
            is_full_address(&market.market_id) && markets.insert(&market.market_id),
            "invalid or duplicate market ID"
        );
        ensure!(
            market.captured_positions <= market.expected_positions,
            "capture exceeds expected positions"
        );
        ensure!(
            !report.capture_complete || market.captured_positions == market.expected_positions,
            "complete report has missing positions"
        );
        ensure!(
            sum([market.diagnostics_available, market.diagnostics_unavailable])?
                == market.captured_positions,
            "diagnostic coverage mismatch"
        );
        ensure!(
            sum([
                market.address_owned_positions,
                market.other_owned_positions,
                market.unresolved_owner_positions,
                market.ambiguous_owner_positions,
                market.provisional_owner_positions
            ])? == market.captured_positions,
            "ownership coverage mismatch"
        );
        ensure!(
            market.provisional_owner_positions == 0 || !report.capture_complete,
            "complete report cannot contain provisional ownership"
        );
        ensure!(
            market.unique_direct_wallets as u64 <= market.address_owned_positions,
            "wallet count exceeds direct positions"
        );
        ensure!(
            sum(market.cached_health_buckets.values().map(|b| b.positions))?
                == market.diagnostics_available,
            "health bucket coverage mismatch"
        );
        for (bucket, exposure) in &market.cached_health_buckets {
            ensure!(BUCKETS.contains(&bucket.as_str()), "unknown health bucket");
            ensure!(
                exposure.cached_debt_usd >= Decimal::ZERO,
                "negative bucket debt"
            );
        }
        for assets in [
            &market.cached_collateral_by_asset_usd,
            &market.cached_debt_by_asset_usd,
        ] {
            ensure!(assets.len() <= 1024, "too many assets");
            for (asset, amount) in assets {
                ensure!(
                    !asset.is_empty() && asset.len() <= 1024 && amount.0 >= Decimal::ZERO,
                    "invalid asset exposure"
                );
            }
        }
        for top in [
            &market.largest_positions_by_cached_debt,
            &market.largest_direct_wallets_by_cached_debt,
        ] {
            ensure!(top.len() <= 20, "top exposure list exceeds 20");
            for (address, amount) in top {
                ensure!(
                    is_full_address(address) && amount.0 >= Decimal::ZERO,
                    "invalid top exposure"
                );
            }
        }
    }
    super::stress::validate(report)?;
    Ok(timestamp)
}

fn sum(values: impl IntoIterator<Item = u64>) -> Result<u64> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total.checked_add(value).context("count overflow")
    })
}
