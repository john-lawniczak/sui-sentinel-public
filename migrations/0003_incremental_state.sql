CREATE TABLE chain_streams (
    scope TEXT PRIMARY KEY,
    adapter TEXT NOT NULL,
    configuration_hash TEXT NOT NULL,
    chain_identifier TEXT NOT NULL,
    checkpoint BIGINT NOT NULL,
    digest TEXT NOT NULL,
    evaluated_checkpoint BIGINT NOT NULL
);
CREATE TABLE chain_frames (
    scope TEXT NOT NULL REFERENCES chain_streams(scope),
    checkpoint BIGINT NOT NULL,
    payload_hash TEXT NOT NULL,
    raw_frame JSONB NOT NULL,
    PRIMARY KEY(scope,checkpoint)
);
CREATE TABLE chain_objects (
    scope TEXT NOT NULL REFERENCES chain_streams(scope),
    object_id TEXT NOT NULL,
    version BIGINT NOT NULL,
    body JSONB,
    kind TEXT NOT NULL,
    PRIMARY KEY(scope,object_id)
);
CREATE TABLE position_dependencies (
    scope TEXT NOT NULL,
    position_id TEXT NOT NULL,
    dependency TEXT NOT NULL,
    PRIMARY KEY(scope,position_id,dependency)
);
CREATE INDEX dependencies_reverse ON position_dependencies(scope,dependency,position_id);
CREATE TABLE position_evaluations (
    scope TEXT NOT NULL,
    position_id TEXT NOT NULL,
    checkpoint BIGINT NOT NULL,
    evaluation JSONB NOT NULL,
    PRIMARY KEY(scope,position_id)
);
ALTER TABLE chain_objects ADD COLUMN subject TEXT;
CREATE INDEX capabilities_subject ON chain_objects(scope,subject) WHERE kind='capability';
