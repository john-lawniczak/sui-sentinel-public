//! Durable atomic diagnoses and leased webhook delivery.
pub mod delivery;
mod discord;
pub mod health_rules;
mod incident;
mod operations;
mod outbox;
pub mod rules;
pub mod transition;
mod windows;

pub use incident::{Incident, IncidentCandidate, State};
pub use outbox::OutboxEntry;

use anyhow::{Context, Result};
use sqlx::PgPool;

/// Owns the database connection pool. Cheap to clone (an `Arc` internally
/// via `sqlx::Pool`), so callers can hold one `Store` for the process
/// lifetime.
#[derive(Clone)]
pub struct Store {
    pub(crate) pool: PgPool,
}

impl Store {
    /// Connect without running migrations - callers that only read
    /// (a future query service) should not be the ones applying schema
    /// changes.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let options: sqlx::postgres::PgConnectOptions = database_url.parse()?;
        let options = options.application_name("sui-sentinel").options([
            ("statement_timeout", "15000"),
            ("lock_timeout", "5000"),
            ("idle_in_transaction_session_timeout", "20000"),
            ("tcp_keepalives_idle", "15"),
            ("tcp_keepalives_interval", "5"),
            ("tcp_keepalives_count", "3"),
        ]);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_with(options)
            .await
            .context("connecting to the diagnosis database")?;
        Ok(Self { pool })
    }

    /// Apply every migration in `migrations/` that hasn't run yet.
    /// Idempotent and safe to call on every process start (temp.md
    /// section 5: "On restart, resume from committed progress") - a
    /// second process racing this on startup is serialized by sqlx's own
    /// migration-lock table, not left to collide.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("running diagnosis database migrations")
    }
}

impl Store {
    pub async fn prepare(&self) -> Result<()> {
        if std::env::var("SENTINEL_AUTO_MIGRATE").as_deref() == Ok("false") {
            let (version,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(max(version),0) FROM _sqlx_migrations WHERE success",
            )
            .fetch_one(&self.pool)
            .await?;
            anyhow::ensure!(
                version >= 8,
                "database migrations required; run migrate with the migration role"
            );
            Ok(())
        } else {
            self.migrate().await
        }
    }
    pub async fn scope_lease(&self, scope: &str) -> Result<sqlx::PgConnection> {
        let mut connection = self.pool.acquire().await?.detach();
        let (held,): (bool,) =
            sqlx::query_as("SELECT pg_try_advisory_lock(hashtextextended($1,2))")
                .bind(scope)
                .fetch_one(&mut connection)
                .await?;
        anyhow::ensure!(held, "another live coordinator owns this scope");
        Ok(connection)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared by `incident` and `outbox`'s own test modules. Requires a
    //! reachable Postgres server named by `SENTINEL_TEST_DATABASE_URL`;
    //! tests here are skipped (not failed) when that variable is unset, so
    //! `cargo test` stays usable without a database in environments that
    //! don't have one configured. CI and local development that DO want
    //! this coverage set that variable to a scratch database.
    use super::Store;

    pub async fn store() -> Option<Store> {
        let url = std::env::var("SENTINEL_TEST_DATABASE_URL").ok()?;
        let store = Store::connect(&url)
            .await
            .expect("connecting to test database");
        store.migrate().await.expect("migrating test database");
        // Tests run against a shared scratch database and clean up after
        // themselves via unique dedup keys per test, rather than each
        // test owning an isolated schema - simpler, and fast enough at
        // this test count.
        Some(store)
    }

    /// A per-process-and-time discriminator for dedup keys, so concurrent
    /// test runs against a shared scratch database never collide.
    pub fn unique_suffix() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        )
    }
}

#[cfg(test)]
mod operations_tests;
