use super::*;
use crate::{
    diagnosis::test_support, model::Manifest, protocols::suilend_incremental::Suilend, storage,
};
#[tokio::test]
async fn clock_advances_observation_without_relabeling_cached_calculation() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, positions) = super::tests::fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let before = engine.evaluation(&positions[0].address).await.unwrap();
    let mut frame = super::tests::next(&manifest);
    let mut clock = manifest.clock.clone();
    clock.version += 1;
    clock.contents["timestamp_ms"] = serde_json::json!(
        (crate::model::integer(&clock.contents, "timestamp_ms").unwrap() + 1000).to_string()
    );
    frame.changes.push(Change::Upsert { object: clock });
    assert!(engine.apply(&frame).await.unwrap());
    assert_eq!(
        engine.checkpoint().await.unwrap(),
        frame.checkpoint.sequence_number
    );
    assert_eq!(
        engine.evaluation(&positions[0].address).await.unwrap(),
        before
    );
    assert!(!engine.apply(&frame).await.unwrap());
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn cached_market_reuse_still_rejects_changed_reserve_references() {
    let Some(store) = test_support::store().await else {
        return;
    };
    let (path, config, positions) = super::tests::fixture();
    let manifest: Manifest = storage::read_json(&path.join("manifest.json")).unwrap();
    let engine = Engine::new(
        store,
        Arc::new(Suilend { config }),
        test_support::unique_suffix(),
    )
    .unwrap();
    engine.bootstrap(&path).await.unwrap();
    let before = engine.evaluation(&positions[0].address).await.unwrap();
    let mut frame = super::tests::next(&manifest);
    let mut market = manifest.markets[0].market.object.clone();
    market.version += 1;
    frame.changes = vec![Change::Upsert {
        object: market.clone(),
    }];
    engine.apply(&frame).await.unwrap();
    assert_eq!(
        engine.evaluation(&positions[0].address).await.unwrap(),
        before
    );
    frame.previous_digest = frame.checkpoint.digest.clone();
    frame.checkpoint.sequence_number += 1;
    frame.checkpoint.digest = "invalid-reserve-reference".into();
    market.version += 1;
    // The fixture's positions use this reserve: changing its type invalidates them.
    let index = positions
        .iter()
        .find_map(|p| {
            p.contents["deposits"]
                .as_array()?
                .first()?
                .get("reserve_array_index")
        })
        .unwrap();
    let index = if let Some(s) = index.as_str() {
        s.parse::<usize>().unwrap()
    } else {
        index.as_u64().unwrap() as usize
    };
    market.contents["reserves"][index]["coin_type"] = serde_json::json!("0x2::unsupported::COIN");
    frame.changes = vec![Change::Upsert { object: market }];
    assert!(engine.apply(&frame).await.is_err());
    assert_eq!(
        engine.checkpoint().await.unwrap(),
        manifest.checkpoint.sequence_number + 1
    );
    std::fs::remove_dir_all(path).unwrap();
}
