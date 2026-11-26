-- Durable completed-turn cursor index for cold-history native forks and
-- durable reservation/idempotency receipts for Linked Fork (duplicate) operations.
CREATE TABLE session_turn_completions (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    context_generation BIGINT NOT NULL DEFAULT 0,
    message_id TEXT NOT NULL,
    before_seq BIGINT NOT NULL,
    completed_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, context_generation, message_id)
);

CREATE INDEX idx_session_turn_completions_latest
    ON session_turn_completions (session_id, context_generation, before_seq DESC);

CREATE TABLE session_duplicate_receipts (
    owner_user_id TEXT NOT NULL DEFAULT '',
    source_session_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    destination_session_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'reserved',
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_user_id, source_session_id, operation_id)
);

CREATE INDEX idx_session_duplicate_receipts_source
    ON session_duplicate_receipts (source_session_id);

CREATE INDEX idx_session_duplicate_receipts_dest
    ON session_duplicate_receipts (destination_session_id);
