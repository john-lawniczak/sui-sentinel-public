pub mod admin;
pub mod follow;
pub mod live;
pub mod mcp;
pub mod monitor;
mod monitor_workers;
// One function per CLI command, so `main.rs`'s `match` is a thin
// dispatcher instead of a growing pile of inline handler bodies. Add a new
// command by adding a function here and one `match` arm in `main.rs`, not
// by making an existing arm longer.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use sui_sentinel::{
    analysis::{report, stress},
    config::Config,
    export, model,
};

use crate::client;

pub async fn serve(report: PathBuf, listen: SocketAddr) -> Result<()> {
    export::serve(&report, listen).await
}

pub async fn serve_dir(
    base: PathBuf,
    listen: SocketAddr,
    poll_interval_seconds: u64,
    policy: PathBuf,
) -> Result<()> {
    export::serve_dir_with_policy(
        &base,
        listen,
        Duration::from_secs(poll_interval_seconds),
        sui_sentinel::monitoring::Policy::load(&policy)?,
    )
    .await
}

pub fn check_config(config: PathBuf) -> Result<()> {
    Config::load(&config)?;
    println!("Configuration is locally valid: {}", config.display());
    println!(
        "Mainnet behavior is unverified; live monitoring remains blocked. \
         See docs/suilend-notes.md."
    );
    Ok(())
}

pub async fn markets(config: PathBuf, checkpoint: Option<u64>) -> Result<()> {
    let config = Config::load(config)?;
    let client = client(4)?;
    let checkpoint = client.checkpoint(checkpoint).await?;
    let markets =
        sui_sentinel::ingest::snapshot::discover(&client, &config, &checkpoint, 50).await?;
    let rows: Vec<_> = markets
        .iter()
        .map(|m| {
            serde_json::json!({
                "market_id": m.object.address, "type_argument": m.type_argument,
                "obligations": m.expected_obligations,
                "reserves": model::array(&m.object.contents, "reserves").map(|r| r.len()).unwrap_or(0),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "checkpoint": checkpoint, "scope": "type_instances_not_endorsements", "markets": rows
        }))?
    );
    Ok(())
}

/// Fields shared by `Command::Snapshot` and `Command::CaptureOnce` - the
/// knobs that pick scope and pacing for one capture, independent of
/// whether that capture is a one-shot `snapshot` or a supervised
/// `capture-once` attempt.
pub struct CaptureScope {
    pub config: PathBuf,
    pub checkpoint: Option<u64>,
    pub markets: Vec<String>,
    pub all_markets: bool,
    pub page_size: u32,
    pub concurrency: u32,
    pub max_pages: Option<u64>,
}

fn snapshot_lag(explicit: Option<u64>, configured: &str) -> Result<u64> {
    if explicit.is_some() {
        return Ok(0);
    }
    let lag: u64 = configured
        .parse()
        .context("SUI_SNAPSHOT_CHECKPOINT_LAG must be an unsigned integer")?;
    anyhow::ensure!(lag <= 10000, "snapshot checkpoint lag exceeds 10000");
    Ok(lag)
}

pub async fn snapshot(scope: CaptureScope, out: PathBuf) -> Result<()> {
    let config = Config::load(scope.config)?;
    let client = client(scope.concurrency as usize)?;
    sui_sentinel::runtime::progress::write("collecting", Some(client.transport()), None)?;
    // Choose once before collection. Never change checkpoint to repair a failed page.
    let lag = snapshot_lag(
        scope.checkpoint,
        &std::env::var("SUI_SNAPSHOT_CHECKPOINT_LAG").unwrap_or_else(|_| "0".into()),
    )?;
    let checkpoint = match scope.checkpoint {
        Some(cp) => Some(cp),
        None => {
            if lag == 0 {
                None
            } else {
                Some(
                    client
                        .checkpoint(None)
                        .await?
                        .sequence_number
                        .checked_sub(lag)
                        .context("checkpoint lag exceeds chain head")?,
                )
            }
        }
    };
    let capture = sui_sentinel::ingest::snapshot::capture(
        client.clone(),
        &config,
        sui_sentinel::ingest::snapshot::Options {
            checkpoint,
            markets: scope.markets,
            all_markets: scope.all_markets,
            page_size: scope.page_size,
            max_pages: scope.max_pages,
            out: out.clone(),
        },
    )
    .await;
    let phase = if capture.is_ok() {
        "captured"
    } else {
        "failed"
    };
    let failure = capture.as_ref().err().map(|e| {
        if e.to_string().contains("deployment changed") || e.to_string().contains("schema version")
        {
            "unsupported_deployment"
        } else if client.transport().failed_attempts > 0
            || e.to_string().contains("checkpoint scope unavailable")
        {
            "provider_failure"
        } else {
            "capture_error"
        }
    });
    sui_sentinel::runtime::progress::write(phase, Some(client.transport()), failure)?;
    let manifest = capture?;
    let report_dir = out.join("analysis");
    sui_sentinel::runtime::progress::write("analyzing", None, None)?;
    report::generate(&out, &report_dir)?;
    println!(
        "Captured checkpoint {}: {} positions in {} markets; complete={}.\nRaw data: {}\nCached diagnostics: {}\nFresh liquidation calculations and overall grade remain unverified.",
        manifest.checkpoint.sequence_number,
        manifest.markets.iter().map(|m| m.obligations).sum::<u64>(),
        manifest.markets.len(),
        manifest.complete,
        out.display(),
        report_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod snapshot_lag_tests {
    #[test]
    fn explicit_checkpoint_wins_and_implicit_lag_is_bounded() {
        assert_eq!(super::snapshot_lag(Some(123), "invalid").unwrap(), 0);
        assert_eq!(super::snapshot_lag(None, "200").unwrap(), 200);
        for value in ["-1", "10001", "invalid"] {
            assert!(super::snapshot_lag(None, value).is_err());
        }
    }
}

pub use capture::{CaptureOnceArgs, capture_once, validate_capture};
mod capture;

pub fn report(input: PathBuf, out: PathBuf, stress_config: Option<PathBuf>) -> Result<()> {
    let policy = match stress_config {
        Some(path) => stress::Policy::load(&path)?,
        None => stress::Policy::builtin()?,
    };
    let result = report::generate_with_policy(&input, &out, &policy)?;
    println!(
        "Report written to {}; capture_complete={}; calculation_status={}",
        out.display(),
        result.capture_complete,
        result.calculation_status
    );
    Ok(())
}

pub async fn probe(config: PathBuf) -> Result<()> {
    let config = Config::load(config)?;
    let source = sui_sentinel::ingest::grpc::Grpc::from_env()?;
    let info = source.info().await?;
    let checkpoint = source.checkpoint(None).await?;
    let market = source
        .object(&config.suilend.lending_market_id, None)
        .await?;
    let upgrade = source.object(&config.suilend.upgrade_cap_id, None).await?;
    let normalized = sui_sentinel::ingest::grpc::normalize::normalize(
        &source,
        &config,
        checkpoint.clone(),
        &std::collections::BTreeSet::new(),
    )
    .await?;
    use futures_util::StreamExt;
    let mut resume_at = checkpoint
        .sequence_number
        .context("missing checkpoint")?
        .saturating_sub(1);
    let mut smoke = Vec::new();
    for _ in 0..2 {
        let mut stream = Box::pin(source.resume(resume_at));
        for _ in 0..2 {
            loop {
                let item = tokio::time::timeout(Duration::from_secs(45), stream.next())
                    .await?
                    .context("gRPC stream ended")??;
                if let Some(cp) = item.checkpoint {
                    anyhow::ensure!(cp.sequence_number == Some(resume_at), "probe detected gap");
                    let frame = sui_sentinel::ingest::grpc::normalize::normalize(
                        &source,
                        &config,
                        cp,
                        &std::collections::BTreeSet::new(),
                    )
                    .await?;
                    smoke.push(
                        serde_json::json!({"checkpoint":resume_at,"changes":frame.changes.len()}),
                    );
                    resume_at += 1;
                    break;
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"provider":info,"checkpoint":checkpoint.sequence_number,"normalized_changes":normalized.changes.len(),"backfill_and_reconnect_probe":smoke,
        "market_id":market.address,"market_type":market.move_type,"market_version":market.contents["version"],
        "package":upgrade.contents,"sample_reserve_assets":market.contents["reserves"].as_array().map(|v|v.iter().map(|r|r["coin_type"].clone()).collect::<Vec<_>>())})
        )?
    );
    Ok(())
}
