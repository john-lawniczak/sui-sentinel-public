use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use sui_sentinel::{analysis::report, model::Manifest, storage};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sui-sentinel-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir")
}

fn copy_fixture(destination: &Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(fixture()).unwrap() {
        let entry = entry.unwrap();
        if entry
            .path()
            .extension()
            .is_some_and(|e| e == "json" || e == "ndjson")
        {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn complete_mainnet_capture_reports_ownership_gaps_and_replays_deterministically() {
    let scratch = Scratch::new();
    let first = scratch.0.join("first");
    let second = scratch.0.join("second");
    let result = report::generate(&fixture(), &first).unwrap();
    report::generate(&fixture(), &second).unwrap();
    assert!(result.capture_complete);
    assert!(result.overall_grade.is_none());
    assert_eq!(result.calculation_status, "unverified_cached_diagnostics");
    let market = &result.markets[0];
    assert_eq!(market.captured_positions, 90);
    assert_eq!(market.diagnostics_available, 90);
    assert_eq!(market.address_owned_positions, 89);
    assert_eq!(market.unresolved_owner_positions, 1);
    assert_eq!(market.cached_health_buckets["at_least_1_25"].positions, 1);
    assert_eq!(
        market.cached_health_buckets["at_least_1_25"]
            .cached_debt_usd
            .to_string(),
        "59.9909004"
    );
    // positions.ndjson carries no generation-time metadata and must replay byte-identical.
    assert_eq!(
        fs::read(first.join("positions.ndjson")).unwrap(),
        fs::read(second.join("positions.ndjson")).unwrap()
    );
    // report.json replays identically except for provenance.generated_at (wall-clock
    // generation time, not valuation time) and the checksum manifest it feeds.
    let mut first_report: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join("report.json")).unwrap()).unwrap();
    let mut second_report: serde_json::Value =
        serde_json::from_slice(&fs::read(second.join("report.json")).unwrap()).unwrap();
    for report in [&mut first_report, &mut second_report] {
        assert!(report["provenance"]["generated_at"].is_string());
        report["provenance"]["generated_at"] = serde_json::Value::Null;
    }
    assert_eq!(first_report, second_report);
    for directory in [&first, &second] {
        storage::verify_checksums(directory, &["positions.ndjson", "report.json"]).unwrap();
    }
    assert!(report::generate(&fixture(), &first).is_err());
}

#[test]
fn refuses_truncated_or_duplicate_position_files() {
    for duplicate in [false, true] {
        let scratch = Scratch::new();
        let input = scratch.0.join("input");
        copy_fixture(&input);
        let manifest: Manifest = storage::read_json(&input.join("manifest.json")).unwrap();
        let path = storage::market_file(
            &input,
            &manifest.markets[0].market.object.address,
            "obligations",
        );
        let contents = fs::read_to_string(&path).unwrap();
        let first = contents.lines().next().unwrap();
        fs::write(
            &path,
            if duplicate {
                format!("{contents}{first}\n")
            } else {
                format!("{first}\n")
            },
        )
        .unwrap();
        let out = scratch.0.join("report");
        assert!(report::generate(&input, &out).is_err());
        assert!(!out.join("report.json").exists());
    }
}

#[test]
fn rejects_manifest_metadata_that_disagrees_with_raw_market() {
    let scratch = Scratch::new();
    let input = scratch.0.join("input");
    copy_fixture(&input);
    let path = input.join("manifest.json");
    let mut manifest: Manifest = storage::read_json(&path).unwrap();
    manifest.markets[0].market.expected_obligations = 1;
    manifest.markets[0].obligations = 1;
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(report::generate(&input, &scratch.0.join("out")).is_err());
}

#[test]
fn sampled_capture_stays_incomplete_and_does_not_invent_owners() {
    let scratch = Scratch::new();
    let input = scratch.0.join("input");
    copy_fixture(&input);
    let path = input.join("manifest.json");
    let mut manifest: Manifest = storage::read_json(&path).unwrap();
    let capture = &mut manifest.markets[0];
    let obligations = storage::market_file(&input, &capture.market.object.address, "obligations");
    let first = fs::read_to_string(&obligations)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    fs::write(obligations, format!("{first}\n")).unwrap();
    fs::write(
        storage::market_file(&input, &capture.market.object.address, "caps"),
        "",
    )
    .unwrap();
    capture.obligations = 1;
    capture.obligations_exhausted = false;
    capture.owner_caps = 0;
    capture.owner_caps_exhausted = false;
    manifest.complete = false;
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let report = report::generate(&input, &scratch.0.join("out")).unwrap();
    assert!(!report.capture_complete);
    assert_eq!(report.markets[0].unresolved_owner_positions, 1);
}

#[test]
fn refuses_mismatched_object_ids_and_reserve_indexes_on_replay() {
    for field in ["id", "reserve_array_index"] {
        let scratch = Scratch::new();
        let input = scratch.0.join("input");
        copy_fixture(&input);
        let manifest: Manifest = storage::read_json(&input.join("manifest.json")).unwrap();
        let path = storage::market_file(
            &input,
            &manifest.markets[0].market.object.address,
            "obligations",
        );
        let mut records: Vec<serde_json::Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        let record = records
            .iter_mut()
            .find(|r| !r["contents"]["deposits"].as_array().unwrap().is_empty())
            .unwrap();
        if field == "id" {
            record["contents"]["id"] = "0xdead".into();
        } else {
            record["contents"]["deposits"][0]["reserve_array_index"] = "999999".into();
        }
        fs::write(
            &path,
            records.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        assert!(report::generate(&input, &scratch.0.join("out")).is_err());
    }
}

#[test]
fn completion_marker_does_not_appear_when_serialization_fails() {
    use serde::{Serialize, Serializer, ser::Error};
    struct Broken;
    impl Serialize for Broken {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("deliberate failure"))
        }
    }
    let scratch = Scratch::new();
    let path = scratch.0.join("manifest.json");
    assert!(storage::json(&path, &Broken).is_err());
    assert!(!path.exists());
    let completed = scratch.0.join("report.json");
    storage::json(&completed, &serde_json::json!({"complete":true})).unwrap();
    let original = fs::read(&completed).unwrap();
    assert!(storage::json(&completed, &serde_json::json!({"complete":false})).is_err());
    assert_eq!(fs::read(completed).unwrap(), original);
}

#[test]
fn single_observed_cap_stays_provisional_until_capability_scan_is_exhaustive() {
    let scratch = Scratch::new();
    let input = scratch.0.join("input");
    copy_fixture(&input);
    let manifest_path = input.join("manifest.json");
    let mut manifest: Manifest = storage::read_json(&manifest_path).unwrap();
    let capture = &mut manifest.markets[0];
    capture.owner_caps_exhausted = false;
    manifest.complete = false;
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let out = scratch.0.join("out");
    let result = report::generate(&input, &out).unwrap();
    let market = &result.markets[0];
    // The fixture has 89 single-capability obligations; with an incomplete capability
    // scan none of them can be a definitive direct-address owner.
    assert_eq!(market.address_owned_positions, 0);
    assert_eq!(market.provisional_owner_positions, 89);
    assert_eq!(market.unresolved_owner_positions, 1);
    assert_eq!(market.unique_direct_wallets, 0);
    assert!(market.largest_direct_wallets_by_cached_debt.is_empty());
    let positions: Vec<serde_json::Value> = fs::read_to_string(out.join("positions.ndjson"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let provisional = positions
        .iter()
        .filter(|p| p["ownership_status"] == "provisional_incomplete_capability_scan")
        .count();
    assert_eq!(provisional, 89);
    // The observed capability ID is still preserved even while attribution is provisional.
    assert!(
        positions
            .iter()
            .filter(|p| p["ownership_status"] == "provisional_incomplete_capability_scan")
            .all(|p| !p["owner_cap_id"].is_null())
    );
}

#[test]
fn exhaustive_capability_scan_upgrades_provisional_ownership_to_definitive() {
    let scratch = Scratch::new();
    let incomplete = scratch.0.join("incomplete");
    let complete = scratch.0.join("complete");
    copy_fixture(&incomplete);
    let manifest_path = incomplete.join("manifest.json");
    let mut manifest: Manifest = storage::read_json(&manifest_path).unwrap();
    manifest.markets[0].owner_caps_exhausted = false;
    manifest.complete = false;
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let provisional = report::generate(&incomplete, &scratch.0.join("out_provisional")).unwrap();
    assert_eq!(provisional.markets[0].address_owned_positions, 0);
    assert_eq!(provisional.markets[0].provisional_owner_positions, 89);

    copy_fixture(&complete);
    let exhaustive = report::generate(&complete, &scratch.0.join("out_exhaustive")).unwrap();
    assert_eq!(exhaustive.markets[0].address_owned_positions, 89);
    assert_eq!(exhaustive.markets[0].provisional_owner_positions, 0);
}

#[test]
fn preserves_multiple_owner_caps_without_assigning_or_double_counting_debt() {
    let scratch = Scratch::new();
    let input = scratch.0.join("input");
    copy_fixture(&input);
    let manifest_path = input.join("manifest.json");
    let mut manifest: Manifest = storage::read_json(&manifest_path).unwrap();
    let capture = &mut manifest.markets[0];
    let path = storage::market_file(&input, &capture.market.object.address, "caps");
    let contents = fs::read_to_string(&path).unwrap();
    let mut extra: serde_json::Value =
        serde_json::from_str(contents.lines().next().unwrap()).unwrap();
    let new_id = format!("0x{}", "a".repeat(64));
    extra["address"] = new_id.clone().into();
    extra["contents"]["id"] = new_id.into();
    extra["owner"]["address"] = format!("0x{}", "b".repeat(64)).into();
    fs::write(path, format!("{contents}{extra}\n")).unwrap();
    capture.owner_caps += 1;
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let out = scratch.0.join("out");
    let result = report::generate(&input, &out).unwrap();
    assert!(result.capture_complete);
    assert_eq!(result.markets[0].ambiguous_owner_positions, 1);
    assert_eq!(result.markets[0].address_owned_positions, 88);
    assert_eq!(
        result.markets[0].cached_health_buckets["at_least_1_25"]
            .cached_debt_usd
            .to_string(),
        "59.9909004"
    );
    let positions: Vec<serde_json::Value> = fs::read_to_string(out.join("positions.ndjson"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let position = positions
        .iter()
        .find(|p| p["obligation_id"] == extra["contents"]["obligation_id"])
        .unwrap();
    assert_eq!(position["ownership_status"], "multiple_owner_capabilities");
    assert_eq!(position["owner_capabilities"].as_array().unwrap().len(), 2);
    assert!(position["owner"].is_null());
}

#[test]
fn non_wallet_owners_never_enter_wallet_totals_even_with_exhaustive_coverage() {
    for exhaustive in [false, true] {
        let scratch = Scratch::new();
        let input = scratch.0.join("input");
        copy_fixture(&input);
        let manifest_path = input.join("manifest.json");
        let mut manifest: Manifest = storage::read_json(&manifest_path).unwrap();
        let capture = &mut manifest.markets[0];
        capture.owner_caps_exhausted = exhaustive;
        let caps = storage::market_file(&input, &capture.market.object.address, "caps");
        let rewritten = fs::read_to_string(&caps)
            .unwrap()
            .lines()
            .map(|line| {
                let mut cap: serde_json::Value = serde_json::from_str(line).unwrap();
                cap["owner"]["kind"] = "ObjectOwner".into();
                format!("{cap}\n")
            })
            .collect::<String>();
        fs::write(caps, rewritten).unwrap();
        manifest.complete = exhaustive;
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let result = report::generate(&input, &scratch.0.join("report")).unwrap();
        let market = &result.markets[0];
        assert_eq!(
            market.other_owned_positions,
            if exhaustive { 89 } else { 0 }
        );
        assert_eq!(
            market.provisional_owner_positions,
            if exhaustive { 0 } else { 89 }
        );
        assert_eq!(market.address_owned_positions, 0);
        assert_eq!(market.unique_direct_wallets, 0);
        assert!(market.largest_direct_wallets_by_cached_debt.is_empty());
    }
}
