//! Killable process trees. Nested workers inherit the supervisor's process group.
use anyhow::{Result, ensure};
use tokio::{process::Command, time::Instant};

#[derive(Debug)]
pub struct DeadlineExceeded;
impl std::fmt::Display for DeadlineExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "worker deadline exceeded")
    }
}
impl std::error::Error for DeadlineExceeded {}
struct Group(Option<u32>);
impl Drop for Group {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // The shell builtin is available in slim images; the numeric PID is
            // an argument, never interpolated into shell code. Only groups created
            // by this function are eligible. Drop also handles async cancellation.
            let _ = std::process::Command::new("sh")
                .args(["-c", "kill -KILL \"-$1\" 2>/dev/null", "sentinel-stop"])
                .arg(pid.to_string())
                .status();
        }
    }
}
pub async fn run_until(mut command: Command, deadline: Instant) -> Result<()> {
    let owns_group = std::env::var_os("SENTINEL_WORKER_GROUP").is_none();
    #[cfg(unix)]
    if owns_group {
        command.process_group(0).env("SENTINEL_WORKER_GROUP", "1");
    }
    let mut child = command.kill_on_drop(true).spawn()?;
    let group = Group(if owns_group { child.id() } else { None });
    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            drop(group);
            child.kill().await?;
            child.wait().await?;
            return Err(DeadlineExceeded.into());
        }
    };
    // A worker can exit (including by signal) while descendants keep running.
    // Reap the entire owned group on every exit, not just deadline/cancellation.
    drop(group);
    ensure!(status.success(), "capture worker exited with {status}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_exit_cannot_leave_descendants_running() {
        for code in [0, 1] {
            let path =
                std::env::temp_dir().join(format!("sentinel-orphan-{}-{code}", std::process::id()));
            let mut command = Command::new("sh");
            command
                .args([
                    "-c",
                    "(sleep 0.3; echo late > \"$1\") & exit \"$2\"",
                    "worker",
                ])
                .arg(&path)
                .arg(code.to_string());
            let result =
                run_until(command, Instant::now() + std::time::Duration::from_secs(5)).await;
            assert_eq!(result.is_ok(), code == 0);
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            assert!(!path.exists(), "descendant survived parent exit {code}");
        }
    }
    #[tokio::test]
    async fn deadline_and_cancellation_kill_worker_descendants() {
        for cancel in [false, true] {
            let path =
                std::env::temp_dir().join(format!("sentinel-late-{}-{cancel}", std::process::id()));
            let mut command = Command::new("sh");
            command
                .args(["-c", "(sleep 0.3; echo late > \"$1\") & wait", "worker"])
                .arg(&path);
            let task = tokio::spawn(run_until(
                command,
                Instant::now() + std::time::Duration::from_millis(if cancel { 5000 } else { 40 }),
            ));
            if cancel {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                task.abort();
            }
            let outcome = task.await;
            assert!(outcome.is_err() || outcome.unwrap().is_err());
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            assert!(!path.exists(), "descendant survived shutdown");
        }
    }
}
