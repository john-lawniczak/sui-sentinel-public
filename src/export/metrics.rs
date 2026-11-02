//! Prometheus text exposition. Wallet/obligation addresses never become labels.

use std::fmt::{Display, Write};

use anyhow::{Context, Result};
use rust_decimal::Decimal;

use super::validate;
use crate::analysis::report::{MarketReport, Report};

pub(super) struct Sample {
    labels: Vec<(&'static str, String)>,
    value: String,
}

pub(super) fn sample<const N: usize>(
    labels: [(&'static str, &str); N],
    value: impl Display,
) -> Sample {
    Sample {
        labels: labels.into_iter().map(|(k, v)| (k, v.to_owned())).collect(),
        value: value.to_string(),
    }
}

pub(super) fn family(
    out: &mut String,
    name: &str,
    help: &str,
    samples: impl IntoIterator<Item = Sample>,
) {
    let name = format!("sui_sentinel_{name}");
    // Writing to a String cannot fail.
    writeln!(out, "# HELP {name} {help}\n# TYPE {name} gauge").unwrap();
    for row in samples {
        out.push_str(&name);
        if !row.labels.is_empty() {
            out.push('{');
            for (index, (key, value)) in row.labels.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write!(out, "{key}=\"{}\"", escape(value)).unwrap();
            }
            out.push('}');
        }
        writeln!(out, " {}", row.value).unwrap();
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

pub fn render(report: &Report) -> Result<String> {
    let timestamp = validate::report(report)?;
    let mut out = String::new();
    family(
        &mut out,
        "snapshot_info",
        "Snapshot provenance; metrics are cached and unverified.",
        [sample(
            [
                ("scope", &report.scope),
                ("calculation_status", &report.calculation_status),
                ("checkpoint_time", &report.checkpoint.timestamp),
            ],
            1,
        )],
    );
    family(
        &mut out,
        "snapshot_checkpoint",
        "Sui checkpoint sequence number.",
        [sample([], report.checkpoint.sequence_number)],
    );
    family(
        &mut out,
        "snapshot_timestamp_seconds",
        "Source checkpoint time, not scrape time.",
        [sample(
            [],
            Decimal::from_i128_with_scale(timestamp.unix_timestamp_nanos(), 9),
        )],
    );
    family(
        &mut out,
        "snapshot_complete",
        "One if the selected-market census is complete; not valuation verification.",
        [sample([], u8::from(report.capture_complete))],
    );
    family(
        &mut out,
        "snapshot_markets",
        "Number of market type instances in this report.",
        [sample([], report.markets.len())],
    );
    type Count = fn(&MarketReport) -> u64;
    for (name, help, count) in [
        (
            "positions",
            "Captured obligations by market.",
            (|m: &MarketReport| m.captured_positions) as Count,
        ),
        (
            "expected_positions",
            "Obligation table size at the source checkpoint.",
            (|m: &MarketReport| m.expected_positions) as Count,
        ),
        (
            "unique_direct_wallets",
            "Distinct single-cap direct wallets within a market; not additive across markets.",
            (|m: &MarketReport| m.unique_direct_wallets as u64) as Count,
        ),
    ] {
        family(
            &mut out,
            name,
            help,
            report
                .markets
                .iter()
                .map(|m| sample([("market", &m.market_id)], count(m))),
        );
    }
    family(
        &mut out,
        "diagnostic_positions",
        "Cached diagnostic coverage by market.",
        report.markets.iter().flat_map(|m| {
            [
                sample(
                    [("market", &m.market_id), ("status", "available")],
                    m.diagnostics_available,
                ),
                sample(
                    [("market", &m.market_id), ("status", "unavailable")],
                    m.diagnostics_unavailable,
                ),
            ]
        }),
    );
    family(
        &mut out,
        "ownership_positions",
        "Position ownership resolution; multiple capabilities are ambiguous.",
        report.markets.iter().flat_map(|m| {
            [
                sample(
                    [("market", &m.market_id), ("status", "direct_address")],
                    m.address_owned_positions,
                ),
                sample(
                    [("market", &m.market_id), ("status", "other")],
                    m.other_owned_positions,
                ),
                sample(
                    [("market", &m.market_id), ("status", "unresolved")],
                    m.unresolved_owner_positions,
                ),
                sample(
                    [("market", &m.market_id), ("status", "ambiguous")],
                    m.ambiguous_owner_positions,
                ),
                sample(
                    [("market", &m.market_id), ("status", "provisional")],
                    m.provisional_owner_positions,
                ),
            ]
        }),
    );
    family(
        &mut out,
        "cached_health_positions",
        "Positions grouped by cached HF; below one does not prove current liquidation eligibility.",
        report.markets.iter().flat_map(|m| {
            validate::BUCKETS.map(|bucket| {
                sample(
                    [("market", &m.market_id), ("bucket", bucket)],
                    m.cached_health_buckets
                        .get(bucket)
                        .map_or(0, |b| b.positions),
                )
            })
        }),
    );
    family(
        &mut out,
        "cached_health_debt_usd",
        "Unweighted cached debt by HF bucket; valuations may originate at different times.",
        report.markets.iter().flat_map(|m| {
            validate::BUCKETS.map(|bucket| {
                sample(
                    [("market", &m.market_id), ("bucket", bucket)],
                    m.cached_health_buckets
                        .get(bucket)
                        .map_or(Decimal::ZERO, |b| b.cached_debt_usd),
                )
            })
        }),
    );
    family(
        &mut out,
        "cached_collateral_usd",
        "Cached obligation collateral by asset; not protocol TVL or reserve supply.",
        report.markets.iter().flat_map(|m| {
            m.cached_collateral_by_asset_usd
                .iter()
                .map(|(asset, amount)| {
                    sample(
                        [
                            ("market", &m.market_id),
                            ("asset", asset),
                            ("asset_name", &super::assets::display_name(asset)),
                        ],
                        amount.0,
                    )
                })
        }),
    );
    family(
        &mut out,
        "cached_debt_usd",
        "Cached obligation debt by asset.",
        report.markets.iter().flat_map(|m| {
            m.cached_debt_by_asset_usd.iter().map(|(asset, amount)| {
                sample(
                    [
                        ("market", &m.market_id),
                        ("asset", asset),
                        ("asset_name", &super::assets::display_name(asset)),
                    ],
                    amount.0,
                )
            })
        }),
    );
    let top_wallets = report
        .markets
        .iter()
        .map(|m| {
            let debt = m.largest_direct_wallets_by_cached_debt.iter().try_fold(
                Decimal::ZERO,
                |total, (_, amount)| {
                    total
                        .checked_add(amount.0)
                        .context("top wallet total overflow")
                },
            )?;
            Ok(sample([("market", &m.market_id)], debt))
        })
        .collect::<Result<Vec<_>>>()?;
    family(
        &mut out,
        "cached_top20_wallet_debt_usd",
        "Cached debt held by up to 20 largest directly attributable wallets per market.",
        top_wallets,
    );
    super::stress::render(report, &mut out);
    Ok(out)
}
