-- Event store schema for cqrs-es / postgres-es.
-- Idempotent: safe to run repeatedly (used by App::run_migrations).

-- Single table holding every event across all aggregate types.
-- `global_seq` is OURS (not written by postgres-es, whose INSERT lists a fixed
-- column set): an IDENTITY column giving a strict global commit order, needed to
-- reconstruct cross-aggregate ordering (e.g. match completion order) on replay.
CREATE TABLE IF NOT EXISTS events
(
    aggregate_type text                         NOT NULL,
    aggregate_id   text                         NOT NULL,
    sequence       bigint CHECK (sequence >= 0) NOT NULL,
    event_type     text                         NOT NULL,
    event_version  text                         NOT NULL,
    payload        json                         NOT NULL,
    metadata       json                         NOT NULL,
    global_seq     bigint GENERATED ALWAYS AS IDENTITY,
    PRIMARY KEY (aggregate_type, aggregate_id, sequence)
);

-- Add the column on databases created before it existed (backfills existing rows).
ALTER TABLE events ADD COLUMN IF NOT EXISTS global_seq bigint GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS idx_events_global_seq ON events (global_seq);

-- Only needed if snapshotting is enabled (not currently used).
CREATE TABLE IF NOT EXISTS snapshots
(
    aggregate_type   text                                 NOT NULL,
    aggregate_id     text                                 NOT NULL,
    last_sequence    bigint CHECK (last_sequence >= 0)    NOT NULL,
    current_snapshot bigint CHECK (current_snapshot >= 0) NOT NULL,
    payload          json                                 NOT NULL,
    PRIMARY KEY (aggregate_type, aggregate_id, last_sequence)
);
