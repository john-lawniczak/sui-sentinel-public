//! Raw snapshot integrity. Schema 2 requires the entire replay input set;
//! schema 1 remains explicitly unbound legacy evidence.
use crate::model::Manifest;
use anyhow::{Result, ensure};
use std::path::Path;

fn files(manifest: &Manifest) -> Result<Vec<String>> {
    let mut files = vec!["manifest.json".to_owned(), "capture-config.json".to_owned()];
    for market in &manifest.markets {
        ensure!(
            crate::config::is_full_address(&market.market.object.address),
            "invalid raw market ID"
        );
        for kind in ["obligations", "caps"] {
            files.push(format!("{}.{kind}.ndjson", market.market.object.address));
        }
    }
    Ok(files)
}
pub fn seal(path: &Path, manifest: &Manifest) -> Result<()> {
    let files = files(manifest)?;
    super::write_checksums(path, &files.iter().map(String::as_str).collect::<Vec<_>>())
}
pub fn verify(path: &Path, manifest: &Manifest) -> Result<()> {
    ensure!(
        matches!(manifest.schema_version, 1 | 2),
        "unsupported raw schema"
    );
    if manifest.schema_version == 2 || path.join("checksums.json").exists() {
        let files = files(manifest)?;
        super::verify_checksums(path, &files.iter().map(String::as_str).collect::<Vec<_>>())?;
    }
    Ok(())
}
