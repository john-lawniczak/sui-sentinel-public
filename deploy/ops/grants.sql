-- Run after migrations as migration owner. Reapply after additive migrations.
\set ON_ERROR_STOP on
GRANT SELECT ON _sqlx_migrations TO sentinel_monitor,sentinel_delivery,sentinel_reader,sentinel_operator;
GRANT SELECT,INSERT,UPDATE ON incidents,diagnosis_signals,diagnosis_cursors,rule_windows TO sentinel_monitor;
GRANT SELECT,INSERT ON outbox TO sentinel_monitor;
GRANT SELECT ON operator_audit TO sentinel_monitor;
GRANT USAGE ON SEQUENCE incidents_id_seq,outbox_id_seq TO sentinel_monitor;
GRANT SELECT ON incidents,outbox TO sentinel_delivery;
GRANT UPDATE ON outbox TO sentinel_delivery;
GRANT SELECT ON incidents,outbox,operator_audit,chain_streams TO sentinel_reader;
GRANT SELECT ON incidents,outbox,operator_audit TO sentinel_operator;
GRANT UPDATE(state) ON incidents TO sentinel_operator;
GRANT UPDATE(dead_lettered_at,attempts,next_attempt_at,lease_until,lease_token,last_error) ON outbox TO sentinel_operator;
GRANT INSERT ON operator_audit TO sentinel_operator;
GRANT USAGE ON SEQUENCE operator_audit_id_seq TO sentinel_operator;
-- MCP/query identity: read-only tables, no migration or notification mutation rights.
GRANT SELECT ON position_evaluations,chain_frames,query_scope_versions,incident_changes TO sentinel_reader;
