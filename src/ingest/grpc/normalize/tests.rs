use super::*;
use serde_json::json;
fn checkpoint(changes: Vec<serde_json::Value>, config: &Config) -> pb::Checkpoint {
    let ty = suilend::obligation_type(
        &config.suilend.original_package_id,
        &config.suilend.lending_market_type,
    );
    serde_json::from_value(json!({"sequenceNumber":"12","digest":"cp12",
        "summary":{"sequenceNumber":"12","previousDigest":"cp11","timestamp":"2026-11-02T00:00:00Z"},
        "contents":{"transactions":changes.iter().enumerate().map(|(n,_)|json!({"transaction":format!("t{n}"),"effects":format!("e{n}")})).collect::<Vec<_>>()},
        "transactions":changes.into_iter().enumerate().map(|(n,change)|json!({"digest":format!("t{n}"),"effects":{"digest":format!("e{n}"),"lamportVersion":"22","changedObjects":[change]}})).collect::<Vec<_>>(),
        "objects":{"objects":[{"objectId":id(),"version":"21","objectType":ty}]}
    })).unwrap()
}
fn id() -> String {
    format!("0x{}", "1".repeat(64))
}
fn write() -> serde_json::Value {
    json!({"objectId":id(),"outputVersion":"21","outputDigest":"object-digest","outputState":"OUTPUT_OBJECT_STATE_OBJECT_WRITE","idOperation":"CREATED"})
}
fn remove(wrapped: bool) -> serde_json::Value {
    json!({"objectId":id(),"inputVersion":"21","outputState":"OUTPUT_OBJECT_STATE_DOES_NOT_EXIST","idOperation":if wrapped {"NONE"} else {"DELETED"}})
}
#[tokio::test]
async fn uses_checkpoint_object_types_and_pins_output_versions() {
    let config = Config::load("deploy/config/suilend.toml").unwrap();
    let cp = checkpoint(vec![write()], &config);
    let frame = normalize_with(
        &config,
        cp,
        &BTreeSet::new(),
        |address, version| async move {
            assert_eq!(version, 21);
            Ok(crate::model::Object {
                address,
                version,
                digest: "object-digest".into(),
                move_type: "test".into(),
                contents: json!({}),
                owner: None,
            })
        },
    )
    .await
    .unwrap();
    assert_eq!(frame.changes.len(), 1);
    assert_eq!(frame.previous_digest, "cp11");
}
#[tokio::test]
async fn creation_and_deletion_within_one_checkpoint_leave_no_live_object() {
    let config = Config::load("deploy/config/suilend.toml").unwrap();
    let frame = normalize_with(
        &config,
        checkpoint(vec![write(), remove(false)], &config),
        &BTreeSet::new(),
        |_, _| async { panic!("deleted object must not be fetched") },
    )
    .await
    .unwrap();
    assert!(frame.changes.is_empty());
    assert_eq!(
        frame.source.unwrap()["relevant_effects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
async fn wrapping_is_distinguished_from_permanent_deletion() {
    let config = Config::load("deploy/config/suilend.toml").unwrap();
    for wrapped in [true, false] {
        let frame = normalize_with(
            &config,
            checkpoint(vec![remove(wrapped)], &config),
            &BTreeSet::from([id()]),
            |_, _| async { panic!("absence must not fetch latest") },
        )
        .await
        .unwrap();
        assert!(
            matches!(&frame.changes[0],Change::Delete{wrapped:value,version:22,..} if *value==wrapped)
        );
    }
}
#[tokio::test]
async fn incomplete_checkpoints_or_object_version_mismatches_are_rejected() {
    let config = Config::load("deploy/config/suilend.toml").unwrap();
    let mut cp = checkpoint(vec![write()], &config);
    cp.transactions.clear();
    assert!(
        normalize_with(&config, cp, &BTreeSet::new(), |_, _| async {
            panic!("must reject partial transactions first")
        })
        .await
        .is_err()
    );
    let cp = checkpoint(vec![write()], &config);
    assert!(
        normalize_with(&config, cp, &BTreeSet::new(), |address, _| async move {
            Ok(crate::model::Object {
                address,
                version: 99,
                digest: "object-digest".into(),
                move_type: "test".into(),
                contents: json!({}),
                owner: None,
            })
        })
        .await
        .is_err()
    );
}
