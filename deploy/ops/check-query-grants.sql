-- Run as migration owner AFTER grants.sql on an isolated database. Rolls back all data.
\set ON_ERROR_STOP on
BEGIN;
SET LOCAL ROLE sentinel_monitor;
INSERT INTO incidents(dedup_key,rule_id,rule_version,scope,severity,state,valuation_basis,measured_trigger,evidence,opened_at,updated_at)
VALUES('query-grants-check','test',1,'query-grants-check','info','opened','test','{}','{}',now(),now());
SET LOCAL ROLE sentinel_reader;
DO $$ BEGIN
  IF NOT EXISTS(SELECT 1 FROM query_scope_versions WHERE scope='query-grants-check' AND revision=1) THEN
    RAISE EXCEPTION 'monitor trigger did not update generation';
  END IF;
  BEGIN
    UPDATE incidents SET state='acknowledged' WHERE scope='query-grants-check';
    RAISE EXCEPTION 'reader unexpectedly has mutation permission';
  EXCEPTION WHEN insufficient_privilege THEN NULL;
  END;
END $$;
SET LOCAL ROLE sentinel_operator;
UPDATE incidents SET state='acknowledged' WHERE scope='query-grants-check';
SET LOCAL ROLE sentinel_reader;
DO $$ BEGIN
  IF NOT EXISTS(SELECT 1 FROM query_scope_versions WHERE scope='query-grants-check' AND revision=2) THEN
    RAISE EXCEPTION 'operator trigger did not update generation';
  END IF;
  IF NOT EXISTS(SELECT 1 FROM incident_changes WHERE scope='query-grants-check' AND revision=2 AND payload->>'state'='acknowledged') THEN
    RAISE EXCEPTION 'operator trigger did not record immutable transition';
  END IF;
  BEGIN
    DELETE FROM incident_changes WHERE scope='query-grants-check';
    RAISE EXCEPTION 'reader unexpectedly has journal mutation permission';
  EXCEPTION WHEN insufficient_privilege THEN NULL;
  END;
END $$;
ROLLBACK;
