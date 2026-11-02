use super::*;
use crate::{
    config::Config, diagnosis::test_support, model::Manifest,
    protocols::suilend_incremental::Suilend, storage,
};
use std::path::{Path, PathBuf};

pub(super) fn fixture() -> (PathBuf, Config, Vec<Object>) {
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/elixir");
    let path =
        std::env::temp_dir().join(format!("sentinel-state-{}", test_support::unique_suffix()));
    std::fs::create_dir(&path).unwrap();
    for file in std::fs::read_dir(&input).unwrap() {
        let file = file.unwrap();
        if file.file_type().unwrap().is_file() {
            std::fs::copy(file.path(), path.join(file.file_name())).unwrap();
        }
    }
    let mut manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let mut config = Config::load("deploy/config/suilend.toml").unwrap();
    config.suilend.original_package_id = manifest.original_package_id.clone();
    config.suilend.published_package_id = manifest.upgrade_cap.contents["package"]
        .as_str()
        .unwrap()
        .into();
    config.suilend.package_version =
        crate::model::integer(&manifest.upgrade_cap.contents, "version").unwrap();
    config.suilend.upgrade_cap_id = manifest.upgrade_cap.address.clone();
    config.suilend.lending_market_id = manifest.markets[0].market.object.address.clone();
    config.suilend.lending_market_type = manifest.markets[0].market.type_argument.clone();
    config.suilend.market_version =
        crate::model::integer(&manifest.markets[0].market.object.contents, "version").unwrap();
    manifest.schema_version = 2;
    std::fs::write(
        path.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    storage::json(&path.join("capture-config.json"), &config).unwrap();
    storage::raw::seal(&path, &manifest).unwrap();
    let mut positions = vec![];
    storage::read_lines(
        &storage::market_file(&path, &config.suilend.lending_market_id, "obligations"),
        |o: Object| {
            positions.push(o);
            Ok(())
        },
    )
    .unwrap();
    (path, config, positions)
}
pub(super) fn next(manifest: &Manifest) -> Frame {
    let mut checkpoint = manifest.checkpoint.clone();
    checkpoint.sequence_number += 1;
    checkpoint.digest = "next-digest".into();
    Frame {
        schema_version: 1,
        chain_identifier: manifest.chain_identifier.clone(),
        checkpoint,
        previous_digest: manifest.checkpoint.digest.clone(),
        changes: vec![],
        dependency_changes: vec![],
        source: None,
    }
}

#[tokio::test]
async fn evaluation_pages_past_absent_positions() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, positions) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let mut tx = engine.store.pool.begin().await.unwrap();
    sqlx::query("CREATE TEMP TABLE affected_positions (id TEXT PRIMARY KEY) ON COMMIT DROP")
        .execute(&mut *tx)
        .await
        .unwrap();
    // A full page of absent IDs sorts before the real position. This exercises
    // pagination independently of which rows survive the object lookup.
    sqlx::query(
        "INSERT INTO affected_positions SELECT '!' || g::text FROM generate_series(1,256) g",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO affected_positions VALUES ($1)")
        .bind(&positions[0].address)
        .execute(&mut *tx)
        .await
        .unwrap();
    let checkpoint = next(&manifest).checkpoint;
    engine
        .evaluate_affected(&mut tx, &checkpoint)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        engine
            .evaluation(&positions[0].address)
            .await
            .unwrap()
            .unwrap()["checkpoint"]["sequenceNumber"],
        checkpoint.sequence_number
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn bootstrap_replay_dependency_changes_reconciliation_and_gap_rejection() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, positions) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    engine.bootstrap(&path).await.unwrap();
    engine.reconcile(&path).await.unwrap();
    let old = engine
        .evaluation(&positions[0].address)
        .await
        .unwrap()
        .unwrap();
    let mut frame = next(&manifest);
    let mut market = manifest.markets[0].market.object.clone();
    market.version += 1;
    frame.changes.push(Change::Upsert { object: market });
    let (a, b) = tokio::join!(engine.apply(&frame), engine.apply(&frame));
    assert_ne!(a.unwrap(), b.unwrap());
    let new = engine
        .evaluation(&positions[0].address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old["checkpoint"], new["checkpoint"]); // Market version alone does not refresh cached math.
    assert_eq!(old["cached_diagnostics"], new["cached_diagnostics"]);
    assert!(new["fresh_risk"].is_null());
    assert!(engine.reconcile(&path).await.is_err());
    frame.previous_digest = "wrong".into();
    frame.checkpoint.sequence_number += 1;
    assert!(engine.apply(&frame).await.is_err());
    assert_eq!(
        engine.checkpoint().await.unwrap(),
        manifest.checkpoint.sequence_number + 1
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn failed_checkpoint_rolls_back_earlier_mutations_and_rejects_conflicting_replay() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, positions) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let old = engine.evaluation(&positions[0].address).await.unwrap();
    let mut frame = next(&manifest);
    let mut position = positions[0].clone();
    position.version += 1;
    let mut upgrade = manifest.upgrade_cap.clone();
    upgrade.version += 1;
    upgrade.contents["version"] = serde_json::json!(99999);
    frame.changes = vec![
        Change::Upsert { object: position },
        Change::Upsert { object: upgrade },
    ];
    assert!(engine.apply(&frame).await.is_err());
    assert_eq!(
        engine.checkpoint().await.unwrap(),
        manifest.checkpoint.sequence_number
    );
    assert_eq!(engine.evaluation(&positions[0].address).await.unwrap(), old);
    frame.changes.pop();
    engine.apply(&frame).await.unwrap();
    frame.checkpoint.digest = "conflicting".into();
    assert!(engine.apply(&frame).await.is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn capability_transfer_and_deletion_invalidate_the_related_unchanged_position() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, _) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let mut caps = vec![];
    storage::read_lines(
        &storage::market_file(&path, &config.suilend.lending_market_id, "caps"),
        |o: Object| {
            caps.push(o);
            Ok(())
        },
    )
    .unwrap();
    let mut cap = caps.remove(0);
    let position = cap.contents["obligation_id"].as_str().unwrap().to_owned();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let before = engine.evaluation(&position).await.unwrap().unwrap();
    cap.version += 1;
    cap.owner.as_mut().unwrap().address = Some(format!("0x{}", "1".repeat(64)));
    let mut frame = next(&manifest);
    frame.changes.push(Change::Upsert {
        object: cap.clone(),
    });
    engine.apply(&frame).await.unwrap();
    let after = engine.evaluation(&position).await.unwrap().unwrap();
    assert_ne!(before["capabilities"], after["capabilities"]);
    frame.previous_digest = frame.checkpoint.digest.clone();
    frame.checkpoint.sequence_number += 1;
    frame.checkpoint.digest = "delete".into();
    frame.changes = vec![Change::Delete {
        object_id: cap.address,
        version: cap.version + 1,
        wrapped: false,
    }];
    engine.apply(&frame).await.unwrap();
    assert_eq!(
        engine.evaluation(&position).await.unwrap().unwrap()["capabilities"],
        serde_json::json!([])
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn actor_applies_backpressure_while_database_commit_is_blocked() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, _) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let scope = test_support::unique_suffix();
    let engine = Engine::new(store.clone(), Arc::new(Suilend { config }), scope.clone()).unwrap();
    engine.bootstrap(&path).await.unwrap();
    let mut blocker = store.pool.begin().await.unwrap();
    sqlx::query("SELECT scope FROM chain_streams WHERE scope=$1 FOR UPDATE")
        .bind(&scope)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let (sender, task) = worker::spawn(engine, 1).unwrap();
    let (committed, ack) = tokio::sync::oneshot::channel();
    sender
        .send(worker::Request {
            frame: next(&manifest),
            committed,
        })
        .await
        .unwrap();
    let (committed, ack2) = tokio::sync::oneshot::channel();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sender.send(worker::Request {
            frame: next(&manifest),
            committed,
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let (committed, _) = tokio::sync::oneshot::channel();
    assert!(matches!(
        sender.try_send(worker::Request {
            frame: next(&manifest),
            committed
        }),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_))
    ));
    blocker.rollback().await.unwrap();
    assert!(ack.await.unwrap().unwrap());
    assert!(!ack2.await.unwrap().unwrap());
    drop(sender);
    task.await.unwrap();
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn wrapped_capabilities_can_return_but_deleted_ids_cannot() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, _) = fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let mut caps = vec![];
    storage::read_lines(
        &storage::market_file(&path, &config.suilend.lending_market_id, "caps"),
        |o: Object| {
            caps.push(o);
            Ok(())
        },
    )
    .unwrap();
    let mut cap = caps.remove(0);
    let position = cap.contents["obligation_id"].as_str().unwrap().to_owned();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let mut frame = next(&manifest);
    frame.changes = vec![Change::Delete {
        object_id: cap.address.clone(),
        version: cap.version + 1,
        wrapped: true,
    }];
    engine.apply(&frame).await.unwrap();
    assert_eq!(
        engine.evaluation(&position).await.unwrap().unwrap()["capabilities"],
        serde_json::json!([])
    );
    cap.version += 2;
    frame.previous_digest = frame.checkpoint.digest.clone();
    frame.checkpoint.sequence_number += 1;
    frame.checkpoint.digest = "unwrap".into();
    frame.changes = vec![Change::Upsert {
        object: cap.clone(),
    }];
    engine.apply(&frame).await.unwrap();
    assert_eq!(
        engine.evaluation(&position).await.unwrap().unwrap()["capabilities"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    frame.previous_digest = frame.checkpoint.digest.clone();
    frame.checkpoint.sequence_number += 1;
    frame.checkpoint.digest = "permanent-delete".into();
    frame.changes = vec![Change::Delete {
        object_id: cap.address.clone(),
        version: cap.version + 1,
        wrapped: false,
    }];
    engine.apply(&frame).await.unwrap();
    cap.version += 2;
    frame.previous_digest = frame.checkpoint.digest.clone();
    frame.checkpoint.sequence_number += 1;
    frame.checkpoint.digest = "invalid-reuse".into();
    frame.changes = vec![Change::Upsert { object: cap }];
    assert!(engine.apply(&frame).await.is_err());
    std::fs::remove_dir_all(path).unwrap();
}
