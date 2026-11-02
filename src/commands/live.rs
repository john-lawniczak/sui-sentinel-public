//! Public-provider development mode. Start from a complete GraphQL census;
//! durable commit acknowledgments, not stream cursors, determine resumption.
use anyhow::{Context, Result, ensure};
use clap::Args;
use futures_util::StreamExt;
use std::{path::PathBuf, time::Duration};
use sui_sentinel::{
    config::Config,
    diagnosis::Store,
    ingest::grpc::{Grpc, normalize},
    protocols::registry,
    runtime,
    state::{Engine, worker},
    storage,
};

#[derive(Args)]
pub struct LiveArgs {
    /// Complete schema-2 GraphQL snapshot. If absent, capture one here first.
    #[arg(long)]
    pub bootstrap: PathBuf,
    #[arg(long, default_value = "suilend-main")]
    pub scope: String,
    #[arg(long, default_value = "deploy/config/suilend.toml")]
    pub config: PathBuf,
    #[arg(long, env = "DIAGNOSIS_DATABASE_URL", hide_env_values = true)]
    pub database_url: String,
    #[arg(long)]
    pub reconciliation_dir: PathBuf,
    #[arg(long,default_value_t=10000,value_parser=clap::value_parser!(u64).range(1..))]
    pub reconcile_every: u64,
    #[arg(long,default_value_t=600,value_parser=clap::value_parser!(u64).range(1..))]
    pub census_deadline_seconds: u64,
    /// Bounded development run; omitted means follow until stopped.
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..))]
    pub max_checkpoints: Option<u64>,
}

pub async fn live(args: LiveArgs) -> Result<()> {
    let config = Config::load(&args.config)?;
    ensure!(
        config.suilend.lending_market_id == sui_sentinel::protocols::suilend::MAIN_MARKET_ID,
        "live development scope must be the SDK main market"
    );
    let source = Grpc::from_env()?;
    source.info().await?;
    let store = Store::connect(&args.database_url).await?;
    store.prepare().await?;
    std::fs::create_dir_all(&args.reconciliation_dir)?;
    // One live coordinator per database scope: a session lock includes the
    // reconciliation pause, unlike only locking each checkpoint transaction.
    let _lease = store.scope_lease(&format!("live:{}", args.scope)).await?;
    if !args.bootstrap.try_exists()? {
        let cp = crate::client(2)?.checkpoint(None).await?.sequence_number;
        census(&args, &args.bootstrap, cp).await?;
    }
    let engine = Engine::new(
        store,
        registry::build(
            "suilend.cached.v1",
            serde_json::to_value(Config::load(&args.config)?)?,
        )?,
        args.scope.clone(),
    )?;
    let bootstrap_started = std::time::Instant::now();
    engine.bootstrap(&args.bootstrap).await?;
    println!(
        "{}",
        serde_json::json!({"event":"bootstrap_ready", "checkpoint":engine.checkpoint().await?,
        "elapsed_seconds":bootstrap_started.elapsed().as_secs_f64()})
    );
    let info = source.info().await?;
    let start = engine
        .checkpoint()
        .await?
        .checked_add(1)
        .context("checkpoint overflow")?;
    ensure!(
        info.lowest_available_checkpoint_objects
            .is_none_or(|low| start >= low),
        "restart checkpoint is outside provider object retention; configure a retained-history provider or explicitly rebootstrap a new scope"
    );
    let (sender, task) = worker::spawn(engine.clone(), 2)?;
    let result = consume(&args, &config, &source, &engine, &sender).await;
    drop(sender);
    task.await?;
    result
}

async fn consume(
    args: &LiveArgs,
    config: &Config,
    source: &Grpc,
    engine: &Engine,
    sender: &tokio::sync::mpsc::Sender<worker::Request>,
) -> Result<()> {
    let mut processed = 0_u64;
    let mut delay = 1_u64;
    loop {
        let next = engine
            .checkpoint()
            .await?
            .checked_add(1)
            .context("checkpoint overflow")?;
        let mut stream = Box::pin(source.resume(next));
        let stop = super::monitor::shutdown();
        tokio::pin!(stop);
        loop {
            let message = tokio::select! {_=&mut stop=>return Ok(()), item=stream.next()=>item};
            let item = match message {
                Some(Ok(item)) => item,
                Some(Err(error)) if retryable(error.code()) => break,
                Some(Err(error)) => return Err(error.into()),
                None => break,
            };
            let Some(checkpoint) = item.checkpoint else {
                continue;
            }; // Progress is not committed state.
            let wanted = engine
                .checkpoint()
                .await?
                .checked_add(1)
                .context("checkpoint overflow")?;
            ensure!(
                checkpoint.sequence_number == Some(wanted),
                "gRPC stream skipped/reordered a checkpoint"
            );
            let normalize_started = std::time::Instant::now();
            let tracked = engine.tracked(&normalize::changes_ids(&checkpoint)).await?;
            let normalization = tokio::time::timeout(
                Duration::from_secs(120),
                normalize::normalize(source, config, checkpoint, &tracked),
            );
            let outcome = tokio::select! {_=&mut stop=>return Ok(()), result=normalization=>result};
            let frame = match outcome {
                Ok(Ok(frame)) => frame,
                Err(_) => break,
                Ok(Err(error))
                    if error
                        .downcast_ref::<tonic::Status>()
                        .is_some_and(|s| retryable(s.code())) =>
                {
                    break;
                }
                Ok(Err(error)) => return Err(error),
            };
            let normalize_seconds = normalize_started.elapsed().as_secs_f64();
            let changes = frame.changes.len();
            let checkpoint_time = time::OffsetDateTime::parse(
                &frame.checkpoint.timestamp,
                &time::format_description::well_known::Rfc3339,
            )?;
            let commit_started = std::time::Instant::now();
            let (committed, ack) = tokio::sync::oneshot::channel();
            sender
                .send(worker::Request { frame, committed })
                .await
                .context("state writer stopped")?;
            ack.await.context("state writer stopped before commit")??;
            processed += 1;
            delay = 1;
            println!(
                "{}",
                serde_json::json!({"event":"checkpoint_committed", "checkpoint":wanted,
                "changes":changes,"normalize_seconds":normalize_seconds,
                "commit_seconds":commit_started.elapsed().as_secs_f64(),
                "chain_timestamp_lag_seconds":(time::OffsetDateTime::now_utc()-checkpoint_time).as_seconds_f64(),
                "valuation_basis":"unverified_cached_diagnostics"})
            );
            if wanted.is_multiple_of(args.reconcile_every) {
                let directory = runtime::schedule::unique_run_directory(
                    &args.reconciliation_dir,
                    time::OffsetDateTime::now_utc(),
                );
                census(args, &directory, wanted).await?;
                engine.reconcile(&directory).await?;
                storage::write_pointer(
                    &args.reconciliation_dir.join("last-reconciled"),
                    &wanted.to_string(),
                )?;
            }
            if args.max_checkpoints.is_some_and(|limit| processed >= limit) {
                return Ok(());
            }
        }
        // SDK retries transient stream failures too. If a stream ends, reopen
        // from the database watermark; never jump to the new subscription tip.
        eprintln!("gRPC interrupted; reconnecting from committed checkpoint after {delay}s");
        tokio::select! {_=super::monitor::shutdown()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
        delay = (delay * 2).min(30);
    }
}
fn retryable(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable
            | tonic::Code::ResourceExhausted
            | tonic::Code::DeadlineExceeded
            | tonic::Code::Aborted
            | tonic::Code::Internal
    )
}
async fn census(args: &LiveArgs, out: &std::path::Path, checkpoint: u64) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .arg("snapshot")
        .arg("--config")
        .arg(&args.config)
        .arg("--out")
        .arg(out)
        .arg("--checkpoint")
        .arg(checkpoint.to_string())
        .arg("--concurrency")
        .arg("2");
    runtime::process::run_until(
        command,
        tokio::time::Instant::now() + Duration::from_secs(args.census_deadline_seconds),
    )
    .await
}
