//! Versioned report artifacts shared by offline analysis and presentation.

pub use crate::decimal::DecimalString;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub checkpoint: crate::model::Checkpoint,
    pub scope: String,
    pub capture_complete: bool,
    pub calculation_status: String,
    pub overall_grade: Option<String>,
    pub limitations: Vec<String>,
    pub markets: Vec<MarketReport>,
    #[serde(default)]
    pub stress_policy: Option<crate::analysis::stress::Policy>,
    /// Added in a later schema revision; absent on reports generated before
    /// provenance tracking. Its absence does not downgrade an old report's
    /// `calculation_status` - it is simply unavailable for that artifact.
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

/// Identifies the adapter build, deployment, and valuation-time context that
/// produced a report, so a diagnosis can be reproduced from identified inputs
/// rather than taken on trust. Does not itself establish deployed-source or
/// oracle correspondence - see docs/suilend-notes.md for that research.
#[derive(Serialize, Deserialize)]
pub struct Provenance {
    #[serde(default)]
    pub build: Option<crate::build_identity::BuildIdentity>,
    #[serde(default)]
    pub artifact_revision: u32,
    #[serde(default)]
    pub raw_input_sha256: Option<String>,
    #[serde(default)]
    pub capture_config_sha256: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub adapter_version: String,
    pub chain_identifier: String,
    pub original_package_id: String,
    pub upgrade_cap_object_version: u64,
    /// sha256 of the canonical JSON encoding of the stress policy applied to
    /// this report, so two reports can be compared for identical policy
    /// without diffing the whole embedded policy document.
    pub stress_policy_sha256: String,
    /// Wall-clock report generation time, RFC3339. This is not the time
    /// of the cached valuations, which may differ between positions.
    /// `Report::checkpoint` retains the chain-observation time.
    pub generated_at: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Exposure {
    pub positions: u64,
    #[serde(with = "rust_decimal::serde::str")]
    pub cached_debt_usd: Decimal,
}

#[derive(Serialize, Deserialize)]
pub struct MarketReport {
    pub market_id: String,
    pub expected_positions: u64,
    pub captured_positions: u64,
    pub diagnostics_available: u64,
    pub diagnostics_unavailable: u64,
    pub address_owned_positions: u64,
    pub other_owned_positions: u64,
    pub unresolved_owner_positions: u64,
    pub ambiguous_owner_positions: u64,
    #[serde(default)]
    pub provisional_owner_positions: u64,
    pub unique_direct_wallets: usize,
    pub cached_health_buckets: BTreeMap<String, Exposure>,
    pub cached_collateral_by_asset_usd: BTreeMap<String, DecimalString>,
    pub cached_debt_by_asset_usd: BTreeMap<String, DecimalString>,
    pub largest_positions_by_cached_debt: Vec<(String, DecimalString)>,
    pub largest_direct_wallets_by_cached_debt: Vec<(String, DecimalString)>,
    pub reserves: Vec<Value>,
    #[serde(default)]
    pub stress: Option<crate::analysis::stress::Summary>,
}
