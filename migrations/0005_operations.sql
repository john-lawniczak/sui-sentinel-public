CREATE TABLE rule_windows (
    rule_key TEXT PRIMARY KEY,
    condition TEXT NOT NULL,
    since_at TIMESTAMPTZ NOT NULL,
    last_notified_at TIMESTAMPTZ
);
CREATE TABLE operator_audit (
    id BIGSERIAL PRIMARY KEY,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor TEXT NOT NULL DEFAULT current_user,
    scope TEXT NOT NULL,
    action TEXT NOT NULL,
    target_id BIGINT NOT NULL,
    reason TEXT NOT NULL,
    evidence JSONB NOT NULL
);
CREATE INDEX outbox_scope_operations ON outbox(incident_id,created_at) WHERE delivered_at IS NULL;
