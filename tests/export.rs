use std::{fs, path::Path, sync::OnceLock, time::Duration};

use rust_decimal::Decimal;
use sui_sentinel::{
    analysis::report::{self, DecimalString, Report},
    export, storage,
};

fn fixture() -> Report {
    static JSON: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = JSON.get_or_init(|| {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");
        let out = std::env::temp_dir().join(format!("sentinel-export-test-{}", std::process::id()));
        report::generate(&input, &out).unwrap();
        let bytes = fs::read(out.join("report.json")).unwrap();
        fs::remove_dir_all(out).unwrap();
        bytes
    });
    serde_json::from_slice(bytes).unwrap()
}

#[test]
fn reports_predating_provisional_ownership_still_deserialize_and_render() {
    let mut legacy = serde_json::to_value(fixture()).unwrap();
    legacy.as_object_mut().unwrap().remove("provenance");
    for market in legacy["markets"].as_array_mut().unwrap() {
        market
            .as_object_mut()
            .unwrap()
            .remove("provisional_owner_positions");
        for scenario in market["stress"]["scenarios"].as_array_mut().unwrap() {
            scenario
                .as_object_mut()
                .unwrap()
                .remove("matched_price_overrides");
            scenario
                .as_object_mut()
                .unwrap()
                .remove("unmatched_price_overrides");
        }
    }
    let report: Report = serde_json::from_value(legacy).unwrap();
    assert_eq!(report.markets[0].provisional_owner_positions, 0);
    assert!(
        export::render(&report)
            .unwrap()
            .contains("snapshot_complete 1")
    );
}

#[test]
fn real_report_exports_counts_decimal_values_and_zero_buckets_without_wallet_labels() {
    let report = fixture();
    let text = export::render(&report).unwrap();
    let market = &report.markets[0];
    assert!(text.contains(&format!(
        "sui_sentinel_positions{{market=\"{}\"}} 90\n",
        market.market_id
    )));
    assert!(text.contains("status=\"unresolved\"} 1\n"));
    assert!(text.contains("bucket=\"at_least_1_25\"} 59.9909004\n"));
    assert!(text.contains("bucket=\"below_1\"} 0\n"));
    assert!(text.contains("sui_sentinel_snapshot_complete 1\n"));
    for (address, _) in &market.largest_direct_wallets_by_cached_debt {
        assert!(!text.contains(address));
    }
    for (address, _) in &market.largest_positions_by_cached_debt {
        assert!(!text.contains(address));
    }
}

#[test]
fn incomplete_reports_remain_explicit_and_timestamp_uses_source_time() {
    let mut report = fixture();
    report.capture_complete = false;
    report.markets[0].stress.as_mut().unwrap().status =
        sui_sentinel::analysis::stress::Status::IncompleteCoverage;
    report.markets[0].expected_positions += 10;
    report.checkpoint.timestamp = "1970-01-01T01:00:01.123+01:00".into();
    let text = export::render(&report).unwrap();
    assert!(text.contains("sui_sentinel_snapshot_complete 0\n"));
    assert!(text.contains("sui_sentinel_snapshot_timestamp_seconds 1.123000000\n"));
}

#[test]
fn rejects_inconsistent_or_unsupported_reports() {
    let cases: &[fn(&mut Report)] = &[
        |r| r.schema_version += 1,
        |r| r.calculation_status = "verified".into(),
        |r| r.overall_grade = Some("A".into()),
        |r| r.checkpoint.timestamp = "not a timestamp".into(),
        |r| r.checkpoint.sequence_number = 1 << 53,
        |r| r.scope = "unknown".into(),
        |r| r.markets[0].expected_positions += 1,
        |r| r.markets[0].diagnostics_available += 1,
        |r| r.markets[0].address_owned_positions += 1,
        |r| r.markets[0].unique_direct_wallets = 1000,
        |r| r.markets[0].cached_health_buckets.clear(),
        |r| {
            r.markets[0]
                .cached_collateral_by_asset_usd
                .insert("bad".into(), DecimalString(Decimal::NEGATIVE_ONE));
        },
        |r| {
            let duplicate =
                serde_json::from_value(serde_json::to_value(&r.markets[0]).unwrap()).unwrap();
            r.markets.push(duplicate);
        },
        |r| {
            r.markets[0].diagnostics_available = u64::MAX;
            r.markets[0].diagnostics_unavailable = 1;
        },
    ];
    for (index, modify) in cases.iter().enumerate() {
        let mut report = fixture();
        modify(&mut report);
        assert!(
            export::render(&report).is_err(),
            "accepted invalid case {index}"
        );
    }
}

#[test]
fn escapes_prometheus_labels_and_rejects_top_wallet_sum_overflow() {
    let mut report = fixture();
    report.markets[0]
        .cached_debt_by_asset_usd
        .insert("quote\"line\nslash\\".into(), DecimalString(Decimal::ONE));
    assert!(
        export::render(&report)
            .unwrap()
            .contains("asset=\"quote\\\"line\\nslash\\\\\",asset_name=")
    );
    report.markets[0].largest_direct_wallets_by_cached_debt = vec![
        (format!("0x{}", "a".repeat(64)), DecimalString(Decimal::MAX)),
        (format!("0x{}", "b".repeat(64)), DecimalString(Decimal::ONE)),
    ];
    assert!(export::render(&report).is_err());
}

#[test]
fn stress_scores_fail_closed_and_legacy_reports_remain_readable() {
    let mut report = fixture();
    let summary = report.markets[0].stress.as_ref().unwrap();
    assert!(summary.score.is_none());
    assert_eq!(summary.evaluated_positions + summary.excluded_positions, 90);
    let text = export::render(&report).unwrap();
    assert!(text.contains("sui_sentinel_cached_stress_shortfall_usd{"));
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("sui_sentinel_economic_score{"))
    );
    report.markets[0].stress.as_mut().unwrap().score = Some(Decimal::ZERO);
    assert!(export::render(&report).is_err());
    report.stress_policy = None;
    report.markets[0].stress = None;
    let legacy = export::render(&report).unwrap();
    assert!(legacy.contains("status=\"not_computed\""));
}

#[tokio::test]
async fn http_serves_immutable_artifacts_and_health() {
    let report = fixture();
    let expected: serde_json::Value = serde_json::to_value(&report).unwrap();
    let expected_metrics = export::render(&report).unwrap();
    let app = export::router(report).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    for _ in 0..2 {
        let response = client.get(format!("{base}/metrics")).send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; version=0.0.4; charset=utf-8"
        );
        assert_eq!(response.text().await.unwrap(), expected_metrics);
    }
    let response = client.get(format!("{base}/report")).send().await.unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        expected
    );
    assert!(
        client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(
        client
            .get(format!("{base}/missing"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.abort();
}

#[tokio::test]
async fn serve_dir_reloads_without_restart_and_keeps_last_good_on_a_bad_pointer() {
    let base = std::env::temp_dir().join(format!(
        "sui-sentinel-serve-dir-test-{}",
        std::process::id()
    ));
    fs::create_dir(&base).unwrap();
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");

    fs::create_dir(base.join("gen-a")).unwrap();
    let dir = base.join("gen-a/analysis");
    let mut fresh = report::generate(&input, &dir).unwrap();
    fresh.checkpoint.timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    fs::write(dir.join("report.json"), serde_json::to_vec(&fresh).unwrap()).unwrap();
    fs::remove_file(dir.join("checksums.json")).unwrap();
    storage::write_checksums(&dir, &["positions.ndjson", "report.json"]).unwrap();
    storage::write_pointer(&base.join("current"), "gen-a").unwrap();

    // export::serve_dir binds its own listener; find a free loopback port
    // by binding briefly and releasing it, then hand that address to
    // serve_dir. Nothing else in this test process binds concurrently, so
    // the small bind/release/rebind race is not a practical flake risk
    // here.
    let address = {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap()
    };
    let base_for_server = base.clone();
    let server = tokio::spawn(async move {
        export::serve_dir(&base_for_server, address, Duration::from_millis(50))
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    let http_base = format!("http://{address}");

    // Restart-free pickup: serve_dir's own initial load (not a poll tick)
    // should already make /readyz succeed shortly after startup.
    wait_until(&client, &format!("{http_base}/readyz"), 200).await;
    let first = client
        .get(format!("{http_base}/report"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(first["schema_version"], 1);
    assert!(first["provenance"]["generated_at"].is_string());

    // A pointer naming a generation that was never published must not take
    // the server down or drop the last good generation.
    storage::write_pointer(&base.join("current"), "gen-missing").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let status: serde_json::Value = client
        .get(format!("{http_base}/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["current_generation"], "gen-a");
    assert!(status["last_reload_error"].is_string());
    assert_eq!(
        client
            .get(format!("{http_base}/readyz"))
            .send()
            .await
            .unwrap()
            .status(),
        503,
        "reload failure degrades readiness while historical report stays readable"
    );

    assert_eq!(
        client
            .get(format!("{http_base}/report"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{http_base}/livez"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    // A newly loaded old checkpoint stays stale; loading cannot refresh it.
    fs::create_dir(base.join("gen-b")).unwrap();
    report::generate(&input, &base.join("gen-b").join("analysis")).unwrap();
    storage::write_pointer(&base.join("current"), "gen-b").unwrap();
    wait_until_generation(&client, &http_base, "gen-b").await;
    wait_until(&client, &format!("{http_base}/readyz"), 503).await;
    let status: serde_json::Value = client
        .get(format!("{http_base}/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["readiness_reason"], "stale_observation");

    server.abort();
    fs::remove_dir_all(&base).unwrap();
}

async fn wait_until(client: &reqwest::Client, url: &str, expected_status: u16) {
    for _ in 0..100 {
        if let Ok(response) = client.get(url).send().await
            && response.status().as_u16() == expected_status
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{url} never reached status {expected_status}");
}

async fn wait_until_generation(client: &reqwest::Client, base: &str, generation: &str) {
    for _ in 0..100 {
        if let Ok(response) = client.get(format!("{base}/status")).send().await
            && let Ok(status) = response.json::<serde_json::Value>().await
            && status["current_generation"] == generation
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{base} never swapped to generation {generation}");
}
