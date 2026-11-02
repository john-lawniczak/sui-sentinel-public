//! Transactional checkpoint projections. The input contract requires complete,
//! ordered changes from a trusted checkpoint provider; JSON alone is not proof.
mod apply;
mod bootstrap;
pub mod worker;
use crate::{
    diagnosis::Store,
    model::{Checkpoint, Object},
    protocols::adapter::Adapter,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct Engine {
    store: Store,
    adapter: Arc<dyn Adapter>,
    scope: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub schema_version: u32,
    pub chain_identifier: String,
    pub checkpoint: Checkpoint,
    pub previous_digest: String,
    pub changes: Vec<Change>,
    /// Dependency observations are evidence, never substituted into cached math.
    pub dependency_changes: Vec<DependencyChange>,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Upsert {
        object: Object,
    },
    Delete {
        object_id: String,
        version: u64,
        #[serde(default)]
        wrapped: bool,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyChange {
    pub key: String,
    pub provenance: String,
    pub evidence: serde_json::Value,
}
impl Engine {
    pub fn new(store: Store, adapter: Arc<dyn Adapter>, scope: String) -> Result<Self> {
        ensure!(!scope.is_empty(), "empty state scope");
        Ok(Self {
            store,
            adapter,
            scope,
        })
    }
    pub async fn tracked(&self, ids: &[String]) -> Result<std::collections::BTreeSet<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT object_id FROM chain_objects WHERE scope=$1 AND object_id=ANY($2)",
        )
        .bind(&self.scope)
        .bind(ids)
        .fetch_all(&self.store.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }
    pub async fn checkpoint(&self) -> Result<u64> {
        let (cp,): (i64,) = sqlx::query_as("SELECT checkpoint FROM chain_streams WHERE scope=$1")
            .bind(&self.scope)
            .fetch_one(&self.store.pool)
            .await?;
        Ok(u64::try_from(cp)?)
    }
    pub async fn evaluation(&self, position: &str) -> Result<Option<serde_json::Value>> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT evaluation FROM position_evaluations WHERE scope=$1 AND position_id=$2",
        )
        .bind(&self.scope)
        .bind(position)
        .fetch_optional(&self.store.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod clock_tests;
