-- Indexed turn-level cache telemetry projection surviving transcript Clear and
-- short-lived session purge (retained independently by occurred_at_ms).
CREATE TABLE IF NOT EXISTS turn_cache_facts (
    session_id TEXT NOT NULL,
    session_created_at_ms INTEGER NOT NULL DEFAULT 0,
    turn_end_seq INTEGER NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    owner_user_id TEXT,
    provider TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    provider_version TEXT NOT NULL DEFAULT '',
    provider_generation_digest TEXT NOT NULL DEFAULT '',
    model TEXT,
    stop_reason TEXT NOT NULL,
    prompt_origin TEXT NOT NULL DEFAULT 'unknown',
    prev_turn_end_ms INTEGER,
    idle_gap_ms INTEGER,
    renewed INTEGER NOT NULL DEFAULT 0,
    compacted INTEGER NOT NULL DEFAULT 0,
    usage_observed INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER,
    inclusive_input_tokens INTEGER,
    cached_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    other_uncached_input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    accounting_source TEXT NOT NULL DEFAULT 'turn_end',
    PRIMARY KEY (session_id, session_created_at_ms, turn_end_seq)
);

CREATE INDEX IF NOT EXISTS turn_cache_facts_account_time_idx
    ON turn_cache_facts (owner_user_id, provider, provider_account_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_provider_time_idx
    ON turn_cache_facts (provider, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_session_time_idx
    ON turn_cache_facts (session_id, session_created_at_ms, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS turn_cache_facts_occurred_at_idx
    ON turn_cache_facts (occurred_at_ms);

CREATE TABLE IF NOT EXISTS turn_cache_backfill_state (
    slot INTEGER PRIMARY KEY CHECK (slot = 1),
    cursor_session_id TEXT NOT NULL DEFAULT '',
    cursor_seq INTEGER NOT NULL DEFAULT 0,
    completed_at_ms INTEGER,
    oldest_backfilled_ms INTEGER,
    rows_backfilled INTEGER NOT NULL DEFAULT 0,
    updated_at_ms INTEGER NOT NULL DEFAULT 0
);
