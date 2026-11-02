use super::CaptureScope;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use sui_sentinel::{
    analysis::report,
    diagnosis, export,
    runtime::{self, RunRecord},
    storage,
};

pub struct CaptureOnceArgs {
    pub scope: CaptureScope,
    pub out: PathBuf,
    pub deadline_seconds: u64,
    /// Incident dedup scope (temp.md task O04). Defaults to `out`'s own
    /// directory name when not given.
    pub incident_scope: Option<String>,
    pub diagnosis_database_url: Option<String>,
}

pub async fn capture_once(args: CaptureOnceArgs) -> Result<()> {
    let CaptureOnceArgs {
        scope,
        out,
        deadline_seconds,
        incident_scope,
        diagnosis_database_url,
    } = args;
    let incident_scope = incident_scope.unwrap_or_else(|| {
        out.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("default")
            .to_owned()
    });
    sui_sentinel::config::Config::load(&scope.config)?;
    ensure!(deadline_seconds > 0, "deadline must be positive");
    let lock_path = out.join(".capture.lock");
    std::fs::create_dir_all(&out)?;
    let runs_dir = out.join("runs");
    std::fs::create_dir_all(&runs_dir)?;
    let publication_deadline = std::time::Instant::now() + Duration::from_secs(deadline_seconds);
    let progress_base = out.clone();
    let run = runtime::run_supervised_capture(
        &out,
        &lock_path,
        time::OffsetDateTime::now_utc(),
        Duration::from_secs(deadline_seconds),
        move |output_directory| async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(deadline_seconds);
            let mut child = tokio::process::Command::new(std::env::current_exe()?);
            child
                .arg("snapshot")
                .arg("--config")
                .arg(&scope.config)
                .arg("--out")
                .arg(&output_directory)
                .arg("--page-size")
                .arg(scope.page_size.to_string())
                .arg("--concurrency")
                .arg(scope.concurrency.to_string());
            if let Some(checkpoint) = scope.checkpoint {
                child.arg("--checkpoint").arg(checkpoint.to_string());
            }
            if scope.all_markets {
                child.arg("--all-markets");
            }
            for market in &scope.markets {
                child.arg("--market").arg(market);
            }
            if let Some(pages) = scope.max_pages {
                child.arg("--max-pages").arg(pages.to_string());
            }
            let progress = runtime::progress::path(&progress_base, &output_directory);
            child.env("SENTINEL_PROGRESS_PATH", &progress);
            // The child can write immutable artifacts but has no publication authority.
            child.stdout(std::process::Stdio::null());
            runtime::process::run_until(child, deadline).await?;
            runtime::progress::save(&progress, "validating", None, None)?;
            let mut validator = tokio::process::Command::new(std::env::current_exe()?);
            validator
                .arg("validate-capture")
                .arg("--input")
                .arg(&output_directory);
            runtime::process::run_until(validator, deadline).await?;
            let manifest: sui_sentinel::model::Manifest =
                storage::read_json(&output_directory.join("manifest.json"))?;
            Ok(runtime::CaptureOutcome {
                checkpoint: manifest.checkpoint.sequence_number,
                position_count: manifest.markets.iter().map(|m| m.obligations).sum(),
                market_count: manifest.markets.len() as u64,
            })
        },
        |directory, _| write_current(&out, directory, Some(publication_deadline)),
    )
    .await?;

    record_run(&runs_dir, &run)?;
    if let Some(database_url) = diagnosis_database_url
        && tokio::time::timeout(
            Duration::from_secs(10),
            record_incident(&database_url, &incident_scope, &run),
        )
        .await
        .is_err()
    {
        eprintln!("Diagnosis deadline exceeded; lifecycle record retained for replay");
    }
    println!("{}", serde_json::to_string_pretty(&run)?);
    if matches!(run.state, runtime::State::Failed) {
        std::process::exit(1);
    }
    Ok(())
}

fn record_run(runs_dir: &Path, run: &RunRecord) -> Result<()> {
    let record_path = runs_dir.join(format!("{}.json", run_name(run)));
    if !record_path.exists() {
        storage::json(&record_path, run)?;
    }
    Ok(())
}

pub fn validate_capture(directory: &Path) -> Result<()> {
    let manifest: sui_sentinel::model::Manifest =
        storage::read_json(&directory.join("manifest.json"))?;
    ensure!(
        manifest.schema_version == 2,
        "production publication requires bound raw schema 2"
    );
    storage::raw::verify(directory, &manifest)?;
    validate_report(directory)?;
    let candidate: report::Report = storage::read_json(&directory.join("analysis/report.json"))?;
    ensure!(
        candidate.checkpoint == manifest.checkpoint,
        "report checkpoint differs from raw inputs"
    );
    let provenance = candidate.provenance.context("missing report provenance")?;
    ensure!(
        provenance.raw_input_sha256.as_deref()
            == Some(storage::sha256_hex(&directory.join("checksums.json"))?.as_str()),
        "raw/report binding mismatch"
    );
    Ok(())
}

fn validate_report(directory: &Path) -> Result<()> {
    // Called by the supervisor while its capture lock is still held.
    // Validate the persisted candidate before changing the last-good pointer.
    let analysis = directory.join("analysis");
    storage::verify_checksums(&analysis, &["positions.ndjson", "report.json"])?;
    let candidate: report::Report = storage::read_json(&analysis.join("report.json"))?;
    ensure!(
        candidate.capture_complete,
        "incomplete capture cannot replace the complete generation"
    );
    export::render(&candidate)?;
    Ok(())
}

fn write_current(out: &Path, directory: &Path, deadline: Option<std::time::Instant>) -> Result<()> {
    let generation_id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("run output directory had no file name")?;
    if let Ok(previous) = std::fs::read_to_string(out.join("current")) {
        storage::write_pointer(&out.join("last-good"), previous.trim())?;
    }
    storage::write_pointer_before(&out.join("current"), generation_id, deadline)
}

fn run_name(run: &RunRecord) -> &str {
    Path::new(&run.output_directory)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("run")
}

/// Diagnosis writes follow publication. A database failure does not undo
/// the capture; durable retry of these lifecycle records remains O04 work.
async fn record_incident(database_url: &str, scope: &str, run: &RunRecord) {
    let store = match diagnosis::Store::connect(database_url).await {
        Ok(store) => store,
        Err(error) => {
            eprintln!("Warning: could not reach the diagnosis database: {error:#}");
            return;
        }
    };
    if let Err(error) = store.prepare().await {
        eprintln!("Warning: diagnosis database migration failed: {error:#}");
        return;
    }
    if let Err(error) = diagnosis::rules::apply_capture_outcome(&store, scope, run).await {
        eprintln!("Warning: recording capture_failure incident failed: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lifecycle records use the generation's directory name.
    #[test]
    fn run_name_is_the_output_directorys_final_component() {
        let mut run = RunRecord::start(Path::new("/tmp/base/run-123-0"));
        assert_eq!(run_name(&run), "run-123-0");
        run.output_directory = String::new();
        assert_eq!(run_name(&run), "run");
    }

    #[test]
    fn publication_rejects_incomplete_corrupt_and_invalid_reports() {
        let base = std::env::temp_dir().join(format!(
            "sentinel-publication-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");
        for case in ["complete", "incomplete", "corrupt", "invalid"] {
            let directory = base.join(case);
            std::fs::create_dir(&directory).unwrap();
            let analysis = directory.join("analysis");
            let mut candidate = report::generate(&input, &analysis).unwrap();
            if case == "incomplete" || case == "invalid" {
                if case == "incomplete" {
                    candidate.capture_complete = false;
                } else {
                    candidate.schema_version = 99;
                }
                std::fs::write(
                    analysis.join("report.json"),
                    serde_json::to_vec(&candidate).unwrap(),
                )
                .unwrap();
                std::fs::remove_file(analysis.join("checksums.json")).unwrap();
                storage::write_checksums(&analysis, &["positions.ndjson", "report.json"]).unwrap();
            }
            if case == "corrupt" {
                std::fs::write(analysis.join("positions.ndjson"), "corrupt").unwrap();
            }
            let result =
                validate_report(&directory).and_then(|_| write_current(&base, &directory, None));
            if case == "complete" {
                result.unwrap();
            } else {
                assert!(result.is_err(), "accepted {case} report");
            }
            assert_eq!(
                std::fs::read_to_string(base.join("current")).unwrap(),
                "complete"
            );
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
