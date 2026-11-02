-- Per-scope revisions are serialized by migration 7's row update and committed
-- with the incident. Unlike a global sequence, they cannot commit out of order.
CREATE TABLE incident_changes (
  scope TEXT NOT NULL,
  revision BIGINT NOT NULL,
  event_kind TEXT NOT NULL,
  payload JSONB NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  PRIMARY KEY(scope,revision)
);
-- Existing history is unknown: explicitly require a current-state bootstrap.
INSERT INTO incident_changes(scope,revision,event_kind,payload)
SELECT scope,revision,'bootstrap_required','{}' FROM query_scope_versions;
CREATE FUNCTION record_incident_query_change() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE item jsonb;
BEGIN
  IF TG_OP='DELETE' THEN item=to_jsonb(OLD); ELSE item=to_jsonb(NEW); END IF;
  item=jsonb_build_object('incident_id',item->>'id','rule_id',item->'rule_id',
    'rule_version',item->'rule_version','state',item->'state','severity',item->'severity',
    'checkpoint',item->>'checkpoint_sequence','valuation_basis',item->'valuation_basis',
    'updated_at',item->'updated_at',
    'measured_trigger',CASE WHEN octet_length((item->'measured_trigger')::text)<=32768
      THEN item->'measured_trigger' ELSE '{"error":"evidence_too_large"}'::jsonb END,
    'evidence',CASE WHEN octet_length((item->'evidence')::text)<=32768
      THEN item->'evidence' ELSE '{"error":"evidence_too_large"}'::jsonb END);
  IF TG_OP='DELETE' THEN
    INSERT INTO public.incident_changes(scope,revision,event_kind,payload)
    SELECT OLD.scope,revision,'deleted',item FROM public.query_scope_versions WHERE scope=OLD.scope;
  ELSE
    INSERT INTO public.incident_changes(scope,revision,event_kind,payload)
    SELECT NEW.scope,revision,'upsert',item FROM public.query_scope_versions WHERE scope=NEW.scope;
    IF TG_OP='UPDATE' AND OLD.scope<>NEW.scope THEN
      INSERT INTO public.incident_changes(scope,revision,event_kind,payload)
      SELECT OLD.scope,revision,'removed_from_scope',jsonb_build_object('incident_id',OLD.id::text)
      FROM public.query_scope_versions WHERE scope=OLD.scope;
    END IF;
  END IF;
  RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION record_incident_query_change() FROM PUBLIC;
-- PostgreSQL fires same-kind triggers alphabetically: generation precedes history.
CREATE TRIGGER incident_query_history AFTER INSERT OR UPDATE OR DELETE ON incidents
FOR EACH ROW EXECUTE FUNCTION record_incident_query_change();
