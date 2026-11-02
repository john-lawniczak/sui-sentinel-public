//! Independent bounded workers supervised by a heartbeat loop.
use anyhow::{Result, ensure};
use clap::Args;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use sui_sentinel::{
    diagnosis::{Store, delivery::Webhook},
    export::Registry,
    monitoring::Policy,
    runtime,
};
pub(super) type Shared = Arc<Mutex<runtime::operations::Operations>>;
#[derive(Args)]
pub struct MonitorArgs {
    #[arg(long, default_value = "deploy/config/suilend.toml")]
    pub config: PathBuf,
    #[arg(long, default_value = "deploy/config/monitoring.mainnet.toml")]
    pub policy: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
    #[arg(long)]
    pub scope: String,
    #[arg(long, env = "DIAGNOSIS_DATABASE_URL", hide_env_values = true)]
    pub database_url: String,
    #[arg(long, env = "SENTINEL_WEBHOOK_URL", hide_env_values = true)]
    pub webhook_url: String,
    #[arg(long, env = "SENTINEL_WEBHOOK_TOKEN", hide_env_values = true)]
    pub webhook_token: Option<String>,
    /// Serve diagnostics from this monitor; production requires SENTINEL_READ_TOKEN.
    #[arg(long)]
    pub listen: Option<std::net::SocketAddr>,
}

pub async fn monitor(args: MonitorArgs) -> Result<()> {
    let policy = Policy::load(&args.policy)?;
    sui_sentinel::config::Config::load(&args.config)?;
    if std::env::var("SENTINEL_DEPLOYMENT_MODE").as_deref() == Ok("production") {
        ensure!(
            policy.operations.require_monitor && !policy.readiness.expected_market_ids.is_empty(),
            "production requires monitor health and exact market scope"
        );
        ensure!(
            std::env::var("SENTINEL_AUTO_MIGRATE").as_deref() == Ok("false"),
            "production migrations must use a separate role"
        );
        let provider = reqwest::Url::parse(&std::env::var("SUI_GRAPHQL_URL")?)?;
        ensure!(
            provider.scheme() == "https" && provider.host_str() != Some("graphql.mainnet.sui.io"),
            "configure a production provider; public endpoints are development-only"
        );
        ensure!(
            reqwest::Url::parse(&args.webhook_url)?.scheme() == "https",
            "production webhook requires HTTPS"
        );
        if args.listen.is_some() {
            ensure!(
                std::env::var("SENTINEL_READ_TOKEN").is_ok_and(|t| t.len() >= 32),
                "production HTTP requires a read token of at least 32 bytes"
            );
        }
        let revision = sui_sentinel::build_identity::current().revision;
        ensure!(
            revision.len() == 40 && revision.bytes().all(|c| c.is_ascii_hexdigit()),
            "production requires a source-pinned build"
        );
    }
    ensure!(
        policy.readiness.expected_scope == "explicit_markets" && !args.scope.is_empty(),
        "invalid monitor scope"
    );
    std::fs::create_dir_all(args.out.join("runs"))?;
    let _lock = runtime::lock::HostLock::try_acquire(&args.out.join(".monitor.lock"))?
        .ok_or_else(|| anyhow::anyhow!("monitor already running"))?;
    let store = Store::connect(&args.database_url).await?;
    store.prepare().await?;
    let delivery_store = match std::env::var("SENTINEL_DELIVERY_DATABASE_URL") {
        Ok(url) => Store::connect(&url).await?,
        Err(_) => store.clone(),
    };
    let webhook = Webhook::with_format(
        &args.webhook_url,
        args.webhook_token.clone(),
        &std::env::var("SENTINEL_WEBHOOK_FORMAT").unwrap_or_else(|_| "generic".into()),
    )?;
    let registry = Arc::new(Registry::with_policy(args.out.clone(), policy.clone()));
    let mut previous = runtime::operations::Operations::default();
    // Carry cumulative counters across restart, never carry healthy heartbeats.
    if let Ok(old) = sui_sentinel::storage::read_json::<runtime::operations::Operations>(
        &args.out.join("operations.json"),
    ) {
        previous.failure_streak = old.failure_streak;
        previous.missed_ticks = old.missed_ticks;
    }
    let shared = Arc::new(Mutex::new(previous));
    let args = Arc::new(args);
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(super::monitor_workers::collect(
        args.clone(),
        policy.clone(),
        shared.clone(),
    ));
    if let Some(listen) = args.listen {
        let registry = registry.clone();
        let token = std::env::var("SENTINEL_READ_TOKEN").ok();
        workers.spawn(async move {
            sui_sentinel::export::serve_registry(registry, listen, token).await
        });
    }
    workers.spawn(super::monitor_workers::observe(
        store.clone(),
        registry,
        args.out.clone(),
        args.scope.clone(),
        policy,
        shared.clone(),
    ));
    workers.spawn(super::monitor_workers::deliver(
        delivery_store,
        webhook,
        args.scope.clone(),
        shared.clone(),
    ));
    let mut ticks = tokio::time::interval(Duration::from_secs(5));
    let stop = shutdown();
    tokio::pin!(stop);
    let result = loop {
        tokio::select! {
            _=&mut stop=>break Ok(()),
            result=workers.join_next()=>break Err(anyhow::anyhow!("monitor worker exited: {result:?}")),
            _=ticks.tick()=>{
                let status={let mut ops=shared.lock().unwrap();ops.observed_at=time::OffsetDateTime::now_utc().unix_timestamp();ops.scheduler_heartbeat=Some(ops.observed_at);ops.clone()};
                let base=args.out.clone();
                if !matches!(tokio::time::timeout(Duration::from_secs(3),tokio::task::spawn_blocking(move ||status.save(&base))).await,Ok(Ok(Ok(())))) {
                    eprintln!("Operational heartbeat could not be persisted");
                }
            }
        }
    };
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    result
}

pub(super) async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
