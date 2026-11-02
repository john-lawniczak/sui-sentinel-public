//! Bounded, scope-authorized reads. Each answer uses one read-only MVCC snapshot.
mod read;
pub mod types;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
pub use types::Request;

#[derive(Clone)]
pub struct Queries {
    pool: sqlx::PgPool,
    scopes: Arc<BTreeSet<String>>,
    admission: Arc<Semaphore>,
}
impl Queries {
    pub async fn connect(url: &str, scopes: Vec<String>) -> Result<Self> {
        ensure!(
            !scopes.is_empty()
                && scopes.len() <= 32
                && scopes.iter().all(|s| !s.is_empty() && s.len() <= 128),
            "configure 1..32 explicit scopes"
        );
        let options: PgConnectOptions = url.parse()?;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(1))
            .connect_with(options.application_name("sentinel-read-only").options([
                ("default_transaction_read_only", "on"),
                ("statement_timeout", "2000"),
                ("lock_timeout", "500"),
                ("idle_in_transaction_session_timeout", "5000"),
            ]))
            .await?;
        Ok(Self {
            pool,
            scopes: Arc::new(scopes.into_iter().collect()),
            admission: Arc::new(Semaphore::new(4)),
        })
    }
    pub fn scopes(&self) -> Vec<String> {
        self.scopes.iter().cloned().collect()
    }
    pub async fn execute(&self, kind: &str, request: Request) -> Value {
        let result = async {
            ensure!(self.scopes.contains(&request.scope), "scope_forbidden");
            request.validate()?;
            ensure!(
                kind == "incident_changes"
                    || request.after.is_none()
                    || request.generation.is_some(),
                "invalid_arguments"
            );
            let _permit = self
                .admission
                .try_acquire()
                .map_err(|_| anyhow::anyhow!("query_busy"))?;
            let answer = tokio::time::timeout(Duration::from_secs(3), self.read(kind, request))
                .await
                .map_err(|_| anyhow::anyhow!("query_deadline"))??;
            ensure!(
                serde_json::to_vec(&answer)?.len() <= 120 * 1024,
                "response_too_large_reduce_limit"
            );
            Ok::<_, anyhow::Error>(answer)
        }
        .await;
        match result {
            Ok(value) => value,
            Err(error) => {
                // Only fixed application error codes cross the model boundary.
                let text = error.to_string();
                let code = if [
                    "scope_forbidden",
                    "query_busy",
                    "query_deadline",
                    "generation_expired",
                    "history_expired",
                    "stream_unavailable",
                    "invalid_arguments",
                    "unknown_query",
                    "response_too_large_reduce_limit",
                    "evidence_not_found",
                ]
                .contains(&text.as_str())
                {
                    text.as_str()
                } else {
                    "query_unavailable"
                };
                json!({"error":{"code":code},"execution_eligible":false})
            }
        }
    }
}

#[cfg(test)]
mod tests;
