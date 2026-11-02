//! Preserve checkpoint order; coalesce multiple writes to an object to its
//! final checkpoint state while retaining relevant transaction effects.
use super::Grpc;
use crate::{
    config::Config,
    model::Checkpoint,
    protocols::suilend,
    state::{Change, Frame},
};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
use sui_rpc::proto::sui::rpc::v2 as pb;

pub fn relevant_type(config: &Config, ty: &str) -> bool {
    ty == suilend::obligation_type(
        &config.suilend.original_package_id,
        &config.suilend.lending_market_type,
    ) || ty
        == suilend::cap_type(
            &config.suilend.original_package_id,
            &config.suilend.lending_market_type,
        )
}
pub fn changes_ids(cp: &pb::Checkpoint) -> Vec<String> {
    cp.transactions
        .iter()
        .filter_map(|t| t.effects.as_ref())
        .flat_map(|e| &e.changed_objects)
        .filter_map(|c| c.object_id.clone())
        .collect()
}
struct Final {
    change: pb::ChangedObject,
    delete_version: u64,
}

pub async fn normalize(
    source: &Grpc,
    config: &Config,
    cp: pb::Checkpoint,
    tracked: &BTreeSet<String>,
) -> Result<Frame> {
    normalize_with(config, cp, tracked, |id, version| {
        let source = source.clone();
        async move { source.object(&id, Some(version)).await }
    })
    .await
}

async fn normalize_with<F, Fut>(
    config: &Config,
    cp: pb::Checkpoint,
    tracked: &BTreeSet<String>,
    mut fetch: F,
) -> Result<Frame>
where
    F: FnMut(String, u64) -> Fut,
    Fut: std::future::Future<Output = Result<crate::model::Object>>,
{
    let sequence = cp.sequence_number.context("missing checkpoint sequence")?;
    let summary = cp.summary.as_ref().context("missing checkpoint summary")?;
    ensure!(
        summary.sequence_number == Some(sequence),
        "checkpoint summary sequence mismatch"
    );
    let contents = cp
        .contents
        .as_ref()
        .context("missing checkpoint contents")?;
    ensure!(
        contents.transactions.len() == cp.transactions.len(),
        "incomplete checkpoint transactions"
    );
    let types: BTreeMap<_, _> = cp
        .objects
        .as_ref()
        .into_iter()
        .flat_map(|set| &set.objects)
        .filter_map(|o| {
            Some((
                (o.object_id.as_deref()?, o.version?),
                o.object_type.as_deref()?,
            ))
        })
        .collect();
    let mut final_changes = BTreeMap::<String, Final>::new();
    let mut evidence = Vec::new();
    for (transaction, reference) in cp.transactions.iter().zip(&contents.transactions) {
        ensure!(
            transaction
                .digest
                .as_ref()
                .is_some_and(|d| reference.transaction.as_ref() == Some(d)),
            "checkpoint transaction order/digest mismatch"
        );
        let effects = transaction
            .effects
            .as_ref()
            .context("missing transaction effects")?;
        ensure!(
            effects
                .digest
                .as_ref()
                .is_some_and(|digest| reference.effects.as_ref() == Some(digest)),
            "checkpoint effects digest mismatch"
        );
        for change in &effects.changed_objects {
            let id = change
                .object_id
                .as_ref()
                .context("missing effects object ID")?;
            let known = tracked.contains(id)
                || final_changes.contains_key(id)
                || id == &config.suilend.lending_market_id
                || id == &config.suilend.upgrade_cap_id
                || id == "0x0000000000000000000000000000000000000000000000000000000000000006";
            let ty = change.object_type.as_deref().or_else(|| {
                change
                    .output_version
                    .and_then(|v| types.get(&(id.as_str(), v)).copied())
            });
            if !known {
                if change.output_state() != pb::changed_object::OutputObjectState::ObjectWrite {
                    continue;
                }
                let ty = ty.context(
                    "output object type missing from checkpoint; cannot discover new positions",
                )?;
                if !relevant_type(config, ty) {
                    continue;
                }
            }
            evidence.push(serde_json::json!({"transaction":transaction.digest,"change":change}));
            final_changes.insert(
                id.clone(),
                Final {
                    change: change.clone(),
                    delete_version: effects
                        .lamport_version
                        .context("missing effects lamport version")?,
                },
            );
        }
    }
    let mut changes = Vec::new();
    for (id, final_change) in final_changes {
        let change = final_change.change;
        match change.output_state() {
            pb::changed_object::OutputObjectState::ObjectWrite => {
                let version = change.output_version.context("missing output version")?;
                let object = fetch(id.clone(), version).await?;
                ensure!(
                    object.address == id && object.version == version,
                    "object identity/version differs from effects"
                );
                ensure!(
                    change.output_digest.as_deref() == Some(object.digest.as_str()),
                    "object digest differs from effects"
                );
                changes.push(Change::Upsert { object });
            }
            pb::changed_object::OutputObjectState::DoesNotExist => {
                if tracked.contains(&id) {
                    let operation = change.id_operation();
                    ensure!(
                        matches!(
                            operation,
                            pb::changed_object::IdOperation::Deleted
                                | pb::changed_object::IdOperation::None
                        ),
                        "unsupported ID disappearance"
                    );
                    changes.push(Change::Delete {
                        object_id: id,
                        version: final_change.delete_version,
                        wrapped: operation != pb::changed_object::IdOperation::Deleted,
                    });
                } // A newly created then removed object has no surviving state.
            }
            _ => anyhow::bail!("unsupported relevant output object state"),
        }
    }
    let timestamp = summary
        .timestamp
        .as_ref()
        .context("missing checkpoint timestamp")?;
    let timestamp = time::OffsetDateTime::from_unix_timestamp(timestamp.seconds)?
        .replace_nanosecond(u32::try_from(timestamp.nanos)?)?
        .format(&time::format_description::well_known::Rfc3339)?;
    Ok(Frame {
        schema_version: 1,
        chain_identifier: crate::config::MAINNET_CHAIN.into(),
        checkpoint: Checkpoint {
            sequence_number: sequence,
            digest: cp.digest.context("missing checkpoint digest")?,
            timestamp,
        },
        previous_digest: summary
            .previous_digest
            .clone()
            .context("missing predecessor digest")?,
        changes,
        dependency_changes: vec![],
        source: Some(
            serde_json::json!({"kind":"sui_grpc_v2","summary":summary,"relevant_effects":evidence,
            "transaction_count":cp.transactions.len(),"trust":"provider_observation_not_locally_signature_verified"}),
        ),
    })
}

#[cfg(test)]
mod tests;
