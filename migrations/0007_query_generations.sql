-- MAX(updated_at) is not a generation: a late signal can change an older row.
-- Transactional revisions invalidate query pages on every incident mutation.
CREATE TABLE query_scope_versions(scope TEXT PRIMARY KEY, revision BIGINT NOT NULL);
INSERT INTO query_scope_versions SELECT DISTINCT scope,1 FROM incidents;
CREATE FUNCTION bump_query_scope_version() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
  IF TG_OP='DELETE' THEN
    INSERT INTO public.query_scope_versions VALUES(OLD.scope,1)
    ON CONFLICT(scope) DO UPDATE SET revision=public.query_scope_versions.revision+1;
  ELSE
    INSERT INTO public.query_scope_versions VALUES(NEW.scope,1)
    ON CONFLICT(scope) DO UPDATE SET revision=public.query_scope_versions.revision+1;
    IF TG_OP='UPDATE' AND OLD.scope<>NEW.scope THEN
      INSERT INTO public.query_scope_versions VALUES(OLD.scope,1)
      ON CONFLICT(scope) DO UPDATE SET revision=public.query_scope_versions.revision+1;
    END IF;
  END IF;
  RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION bump_query_scope_version() FROM PUBLIC;
CREATE TRIGGER incident_query_generation AFTER INSERT OR UPDATE OR DELETE ON incidents
FOR EACH ROW EXECUTE FUNCTION bump_query_scope_version();
