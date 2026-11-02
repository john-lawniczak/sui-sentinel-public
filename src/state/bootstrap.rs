use super::Engine;
use crate::{
    model::{Manifest, Object},
    storage,
};
use anyhow::{Result, ensure};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

fn census(path: &Path) -> Result<Manifest> {
    let manifest: Manifest = storage::read_json(&path.join("manifest.json"))?;
    ensure!(
        manifest.schema_version == 2 && manifest.complete,
        "bootstrap/reconciliation requires a complete bound schema-2 census"
    );
    storage::raw::verify(path, &manifest)?;
    ensure!(
        !manifest.markets.is_empty()
            && manifest.chain_identifier == crate::config::MAINNET_CHAIN
            && !manifest.checkpoint.digest.is_empty(),
        "invalid bootstrap identity"
    );
    time::OffsetDateTime::parse(
        &manifest.checkpoint.timestamp,
        &time::format_description::well_known::Rfc3339,
    )?;
    ensure!(
        manifest.markets.iter().all(|m| m.owner_caps_exhausted
            && m.obligations_exhausted
            && m.obligations == m.market.expected_obligations),
        "incomplete census metadata"
    );
    Ok(manifest)
}

fn stream(
    path: PathBuf,
    manifest: Manifest,
) -> (
    tokio::sync::mpsc::Receiver<Object>,
    tokio::task::JoinHandle<Result<()>>,
) {
    // The blocking file reader cannot outrun database writes or allocate an
    // unbounded object buffer. Cancellation closes the receiver and stops it.
    let (sender, receiver) = tokio::sync::mpsc::channel(32);
    let task = tokio::task::spawn_blocking(move || {
        let mut ids = HashSet::new();
        let mut send = |o: Object| -> Result<()> {
            ensure!(
                ids.insert(o.address.clone()),
                "duplicate object in bootstrap census"
            );
            sender
                .blocking_send(o)
                .map_err(|_| anyhow::anyhow!("bootstrap consumer stopped"))
        };
        send(manifest.clock)?;
        send(manifest.upgrade_cap)?;
        for market in manifest.markets {
            send(market.market.object.clone())?;
            for (kind, expected) in [
                ("obligations", market.obligations),
                ("caps", market.owner_caps),
            ] {
                let mut count = 0;
                storage::read_lines(
                    &storage::market_file(&path, &market.market.object.address, kind),
                    |o: Object| {
                        send(o)?;
                        count += 1;
                        Ok(())
                    },
                )?;
                ensure!(count == expected, "raw census count mismatch");
            }
        }
        Ok(())
    });
    (receiver, task)
}

impl Engine {
    pub async fn bootstrap(&self, path: &Path) -> Result<()> {
        let owned = path.to_owned();
        let manifest = tokio::task::spawn_blocking(move || census(&owned)).await??;
        let config: serde_json::Value = storage::read_json(&path.join("capture-config.json"))?;
        let config_hash = storage::sha256_hex_of(&serde_json::to_vec(&config)?);
        // Hash serialization through the typed adapter configuration for stable field order.
        let capture_config: crate::config::Config = serde_json::from_value(config)?;
        let typed_hash = storage::sha256_hex_of(&serde_json::to_vec(&capture_config)?);
        ensure!(
            typed_hash == self.adapter.configuration_hash()?,
            "bootstrap configuration differs from adapter"
        );
        let hash = storage::sha256_hex(&path.join("checksums.json"))?;
        let cp = i64::try_from(manifest.checkpoint.sequence_number)?;
        let checkpoint = manifest.checkpoint.clone();
        let mut tx = self.store.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,1))")
            .bind(&self.scope)
            .execute(&mut *tx)
            .await?;
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT adapter,configuration_hash FROM chain_streams WHERE scope=$1 FOR UPDATE",
        )
        .bind(&self.scope)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((adapter, configuration)) = existing {
            let stored: Option<(String,)> = sqlx::query_as(
                "SELECT payload_hash FROM chain_frames WHERE scope=$1 AND checkpoint=$2",
            )
            .bind(&self.scope)
            .bind(cp)
            .fetch_optional(&mut *tx)
            .await?;
            ensure!(
                adapter == self.adapter.id()
                    && configuration == typed_hash
                    && stored.is_some_and(|(h,)| h == hash),
                "bootstrap identity conflict"
            );
            return Ok(());
        }
        sqlx::query("INSERT INTO chain_streams(scope,adapter,configuration_hash,chain_identifier,checkpoint,digest,evaluated_checkpoint,reconciled_checkpoint,checkpoint_time) VALUES ($1,$2,$3,$4,$5,$6,$5,$5,$7)")
            .bind(&self.scope)
            .bind(self.adapter.id())
            .bind(typed_hash)
            .bind(&manifest.chain_identifier)
            .bind(cp)
            .bind(&checkpoint.digest)
            .bind(time::OffsetDateTime::parse(&checkpoint.timestamp,&time::format_description::well_known::Rfc3339)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO chain_frames VALUES ($1,$2,$3,$4)")
            .bind(&self.scope)
            .bind(cp)
            .bind(hash)
            .bind(serde_json::json!({"bootstrap":manifest,"configuration_hash":config_hash}))
            .execute(&mut *tx)
            .await?;
        let (mut objects, task) = stream(path.to_owned(), manifest);
        while let Some(object) = objects.recv().await {
            self.put(&mut tx, &object).await?;
        }
        task.await??;
        sqlx::query("CREATE TEMP TABLE affected_positions (id TEXT PRIMARY KEY) ON COMMIT DROP")
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO affected_positions SELECT object_id FROM chain_objects WHERE scope=$1 AND kind='position'")
            .bind(&self.scope).execute(&mut *tx).await?;
        self.evaluate_affected(&mut tx, &checkpoint).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Compare an independently collected complete census at the *same*
    /// checkpoint. Mismatches fail visibly; no silent repair or cursor reset.
    pub async fn reconcile(&self, path: &Path) -> Result<()> {
        let owned = path.to_owned();
        let manifest = tokio::task::spawn_blocking(move || census(&owned)).await??;
        let mut tx = self.store.pool.begin().await?;
        let (cp,digest,chain):(i64,String,String)=sqlx::query_as("SELECT checkpoint,digest,chain_identifier FROM chain_streams WHERE scope=$1 FOR UPDATE")
            .bind(&self.scope).fetch_one(&mut *tx).await?;
        ensure!(
            cp == i64::try_from(manifest.checkpoint.sequence_number)?
                && digest == manifest.checkpoint.digest
                && chain == manifest.chain_identifier,
            "reconciliation checkpoint mismatch"
        );
        let (mut objects, task) = stream(path.to_owned(), manifest);
        let mut count = 0_i64;
        while let Some(object) = objects.recv().await {
            let stored:Option<(serde_json::Value,)> =sqlx::query_as("SELECT body FROM chain_objects WHERE scope=$1 AND object_id=$2 AND body IS NOT NULL")
                .bind(&self.scope).bind(&object.address).fetch_optional(&mut *tx).await?;
            ensure!(
                stored.is_some_and(|(v,)| v == serde_json::to_value(&object).unwrap()),
                "reconciliation object mismatch: {}",
                object.address
            );
            count += 1;
        }
        task.await??;
        let (actual,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM chain_objects WHERE scope=$1 AND body IS NOT NULL",
        )
        .bind(&self.scope)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(count == actual, "reconciliation census count mismatch");
        sqlx::query("UPDATE chain_streams SET reconciled_checkpoint=$2 WHERE scope=$1")
            .bind(&self.scope)
            .bind(cp)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
