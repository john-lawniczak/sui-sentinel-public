//! Read-only presentation over a completed report; no collection or calculation I/O.

mod access;
mod assets;
mod metrics;
mod registry;
mod stress;
mod validate;

use std::{fs::File, io::Read, net::SocketAddr, path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use axum::{Router, body::Bytes, extract::State, http::header, routing::get};
use tokio::net::TcpListener;

use crate::{analysis::report::Report, storage};

pub use access::serve_registry;
pub use metrics::render;
pub use registry::{Registry, serve_dir, serve_dir_with_policy};

#[derive(Clone)]
struct Presentation {
    report: Bytes,
    metrics: Bytes,
}

impl Presentation {
    fn render(report: &Report) -> Result<Self> {
        Ok(Self {
            report: serde_json::to_vec(report)?.into(),
            metrics: render(report)?.into(),
        })
    }
}

/// Load, checksum-verify, and parse the `report.json` at `path`. Shared by
/// the single-shot `serve` (below) and the polling reload path in
/// `registry`, so both apply the same integrity and size checks to any
/// report before it is ever presented.
fn load_report(path: &Path) -> Result<Report> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let directory = path.parent().context("report has no parent")?;
    ensure!(
        directory.is_dir(),
        "report directory {} does not exist",
        directory.display()
    );
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("opening report {}", path.display()))?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "report exceeds 8 MiB limit"
    );
    let report: Report = serde_json::from_slice(&bytes).context("invalid report artifact")?;
    if directory.join("checksums.json").exists()
        || report
            .provenance
            .as_ref()
            .is_some_and(|p| p.artifact_revision >= 2)
    {
        storage::verify_checksums(directory, &["positions.ndjson", "report.json"])?;
    } else {
        eprintln!(
            "Warning: legacy report has no integrity binding: {}",
            path.display()
        );
    }
    Ok(report)
}

/// Validate and render once. Every request serves the same immutable report.
pub fn router(report: Report) -> Result<Router> {
    let presentation = Arc::new(Presentation::render(&report)?);
    Ok(Router::new()
        .route(
            "/metrics",
            get(|State(state): State<Arc<Presentation>>| async move {
                (
                    [
                        (
                            header::CONTENT_TYPE,
                            "text/plain; version=0.0.4; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    state.metrics.clone(),
                )
            }),
        )
        .route(
            "/report",
            get(|State(state): State<Arc<Presentation>>| async move {
                (
                    [
                        (header::CONTENT_TYPE, "application/json"),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    state.report.clone(),
                )
            }),
        )
        .route("/healthz", get(|| async { "snapshot report loaded\n" }))
        .with_state(presentation))
}

pub async fn serve(path: &Path, listen: SocketAddr) -> Result<()> {
    let report = load_report(path)?;
    let app = router(report)?;
    let listener = TcpListener::bind(listen).await?;
    eprintln!(
        "Serving immutable snapshot at http://{}; restart to load another report",
        listener.local_addr()?
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch() -> std::path::PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sui-sentinel-export-mod-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    /// Regression: a `current` pointer naming a generation directory that
    /// was never created at all must not be reported as "predates
    /// artifact checksumming" - that message is reassuring and specific
    /// to a real, existing, just-older report, which this is not.
    #[test]
    fn missing_report_directory_is_distinguished_from_a_pre_checksum_report() {
        let base = scratch();
        let missing_generation_report = base.join("gen-missing").join("report.json");
        let message = match load_report(&missing_generation_report) {
            Ok(_) => panic!("expected a missing-directory error"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            message.contains("does not exist"),
            "expected a missing-directory message, got: {message}"
        );
        assert!(
            !message.contains("predates artifact checksumming"),
            "must not claim this is an old pre-checksum report: {message}"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    /// The actual "predates checksumming" case: the directory exists and
    /// holds a real report, just no checksums.json alongside it.
    #[test]
    fn new_reports_require_checksums_while_legacy_reports_remain_readable() {
        let base = scratch();
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");
        crate::analysis::report::generate(&input, &base.join("analysis")).unwrap();
        fs::remove_file(base.join("analysis").join("checksums.json")).unwrap();
        let path = base.join("analysis/report.json");
        assert!(load_report(&path).is_err());
        let mut report: Report = storage::read_json(&path).unwrap();
        report.provenance = None;
        fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        load_report(&path).unwrap();
        fs::remove_dir_all(&base).unwrap();
    }
}
