//! Conservative cleanup under the capture lock. Never follow generation symlinks.
use super::{RunRecord, State, lock::HostLock};
use crate::{monitoring::operations::Operations, storage};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub fn records(base: &Path, maximum: usize) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for item in std::fs::read_dir(base.join("runs"))? {
        let item = item?;
        if item.path().extension().is_some_and(|x| x == "json") {
            ensure!(
                paths.len() < maximum,
                "run journal exceeds bounded traversal budget; archive pinned evidence"
            );
            ensure!(
                item.file_type()?.is_file(),
                "run journal must be a regular file"
            );
            paths.push(item.path());
        }
    }
    paths.sort();
    Ok(paths)
}
pub fn clean(base: &Path, policy: &Operations, incident_pins: &BTreeSet<String>) -> Result<u64> {
    let Some(_lock) = HostLock::try_acquire(&base.join(".capture.lock"))? else {
        return Ok(0);
    };
    let mut pins = incident_pins.clone();
    for name in ["current", "last-good"] {
        match std::fs::read_to_string(base.join(name)) {
            Ok(id) => {
                pins.insert(id.trim().to_owned());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let paths = records(base, policy.max_run_records)?;
    let mut removable = Vec::new();
    for path in paths.into_iter().rev() {
        let run: RunRecord = storage::read_json(&path)?;
        let name = path.file_stem().unwrap().to_str().unwrap_or("");
        ensure!(
            name.starts_with("run-")
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid retention run name"
        );
        if !path.with_extension("applied").exists()
            || !matches!(run.state, State::Published | State::Failed | State::Skipped)
        {
            continue;
        }
        removable.push((path, run));
    }
    let now = time::OffsetDateTime::now_utc();
    let mut removed = 0;
    for (path, run) in removable.into_iter().skip(policy.retain_generations) {
        let name = path.file_stem().unwrap().to_str().unwrap();
        if pins.contains(name) {
            continue;
        }
        let Some(finished) = run.finished_at else {
            continue;
        };
        let at =
            time::OffsetDateTime::parse(&finished, &time::format_description::well_known::Rfc3339)?;
        if (now - at).whole_seconds() < policy.retain_seconds as i64 {
            continue;
        }
        let directory = base.join(name);
        match directory.symlink_metadata() {
            Ok(meta) => {
                ensure!(
                    meta.is_dir() && !meta.file_type().is_symlink(),
                    "unsafe generation cleanup target"
                );
                std::fs::remove_dir_all(directory)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        for extension in ["progress", "applied"] {
            let p = path.with_extension(extension);
            if p.exists() {
                std::fs::remove_file(p)?;
            }
        }
        std::fs::remove_file(path)?;
        removed += 1;
    }
    Ok(removed)
}
