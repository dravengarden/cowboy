-- Durable session lineage edges and decoupled archived history segments.
--
-- Archived segments hold immutable event prefixes copied on Clear or Trash
-- purge so child sessions can render inherited history even after an ancestor
-- clears its transcript or ages out of Trash, without keeping expired parent
-- session rows (or retired provider auth generations) alive.
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
    payload JSONB NOT NULL,
    PRIMARY KEY (segment_id, seq)
);

-- Deliberately has no ON DELETE CASCADE from sessions(id): if intermediate B
-- in A -> B -> C is purged while C survives, B's edge to A stays reachable
-- until C is purged and lineage GC removes unreachable edges and segments.
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
