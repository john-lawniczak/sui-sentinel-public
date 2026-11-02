//! Incident rows and their opened/updated/acknowledged/resolved state machine.
//! Policy debounce and reminder windows live in `windows` and `transition`.

use std::str::FromStr;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;

use super::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Opened,
    Updated,
    Acknowledged,
    Resolved,
}

impl FromStr for State {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "opened" => Ok(Self::Opened),
            "updated" => Ok(Self::Updated),
            "acknowledged" => Ok(Self::Acknowledged),
            "resolved" => Ok(Self::Resolved),
            other => bail!("unknown incident state {other:?}"),
        }
    }
}

/// What a rule computes when it evaluates current signals. Not yet an
/// `Incident` - the store decides, from `dedup_key`, whether this opens a
/// new incident or updates one already open.
#[derive(Serialize)]
pub struct IncidentCandidate {
    pub dedup_key: String,
    pub rule_id: String,
    pub rule_version: i32,
    pub scope: String,
    pub severity: String,
    pub valuation_basis: String,
    pub checkpoint_sequence: Option<i64>,
    pub measured_trigger: Value,
    pub evidence: Value,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    dedup_key: String,
    rule_id: String,
    rule_version: i32,
    scope: String,
    severity: String,
    state: String,
    checkpoint_sequence: Option<i64>,
    valuation_basis: String,
    measured_trigger: Value,
    evidence: Value,
    opened_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    resolved_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize)]
pub struct Incident {
    pub id: i64,
    pub dedup_key: String,
    pub rule_id: String,
    pub rule_version: i32,
    pub scope: String,
    pub severity: String,
    pub state: State,
    pub checkpoint_sequence: Option<i64>,
    pub valuation_basis: String,
    pub measured_trigger: Value,
    pub evidence: Value,
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
}

impl TryFrom<Row> for Incident {
    type Error = anyhow::Error;
    fn try_from(row: Row) -> Result<Self> {
        Ok(Self {
            id: row.id,
            dedup_key: row.dedup_key,
            rule_id: row.rule_id,
            rule_version: row.rule_version,
            scope: row.scope,
            severity: row.severity,
            state: row
                .state
                .parse()
                .context("decoding stored incident state")?,
            checkpoint_sequence: row.checkpoint_sequence,
            valuation_basis: row.valuation_basis,
            measured_trigger: row.measured_trigger,
            evidence: row.evidence,
            opened_at: row.opened_at,
            updated_at: row.updated_at,
            resolved_at: row.resolved_at,
        })
    }
}

impl Store {
    /// Open a new incident for `candidate.dedup_key`, or update the one
    /// already open for it. Returns the resulting row and whether this
    /// call opened it (`true`) or updated an existing open incident
    /// (`false`) - the caller uses that to decide whether to enqueue an
    /// "opened" or "updated" outbox entry.
    ///
    /// Idempotent under retry: calling this twice with an identical
    /// candidate while the first call's incident is still open updates the
    /// same row in place (same `updated_at` effect, no duplicate incident)
    /// rather than creating a second one - the partial unique index on
    /// `(dedup_key) WHERE resolved_at IS NULL` is what enforces this.
    pub async fn open_or_update_incident(
        &self,
        candidate: IncidentCandidate,
    ) -> Result<(Incident, bool)> {
        let now = OffsetDateTime::now_utc();
        let row: Row = sqlx::query_as(
            "INSERT INTO incidents
                (dedup_key, rule_id, rule_version, scope, severity, state,
                 checkpoint_sequence, valuation_basis, measured_trigger, evidence,
                 opened_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'opened', $6, $7, $8, $9, $10, $10)
             ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO UPDATE SET
                severity = EXCLUDED.severity,
                checkpoint_sequence = EXCLUDED.checkpoint_sequence,
                measured_trigger = EXCLUDED.measured_trigger,
                evidence = EXCLUDED.evidence,
                state = 'updated',
                updated_at = EXCLUDED.updated_at
             RETURNING id, dedup_key, rule_id, rule_version, scope, severity, state,
                       checkpoint_sequence, valuation_basis, measured_trigger, evidence,
                       opened_at, updated_at, resolved_at",
        )
        .bind(&candidate.dedup_key)
        .bind(&candidate.rule_id)
        .bind(candidate.rule_version)
        .bind(&candidate.scope)
        .bind(&candidate.severity)
        .bind(candidate.checkpoint_sequence)
        .bind(&candidate.valuation_basis)
        .bind(&candidate.measured_trigger)
        .bind(&candidate.evidence)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .context("opening or updating incident")?;
        let opened = row.state == "opened";
        Ok((row.try_into()?, opened))
    }

    /// Resolve the open incident for `dedup_key`, if any. `Ok(None)` means
    /// there was nothing open for that key - not an error (a rule
    /// resolving a condition that was already resolved, or never opened,
    /// is the normal steady state).
    pub async fn resolve_incident(&self, dedup_key: &str) -> Result<Option<Incident>> {
        let now = OffsetDateTime::now_utc();
        let row: Option<Row> = sqlx::query_as(
            "UPDATE incidents SET state = 'resolved', updated_at = $2, resolved_at = $2
             WHERE dedup_key = $1 AND resolved_at IS NULL
             RETURNING id, dedup_key, rule_id, rule_version, scope, severity, state,
                       checkpoint_sequence, valuation_basis, measured_trigger, evidence,
                       opened_at, updated_at, resolved_at",
        )
        .bind(dedup_key)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .context("resolving incident")?;
        row.map(TryInto::try_into).transpose()
    }

    /// Bounded, most-recent-first read (temp.md: "bounded incident ...
    /// reads"). `limit` is clamped to 500 regardless of the caller's
    /// request, so a future query service cannot accidentally stream an
    /// unbounded incident history into one response.
    pub async fn list_incidents(&self, limit: i64) -> Result<Vec<Incident>> {
        let limit = limit.clamp(1, 500);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, dedup_key, rule_id, rule_version, scope, severity, state,
                    checkpoint_sequence, valuation_basis, measured_trigger, evidence,
                    opened_at, updated_at, resolved_at
             FROM incidents ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("listing incidents")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
}

#[cfg(test)]
mod tests;
