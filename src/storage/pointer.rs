//! Atomic replacement with an exclusively owned temporary file per writer.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Replace a mutable pointer or status document without exposing partial writes.
pub fn write_pointer(path: &Path, contents: &str) -> Result<()> {
    write_pointer_before(path, contents, None)
}

/// The final rename is admitted only before the deadline. Filesystem completion
/// is not a hard real-time guarantee. Concurrent writers each publish whole files.
pub fn write_pointer_before(path: &Path, contents: &str, deadline: Option<Instant>) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().context("pointer has no parent")?;
    let name = path.file_name().context("pointer has no file name")?;
    let (temporary, mut file) = loop {
        let mut name = name.to_os_string();
        name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (Temporary(temporary), file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("creating pointer temporary file"),
        }
    };
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);
    ensure!(
        deadline.is_none_or(|d| Instant::now() < d),
        "publication deadline exceeded"
    );
    fs::rename(&temporary.0, path)
        .with_context(|| format!("publishing pointer {}", path.display()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_same_and_sibling_paths_publish_only_complete_documents() {
        let base = std::env::temp_dir().join(format!("sentinel-pointer-{}", std::process::id()));
        fs::create_dir(&base).unwrap();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for index in 0..8 {
                let base = &base;
                let barrier = &barrier;
                scope.spawn(move || {
                    let path = base
                        .join(["run.json", "run.progress", "run.applied", "current"][index % 4]);
                    let payload = format!("{index}").repeat(8192);
                    barrier.wait();
                    for _ in 0..50 {
                        write_pointer(&path, &payload).unwrap();
                        let observed = fs::read(&path).unwrap();
                        assert_eq!(observed.len(), payload.len());
                        assert!(observed.iter().all(|b| *b == observed[0]));
                        assert!(
                            [b'0' + (index % 4) as u8, b'4' + (index % 4) as u8]
                                .contains(&observed[0])
                        );
                    }
                });
            }
        });
        let current = base.join("current");
        let before = fs::read(&current).unwrap();
        assert!(write_pointer_before(&current, "expired", Some(Instant::now())).is_err());
        assert_eq!(fs::read(&current).unwrap(), before);
        assert_eq!(
            fs::read_dir(&base).unwrap().count(),
            4,
            "temporary files leaked"
        );
        fs::remove_dir_all(base).unwrap();
    }
}
