//! Buffered JSON artifacts; the manifest is written only after successful capture.

mod pointer;
pub mod raw;
pub use pointer::{write_pointer, write_pointer_before};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub fn writer(path: &Path) -> Result<BufWriter<File>> {
    Ok(BufWriter::with_capacity(
        256 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("creating {} (outputs must not exist)", path.display()))?,
    ))
}

pub fn line(output: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

pub fn json(path: &Path, value: &impl Serialize) -> Result<()> {
    // Publish only after serialization and fsync. hard_link is atomic and cannot
    // overwrite an existing completion marker (unlike rename on Unix).
    let temporary = path.with_extension("json.partial");
    let mut output = writer(&temporary)?;
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    output.get_ref().sync_all()?;
    fs::hard_link(&temporary, path).with_context(|| format!("publishing {}", path.display()))?;
    fs::remove_file(temporary)?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(BufReader::new(File::open(path)?))
        .with_context(|| format!("reading {}", path.display()))
}

pub fn read_lines<T: DeserializeOwned>(
    path: &Path,
    mut visit: impl FnMut(T) -> Result<()>,
) -> Result<()> {
    let mut reader = BufReader::with_capacity(256 * 1024, File::open(path)?);
    let mut buffer = String::new();
    let mut index = 0;
    loop {
        buffer.clear();
        if reader.read_line(&mut buffer)? == 0 {
            break;
        }
        index += 1;
        let value = serde_json::from_str(&buffer)
            .with_context(|| format!("{} line {index}", path.display()))?;
        visit(value).with_context(|| format!("{} line {index}", path.display()))?;
    }
    Ok(())
}

pub fn market_file(directory: &Path, market: &str, kind: &str) -> PathBuf {
    directory.join(format!("{market}.{kind}.ndjson"))
}

pub fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir(path)
        .with_context(|| format!("creating new output directory {}", path.display()))
}

#[derive(Serialize, Deserialize)]
pub struct Checksums {
    pub algorithm: String,
    pub files: BTreeMap<String, String>,
}

pub fn sha256_hex(path: &Path) -> Result<String> {
    let mut reader = BufReader::with_capacity(256 * 1024, File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 256 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_hex_of(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind a named set of sibling files in `directory` into one checksum manifest,
/// written and fsynced like any other completion artifact. The manifest itself
/// is not atomically published; callers write it only after every bound file
/// has been flushed and synced, and before any external consumer is told the
/// directory is complete.
pub fn write_checksums(directory: &Path, files: &[&str]) -> Result<()> {
    let mut bound = BTreeMap::new();
    for name in files {
        bound.insert((*name).to_owned(), sha256_hex(&directory.join(name))?);
    }
    json(
        &directory.join("checksums.json"),
        &Checksums {
            algorithm: "sha256".to_owned(),
            files: bound,
        },
    )
}

/// Verify every file named in `directory/checksums.json` still matches its
/// recorded digest, and that the manifest covers exactly `expected` files -
/// catching both corruption and a manifest that silently stopped binding a
/// file. Local checksums detect corruption; they do not establish
/// cryptographic correspondence with any upstream provider's own digests.
pub fn verify_checksums(directory: &Path, expected: &[&str]) -> Result<()> {
    let checksums: Checksums = read_json(&directory.join("checksums.json"))?;
    ensure!(
        checksums.algorithm == "sha256",
        "unsupported checksum algorithm"
    );
    let expected: std::collections::BTreeSet<&str> = expected.iter().copied().collect();
    let bound: std::collections::BTreeSet<&str> =
        checksums.files.keys().map(String::as_str).collect();
    ensure!(
        bound == expected,
        "checksum manifest does not cover the expected artifact set"
    );
    for (name, recorded) in &checksums.files {
        let actual = sha256_hex(&directory.join(name))?;
        if actual != *recorded {
            bail!("checksum mismatch for {name}: recorded {recorded}, actual {actual}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        // A per-process atomic counter, not just a nanosecond timestamp:
        // parallel test threads can observe the same clock tick on
        // coarser-resolution clocks, and a nanosecond-only name has
        // caused real collisions (and flaky failures) under `cargo test`'s
        // default parallelism.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sui-sentinel-storage-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn verifies_bound_files_and_detects_corruption_and_scope_drift() {
        let directory = scratch();
        fs::write(directory.join("a.txt"), b"alpha").unwrap();
        fs::write(directory.join("b.txt"), b"beta").unwrap();
        write_checksums(&directory, &["a.txt", "b.txt"]).unwrap();
        verify_checksums(&directory, &["a.txt", "b.txt"]).unwrap();

        // Corrupting a bound file after the manifest was written is detected.
        fs::write(directory.join("a.txt"), b"tampered").unwrap();
        assert!(verify_checksums(&directory, &["a.txt", "b.txt"]).is_err());

        // Restoring the file but asking for a different expected set is also rejected,
        // even though the manifest's own files still match each other.
        fs::write(directory.join("a.txt"), b"alpha").unwrap();
        assert!(verify_checksums(&directory, &["a.txt"]).is_err());
        assert!(verify_checksums(&directory, &["a.txt", "b.txt", "c.txt"]).is_err());

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn missing_checksum_manifest_fails_rather_than_treating_absence_as_valid() {
        let directory = scratch();
        fs::write(directory.join("a.txt"), b"alpha").unwrap();
        assert!(verify_checksums(&directory, &["a.txt"]).is_err());
        fs::remove_dir_all(&directory).unwrap();
    }
}
