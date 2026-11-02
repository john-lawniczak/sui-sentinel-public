//! Child progress is evidence only. Only the supervisor can publish a generation.
use crate::{ingest::graphql::Transport, storage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Default, Serialize, Deserialize)]
pub struct Progress {
    pub phase: String,
    pub updated_at: String,
    pub phases: BTreeMap<String, String>,
    pub transport: Transport,
    pub failure_kind: Option<String>,
}
pub fn write(phase: &str, transport: Option<Transport>, failure: Option<&str>) -> Result<()> {
    let Some(path) = std::env::var_os("SENTINEL_PROGRESS_PATH") else {
        return Ok(());
    };
    save(Path::new(&path), phase, transport, failure)
}
pub fn save(
    path: &Path,
    phase: &str,
    transport: Option<Transport>,
    failure: Option<&str>,
) -> Result<()> {
    let mut state: Progress = if path.exists() {
        storage::read_json(path)?
    } else {
        Progress::default()
    };
    state.updated_at =
        time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    state
        .phases
        .entry(phase.into())
        .or_insert_with(|| state.updated_at.clone());
    state.phase = phase.into();
    if let Some(value) = transport {
        state.transport = value;
    }
    state.failure_kind = failure.map(str::to_owned);
    storage::write_pointer(path, &serde_json::to_string(&state)?)
}
pub fn path(base: &Path, directory: &Path) -> std::path::PathBuf {
    base.join("runs").join(format!(
        "{}.progress",
        directory.file_name().unwrap().to_string_lossy()
    ))
}
