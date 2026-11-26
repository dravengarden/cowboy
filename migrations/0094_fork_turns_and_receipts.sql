CREATE TABLE IF NOT EXISTS session_turn_completions (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    context_generation BIGINT NOT NULL DEFAULT 0,
    message_id TEXT NOT NULL,
    before_seq BIGINT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, context_generation, message_id)
);

CREATE INDEX IF NOT EXISTS session_turn_completions_latest_idx
    ON session_turn_completions (session_id, context_generation, before_seq DESC);

CREATE TABLE IF NOT EXISTS session_duplicate_receipts (
    owner_user_id TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    destination_session_id TEXT NOT NULL,
    before_seq BIGINT NOT NULL DEFAULT 0,
    workspace_base TEXT,
    workspace_include_uncommitted BOOLEAN,
    state TEXT NOT NULL DEFAULT 'ready',
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (owner_user_id, source_session_id, operation_id)
);

CREATE INDEX IF NOT EXISTS session_duplicate_receipts_dest_idx
    ON session_duplicate_receipts (destination_session_id);
