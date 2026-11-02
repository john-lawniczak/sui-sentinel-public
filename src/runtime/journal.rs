//! Capture crash recovery. Call recovery only while holding the capture lock.
use super::{RunRecord, State};
use crate::storage;
use anyhow::Result;
use std::path::Path;

pub fn save(base: &Path, run: &RunRecord) -> Result<()> {
    let name = Path::new(&run.output_directory)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("run name missing"))?
        .to_string_lossy();
    let dir = base.join("runs");
    std::fs::create_dir_all(&dir)?;
    storage::write_pointer(
        &dir.join(format!("{name}.json")),
        &serde_json::to_string_pretty(run)?,
    )
}
pub fn recover(base: &Path) -> Result<()> {
    let dir = base.join("runs");
    std::fs::create_dir_all(&dir)?;
    let current = std::fs::read_to_string(base.join("current")).ok();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let mut run: RunRecord = storage::read_json(&path)?;
        if matches!(run.state, State::Published | State::Failed | State::Skipped) {
            continue;
        }
        let name = Path::new(&run.output_directory)
            .file_name()
            .and_then(|n| n.to_str());
        if run.state == State::Validating && name == current.as_deref().map(str::trim) {
            run.finish_ok(State::Published);
        } else {
            run.finish_failed(
                "interrupted",
                &anyhow::anyhow!("previous process stopped before publication"),
            );
        }
        run.duration_seconds = None; // Cannot recover a monotonic timer after process death.
        save(base, &run)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_both_sides_of_the_pointer_lifecycle_crash_window() {
        let base = std::env::temp_dir().join(format!("sentinel-journal-{}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        for (id, state) in [
            ("collecting", State::Collecting),
            ("published", State::Validating),
        ] {
            let mut run = RunRecord::start(&base.join(id));
            run.transition(state);
            save(&base, &run).unwrap();
        }
        storage::write_pointer(&base.join("current"), "published").unwrap();
        recover(&base).unwrap();
        let a: RunRecord = storage::read_json(&base.join("runs/collecting.json")).unwrap();
        let b: RunRecord = storage::read_json(&base.join("runs/published.json")).unwrap();
        assert_eq!(a.state, State::Failed);
        assert_eq!(b.state, State::Published);
        std::fs::remove_dir_all(base).unwrap();
    }
}
