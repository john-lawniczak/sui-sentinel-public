//! Tail complete normalized checkpoint files, in sequence, through the state
//! actor. A provider adapter must supply these files atomically; no latest-read
//! polling is presented as a complete chain-change stream.
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{io::Read, path::PathBuf, time::Duration};
use sui_sentinel::{
    config::Config,
    diagnosis::Store,
    protocols::registry,
    state::{Engine, Frame, worker},
};
#[derive(Args)]
pub struct FollowArgs {
    #[arg(long)]
    pub bootstrap: PathBuf,
    #[arg(long)]
    pub input_dir: PathBuf,
    #[arg(long)]
    pub scope: String,
    #[arg(long, default_value = "deploy/config/suilend.toml")]
    pub config: PathBuf,
    #[arg(long, env = "DIAGNOSIS_DATABASE_URL", hide_env_values = true)]
    pub database_url: String,
    /// Compare this independently captured census and exit.
    #[arg(long)]
    pub reconcile: Option<PathBuf>,
    /// Apply all currently available contiguous frames, then exit.
    #[arg(long)]
    pub once: bool,
}
pub async fn follow(args: FollowArgs) -> Result<()> {
    let store = Store::connect(&args.database_url).await?;
    store.prepare().await?;
    let engine = Engine::new(
        store,
        registry::build(
            "suilend.cached.v1",
            serde_json::to_value(Config::load(&args.config)?)?,
        )?,
        args.scope,
    )?;
    engine.bootstrap(&args.bootstrap).await?;
    if let Some(path) = args.reconcile {
        return engine.reconcile(&path).await;
    }
    let mut cp = engine.checkpoint().await?;
    let (sender, task) = worker::spawn(engine, 2)?;
    loop {
        let next = cp.checked_add(1).context("checkpoint overflow")?;
        let path = args.input_dir.join(format!("{next}.json"));
        if !path.try_exists()? {
            if args.once {
                break;
            }
            tokio::select! {_=tokio::signal::ctrl_c()=>break,_=tokio::time::sleep(Duration::from_millis(500))=>{}}
            continue;
        }
        let frame = tokio::task::spawn_blocking(move || -> Result<Frame> {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 32 * 1024 * 1024,
                "checkpoint frame exceeds 32 MiB limit"
            );
            Ok(serde_json::from_slice(&bytes)?)
        })
        .await??;
        ensure!(
            frame.checkpoint.sequence_number == next,
            "frame filename/sequence mismatch"
        );
        let (committed, ack) = tokio::sync::oneshot::channel();
        sender
            .send(worker::Request { frame, committed })
            .await
            .context("state actor stopped")?;
        ack.await
            .context("state actor stopped before commit acknowledgment")??;
        cp = next;
        println!(
            "Committed observation checkpoint {cp}; valuation basis remains unverified cached diagnostics"
        );
    }
    drop(sender);
    task.await?;
    Ok(())
}
