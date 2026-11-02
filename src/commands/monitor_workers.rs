use super::monitor::{MonitorArgs, Shared};
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use sui_sentinel::{
    diagnosis::{self, Store, delivery::Webhook},
    export::Registry,
    monitoring::Policy,
    runtime, storage,
};
fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

pub async fn collect(args: Arc<MonitorArgs>, policy: Policy, shared: Shared) -> Result<()> {
    let mut ticks = tokio::time::interval(Duration::from_secs(policy.capture.interval_seconds));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let scheduled = ticks.tick().await;
        let late = scheduled.elapsed().as_secs() / policy.capture.interval_seconds;
        shared.lock().unwrap().missed_ticks += late;
        let disk = runtime::disk::preflight(&args.out, policy.operations.min_free_bytes).await;
        shared.lock().unwrap().disk_available_bytes =
            runtime::disk::available(&args.out).await.ok();
        if let Err(error) = disk {
            let mut run = runtime::RunRecord::start(&runtime::schedule::unique_run_directory(
                &args.out,
                time::OffsetDateTime::now_utc(),
            ));
            run.finish_failed("disk_pressure", &error);
            runtime::journal::save(&args.out, &run)?;
            shared.lock().unwrap().record(&run);
            continue;
        }
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command
            .arg("capture-once")
            .arg("--config")
            .arg(&args.config)
            .arg("--out")
            .arg(&args.out)
            .arg("--scope")
            .arg(&args.scope)
            .arg("--deadline-seconds")
            .arg(policy.capture.deadline_seconds.to_string())
            .env_remove("DIAGNOSIS_DATABASE_URL");
        let config = sui_sentinel::config::Config::load(&args.config)?;
        let markets = if policy.readiness.expected_market_ids.is_empty() {
            vec![config.suilend.lending_market_id]
        } else {
            policy.readiness.expected_market_ids.clone()
        };
        for market in markets {
            command.arg("--market").arg(market);
        }
        shared.lock().unwrap().capture_active = true;
        let result = runtime::process::run_until(
            command,
            tokio::time::Instant::now() + Duration::from_secs(policy.capture.deadline_seconds + 10),
        )
        .await;
        shared.lock().unwrap().capture_active = false;
        if result.is_err() {
            eprintln!("Scheduled capture failed; inspect run journal");
        }
        let paths = runtime::retention::records(&args.out, policy.operations.max_run_records)?;
        if let Some(path) = paths.last() {
            shared.lock().unwrap().record(&storage::read_json(path)?);
        }
    }
}
pub async fn observe(
    store: Store,
    registry: Arc<Registry>,
    base: PathBuf,
    scope: String,
    policy: Policy,
    shared: Shared,
) -> Result<()> {
    let mut ticks = tokio::time::interval(Duration::from_secs(5));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut loops = 0_u64;
    loop {
        ticks.tick().await;
        let r = registry.clone();
        tokio::task::spawn_blocking(move || r.try_reload()).await??;
        shared.lock().unwrap().database_ok = Some(false);
        let work = async {
            let backlog = replay(&store, &base, &scope, policy.operations.max_run_records).await?;
            let db = store.operations(&scope).await?;
            let progress = runtime::retention::records(&base, policy.operations.max_run_records)?
                .last()
                .and_then(|p| {
                    storage::read_json::<runtime::progress::Progress>(&p.with_extension("progress"))
                        .ok()
                });
            let snapshot = {
                let mut ops = shared.lock().unwrap();
                ops.database_ok = Some(true);
                ops.spool_backlog = Some(backlog);
                ops.notification_pending = db.notification_pending;
                ops.notification_oldest_age_seconds = db.notification_oldest_age_seconds;
                ops.notification_attempts = db.notification_attempts;
                ops.notification_dead_letters = db.notification_dead_letters;
                if let Some(progress) = progress {
                    ops.capture_phase = Some(progress.phase);
                    ops.transport = Some(progress.transport);
                }
                ops.clone()
            };
            diagnosis::health_rules::observe(
                &store,
                &scope,
                &serde_json::to_value(registry.status())?,
                &snapshot,
                &policy,
            )
            .await?;
            if loops.is_multiple_of(60) {
                let pins = store.evidence_pins(&scope).await?;
                let directory = base.clone();
                let retention = policy.operations.clone();
                tokio::task::spawn_blocking(move || {
                    runtime::retention::clean(&directory, &retention, &pins)
                })
                .await??;
            }
            Ok::<_, anyhow::Error>(())
        };
        let ok = matches!(
            tokio::time::timeout(Duration::from_secs(20), work).await,
            Ok(Ok(()))
        );
        let free = runtime::disk::available(&base).await.ok();
        let mut ops = shared.lock().unwrap();
        ops.observer_heartbeat = Some(now());
        ops.observer_ok = Some(ok);
        ops.disk_available_bytes = free;
        if !ok {
            eprintln!("Observation/database work failed; durable inputs remain retryable");
        }
        loops += 1;
    }
}
pub async fn deliver(store: Store, webhook: Webhook, scope: String, shared: Shared) -> Result<()> {
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        let ok = matches!(
            tokio::time::timeout(Duration::from_secs(20), webhook.deliver_one(&store, &scope))
                .await,
            Ok(Ok(_))
        );
        let mut ops = shared.lock().unwrap();
        if ok {
            ops.delivery_heartbeat = Some(now());
        } else {
            eprintln!("Delivery/database unavailable; leased work remains retryable");
        }
    }
}
async fn replay(store: &Store, base: &Path, scope: &str, maximum: usize) -> Result<u64> {
    let directory = base.to_owned();
    let (backlog, records) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut pending = Vec::new();
        for path in runtime::retention::records(&directory, maximum)? {
            if !path.with_extension("applied").exists() {
                pending.push(path);
            }
        }
        let count = pending.len() as u64;
        let records = pending
            .into_iter()
            .take(100)
            .map(|p| Ok((p.clone(), storage::read_json::<runtime::RunRecord>(&p)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok((count, records))
    })
    .await??;
    for (path, run) in records {
        if !matches!(
            run.state,
            runtime::State::Failed | runtime::State::Published | runtime::State::Skipped
        ) {
            continue;
        }
        diagnosis::rules::apply_capture_outcome(store, scope, &run).await?;
        storage::write_pointer(&path.with_extension("applied"), "applied")?;
    }
    Ok(backlog)
}
