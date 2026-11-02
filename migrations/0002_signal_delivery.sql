-- Each rule/scope serializes transitions; signal receipt, incident and outbox commit together.
CREATE TABLE diagnosis_signals (
    rule_key TEXT NOT NULL,
    signal_id TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    payload_hash TEXT NOT NULL,
    PRIMARY KEY (rule_key, signal_id)
);
CREATE TABLE diagnosis_cursors (
    rule_key TEXT PRIMARY KEY,
    observed_at TIMESTAMPTZ NOT NULL
);
ALTER TABLE outbox ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE outbox ADD COLUMN lease_until TIMESTAMPTZ;
ALTER TABLE outbox ADD COLUMN lease_token BIGINT NOT NULL DEFAULT 0;
ALTER TABLE outbox ADD COLUMN dead_lettered_at TIMESTAMPTZ;
CREATE INDEX outbox_due ON outbox (next_attempt_at, id)
    WHERE delivered_at IS NULL AND dead_lettered_at IS NULL;
