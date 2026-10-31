-- Durable transcript anchors (docs/background-tasks.md#durable-anchors). Each
-- subagent child (`subagent:<child>`) and accepted Call (`call:<call_id>`) owns
-- exactly one transcript row; every Controller lifetime and both reducers
-- resolve its position here. `backfill:v1` (seq 0) marks a session whose rows
-- from before this table were scanned once. Older Controllers ignore it.
CREATE TABLE IF NOT EXISTS transcript_anchors (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    anchor_key TEXT NOT NULL CHECK (length(anchor_key) BETWEEN 1 AND 300),
    seq BIGINT NOT NULL CHECK (seq >= 0),
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, anchor_key)
);
