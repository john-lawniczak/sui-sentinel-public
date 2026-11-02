//! Immutable presentations with short swaps; freshness is evaluated on every request.
mod health;
pub(super) mod http;
mod operational;
mod pipeline;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use time::OffsetDateTime;
use tokio::net::TcpListener;

use super::Presentation;
use crate::analysis::report::Report;

struct Inner {
    operations: Option<crate::runtime::operations::Operations>,
    presentation: Option<Arc<Presentation>>,
    health: Option<health::Observation>,
    generation_id: Option<String>,
    loaded_at: Option<OffsetDateTime>,
    last_reload_attempted_at: Option<OffsetDateTime>,
    last_reload_error: Option<String>,
}

/// Shared across requests and the background poller. Readers never block on
/// a reload in progress: `try_reload` builds the new `Presentation` fully
/// before taking the write lock, so the swap itself is instant and every
/// request during a reload sees either the old or the new generation
/// whole - never a partially-applied one.
pub struct Registry {
    base: PathBuf,
    policy: crate::monitoring::Policy,
    inner: RwLock<Inner>,
}

#[derive(Serialize)]
pub struct Status {
    pub build: crate::build_identity::BuildIdentity,
    pub operations: Option<crate::runtime::operations::Operations>,
    pub policy_sha256: String,
    #[serde(flatten)]
    pub health: health::Health,
    pub ready: bool,
    current_generation: Option<String>,
    current_generation_loaded_at: Option<String>,
    last_reload_attempted_at: Option<String>,
    last_reload_error: Option<String>,
}

impl Registry {
    #[cfg(test)]
    fn new(base: PathBuf) -> Self {
        Self::with_policy(base, crate::monitoring::Policy::builtin().unwrap())
    }

    pub fn with_policy(base: PathBuf, policy: crate::monitoring::Policy) -> Self {
        Self {
            base,
            policy,
            inner: RwLock::new(Inner {
                operations: None,
                presentation: None,
                health: None,
                generation_id: None,
                loaded_at: None,
                last_reload_attempted_at: None,
                last_reload_error: None,
            }),
        }
    }

    /// Read `base/current`, and if it names a generation different from the
    /// one currently served, load/validate/pre-render it and swap. Returns
    /// `Ok(true)` only when a swap happened. A load failure (missing
    /// pointer, missing/corrupt report, checksum mismatch, schema
    /// violation) is recorded on `last_reload_error` and never removes the
    /// last good generation - this is the "keep the last good generation if
    /// reload fails" requirement.
    pub fn try_reload(&self) -> Result<bool> {
        let attempted_at = OffsetDateTime::now_utc();
        let path = self.base.join("operations.json");
        let operations = path
            .metadata()
            .ok()
            .filter(|m| m.len() <= 65536)
            .and_then(|_| crate::storage::read_json(&path).ok());
        // Validate and render outside the write lock: requests continue
        // serving the last good generation while the candidate is prepared.
        let result = self.load_candidate().and_then(|candidate| {
            candidate
                .map(|(id, report)| {
                    Presentation::render(&report).map(|presentation| {
                        (id, presentation, health::Observation::from_report(&report))
                    })
                })
                .transpose()
        });
        let mut inner = self.inner.write().expect("registry lock poisoned");
        inner.operations = operations;
        inner.last_reload_attempted_at = Some(attempted_at);
        match result {
            Ok(None) => {
                inner.last_reload_error = None;
                Ok(false)
            }
            Ok(Some((id, presentation, health))) => {
                inner.presentation = Some(Arc::new(presentation));
                inner.health = Some(health);
                inner.generation_id = Some(id);
                inner.loaded_at = Some(attempted_at);
                inner.last_reload_error = None;
                Ok(true)
            }
            Err(error) => {
                inner.last_reload_error = Some(error.to_string());
                Ok(false)
            }
        }
    }

    /// Returns the candidate generation id and its loaded report if the
    /// pointer names a generation not already current. `Ok(None)` is the
    /// steady-state case (no initial pointer, or already serving this
    /// generation). Losing a pointer after loading a generation is an error.
    fn load_candidate(&self) -> Result<Option<(String, Report)>> {
        let pointer = self.base.join("current");
        let id = match std::fs::read_to_string(&pointer) {
            Ok(id) => id,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && self
                        .inner
                        .read()
                        .expect("registry lock poisoned")
                        .generation_id
                        .is_none() =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("reading current generation pointer"),
        };
        let id = id.trim().to_owned();
        ensure!(
            !id.is_empty()
                && id != "."
                && id != ".."
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')),
            "generation pointer must name a single directory under its base"
        );
        if self
            .inner
            .read()
            .expect("registry lock poisoned")
            .generation_id
            .as_deref()
            == Some(id.as_str())
        {
            return Ok(None);
        }
        let base = self
            .base
            .canonicalize()
            .context("resolving generation base")?;
        let report_path = base
            .join(&id)
            .join("analysis")
            .join("report.json")
            .canonicalize()
            .context("resolving candidate report")?;
        ensure!(
            report_path.starts_with(&base),
            "generation report escapes its base"
        );
        let report = super::load_report(&report_path)
            .with_context(|| format!("loading candidate generation {id}"))?;
        Ok(Some((id, report)))
    }

    /// Self-health metrics the exporter can always report, independent of
    /// whether any report content has ever loaded - these answer "is
    /// Sentinel functioning?" (temp.md's Monitor Operations dashboard row),
    /// not "what does the latest report say?".
    fn operational_metrics(&self) -> String {
        let status = self.status();
        let inner = self.inner.read().expect("registry lock poisoned");
        let mut text = operational::render(
            status.ready,
            inner.loaded_at,
            inner.last_reload_attempted_at,
            inner
                .last_reload_attempted_at
                .map(|_| inner.last_reload_error.is_some()),
        );
        text.push_str(&pipeline::render(&status, &self.policy));
        text
    }
    fn snapshot(&self) -> (Option<Arc<Presentation>>, Status) {
        let inner = self.inner.read().expect("registry lock poisoned");
        let now = OffsetDateTime::now_utc();
        let mut health = health::evaluate(
            inner.health.as_ref(),
            &self.policy,
            now,
            inner.last_reload_error.is_some(),
        );
        if health.ready
            && self.policy.operations.require_monitor
            && !inner.operations.as_ref().is_some_and(|o| {
                o.healthy(
                    now.unix_timestamp(),
                    self.policy.operations.heartbeat_max_age_seconds,
                )
            })
        {
            health.ready = false;
            health.readiness_reason = "monitor_unhealthy";
        }
        (
            inner.presentation.clone(),
            Status {
                build: crate::build_identity::current(),
                ready: health.ready,
                health,
                operations: inner.operations.clone(),
                policy_sha256: crate::storage::sha256_hex_of(
                    &serde_json::to_vec(&self.policy).expect("serialize policy"),
                ),
                current_generation: inner.generation_id.clone(),
                current_generation_loaded_at: inner.loaded_at.map(http::format_time),
                last_reload_attempted_at: inner.last_reload_attempted_at.map(http::format_time),
                last_reload_error: inner.last_reload_error.clone(),
            },
        )
    }
}

/// Serve `base/<generation>/analysis/{report.json,positions.ndjson}` where
/// `<generation>` is named by `base/current`, polling that pointer every
/// `poll_interval` for a new generation to validate and swap in. An initial
/// reload attempt runs before the listener binds; if it fails (no pointer
/// yet, e.g. on a fresh deployment before the first `capture-once` has
/// published), the server still starts and `/readyz` reports not-ready
/// until a generation loads.
pub async fn serve_dir(base: &Path, listen: SocketAddr, poll_interval: Duration) -> Result<()> {
    serve_dir_with_policy(
        base,
        listen,
        poll_interval,
        crate::monitoring::Policy::builtin()?,
    )
    .await
}

pub async fn serve_dir_with_policy(
    base: &Path,
    listen: SocketAddr,
    poll_interval: Duration,
    policy: crate::monitoring::Policy,
) -> Result<()> {
    ensure!(!poll_interval.is_zero(), "poll interval must be positive");
    policy.validate()?;
    let registry = Arc::new(Registry::with_policy(base.to_owned(), policy));
    if let Err(error) = reload(registry.clone()).await {
        eprintln!("Warning: initial generation load failed: {error:#}");
    }
    let app = http::router(registry.clone());
    let listener = TcpListener::bind(listen).await?;
    eprintln!(
        "Serving generations from {} at http://{}; polling {} for a new `current` pointer every {:?}",
        base.display(),
        listener.local_addr()?,
        base.join("current").display(),
        poll_interval
    );
    let poller = {
        let registry = registry.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(poll_interval);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticks.tick().await;
                if let Err(error) = reload(registry.clone()).await {
                    eprintln!("Warning: generation reload attempt failed: {error:#}");
                }
            }
        })
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(super::shutdown())
        .await?;
    poller.abort();
    Ok(())
}

async fn reload(registry: Arc<Registry>) -> Result<bool> {
    tokio::task::spawn_blocking(move || registry.try_reload()).await?
}

impl Registry {
    pub fn status(&self) -> Status {
        self.snapshot().1
    }
}

#[cfg(test)]
mod tests;
