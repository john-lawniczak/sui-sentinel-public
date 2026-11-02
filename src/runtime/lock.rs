//! Single-host advisory lock preventing overlapping supervised runs.
//!
//! This is explicitly *not* distributed lease fencing (temp.md section 4:
//! "use a renewable lease with fencing tokens if multiple workers can
//! schedule the same job ... add distributed lease fencing only if multiple
//! schedulers are deployed"). One `flock`-backed file is sufficient for one
//! host running one scheduler; a second host pointed at the same output base
//! directory is out of scope until a distributed scheduler is actually
//! deployed.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::Write,
    path::Path,
};

use anyhow::{Context, Result};

/// Held for the lifetime of one supervised run. The OS releases the
/// underlying `flock` when this (or the whole process) is dropped, so a
/// crashed holder can never block a replacement - there is no lease to
/// expire because the lock is tied to the file descriptor's lifetime, not a
/// renewable timer.
pub struct HostLock {
    file: File,
}

impl HostLock {
    /// Attempt to acquire the lock at `path` without blocking. `Ok(None)`
    /// means another run currently holds it - the caller should skip this
    /// tick, not treat it as an error.
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("opening lock file {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("locking {}", path.display()));
            }
        }
        file.set_len(0)?;
        write!(
            file,
            "pid={}\nheld_since={}\n",
            std::process::id(),
            time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .context("formatting lock timestamp")?
        )?;
        file.sync_all()?;
        Ok(Some(Self { file }))
    }
}

impl Drop for HostLock {
    fn drop(&mut self) {
        // Closing the file descriptor (which happens regardless of this
        // call) already releases the flock; this just makes the release
        // immediate rather than deferred to whenever the fd is actually
        // closed. Deliberately does NOT remove the lock file: unlinking it
        // here would race a concurrent try_acquire that is about to open
        // the same path. If that acquirer's `open(path)` runs after this
        // unlock but its matching `unlink` races in between, the acquirer
        // can end up locking a *freshly created* inode at the same path
        // while a third process later creates yet another new inode there
        // too - each holder locks a different inode, so none of them
        // actually exclude each other and the lock stops working. Leaving
        // the (now-unlocked) file in place keeps every acquirer opening
        // and locking the *same* inode, which is what makes flock-based
        // mutual exclusion correct. A stray lock file with nothing holding
        // it is harmless and permanent, like a `Cargo.lock`.
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_path() -> PathBuf {
        // A per-process atomic counter, not just a nanosecond timestamp -
        // see storage::tests::scratch for why a timestamp alone collides
        // under parallel test execution.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sui-sentinel-lock-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn second_acquirer_is_rejected_while_first_holds_the_lock() {
        let path = scratch_path();
        let first = HostLock::try_acquire(&path).unwrap();
        assert!(first.is_some());
        let second = HostLock::try_acquire(&path).unwrap();
        assert!(
            second.is_none(),
            "overlapping run must not acquire the lock"
        );
        drop(first);
        let third = HostLock::try_acquire(&path).unwrap();
        assert!(third.is_some(), "lock must be acquirable once released");
        drop(third);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dropping_the_holder_releases_the_lock_like_a_crashed_worker() {
        let path = scratch_path();
        {
            let held = HostLock::try_acquire(&path).unwrap();
            assert!(held.is_some());
            // Simulate a crashed worker: the lock is released when its file
            // descriptor goes away, with no lease timer to wait out.
        }
        let reacquired = HostLock::try_acquire(&path).unwrap();
        assert!(reacquired.is_some());
        drop(reacquired);
        let _ = std::fs::remove_file(&path);
    }

    /// Regression: dropping a held lock must not unlink the lock file.
    /// Unlinking it would let a concurrent acquirer's `open()` create a
    /// brand new inode at the same path, so a *third* acquirer opening
    /// the path again later would lock yet another new inode - two
    /// "holders" locking different inodes at the same path no longer
    /// exclude each other, silently breaking the single-host lock this
    /// type exists to provide. Asserting the path still exists after drop
    /// is what pins this down (the actual race additionally needs an
    /// open() to interleave between unlock() and a hypothetical unlink(),
    /// which isn't practical to assert directly - not deleting the file at
    /// all removes the race by construction, so "still exists" fully
    /// covers the fix).
    #[test]
    fn dropping_the_holder_does_not_unlink_the_lock_file() {
        let path = scratch_path();
        let held = HostLock::try_acquire(&path).unwrap();
        assert!(held.is_some());
        drop(held);
        assert!(
            path.exists(),
            "the lock file must survive its holder being dropped"
        );
        let _ = std::fs::remove_file(&path);
    }
}
