//! Bounded POSIX disk preflight. Linux release images include coreutils df.
use anyhow::{Context, Result, ensure};
use std::path::Path;
pub async fn available(base: &Path) -> Result<u64> {
    let mut command = tokio::process::Command::new("df");
    command
        .args(["-Pk"])
        .arg(base.canonicalize()?)
        .kill_on_drop(true);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await??;
    ensure!(
        output.status.success() && output.stdout.len() < 8192,
        "disk space probe failed"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let columns: Vec<_> = text
        .lines()
        .last()
        .context("missing disk result")?
        .split_whitespace()
        .collect();
    let blocks: u64 = columns
        .get(3)
        .context("missing available disk blocks")?
        .parse()?;
    blocks.checked_mul(1024).context("disk size overflow")
}
pub async fn preflight(base: &Path, minimum: u64) -> Result<u64> {
    let bytes = available(base).await?;
    ensure!(
        bytes >= minimum,
        "insufficient disk space: {bytes} available, {minimum} required"
    );
    Ok(bytes)
}
