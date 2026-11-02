//! Offline, deterministic reports over streamed capture files.

use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Write,
    path::Path,
};

use crate::{
    analysis::{risk, stress},
    config::MAINNET_CHAIN,
    model::{Manifest, Market, Object, Owner, array, integer, string},
    storage,
};

mod types;
pub use types::{DecimalString, Exposure, MarketReport, Provenance, Report};

#[derive(Serialize)]
struct PositionReport<'a> {
    market_id: &'a str,
    obligation_id: &'a str,
    object_version: u64,
    owner_cap_id: Option<&'a str>,
    owner: Option<&'a Owner>,
    owner_capabilities: &'a [CapabilityOwner],
    ownership_status: &'static str,
    cached_diagnostics: Option<risk::Diagnostics>,
    diagnostic_error: Option<String>,
}

#[derive(Serialize)]
struct CapabilityOwner {
    cap_id: String,
    owner: Option<Owner>,
}

pub fn generate(input: &Path, out: &Path) -> Result<Report> {
    generate_with_policy(input, out, &stress::Policy::builtin()?)
}

pub fn generate_with_policy(input: &Path, out: &Path, policy: &stress::Policy) -> Result<Report> {
    policy.validate()?;
    let manifest: Manifest = storage::read_json(&input.join("manifest.json"))?;
    ensure!(
        matches!(manifest.schema_version, 1 | 2),
        "unsupported snapshot schema version"
    );
    crate::storage::raw::verify(input, &manifest)?;
    ensure!(
        manifest.chain_identifier == MAINNET_CHAIN,
        "not a mainnet snapshot"
    );
    ensure!(!manifest.markets.is_empty(), "snapshot contains no markets");
    ensure!(
        crate::config::is_full_address(&manifest.original_package_id),
        "invalid original package ID"
    );
    manifest.upgrade_cap.validate()?;
    manifest.clock.validate()?;
    integer(&manifest.clock.contents, "timestamp_ms")?;
    ensure!(
        manifest.complete
            == manifest.markets.iter().all(|c| c.obligations_exhausted
                && c.owner_caps_exhausted
                && c.obligations == c.market.expected_obligations),
        "invalid complete manifest"
    );
    for capture in &manifest.markets {
        let derived = Market::new(capture.market.object.clone(), &manifest.original_package_id)?;
        ensure!(
            derived.table_id == capture.market.table_id
                && derived.type_argument == capture.market.type_argument
                && derived.expected_obligations == capture.market.expected_obligations,
            "market metadata disagrees with raw contents"
        );
        ensure!(
            capture.obligations <= derived.expected_obligations,
            "capture exceeds table size"
        );
    }
    storage::create_directory(out)?;
    let mut positions = storage::writer(&out.join("positions.ndjson"))?;
    let mut summaries = Vec::new();
    let mut seen_markets = HashSet::new();
    for capture in &manifest.markets {
        let market = &capture.market;
        let market_id = &market.object.address;
        ensure!(
            crate::config::is_full_address(market_id),
            "invalid market address"
        );
        ensure!(seen_markets.insert(market_id), "duplicate market");
        ensure!(
            !capture.obligations_exhausted || capture.obligations == market.expected_obligations,
            "invalid completeness declaration"
        );
        let mut caps: HashMap<String, Vec<CapabilityOwner>> = HashMap::new();
        let mut cap_ids = HashSet::new();
        storage::read_lines(
            &storage::market_file(input, market_id, "caps"),
            |cap: Object| {
                cap.validate()?;
                ensure!(
                    cap.move_type == market.cap_type(&manifest.original_package_id),
                    "cap type mismatch"
                );
                ensure!(cap_ids.insert(cap.address.clone()), "duplicate cap ID");
                let obligation = string(&cap.contents, "obligation_id")?.to_owned();
                ensure!(
                    crate::config::is_full_address(&obligation),
                    "invalid capability obligation ID"
                );
                caps.entry(obligation).or_default().push(CapabilityOwner {
                    cap_id: cap.address,
                    owner: cap.owner,
                });
                Ok(())
            },
        )?;
        ensure!(
            cap_ids.len() as u64 == capture.owner_caps,
            "owner cap file count mismatch"
        );
        let mut summary = MarketReport {
            market_id: market_id.clone(), expected_positions: market.expected_obligations, captured_positions: 0,
            diagnostics_available: 0, diagnostics_unavailable: 0, address_owned_positions: 0,
            other_owned_positions: 0, unresolved_owner_positions: 0, ambiguous_owner_positions: 0,
            provisional_owner_positions: 0, unique_direct_wallets: 0,
            cached_health_buckets: BTreeMap::new(), cached_collateral_by_asset_usd: BTreeMap::new(),
            cached_debt_by_asset_usd: BTreeMap::new(), largest_positions_by_cached_debt: Vec::new(),
            largest_direct_wallets_by_cached_debt: Vec::new(), stress: None,
            reserves: array(&market.object.contents, "reserves")?.iter().map(|r| json!({
                "coin_type":r["coin_type"], "array_index":r["array_index"], "mint_decimals":r["mint_decimals"],
                "config":r["config"], "price":r["price"], "smoothed_price":r["smoothed_price"],
                "price_last_update_timestamp_s":r["price_last_update_timestamp_s"],
                "interest_last_update_timestamp_s":r["interest_last_update_timestamp_s"]
            })).collect(),
        };
        let mut wallets = HashMap::<String, Decimal>::new();
        let mut stress = stress::Accumulator::new(policy)?;
        let mut ids = HashSet::new();
        storage::read_lines(
            &storage::market_file(input, market_id, "obligations"),
            |object: Object| {
                market.validate_obligation(&object, &manifest.original_package_id)?;
                ensure!(ids.insert(object.address.clone()), "duplicate obligation");
                summary.captured_positions += 1;
                let owner_capabilities = caps
                    .get(&object.address)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let cap = if owner_capabilities.len() == 1 {
                    owner_capabilities.first()
                } else {
                    None
                };
                // A single observed capability is only a DEFINITIVE owner when the capability
                // scan for this market is exhaustive. An incomplete scan may be missing a
                // second, unseen capability for the same obligation, so single-cap resolution
                // stays provisional and is excluded from wallet totals until coverage completes.
                let scan_exhaustive = capture.owner_caps_exhausted;
                let owner = cap
                    .filter(|_| scan_exhaustive)
                    .and_then(|c| c.owner.as_ref());
                let ownership_status = if owner_capabilities.len() > 1 {
                    summary.ambiguous_owner_positions += 1;
                    "multiple_owner_capabilities"
                } else if owner_capabilities.len() == 1 && !scan_exhaustive {
                    summary.provisional_owner_positions += 1;
                    "provisional_incomplete_capability_scan"
                } else {
                    match owner {
                        Some(owner)
                            if matches!(
                                owner.kind.as_str(),
                                "AddressOwner" | "ConsensusAddressOwner"
                            ) && owner.address.is_some() =>
                        {
                            summary.address_owned_positions += 1;
                            wallets
                                .entry(owner.address.as_ref().unwrap().clone())
                                .or_default();
                            "direct_address_owner"
                        }
                        Some(owner)
                            if matches!(
                                owner.kind.as_str(),
                                "ObjectOwner" | "Shared" | "Immutable"
                            ) =>
                        {
                            summary.other_owned_positions += 1;
                            "non_wallet_or_unknown_owner_kind"
                        }
                        _ => {
                            summary.unresolved_owner_positions += 1;
                            "unresolved"
                        }
                    }
                };
                let result = risk::assess(&object).and_then(|diagnostics| {
                    Ok((
                        diagnostics,
                        assets(&object.contents, "deposits")?,
                        assets(&object.contents, "borrows")?,
                    ))
                });
                let (diagnostics, error) = match result {
                    Ok((d, deposits, borrows)) => {
                        if d.health_bucket == "unknown_zero_cached_debt" {
                            stress.exclude()?;
                        } else {
                            stress.add(&deposits, &borrows)?;
                        }
                        let debt = risk::usd(&object.contents["unweighted_borrowed_value_usd"])?;
                        let bucket = summary
                            .cached_health_buckets
                            .entry(d.health_bucket.to_owned())
                            .or_default();
                        bucket.positions += 1;
                        bucket.cached_debt_usd = bucket
                            .cached_debt_usd
                            .checked_add(debt)
                            .context("bucket total overflow")?;
                        add_assets(&mut summary.cached_collateral_by_asset_usd, deposits)?;
                        add_assets(&mut summary.cached_debt_by_asset_usd, borrows)?;
                        if ownership_status == "direct_address_owner"
                            && let Some(total) = owner
                                .and_then(|o| o.address.as_ref())
                                .and_then(|a| wallets.get_mut(a))
                        {
                            *total = total.checked_add(debt).context("wallet debt overflow")?;
                        }
                        if !debt.is_zero() {
                            summary
                                .largest_positions_by_cached_debt
                                .push((object.address.clone(), DecimalString(debt)));
                            summary
                                .largest_positions_by_cached_debt
                                .sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
                            summary.largest_positions_by_cached_debt.truncate(20);
                        }
                        summary.diagnostics_available += 1;
                        (Some(d), None)
                    }
                    Err(error) => {
                        stress.exclude()?;
                        summary.diagnostics_unavailable += 1;
                        (None, Some(error.to_string()))
                    }
                };
                storage::line(
                    &mut positions,
                    &PositionReport {
                        market_id,
                        obligation_id: &object.address,
                        object_version: object.version,
                        owner_cap_id: cap.map(|c| c.cap_id.as_str()),
                        owner,
                        owner_capabilities,
                        ownership_status,
                        cached_diagnostics: diagnostics,
                        diagnostic_error: error,
                    },
                )?;
                Ok(())
            },
        )?;
        ensure!(
            summary.captured_positions == capture.obligations,
            "obligation file count mismatch"
        );
        summary.unique_direct_wallets = wallets.len();
        summary.largest_direct_wallets_by_cached_debt = wallets
            .into_iter()
            .filter(|(_, debt)| !debt.is_zero())
            .map(|(owner, debt)| (owner, DecimalString(debt)))
            .collect();
        summary
            .largest_direct_wallets_by_cached_debt
            .sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        summary.largest_direct_wallets_by_cached_debt.truncate(20);
        summary.stress = Some(stress.finish(manifest.complete, stress::Basis::Cached)?);
        summaries.push(summary);
    }
    positions.flush()?;
    positions.get_ref().sync_all()?;
    let provenance = Provenance {
        build: Some(crate::build_identity::current()),
        artifact_revision: 2,
        raw_input_sha256: if manifest.schema_version >= 2 {
            Some(storage::sha256_hex(&input.join("checksums.json"))?)
        } else {
            None
        },
        capture_config_sha256: if manifest.schema_version >= 2 {
            Some(storage::sha256_hex(&input.join("capture-config.json"))?)
        } else {
            None
        },
        run_id: input.file_name().map(|s| s.to_string_lossy().into_owned()),
        adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        chain_identifier: manifest.chain_identifier.clone(),
        original_package_id: manifest.original_package_id.clone(),
        upgrade_cap_object_version: manifest.upgrade_cap.version,
        stress_policy_sha256: storage::sha256_hex_of(&serde_json::to_vec(policy)?),
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .context("formatting generation timestamp")?,
    };
    let report = Report {
        schema_version: 1,
        checkpoint: manifest.checkpoint,
        scope: manifest.scope,
        capture_complete: manifest.complete,
        calculation_status: "unverified_cached_diagnostics".to_owned(),
        overall_grade: None,
        provenance: Some(provenance),
        limitations: vec![
            "Cached obligation valuations may originate at different times; these are not refreshed checkpoint health factors.",
            "Current deployed source correspondence and effective oracle mappings remain unverified.",
            "Ratios use checked decimal arithmetic; they are not Move fixed-point execution equivalence proofs.",
            "Missing or wrapped owner capabilities are unresolved; object-owner addresses are not wallet addresses.",
            "Multiple capabilities for one obligation are preserved and excluded from single-wallet debt attribution.",
            "Obligation collateral is not total protocol supply; deposits may exist outside obligations.",
            "Stress scenarios use cached inputs and assumed recovery discounts; they are illustrative, not executable liquidation simulations. Economic scores and overall grades remain unavailable with unverified inputs.",
        ].into_iter().map(str::to_owned).collect(),
        markets: summaries,
        stress_policy: Some(policy.clone()),
    };
    storage::json(&out.join("report.json"), &report)?;
    storage::write_checksums(out, &["positions.ndjson", "report.json"])?;
    Ok(report)
}

fn add_assets(
    totals: &mut BTreeMap<String, DecimalString>,
    assets: Vec<(String, Decimal)>,
) -> Result<()> {
    for (asset, amount) in assets {
        let total = totals.entry(asset).or_default();
        total.0 = total
            .0
            .checked_add(amount)
            .context("asset total overflow")?;
    }
    Ok(())
}

fn assets(value: &Value, side: &str) -> Result<Vec<(String, Decimal)>> {
    array(value, side)?
        .iter()
        .map(|leg| {
            Ok((
                string(leg, "coin_type")?.to_owned(),
                risk::usd(&leg["market_value"])?,
            ))
        })
        .collect()
}
