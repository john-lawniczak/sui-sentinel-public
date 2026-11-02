-- Cached Suilend math does not read the Clock. Preserve evaluation timestamps;
-- advancing observation must never masquerade as freshly recomputed valuation.
DELETE FROM position_dependencies d USING chain_streams s
WHERE d.scope=s.scope AND s.adapter='suilend.cached.v1'
AND d.dependency='object:0x0000000000000000000000000000000000000000000000000000000000000006';
CREATE INDEX incident_scope_page ON incidents(scope,id);
CREATE INDEX outbox_scope_history ON outbox(incident_id,id);
