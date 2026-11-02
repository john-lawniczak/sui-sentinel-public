use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn checks_config_without_claiming_mainnet_verification() {
    for args in [
        vec!["check-config"],
        vec!["check-config", "--config", "deploy/config/suilend.toml"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("locally valid"));
        assert!(stdout.contains("Mainnet behavior is unverified"));
    }
}

#[test]
fn missing_config_fails_with_path_context() {
    // An existing regular file cannot also be the parent directory of a config.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml/missing.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
        .args(["check-config", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("failed to read configuration"));
    assert!(stderr.contains(path.to_str().unwrap()));
}

#[test]
fn snapshot_rejects_invalid_limits_before_network_access() {
    for args in [
        vec!["--max-pages", "0"],
        vec!["--concurrency", "0"],
        vec!["--page-size", "51"],
        vec!["--all-markets", "--market", "0x1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
            .args(["snapshot", "--out", "/unused"])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
    }
}

#[test]
fn capture_once_rejects_invalid_limits_before_network_access() {
    for args in [
        vec!["--max-pages", "0"],
        vec!["--concurrency", "0"],
        vec!["--page-size", "51"],
        vec!["--all-markets", "--market", "0x1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
            .args(["capture-once", "--out", "/unused"])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
    }
}

#[test]
fn capture_once_skips_without_touching_the_network_while_another_run_holds_the_lock() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let out = std::env::temp_dir().join(format!(
        "sui-sentinel-capture-once-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&out).unwrap();
    let lock_path = out.join(".capture.lock");
    // Hold the lock exactly like a concurrently-running attempt would,
    // using the same lock file the binary itself acquires.
    let held = sui_sentinel::runtime::lock::HostLock::try_acquire(&lock_path).unwrap();
    assert!(held.is_some());

    let output = Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
        .args(["capture-once", "--out"])
        .arg(&out)
        .args(["--deadline-seconds", "1"])
        .output()
        .unwrap();
    // A real capture attempt would fail on network access in this sandbox;
    // success here proves the lock check short-circuited before any such
    // attempt was made.
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"skipped\""), "{stdout}");

    drop(held);
    fs::remove_dir_all(&out).unwrap();
}
