-- Wrapping is temporary absence; deletion destroys the object ID permanently.
ALTER TABLE chain_objects ADD COLUMN permanently_deleted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE chain_streams ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE chain_streams ADD COLUMN checkpoint_time TIMESTAMPTZ;
ALTER TABLE chain_streams ADD COLUMN reconciled_checkpoint BIGINT;
