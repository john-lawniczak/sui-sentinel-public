-- Incident lifecycle and notification outbox (temp.md task O04).
--
-- `dedup_key` identifies the *thing* a rule is about (e.g.
-- "snapshot_freshness" or "capture_failure" for a given scope) - stable
-- across an incident's whole open/resolved lifecycle, so a later diagnosis
-- update finds the same row rather than creating a duplicate. `state`
-- tracks where that incident currently is in its lifecycle.
--
-- Additive-first migration (temp.md section 8: "Use additive/backwards-
-- compatible database migrations first"): this is the initial schema, so
-- there is nothing to migrate from yet, but every column here is intended
-- to be extended by later migrations rather than altered destructively.

CREATE TABLE incidents (
    id BIGSERIAL PRIMARY KEY,
    dedup_key TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    rule_version INT NOT NULL,
    scope TEXT NOT NULL,
    severity TEXT NOT NULL,
    state TEXT NOT NULL,
    checkpoint_sequence BIGINT,
    valuation_basis TEXT NOT NULL,
    measured_trigger JSONB NOT NULL,
    evidence JSONB NOT NULL,
    opened_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    resolved_at TIMESTAMPTZ
);

-- At most one OPEN (not yet resolved) incident per dedup_key. A partial
-- unique index (rather than a plain UNIQUE(dedup_key)) is what lets the
-- same dedup_key be reused by a later, separate incident after the first
-- one resolves - the full history is retained, not overwritten.
CREATE UNIQUE INDEX incidents_open_dedup_key
    ON incidents (dedup_key)
    WHERE resolved_at IS NULL;

CREATE INDEX incidents_scope_severity ON incidents (scope, severity);

CREATE TABLE outbox (
    id BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents (id),
    -- Stable across retries of the SAME transition, so a network retry
    -- that duplicates the enqueue call cannot duplicate the delivery
    -- record a downstream receiver would dedup on.
    dedup_id TEXT NOT NULL UNIQUE,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    delivered_at TIMESTAMPTZ,
    attempts INT NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    last_error TEXT
);

CREATE INDEX outbox_undelivered ON outbox (created_at) WHERE delivered_at IS NULL;
