use std::path::Path;
use sui_sentinel::{analysis::report, model::Manifest, storage};

#[test]
fn bound_raw_capture_detects_corruption_missing_manifest_and_config_changes() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");
    let base = std::env::temp_dir().join(format!("sentinel-raw-{}", std::process::id()));
    std::fs::create_dir(&base).unwrap();
    for file in std::fs::read_dir(&fixture).unwrap() {
        let file = file.unwrap();
        std::fs::copy(file.path(), base.join(file.file_name())).unwrap();
    }
    let mut manifest: Manifest = storage::read_json(&base.join("manifest.json")).unwrap();
    manifest.schema_version = 2;
    std::fs::write(
        base.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    storage::json(
        &base.join("capture-config.json"),
        &serde_json::json!({"test":true}),
    )
    .unwrap();
    storage::raw::seal(&base, &manifest).unwrap();
    let output = base.join("analysis");
    let report = report::generate(&base, &output).unwrap();
    assert_eq!(
        report.provenance.unwrap().raw_input_sha256.unwrap(),
        storage::sha256_hex(&base.join("checksums.json")).unwrap()
    );
    let raw = storage::market_file(&base, &manifest.markets[0].market.object.address, "caps");
    let original = std::fs::read(&raw).unwrap();
    std::fs::write(&raw, b"corrupt").unwrap();
    assert!(report::generate(&base, &base.join("bad")).is_err());
    assert!(!base.join("bad").exists());
    std::fs::write(raw, original).unwrap();
    std::fs::write(base.join("capture-config.json"), b"{}").unwrap();
    assert!(storage::raw::verify(&base, &manifest).is_err());
    std::fs::remove_file(base.join("checksums.json")).unwrap();
    assert!(report::generate(&base, &base.join("unsealed")).is_err());
    std::fs::remove_dir_all(base).unwrap();
}
