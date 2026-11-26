-- Durable session lineage edges and decoupled archived history segments.
-- Mirrors PostgreSQL migration 0093_session_lineage.sql.
CREATE TABLE archived_lineage_segments (
    segment_id TEXT PRIMARY KEY,
    source_session_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL,
    provider TEXT NOT NULL,
    before_seq BIGINT NOT NULL,
    reason TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_archived_lineage_segments_source
    ON archived_lineage_segments (source_session_id);

CREATE TABLE archived_lineage_events (
    segment_id TEXT NOT NULL
        REFERENCES archived_lineage_segments(segment_id) ON DELETE CASCADE,
    seq BIGINT NOT NULL,
    ts_ms BIGINT NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    PRIMARY KEY (segment_id, seq)
) WITHOUT ROWID;

CREATE TABLE session_lineage_edges (
    child_session_id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    before_seq BIGINT NOT NULL,
    source_title TEXT NOT NULL DEFAULT '',
    source_provider TEXT NOT NULL DEFAULT '',
    segment_id TEXT
        REFERENCES archived_lineage_segments(segment_id) ON DELETE SET NULL,
    display_detached BOOLEAN NOT NULL DEFAULT FALSE,
    edge_version BIGINT NOT NULL DEFAULT 1,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_session_lineage_edges_parent
    ON session_lineage_edges (parent_session_id);

CREATE INDEX idx_session_lineage_edges_segment
    ON session_lineage_edges (segment_id)
    WHERE segment_id IS NOT NULL;
