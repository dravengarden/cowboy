-- Observed subagent transcripts (docs/subagents.md#observed-transcripts). Each
-- ACP child session of a parent session has its own append-only timeline of
-- the updates its worker observed passively; streamed chunks of one message
-- are coalesced into one row. `(worker_epoch, runtime_seq)` is the newest
-- runtime event folded into the row and fences replays after a restart.
-- Presentation only; older Controllers ignore the table.
CREATE TABLE IF NOT EXISTS subagent_events (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    child_id TEXT NOT NULL CHECK (length(child_id) BETWEEN 1 AND 300),
    seq BIGINT NOT NULL CHECK (seq >= 1),
    ts_ms BIGINT NOT NULL,
    worker_epoch TEXT NOT NULL,
    runtime_seq BIGINT NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (session_id, child_id, seq)
);

CREATE INDEX IF NOT EXISTS subagent_events_replay_idx
    ON subagent_events (session_id, worker_epoch, runtime_seq);
