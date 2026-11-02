use std::{path::Path, time::Duration};
use sui_sentinel::{diagnosis::Store, runtime::RunRecord, storage};

#[tokio::test]
async fn scheduler_keeps_running_after_failures_replays_outcomes_and_delivers() {
    let Ok(database) = std::env::var("SENTINEL_TEST_DATABASE_URL") else {
        return;
    };
    let scope = format!(
        "monitor-e2e-{}-{}",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let base = std::env::temp_dir().join(&scope);
    std::fs::create_dir(&base).unwrap();
    let policy = base.join("policy.toml");
    std::fs::write(
        &policy,
        "schema_version=1\n[capture]\ninterval_seconds=1\ndeadline_seconds=1\n",
    )
    .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/",
        axum::routing::post(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                "ok"
            }
        }),
    );
    let webhook = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // A closed local port causes fast capture failure; no mainnet access occurs.
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = unused.local_addr().unwrap();
    drop(unused);
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_address = http_listener.local_addr().unwrap();
    drop(http_listener);
    let read_token = "integration-test-token-32-characters";
    let mut monitor = tokio::process::Command::new(env!("CARGO_BIN_EXE_sui-sentinel"))
        .args(["monitor", "--listen", &http_address.to_string(), "--out"])
        .arg(&base)
        .arg("--policy")
        .arg(&policy)
        .arg("--scope")
        .arg(&scope)
        .env("DIAGNOSIS_DATABASE_URL", &database)
        .env("SENTINEL_READ_TOKEN", read_token)
        .env("SENTINEL_WEBHOOK_URL", format!("http://{address}/"))
        .env("SUI_GRAPHQL_URL", format!("http://{unavailable}/graphql"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let store = Store::connect(&database).await.unwrap();
    store.migrate().await.unwrap();
    let mut succeeded = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if calls.load(std::sync::atomic::Ordering::Relaxed) >= 2 && failed_runs(&base) >= 2 {
            succeeded = true;
            break;
        }
        assert!(
            monitor.try_wait().unwrap().is_none(),
            "monitor exited prematurely"
        );
    }
    let client = reqwest::Client::new();
    let url = format!("http://{http_address}/status");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let status: serde_json::Value = client
        .get(&url)
        .bearer_auth(read_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["ready"], false);
    assert_eq!(
        client
            .get(format!("http://{http_address}/livez"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    #[cfg(unix)]
    {
        // Minimal Linux images provide kill as a shell builtin, without a
        // standalone executable. Pass the PID as an argument, not shell code.
        std::process::Command::new("sh")
            .args(["-c", "kill -TERM \"$1\"", "sentinel-monitor-test"])
            .arg(monitor.id().unwrap().to_string())
            .status()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), monitor.wait())
            .await
            .unwrap()
            .unwrap();
    }
    #[cfg(not(unix))]
    {
        monitor.kill().await.unwrap();
    }
    assert!(
        succeeded,
        "scheduler did not persist and deliver multiple failures"
    );
    assert!(!base.join("current").exists());
    assert!(
        store
            .list_incidents(500)
            .await
            .unwrap()
            .iter()
            .any(|i| i.scope == scope && i.rule_id == "capture_failure")
    );
    webhook.abort();
    tokio::time::sleep(Duration::from_millis(200)).await;
    std::fs::remove_dir_all(base).unwrap();
}
fn failed_runs(base: &Path) -> usize {
    let Ok(files) = std::fs::read_dir(base.join("runs")) else {
        return 0;
    };
    files
        .filter_map(Result::ok)
        .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
        .filter(|f| {
            storage::read_json::<RunRecord>(&f.path())
                .is_ok_and(|r| r.state == sui_sentinel::runtime::State::Failed)
        })
        .count()
}
